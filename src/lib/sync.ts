import { Channel, invoke } from '@tauri-apps/api/core';
import type { MessageKey } from '../i18n/en';
import { supabase } from './supabase';

/*
 * Sync between a user's devices, run from the Sync button. The whole on-device library
 * (imported books, favourites, history) is one gzipped SQLite file. The native app does
 * the sync (src-tauri/src/library/remote.rs): it downloads the newer file, merges it row
 * by row, and uploads the result only if no other device synced in between.
 * A signed-in account syncs through the BenNovel server, the Edge Function `sync`
 * (supabase/functions/sync), which holds the storage keys and hands out signed links;
 * the app only sends its session, and the file goes straight to storage in parts. A library set to the user's own S3 storage syncs there instead (s3.ts).
 * Native app only: the web build has no library file.
 */

export interface SyncStatus {
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

/** Words in the native app's errors, and the message each one means (first match wins). */
const MESSAGES: [string, MessageKey][] = [
  ['sync-outdated', 'sync.err.outdated'],
  ['sync-sign-in', 'sync.err.signIn'],
  ['s3-endpoint', 's3.err.endpoint'],
  ['s3-bucket', 's3.err.bucket'],
  ['s3-keys', 's3.err.keys'],
  ['s3-auth', 's3.err.keys'],
  ['sync-network', 'sync.err.network'],
  ['sync-server-off', 'sync.err.serverOff'],
  ['too-large', 'sync.err.tooLarge'],
  ['sync-busy', 'sync.err.busy']
];

/** What the native app's error says went wrong, as a message to show. */
export function syncFailure(error: unknown): SyncFailure {
  console.error(error);
  if (error instanceof SyncFailure) return error;
  const text = String((error as { message?: string } | null)?.message ?? error);
  return new SyncFailure(MESSAGES.find(([word]) => text.includes(word))?.[1] ?? 'sync.err.generic', error);
}

/** The signed-in account, for the BenNovel server; null when signed out. */
async function account() {
  const { data } = await supabase.auth.getSession();
  const token = data.session?.access_token;
  if (!token) return null;
  return {
    url: `${import.meta.env.VITE_SUPABASE_URL}/functions/v1/sync`,
    apiKey: import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY,
    token
  };
}

/** Where a sync is (src-tauri/src/library/remote.rs `Step`); byte counts may have total 0 (unknown). */
export type SyncStep =
  | { step: 'download' | 'upload'; done: number; total: number }
  | { step: 'merge' | 'pack' };

/**
 * Syncs the library of `user` (its key), telling `onStep` how far it has come.
 * Returns the time of the sync (epoch ms).
 */
export async function syncLibrary(user: string, onStep?: (step: SyncStep) => void): Promise<number> {
  try {
    const onProgress = new Channel<SyncStep>(onStep);
    return await invoke<number>('library_sync', { user, account: await account(), onProgress });
  } catch (e) {
    throw syncFailure(e);
  }
}
