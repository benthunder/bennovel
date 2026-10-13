// deno test --node-modules-dir=none --allow-env --allow-net --allow-read supabase/functions/sync
// The S3 test runs against a real server when SYNC_TEST_S3_ENDPOINT, _BUCKET,
// _ACCESS_KEY and _SECRET_KEY are set; skipped otherwise.

import { assertEquals } from 'jsr:@std/assert@1.0.13';
import { type Deps, handle } from './handler.ts';
import { type SyncStorage, s3Storage } from './storage.ts';

/** Storage in memory, with numbered ETags. */
function memory(): SyncStorage & { files: Map<string, { body: Uint8Array<ArrayBuffer>; etag: string }> } {
  const files = new Map<string, { body: Uint8Array<ArrayBuffer>; etag: string }>();
  let n = 0;
  return {
    files,
    get(key, known) {
      const f = files.get(key);
      if (!f) return Promise.resolve({ kind: 'missing' });
      if (f.etag === known) return Promise.resolve({ kind: 'unchanged' });
      return Promise.resolve({ kind: 'file', body: f.body, etag: f.etag });
    },
    put(key, body, expected) {
      if ((files.get(key)?.etag ?? null) !== expected) return Promise.resolve({ kind: 'conflict' });
      const etag = `"${++n}"`;
      files.set(key, { body, etag });
      return Promise.resolve({ kind: 'stored', etag });
    }
  };
}

const deps = (storage: SyncStorage | null): Deps => ({
  userId: token => Promise.resolve(token === 'mai-token' ? 'mai' : token === 'an-token' ? 'an' : null),
  storage: () => storage
});

const call = (d: Deps, method: string, token: string | null, headers: Record<string, string> = {}, body?: string) =>
  handle(
    new Request('https://x.supabase.co/functions/v1/sync', {
      method,
      body,
      headers: { ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers }
    }),
    d
  );

Deno.test('each account reads and writes only its own file, one device at a time', async () => {
  const store = memory();
  const d = deps(store);

  assertEquals((await call(d, 'GET', 'mai-token')).status, 404);
  const first = await call(d, 'PUT', 'mai-token', {}, 'v1');
  assertEquals(first.status, 200);
  const etag = first.headers.get('x-sync-etag')!;
  assertEquals(await first.json(), { etag });
  assertEquals([...store.files.keys()], ['users/mai/library.sqlite.gz']);

  // A device that never synced cannot overwrite it; one that has the file can.
  assertEquals((await call(d, 'PUT', 'mai-token', {}, 'v2')).status, 412);
  assertEquals((await call(d, 'PUT', 'mai-token', { 'x-sync-expected': '"stale"' }, 'v2')).status, 412);
  const second = await call(d, 'PUT', 'mai-token', { 'x-sync-expected': etag }, 'v2');
  assertEquals(second.status, 200);
  const etag2 = second.headers.get('x-sync-etag')!;

  const got = await call(d, 'GET', 'mai-token', { 'x-sync-etag': etag });
  assertEquals(got.status, 200);
  assertEquals(got.headers.get('x-sync-etag'), etag2);
  assertEquals(await got.text(), 'v2');
  assertEquals((await call(d, 'GET', 'mai-token', { 'x-sync-etag': etag2 })).status, 304);

  // Another account sees nothing of it.
  assertEquals((await call(d, 'GET', 'an-token')).status, 404);
});

Deno.test('refuses requests without a valid session or storage', async () => {
  assertEquals((await call(deps(memory()), 'GET', null)).status, 401);
  assertEquals((await call(deps(memory()), 'GET', 'forged')).status, 401);
  assertEquals((await call(deps(memory()), 'DELETE', 'mai-token')).status, 405);
  const off = await call(deps(null), 'GET', 'mai-token');
  assertEquals(off.status, 503);
  assertEquals(await off.json(), { error: 'not-configured' });
  assertEquals((await call(deps(memory()), 'PUT', 'mai-token', {}, '')).status, 400);
});

const env = (name: string) => Deno.env.get(`SYNC_TEST_S3_${name}`);

Deno.test({
  name: 'S3 storage keeps the file and refuses stale writes',
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
    assertEquals(await s3.get(key, null), { kind: 'missing' });
    const first = await s3.put(key, new TextEncoder().encode('v1'), null);
    if (first.kind !== 'stored' || !first.etag) throw new Error(`not stored: ${JSON.stringify(first)}`);
    assertEquals((await s3.put(key, new TextEncoder().encode('v2'), null)).kind, 'conflict');
    assertEquals((await s3.put(key, new TextEncoder().encode('v2'), '"stale"')).kind, 'conflict');
    assertEquals(await s3.get(key, first.etag), { kind: 'unchanged' });
    const second = await s3.put(key, new TextEncoder().encode('v2'), first.etag);
    if (second.kind !== 'stored') throw new Error('not stored');
    const got = await s3.get(key, first.etag);
    if (got.kind !== 'file') throw new Error(`expected the file, got ${got.kind}`);
    assertEquals(new TextDecoder().decode(got.body), 'v2');

    // Clean up.
    const { AwsClient } = await import('npm:aws4fetch@1.0.20');
    const aws = new AwsClient({ accessKeyId: env('ACCESS_KEY')!, secretAccessKey: env('SECRET_KEY')!, region: env('REGION') ?? 'us-east-1', service: 's3' });
    const res = await aws.fetch(`${env('ENDPOINT')}/${env('BUCKET')}/${key}`, { method: 'DELETE' });
    assertEquals(res.status, 204);
  }
});
