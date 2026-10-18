// Data access seam for books on the device. There is no server: everything here
// reads and writes the library file (src-tauri/src/library) or, for a file opened
// without saving, this device's storage.
import { invoke } from '@tauri-apps/api/core';
import { load, save } from '../lib/storage';
import type { ContentLang, ReplaceRule } from './types';

/**
 * Rules from the library file; the first time, those kept before (`legacy`) are moved in.
 * An empty legacy list is not written, so it can't replace another device's list on sync.
 */
async function libraryRulesOr(user: string, novelId: number, lang: ContentLang, legacy: () => Promise<ReplaceRule[]>) {
  const stored = await invoke<ReplaceRule[] | null>('replace_rules_get', { user, kind: 'book', id: novelId, lang });
  if (stored) return stored;
  const old = await legacy().catch(() => []);
  if (old.length) await invoke<void>('replace_rules_set', { user, kind: 'book', id: novelId, lang, rules: old });
  return old;
}

/**
 * Rules for a book in the on-device library. Lists kept on the device before (under
 * `legacyKey`) move into the library file the first time.
 */
export async function getBookRules(user: string, novelId: number, lang: ContentLang, legacyKey: string): Promise<ReplaceRule[]> {
  const rules = await libraryRulesOr(user, novelId, lang, async () => load<ReplaceRule[]>(legacyKey, []));
  save(legacyKey, null);
  return rules;
}

export const saveBookRules = (user: string, novelId: number, lang: ContentLang, rules: ReplaceRule[]) =>
  invoke<void>('replace_rules_set', { user, kind: 'book', id: novelId, lang, rules });

/** Applies each rule to `text` in order, replacing every occurrence (case-sensitive, plain text). */
export const applyReplaceRules = (text: string, rules: ReplaceRule[]) =>
  rules.reduce((s, r) => (r.find ? s.split(r.find).join(r.replace) : s), text);
