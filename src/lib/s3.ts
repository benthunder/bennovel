import { invoke } from '@tauri-apps/api/core';
import type { MessageKey } from '../i18n/en';
import { SyncFailure } from './sync';

/*
 * Sync through the user's own S3-compatible storage (MinIO, RustFS…) instead of the
 * account's Supabase Storage. The native app does all of it (src-tauri/src/library/s3.rs):
 * the keys are kept on this device beside the library, never in the synced file, and
 * the secret key never comes back to this side.
 */

/** The storage set for a library, as shown (no secret key). */
export interface S3Settings {
  endpoint: string;
  region: string;
  bucket: string;
  folder: string;
  accessKey: string;
  hasSecret: boolean;
}

/** Settings to save; an empty secret key keeps the saved one. */
export interface S3Config {
  endpoint: string;
  region: string;
  bucket: string;
  folder: string;
  accessKey: string;
  secretKey: string;
}

/** What the native app's error says went wrong, as a message to show. */
function failure(error: unknown): SyncFailure {
  console.error(error);
  const text = String((error as { message?: string } | null)?.message ?? error);
  const key: MessageKey =
    text.includes('sync-outdated') ? 'sync.err.outdated'
      : text.includes('s3-endpoint') ? 's3.err.endpoint'
        : text.includes('s3-bucket') ? 's3.err.bucket'
          : text.includes('s3-keys') || text.includes('s3-auth') ? 's3.err.keys'
            : text.includes('s3-network') ? 's3.err.network'
              : text.includes('s3-too-large') ? 'sync.err.tooLarge'
                : text.includes('s3-busy') ? 'sync.err.busy'
                  : 'sync.err.generic';
  return new SyncFailure(key, error);
}

export const getS3Settings = (user: string) => invoke<S3Settings | null>('s3_config_get', { user });

/** Saves the storage after checking it answers with these keys; null stops using it. */
export async function setS3Config(user: string, config: S3Config | null): Promise<void> {
  try {
    await invoke('s3_config_set', { user, config });
  } catch (e) {
    throw failure(e);
  }
}

/** Syncs the library of `user` with its storage. Returns the time of the sync (epoch ms). */
export async function syncS3(user: string): Promise<number> {
  try {
    return await invoke<number>('s3_sync', { user });
  } catch (e) {
    throw failure(e);
  }
}

/** `minio.example.com` from the endpoint, to name the storage. */
export const s3Host = (s: S3Settings) => s.endpoint.replace(/^\w+:\/\//, '');
