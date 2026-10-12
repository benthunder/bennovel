import { invoke } from '@tauri-apps/api/core';
import type { MessageKey } from '../i18n/en';
import { supabase } from './supabase';

/*
 * Sync between a user's devices, run from the Sync button. The whole on-device library
 * (imported books, favourites, history) is one gzipped SQLite file in Supabase Storage,
 * in the user's own folder. A sync downloads the newer file, merges it into the library
 * row by row (src-tauri/src/library/sync.rs), uploads the result and commits it with
 * `commit_sync`, which only succeeds when no other device synced in between.
 * Native app only: the web build has no library file.
 */

const BUCKET = 'user-sync';
const ATTEMPTS = 3;

export interface SyncStatus {
  /** Server version this library was last synced with (0 = never). */
  lastVersion: number;
  /** Epoch ms of the last sync. */
  syncedAt: number | null;
}

/** A failed sync, carrying the message to show. */
export class SyncFailure extends Error {
  constructor(readonly key: MessageKey, cause?: unknown) {
    super(key, { cause });
  }
}

export const getSyncStatus = (user: string) => invoke<SyncStatus>('sync_status', { user });

/** A fresh name per upload, so two devices never write the same object. */
const newObject = (uid: string) =>
  `${uid}/library-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}.sqlite.gz`;

function fail(error: unknown): never {
  console.error(error);
  if (error instanceof SyncFailure) throw error;
  const e = error as { status?: number; statusCode?: string | number; code?: string; message?: string; name?: string } | null;
  const status = Number(e?.statusCode ?? e?.status ?? 0);
  const text = String(e?.message ?? error);
  if (text.includes('sync-outdated')) throw new SyncFailure('sync.err.outdated', error);
  if (status === 413 || /maximum allowed size|too large/i.test(text)) throw new SyncFailure('sync.err.tooLarge', error);
  if (status === 401 || status === 403 || e?.code === '42501' || /jwt|not signed in/i.test(text)) throw new SyncFailure('sync.err.signIn', error);
  if (error instanceof TypeError || /failed to fetch|network/i.test(text)) throw new SyncFailure('sync.err.network', error);
  throw new SyncFailure('sync.err.generic', error);
}

/**
 * Syncs the library file of `user` (its key, the email) with account `uid`.
 * Returns the time of the sync (epoch ms).
 */
export async function syncLibrary(user: string, uid: string): Promise<number> {
  try {
    for (let attempt = 0; attempt < ATTEMPTS; attempt++) {
      const { data: server, error } = await supabase.from('user_sync').select('version, object').maybeSingle();
      if (error) fail(error);
      const version = server?.version ?? 0;

      // Take in what other devices synced since this one last did.
      const local = await getSyncStatus(user);
      if (server && version !== local.lastVersion) {
        const { data: file, error: downloadError } = await supabase.storage.from(BUCKET).download(server.object);
        if (downloadError) fail(downloadError);
        await invoke('sync_merge', new Uint8Array(await file.arrayBuffer()), { headers: { 'x-user': encodeURIComponent(user) } });
        await invoke('sync_mark', { user, version });
      }

      const bytes = await invoke<ArrayBuffer>('sync_export', { user });
      const object = newObject(uid);
      const { error: uploadError } = await supabase.storage
        .from(BUCKET)
        .upload(object, new Blob([bytes], { type: 'application/gzip' }), { contentType: 'application/gzip' });
      if (uploadError) fail(uploadError);

      const { data: committed, error: commitError } = await supabase.rpc('commit_sync', {
        expected: version, new_object: object, new_size: bytes.byteLength
      });
      if (commitError) {
        await supabase.storage.from(BUCKET).remove([object]);
        fail(commitError);
      }
      if (committed == null) {
        // Another device synced first: drop this upload and merge theirs.
        await supabase.storage.from(BUCKET).remove([object]);
        continue;
      }
      await invoke('sync_mark', { user, version: committed });
      // The folder keeps only the current file.
      if (server) await supabase.storage.from(BUCKET).remove([server.object]);
      return Date.now();
    }
  } catch (e) {
    fail(e);
  }
  throw new SyncFailure('sync.err.busy');
}
