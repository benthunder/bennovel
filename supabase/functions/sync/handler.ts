// The sync endpoint the app calls. A signed-in account keeps one file,
// `users/<uid>/library.sqlite.gz`, in the storage `loadStorage` sets up. The keys to that
// storage stay with this function: the app sends its Supabase session and gets back
// signed links for that one file, which work for a short time. The file itself goes
// straight between the app and the storage, uploaded in parts.
//
// POST JSON, answered with JSON (errors are `{ error }`):
//   { op: "download" }                          → { url }  GET it (with `if-none-match` to
//                                                  get 304 while unchanged; 404: no file yet)
//   { op: "upload-start", parts: n }            → { uploadId, urls }  PUT part i to urls[i]
//   { op: "upload-finish", uploadId, parts: [etag…], etag?: expected }
//                                               → { etag } once the parts are the file, or
//                                                  412 when the file is no longer `expected`
//                                                  (none given: when there is one already)
//   { op: "upload-abort", uploadId }            → {}

import { type SyncStorage, StorageError } from './storage.ts';

/** Parts are at least 5 MiB (S3's floor) except the last; this caps a file at a few GB. */
export const MAX_PARTS = 500;

export interface Deps {
  /** The account id of a Supabase access token, null if it is not valid. */
  userId(token: string): Promise<string | null>;
  storage(): SyncStorage | null;
}

const json = (status: number, body: Record<string, unknown>) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

interface Body {
  op?: unknown;
  parts?: unknown;
  uploadId?: unknown;
  etag?: unknown;
}

export async function handle(req: Request, deps: Deps): Promise<Response> {
  if (req.method !== 'POST') return json(405, { error: 'method' });
  const token = /^Bearer\s+(.+)$/i.exec(req.headers.get('authorization') ?? '')?.[1];
  const uid = token ? await deps.userId(token) : null;
  if (!uid) return json(401, { error: 'sign-in' });
  const storage = deps.storage();
  if (!storage) return json(503, { error: 'not-configured' });

  const body = ((await req.json().catch(() => null)) ?? {}) as Body;
  const key = `users/${uid}/library.sqlite.gz`;
  const uploadId = typeof body.uploadId === 'string' && body.uploadId ? body.uploadId : null;
  try {
    switch (body.op) {
      case 'download':
        return json(200, { url: await storage.download(key) });
      case 'upload-start': {
        const parts = Number(body.parts);
        if (!Number.isInteger(parts) || parts < 1) return json(400, { error: 'parts' });
        if (parts > MAX_PARTS) return json(413, { error: 'too-large' });
        return json(200, { ...(await storage.startUpload(key, parts)) });
      }
      case 'upload-finish': {
        const parts = body.parts;
        if (!uploadId || !Array.isArray(parts) || !parts.length || !parts.every(p => typeof p === 'string' && p)) {
          return json(400, { error: 'parts' });
        }
        const expected = typeof body.etag === 'string' && body.etag ? body.etag : null;
        const done = await storage.finishUpload(key, uploadId, parts, expected);
        return done.kind === 'conflict' ? json(412, { error: 'conflict' }) : json(200, { etag: done.etag });
      }
      case 'upload-abort':
        if (uploadId) await storage.abortUpload(key, uploadId);
        return json(200, {});
      default:
        return json(400, { error: 'op' });
    }
  } catch (e) {
    console.error(e);
    if (e instanceof StorageError) {
      if (e.code === 'EntityTooLarge') return json(413, { error: 'too-large' });
      return json(502, { error: 'storage', status: e.status, code: e.code });
    }
    return json(502, { error: 'storage' });
  }
}
