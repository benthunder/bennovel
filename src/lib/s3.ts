import { invoke } from '@tauri-apps/api/core';
import type { MessageKey } from '../i18n/en';

/*
 * The user's own S3-compatible storage (MinIO, RustFS…) for backups of the library
 * (backup.ts). The native app does all of it (src-tauri/src/library/s3.rs): the keys
 * are kept on this device beside the library, never in a backup, and the secret key
 * never comes back to this side. The bucket holds one backup per library; a new
 * backup replaces it.
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

/** The backup kept in the storage. */
export interface S3Backup {
  size: number;
  /** When it was stored, as an HTTP date. */
  modified: string | null;
}

/** A failed S3 call, carrying the message to show. */
export class S3Failure extends Error {
  constructor(readonly key: MessageKey, cause?: unknown) {
    super(key, { cause });
  }
}

/** Words in the native app's errors, and the message each one means (first match wins). */
const MESSAGES: [string, MessageKey][] = [
  ['sync-outdated', 's3.err.outdated'],
  ['s3-not-set', 's3.err.notSet'],
  ['s3-no-backup', 's3.err.noBackup'],
  ['s3-endpoint', 's3.err.endpoint'],
  ['s3-bucket', 's3.err.bucket'],
  ['s3-keys', 's3.err.keys'],
  ['s3-auth', 's3.err.keys'],
  ['s3-network', 's3.err.network'],
  ['too-large', 's3.err.tooLarge']
];

/** What the native app's error says went wrong, as a message to show. */
function failure(error: unknown): S3Failure {
  console.error(error);
  const text = String((error as { message?: string } | null)?.message ?? error);
  return new S3Failure(MESSAGES.find(([word]) => text.includes(word))?.[1] ?? 's3.err.generic', error);
}

async function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw failure(e);
  }
}

export const getS3Settings = (user: string) => invoke<S3Settings | null>('s3_config_get', { user });

/** Saves the storage after checking it answers with these keys; null stops using it. */
export const setS3Config = (user: string, config: S3Config | null) => call<void>('s3_config_set', { user, config });

/** The backup in the storage, null when there is none yet. */
export const getS3Backup = (user: string) => call<S3Backup | null>('s3_backup_info', { user });

/** Backs the library up to the storage, replacing the backup there. */
export const backupToS3 = (user: string) => call<S3Backup | null>('s3_backup', { user });

/** Merges the storage's backup into the library. */
export const restoreFromS3 = (user: string) => call<void>('s3_restore', { user });

/** `minio.example.com` from the endpoint, to name the storage. */
export const s3Host = (s: S3Settings) => s.endpoint.replace(/^\w+:\/\//, '');
