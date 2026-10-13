// Where the synced library files are kept. Each kind of storage is a `SyncStorage`; the
// function only talks to that interface, so moving the files elsewhere means adding a
// driver here and choosing it in `loadStorage`, with no change to the app. A driver
// hands out signed links, so the files themselves never pass through the function.

import { AwsClient } from 'npm:aws4fetch@1.0.20';

/** An upload in parts: one signed link per part, in order. */
export interface Upload {
  uploadId: string;
  urls: string[];
}

export type Finished = { kind: 'stored'; etag: string | null } | { kind: 'conflict' };

export interface SyncStorage {
  /** A signed link to download the file at `key` (the app may add `if-none-match`). */
  download(key: string): Promise<string>;
  /** Starts an upload of the file at `key` in `parts` parts. */
  startUpload(key: string, parts: number): Promise<Upload>;
  /**
   * Puts the uploaded parts (their ETags, in order) together as the file, only if the
   * file is still the one the device merged: ETag `expected`, or none at all when null.
   * Otherwise the upload is dropped and the answer is `conflict`.
   */
  finishUpload(key: string, uploadId: string, parts: string[], expected: string | null): Promise<Finished>;
  abortUpload(key: string, uploadId: string): Promise<void>;
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

/** How long a link works. The storage checks it when a request starts. */
export const LINK_SECONDS = 60 * 60;

const xmlValue = (xml: string, tag: string) =>
  new RegExp(`<${tag}>([^<]*)</${tag}>`).exec(xml)?.[1]?.replaceAll('&quot;', '"').replaceAll('&#34;', '"') ?? null;

const escapeXml = (s: string) => s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;');

/** S3-compatible storage (RustFS, MinIO, AWS S3…): presigned links, multipart uploads. */
export function s3Storage(s: S3Settings): SyncStorage {
  const aws = new AwsClient({
    accessKeyId: s.accessKey,
    secretAccessKey: s.secretKey,
    region: s.region,
    service: 's3'
  });
  const objectUrl = (key: string, query = '') =>
    `${s.endpoint}/${encodeURIComponent(s.bucket)}/${key.split('/').map(encodeURIComponent).join('/')}${query}`;
  const presign = async (method: string, url: string) => {
    const u = new URL(url);
    u.searchParams.set('X-Amz-Expires', String(LINK_SECONDS));
    return (await aws.sign(u.toString(), { method, aws: { signQuery: true } })).url;
  };
  const fail = async (res: Response): Promise<never> => {
    throw new StorageError(res.status, xmlValue(await res.text().catch(() => ''), 'Code') ?? '');
  };
  const abortUpload = async (key: string, uploadId: string) => {
    const res = await aws.fetch(objectUrl(key, `?uploadId=${encodeURIComponent(uploadId)}`), { method: 'DELETE' });
    await res.body?.cancel();
  };

  return {
    download: key => presign('GET', objectUrl(key)),

    async startUpload(key, parts) {
      const res = await aws.fetch(objectUrl(key, '?uploads'), {
        method: 'POST',
        headers: { 'content-type': 'application/gzip' }
      });
      if (res.status !== 200) await fail(res);
      const uploadId = xmlValue(await res.text(), 'UploadId');
      if (!uploadId) throw new StorageError(res.status, 'NoUploadId');
      const id = encodeURIComponent(uploadId);
      const urls = await Promise.all(
        Array.from({ length: parts }, (_, i) => presign('PUT', objectUrl(key, `?partNumber=${i + 1}&uploadId=${id}`)))
      );
      return { uploadId, urls };
    },

    async finishUpload(key, uploadId, parts, expected) {
      const xml = `<CompleteMultipartUpload>${parts
        .map((etag, i) => `<Part><PartNumber>${i + 1}</PartNumber><ETag>${escapeXml(etag)}</ETag></Part>`)
        .join('')}</CompleteMultipartUpload>`;
      const res = await aws.fetch(objectUrl(key, `?uploadId=${encodeURIComponent(uploadId)}`), {
        method: 'POST',
        body: xml,
        headers: expected ? { 'if-match': expected } : { 'if-none-match': '*' }
      });
      const text = await res.text();
      // Some servers answer 200 with the error in the body.
      const code = xmlValue(text, 'Code');
      if (res.status === 412 || res.status === 409 || code === 'PreconditionFailed') {
        await abortUpload(key, uploadId).catch(console.error);
        return { kind: 'conflict' };
      }
      if (res.status !== 200 || code) throw new StorageError(res.status, code ?? '');
      return { kind: 'stored', etag: xmlValue(text, 'ETag') ?? res.headers.get('etag') };
    },

    abortUpload
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
