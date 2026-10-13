// Where the synced library files are kept. Each kind of storage is a `SyncStorage`; the
// function only talks to that interface, so moving the files elsewhere means adding a
// driver here and choosing it in `loadStorage`, with no change to the app.

import { AwsClient } from 'npm:aws4fetch@1.0.20';

export type Fetched =
  | { kind: 'unchanged' }
  | { kind: 'missing' }
  | { kind: 'file'; body: Uint8Array<ArrayBuffer>; etag: string | null };

export type Stored = { kind: 'stored'; etag: string | null } | { kind: 'conflict' };

export interface SyncStorage {
  /** The file at `key`; `unchanged` when its ETag is still `known`. */
  get(key: string, known: string | null): Promise<Fetched>;
  /**
   * Writes the file only if it is still the one the device merged: ETag `expected`, or
   * no file at all when `expected` is null. Otherwise `conflict`.
   */
  put(key: string, body: Uint8Array<ArrayBuffer>, expected: string | null): Promise<Stored>;
}

/** The storage answered with an error. */
export class StorageError extends Error {
  constructor(readonly status: number, readonly code: string) {
    super(`storage ${status} ${code}`);
  }
}

export interface S3Settings {
  /** `https://host[:port]`, path-style requests. */
  endpoint: string;
  region: string;
  bucket: string;
  accessKey: string;
  secretKey: string;
}

/** The `<Code>` of an S3 error answer. */
async function errorCode(res: Response): Promise<string> {
  const text = await res.text().catch(() => '');
  return /<Code>([^<]*)<\/Code>/.exec(text)?.[1] ?? '';
}

/** S3-compatible storage (RustFS, MinIO, AWS S3…), using conditional writes. */
export function s3Storage(s: S3Settings): SyncStorage {
  const aws = new AwsClient({
    accessKeyId: s.accessKey,
    secretAccessKey: s.secretKey,
    region: s.region,
    service: 's3'
  });
  const url = (key: string) =>
    `${s.endpoint}/${encodeURIComponent(s.bucket)}/${key.split('/').map(encodeURIComponent).join('/')}`;

  return {
    async get(key, known) {
      const res = await aws.fetch(url(key), { headers: known ? { 'if-none-match': known } : {} });
      if (res.status === 304) return { kind: 'unchanged' };
      if (res.status === 404) {
        await res.body?.cancel();
        return { kind: 'missing' };
      }
      if (res.status !== 200) throw new StorageError(res.status, await errorCode(res));
      return { kind: 'file', body: new Uint8Array(await res.arrayBuffer()), etag: res.headers.get('etag') };
    },
    async put(key, body, expected) {
      const res = await aws.fetch(url(key), {
        method: 'PUT',
        body,
        headers: {
          'content-type': 'application/gzip',
          ...(expected ? { 'if-match': expected } : { 'if-none-match': '*' })
        }
      });
      if (res.status === 412 || res.status === 409) {
        await res.body?.cancel();
        return { kind: 'conflict' };
      }
      if (res.status !== 200) throw new StorageError(res.status, await errorCode(res));
      await res.body?.cancel();
      return { kind: 'stored', etag: res.headers.get('etag') };
    }
  };
}

/**
 * The storage set for this project, from the function's secrets: `SYNC_STORAGE`
 * (only `s3` for now) and its `SYNC_S3_*` settings. Null when it is not set up.
 * An admin panel can later keep these settings elsewhere; only this changes.
 */
export function loadStorage(env: (name: string) => string | undefined = name => Deno.env.get(name)): SyncStorage | null {
  switch (env('SYNC_STORAGE') ?? 's3') {
    case 's3': {
      const endpoint = env('SYNC_S3_ENDPOINT')?.replace(/\/+$/, '');
      const bucket = env('SYNC_S3_BUCKET');
      const accessKey = env('SYNC_S3_ACCESS_KEY');
      const secretKey = env('SYNC_S3_SECRET_KEY');
      if (!endpoint || !bucket || !accessKey || !secretKey) return null;
      return s3Storage({ endpoint, bucket, accessKey, secretKey, region: env('SYNC_S3_REGION') || 'us-east-1' });
    }
    default:
      return null;
  }
}
