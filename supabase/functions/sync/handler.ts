// The sync endpoint the app calls. A signed-in account keeps one file,
// `users/<uid>/library.sqlite.gz`, in the storage `loadStorage` sets up; the keys to that
// storage stay with this function, the app only sends its Supabase session.
//
//   GET  → 200 with the file, 304 when its ETag is still `x-sync-etag`, 404 when there is none.
//   PUT  → stores the body only if the file is still the one with ETag `x-sync-expected`
//          (no header: only if there is no file yet); 412 otherwise, so the device merges
//          the newer file and tries again.
// The ETag travels in `x-sync-etag`. Errors are JSON `{ error }`.

import { type SyncStorage, StorageError } from './storage.ts';

/** Snapshots are held in memory; refuse anything larger. */
export const MAX_BYTES = 100 * 1024 * 1024;

export interface Deps {
  /** The account id of a Supabase access token, null if it is not valid. */
  userId(token: string): Promise<string | null>;
  storage(): SyncStorage | null;
}

const json = (status: number, body: Record<string, unknown>, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

export async function handle(req: Request, deps: Deps): Promise<Response> {
  if (req.method !== 'GET' && req.method !== 'PUT') return json(405, { error: 'method' });
  const token = /^Bearer\s+(.+)$/i.exec(req.headers.get('authorization') ?? '')?.[1];
  const uid = token ? await deps.userId(token) : null;
  if (!uid) return json(401, { error: 'sign-in' });
  const storage = deps.storage();
  if (!storage) return json(503, { error: 'not-configured' });
  const key = `users/${uid}/library.sqlite.gz`;

  try {
    if (req.method === 'GET') {
      const got = await storage.get(key, req.headers.get('x-sync-etag'));
      if (got.kind === 'unchanged') return new Response(null, { status: 304 });
      if (got.kind === 'missing') return json(404, { error: 'missing' });
      return new Response(got.body, {
        headers: { 'content-type': 'application/gzip', ...(got.etag ? { 'x-sync-etag': got.etag } : {}) }
      });
    }

    if (Number(req.headers.get('content-length') ?? 0) > MAX_BYTES) return json(413, { error: 'too-large' });
    const body = new Uint8Array(await req.arrayBuffer());
    if (body.byteLength > MAX_BYTES) return json(413, { error: 'too-large' });
    if (body.byteLength === 0) return json(400, { error: 'empty' });
    const stored = await storage.put(key, body, req.headers.get('x-sync-expected') || null);
    if (stored.kind === 'conflict') return json(412, { error: 'conflict' });
    return json(200, { etag: stored.etag }, stored.etag ? { 'x-sync-etag': stored.etag } : {});
  } catch (e) {
    console.error(e);
    if (e instanceof StorageError) {
      if (e.status === 413 || e.code === 'EntityTooLarge') return json(413, { error: 'too-large' });
      return json(502, { error: 'storage', status: e.status, code: e.code });
    }
    return json(502, { error: 'storage' });
  }
}
