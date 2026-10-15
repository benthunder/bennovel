import { invoke } from '@tauri-apps/api/core';

/*
 * Backups: the whole on-device library (books with their AI translations, favourites,
 * history, replace rules, downloaded novels) in one gzipped file. It goes to a file the
 * user picks. Restoring merges it into the library:
 * what it holds comes back even if it was removed here since.
 * Native app only: the web build has no library file.
 */

/** Asks where to save and writes a backup there; false when cancelled. */
export async function createBackup(user: string): Promise<boolean> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const day = new Date().toISOString().slice(0, 10);
  const path = await save({ defaultPath: `bennovel-backup-${day}.bnbackup`, filters: [{ name: 'BenNovel backup', extensions: ['bnbackup'] }] });
  if (!path) return false;
  await invoke<number>('library_backup', { user, path });
  return true;
}

/** Asks for a backup file and merges it into the library; false when cancelled. */
export async function restoreBackup(user: string): Promise<boolean> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const android = /android/i.test(navigator.userAgent);
  const path = await open({ multiple: false, filters: android ? undefined : [{ name: 'BenNovel backup', extensions: ['bnbackup'] }] });
  if (!path) return false;
  await invoke('library_restore', { user, path });
  return true;
}
