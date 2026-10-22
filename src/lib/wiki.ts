import { invoke } from '@tauri-apps/api/core';
import { AiFailure } from './ai';

/*
 * The wiki of a book on this device: characters, places, factions, items and terms
 * with their relations (src-tauri/src/library/wiki.rs). The AI fills it while it
 * translates; the user can edit it. Search goes by name, and by meaning (sqlite-vec)
 * when an embedding model is set with the LLM.
 */

export const WIKI_KINDS = ['character', 'place', 'faction', 'item', 'level', 'term', 'other'] as const;
export type WikiKind = (typeof WIKI_KINDS)[number];

/**
 * Relation types, as "from … to" ("A master_of B"); the same in every language and
 * named by the app (`wiki.rel.<type>`, `.rev` from the other side). Any other `kind`
 * is a free relation in someone's own words, kept per language in `labels`.
 */
export const REL_TYPES = [
  'master_of', 'disciple_of', 'parent_of', 'child_of', 'sibling_of', 'spouse_of', 'lover_of',
  'friend_of', 'enemy_of', 'member_of', 'leader_of', 'located_in', 'owns', 'has_level'
] as const;
export type RelType = (typeof REL_TYPES)[number];
export const isRelType = (k: string): k is RelType => (REL_TYPES as readonly string[]).includes(k);

/** The wiki's words in the book's own language ("Raw"). */
export const RAW = 'raw';

export interface WikiEntry {
  uid: string;
  kind: WikiKind;
  /** As written in the original text. */
  name: string;
  /** The name to use per language. */
  names: Record<string, string>;
  aliases: string[];
  /** The description per language; `raw` in the book's own language. */
  summaries: Record<string, string>;
  /** A level's place in its ladder, lowest first. */
  rank: number | null;
  /** The chapter it was first met in, in reading order from 1. */
  firstChapter: number | null;
  /** That chapter's own number as the book shows it ("Chương 629"), when it has one. */
  firstChapterShown?: number | null;
  source: 'ai' | 'user';
  updatedAt: number;
}

export interface WikiRelation {
  uid: string; fromUid: string; toUid: string;
  /** A `RelType`, or a free relation's words as first written. */
  kind: string;
  /** A free relation's words per language. */
  labels: Record<string, string>;
  note: string; firstChapter: number | null; firstChapterShown?: number | null;
}

export interface Wiki { entries: WikiEntry[]; relations: WikiRelation[]; auto: boolean; lastError: string | null; byMeaning: boolean }

export interface WikiEntryInput {
  uid?: string; kind: WikiKind; name: string; names: Record<string, string>; aliases: string[];
  summaries: Record<string, string>; rank: number | null;
}

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
/** `kind`: a `RelType`, or free words written in `lang`. */
export const addWikiRelation = (user: string, novelId: number, fromUid: string, toUid: string, kind: string, lang: string, note = '') =>
  invoke<string>('wiki_relate', { user, novelId, fromUid, toUid, kind, lang, note });
export const removeWikiRelation = (user: string, uid: string) => invoke<void>('wiki_unrelate', { user, uid });
/** Turning it on reads in the wiki facts kept from earlier translations; returns the changes made. */
export const setWikiAuto = (user: string, novelId: number, auto: boolean) => invoke<number>('wiki_set_auto', { user, novelId, auto });

/** How many entries and free relations have no words in `lang` yet. */
export const getWikiMissing = (user: string, novelId: number, lang: string) => invoke<number>('wiki_missing', { user, novelId, lang });

/** Fills in what `lang` is missing with the user's LLM; returns how many changed. */
export async function translateWiki(user: string, novelId: number, lang: string) {
  try {
    return await invoke<number>('wiki_translate', { user, novelId, lang });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The name to show in `lang`: its name there, else the raw one. */
export const wikiLabel = (e: WikiEntry, lang: string) => (lang === RAW ? e.name : e.names[lang] || e.name);

/** The description to show in `lang` and whether it is in that language (else raw, else any). */
export function wikiSummary(e: WikiEntry, lang: string): { text: string; inLang: boolean; from: string } {
  if (e.summaries[lang]) return { text: e.summaries[lang], inLang: true, from: lang };
  const from = e.summaries[RAW] ? RAW : Object.keys(e.summaries)[0] ?? RAW;
  const text = e.summaries[from] ?? '';
  return { text, inLang: !text, from };
}

/** The chapter an entry or relation was first met in, as the book numbers it. */
export const wikiChapter = (x: { firstChapter: number | null; firstChapterShown?: number | null }) =>
  x.firstChapterShown ?? x.firstChapter;

/** Whether an entry lacks a name or description in `lang` (it shows the raw ones then). */
export const wikiMissingIn = (e: WikiEntry, lang: string) =>
  lang !== RAW && (!e.names[lang] || (Object.keys(e.summaries).length > 0 && !e.summaries[lang]));

/** What the wiki knew by chapter `number` (reading order, from 1), for the reader. */
export const getWikiAt = (user: string, novelId: number, number: number) => invoke<Wiki>('wiki_at', { user, novelId, number });
