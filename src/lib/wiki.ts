import { invoke } from '@tauri-apps/api/core';
import { AiFailure } from './ai';

/*
 * The wiki of a book on this device: characters, places, factions, items and terms
 * with their relations (src-tauri/src/library/wiki.rs). The AI fills it while it
 * translates; the user can edit it. Search goes by name, and by meaning (sqlite-vec)
 * when an embedding model is set with the LLM.
 */

export const WIKI_KINDS = ['character', 'place', 'faction', 'item', 'term', 'other'] as const;
export type WikiKind = (typeof WIKI_KINDS)[number];

export interface WikiEntry {
  uid: string;
  kind: WikiKind;
  /** As written in the original text. */
  name: string;
  /** The name to use per language. */
  names: Record<string, string>;
  aliases: string[];
  summary: string;
  firstChapter: number | null;
  source: 'ai' | 'user';
  updatedAt: number;
}

export interface WikiRelation { uid: string; fromUid: string; toUid: string; kind: string; note: string; firstChapter: number | null }

export interface Wiki { entries: WikiEntry[]; relations: WikiRelation[]; auto: boolean; lastError: string | null; byMeaning: boolean }

export interface WikiEntryInput { uid?: string; kind: WikiKind; name: string; names: Record<string, string>; aliases: string[]; summary: string }

export const getWiki = (user: string, novelId: number) => invoke<Wiki>('wiki_get', { user, novelId });
export const searchWiki = (user: string, novelId: number, query: string) => invoke<WikiEntry[]>('wiki_search', { user, novelId, query });

export async function saveWikiEntry(user: string, novelId: number, entry: WikiEntryInput) {
  try {
    return await invoke<WikiEntry>('wiki_save', { user, novelId, entry });
  } catch (e) {
    throw new AiFailure(e);
  }
}

export const removeWikiEntry = (user: string, uid: string) => invoke<void>('wiki_remove', { user, uid });
export const addWikiRelation = (user: string, novelId: number, fromUid: string, toUid: string, kind: string, note = '') =>
  invoke<string>('wiki_relate', { user, novelId, fromUid, toUid, kind, note });
export const removeWikiRelation = (user: string, uid: string) => invoke<void>('wiki_unrelate', { user, uid });
export const setWikiAuto = (user: string, novelId: number, auto: boolean) => invoke<void>('wiki_set_auto', { user, novelId, auto });

/** The name to show: in `lang` when the wiki has one, else as in the original. */
export const wikiLabel = (e: WikiEntry, lang: string) => e.names[lang] || e.names.vi || e.names.en || e.name;
