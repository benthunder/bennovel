// deno test --node-modules-dir=none --allow-env --allow-net --allow-read supabase/functions/sync
// The S3 test runs against a real server when SYNC_TEST_S3_ENDPOINT, _BUCKET,
// _ACCESS_KEY and _SECRET_KEY are set; skipped otherwise.

import { assertEquals } from 'jsr:@std/assert@1.0.13';
import { type Deps, handle, MAX_PARTS } from './handler.ts';
import { type SyncStorage, s3Storage } from './storage.ts';

/** Records what it is asked; the file "has" ETag `"1"`. */
function fake(): SyncStorage & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    download: key => Promise.resolve(`get:${key}`),
    startUpload: (key, parts) => {
      calls.push(`start ${key} ${parts}`);
      return Promise.resolve({ uploadId: 'up-1', urls: Array.from({ length: parts }, (_, i) => `put:${key}:${i + 1}`) });
    },
    finishUpload: (key, uploadId, parts, expected) => {
      calls.push(`finish ${key} ${uploadId} ${parts.join(',')} ${expected}`);
      return Promise.resolve(expected === '"1"' ? { kind: 'stored', etag: '"2"' } : { kind: 'conflict' });
    },
    abortUpload: (key, uploadId) => {
      calls.push(`abort ${key} ${uploadId}`);
      return Promise.resolve();
    }
  };
}

const deps = (storage: SyncStorage | null): Deps => ({
  userId: token => Promise.resolve(token === 'mai-token' ? 'mai' : null),
  storage: () => storage
});

async function call(d: Deps, token: string | null, body: unknown, method = 'POST') {
  const res = await handle(
    new Request('https://x.supabase.co/functions/v1/sync', {
      method,
      body: method === 'POST' ? JSON.stringify(body) : undefined,
      headers: token ? { authorization: `Bearer ${token}` } : {}
    }),
    d
  );
  return { status: res.status, body: await res.json() };
}

const KEY = 'users/mai/library.sqlite.gz';

Deno.test("links and uploads are only for the account's own file", async () => {
  const s = fake();
  const d = deps(s);
  assertEquals(await call(d, 'mai-token', { op: 'download' }), { status: 200, body: { url: `get:${KEY}` } });
  assertEquals(await call(d, 'mai-token', { op: 'upload-start', parts: 2 }), {
    status: 200,
    body: { uploadId: 'up-1', urls: [`put:${KEY}:1`, `put:${KEY}:2`] }
  });
  const finish = { op: 'upload-finish', uploadId: 'up-1', parts: ['"a"', '"b"'] };
  assertEquals(await call(d, 'mai-token', { ...finish, etag: '"1"' }), { status: 200, body: { etag: '"2"' } });
  assertEquals((await call(d, 'mai-token', { ...finish, etag: '"0"' })).status, 412);
  assertEquals((await call(d, 'mai-token', finish)).status, 412);
  assertEquals(await call(d, 'mai-token', { op: 'upload-abort', uploadId: 'up-1' }), { status: 200, body: {} });
  assertEquals(s.calls, [
    `start ${KEY} 2`,
    `finish ${KEY} up-1 "a","b" "1"`,
    `finish ${KEY} up-1 "a","b" "0"`,
    `finish ${KEY} up-1 "a","b" null`,
    `abort ${KEY} up-1`
  ]);
});

Deno.test('refuses requests without a valid session, storage or request', async () => {
  const d = deps(fake());
  assertEquals((await call(d, null, { op: 'download' })).status, 401);
  assertEquals((await call(d, 'forged', { op: 'download' })).status, 401);
  assertEquals((await call(d, 'mai-token', null, 'GET')).status, 405);
  assertEquals(await call(deps(null), 'mai-token', { op: 'download' }), { status: 503, body: { error: 'not-configured' } });
  assertEquals((await call(d, 'mai-token', { op: 'delete' })).status, 400);
  assertEquals((await call(d, 'mai-token', { op: 'upload-start', parts: 0 })).status, 400);
  assertEquals((await call(d, 'mai-token', { op: 'upload-start', parts: MAX_PARTS + 1 })).status, 413);
  assertEquals((await call(d, 'mai-token', { op: 'upload-finish', uploadId: 'up-1', parts: [] })).status, 400);
});

const env = (name: string) => Deno.env.get(`SYNC_TEST_S3_${name}`);

Deno.test({
  name: 'S3: parts uploaded through links become the file, stale uploads are refused',
  ignore: !env('ENDPOINT'),
  async fn() {
    const s3 = s3Storage({
      endpoint: env('ENDPOINT')!,
      bucket: env('BUCKET')!,
      region: env('REGION') ?? 'us-east-1',
      accessKey: env('ACCESS_KEY')!,
      secretKey: env('SECRET_KEY')!
    });
    const key = `bennovel-test/${crypto.randomUUID()}/library.sqlite.gz`;
    const MiB = 1024 * 1024;
    /** Uploads `sizes` as the parts of one file, then finishes it expecting `expected`. */
    async function upload(sizes: number[], fill: number, expected: string | null) {
      const { uploadId, urls } = await s3.startUpload(key, sizes.length);
      const etags = [];
      for (const [i, size] of sizes.entries()) {
        const res = await fetch(urls[i], { method: 'PUT', body: new Uint8Array(size).fill(fill) });
        assertEquals(res.status, 200);
        etags.push(res.headers.get('etag')!);
        await res.body?.cancel();
      }
      return s3.finishUpload(key, uploadId, etags, expected);
    }

    const missing = await fetch(await s3.download(key));
    assertEquals(missing.status, 404);
    await missing.body?.cancel();

    const first = await upload([5 * MiB, 10], 1, null);
    if (first.kind !== 'stored' || !first.etag) throw new Error(`not stored: ${JSON.stringify(first)}`);
    assertEquals((await upload([10], 2, null)).kind, 'conflict');
    assertEquals((await upload([10], 2, '"stale"')).kind, 'conflict');
    const second = await upload([7], 3, first.etag);
    if (second.kind !== 'stored') throw new Error('not stored');

    const url = await s3.download(key);
    const unchanged = await fetch(url, { headers: { 'if-none-match': second.etag! } });
    assertEquals(unchanged.status, 304);
    await unchanged.body?.cancel();
    const got = await fetch(url);
    assertEquals([got.status, got.headers.get('etag')], [200, second.etag]);
    assertEquals(new Uint8Array(await got.arrayBuffer()), new Uint8Array(7).fill(3));

    // Clean up.
    const { AwsClient } = await import('npm:aws4fetch@1.0.20');
    const aws = new AwsClient({ accessKeyId: env('ACCESS_KEY')!, secretAccessKey: env('SECRET_KEY')!, region: env('REGION') ?? 'us-east-1', service: 's3' });
    const res = await aws.fetch(`${env('ENDPOINT')}/${env('BUCKET')}/${key}`, { method: 'DELETE' });
    assertEquals(res.status, 204);
  }
});
