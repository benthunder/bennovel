import { invoke } from '@tauri-apps/api/core';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { load, save } from './storage';

/*
 * Backups: everything the app keeps, in one gzipped file the user picks: the
 * on-device library (books with their AI translations, favourites, history, replace
 * rules, glossaries, downloaded novels), the LLM settings with their API key, and the
 * app's settings below. Restoring merges the library (what the backup holds comes back
 * even if it was removed here since) and puts the settings back.
 * Native app only: the web build has no library file.
 */

/** The app's own settings kept in this device's storage that a backup carries. */
const SETTINGS = ['uiLang', 'defaultLang', 'readerPrefs'] as const;

const appSettings = () =>
  JSON.stringify(Object.fromEntries(SETTINGS.flatMap(k => {
    const v = load<unknown>(k, undefined);
    return v === undefined ? [] : [[k, v]];
  })));

/** Puts back settings from a backup; true when there were any. */
function applySettings(json: string | null): boolean {
  if (!json) return false;
  try {
    const got = JSON.parse(json) as Record<string, unknown>;
    let any = false;
    for (const k of SETTINGS) {
      if (k in got) { save(k, got[k]); any = true; }
    }
    return any;
  } catch {
    return false;
  }
}

/** Asks where to save and writes a backup there; false when cancelled. */
export async function createBackup(user: string): Promise<boolean> {
  const day = new Date().toISOString().slice(0, 10);
  const path = await saveDialog({ defaultPath: `bennovel-backup-${day}.bnbackup`, filters: [{ name: 'BenNovel backup', extensions: ['bnbackup'] }] });
  if (!path) return false;
  await invoke<number>('library_backup', { user, path, settings: appSettings() });
  return true;
}

/**
 * Asks for a backup file, merges it into the library and puts its settings back.
 * Resolves to false when cancelled, 'settings' when the app's settings changed (the
 * app should start again to use them), true otherwise.
 */
export async function restoreBackup(user: string): Promise<boolean | 'settings'> {
  const android = /android/i.test(navigator.userAgent);
  const path = await openDialog({ multiple: false, filters: android ? undefined : [{ name: 'BenNovel backup', extensions: ['bnbackup'] }] });
  if (!path) return false;
  const settings = await invoke<string | null>('library_restore', { user, path });
  return applySettings(settings) ? 'settings' : true;
}
