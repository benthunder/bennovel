import { invoke } from '@tauri-apps/api/core';
import { SyncFailure, syncFailure } from './sync';

/*
 * Settings for syncing through the user's own S3-compatible storage (MinIO, RustFS…)
 * instead of the BenNovel server; the sync itself is syncLibrary (sync.ts). The native
 * app does all of it (src-tauri/src/library/s3.rs): the keys are kept on this device
 * beside the library, never in the synced file, and the secret key never comes back
 * to this side.
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

/** A failure while checking the storage: a network error means this server. */
function failure(error: unknown): SyncFailure {
  const f = syncFailure(error);
  return f.key === 'sync.err.network' ? new SyncFailure('s3.err.network', error) : f;
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

/** `minio.example.com` from the endpoint, to name the storage. */
export const s3Host = (s: S3Settings) => s.endpoint.replace(/^\w+:\/\//, '');
