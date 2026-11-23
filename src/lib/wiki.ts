import { invoke } from '@tauri-apps/api/core';
import { AiFailure } from './ai';

/*
 * The wiki of a book on this device: characters, places, factions, items and terms
 * with their relations (src-tauri/src/library/wiki.rs). The AI fills it while it
 * translates; the user can edit it. Search goes by name, aliases and notes.
 */

export const WIKI_KINDS = ['character', 'place', 'faction', 'item', 'level', 'skill', 'term', 'other'] as const;
export type WikiKind = (typeof WIKI_KINDS)[number];

/**
 * Relation types, as "from … to" ("A master_of B"); the same in every language and
 * named by the app (`wiki.rel.<type>`, `.rev` from the other side). Any other `kind`
 * is a free relation in someone's own words, kept per language in `labels`.
 */
export const REL_TYPES = [
  'master_of', 'disciple_of', 'parent_of', 'child_of', 'sibling_of', 'spouse_of', 'lover_of',
  'friend_of', 'enemy_of', 'member_of', 'leader_of', 'located_in', 'owns', 'has_level', 'has_skill', 'equivalent_to'
] as const;
export type RelType = (typeof REL_TYPES)[number];
export const isRelType = (k: string): k is RelType => (REL_TYPES as readonly string[]).includes(k);

/** The wiki's words in the book's own language ("Raw"). */
export const RAW = 'raw';
/** The one language the wiki is written in, whatever a chapter is translated into. */
export const WIKI_LANG: string = 'vi';

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
  /** A level's ladder and stage, a character's current level (wiki.rs `Meta`). */
  meta?: WikiMeta;
  /** The chapter it was first met in, in reading order from 1. */
  firstChapter: number | null;
  /** That chapter's own number as the book shows it ("Chương 629"), when it has one. */
  firstChapterShown?: number | null;
  source: 'ai' | 'user';
  updatedAt: number;
}

/** Where a level stands in its ladder (`system`, beside `rank`) and a character on one
 *  (`level`); `stage` is the step inside a level (early 1, middle 2, late 3, peak 4). */
export interface WikiMeta {
  system?: string; level?: string; stage?: number; stageName?: string;
  /** A character's profile, in the wiki's language (wiki.rs `Meta`). */
  personality?: string; age?: string; origin?: string; abilities?: string;
  /** A skill's grade as the story ranks techniques, and how hard it is to learn or use. */
  grade?: string; difficulty?: string;
}

/** What the wiki keeps of a skill beside its description. */
export const SKILL_FIELDS = ['grade', 'difficulty'] as const;
export type SkillField = (typeof SKILL_FIELDS)[number];

/** A character's profile, in the order it is shown: personality, age, background, skills (then its level). */
export const PROFILE_FIELDS = ['personality', 'age', 'origin', 'abilities'] as const;
export type ProfileField = (typeof PROFILE_FIELDS)[number];

/**
 * How a character's relations with other characters are grouped, in this order; a
 * free relation (someone's own words) goes under `other`.
 */
export const REL_GROUPS = {
  mentor: ['master_of', 'disciple_of'],
  family: ['parent_of', 'child_of', 'sibling_of', 'spouse_of'],
  love: ['lover_of'],
  friend: ['friend_of'],
  enemy: ['enemy_of'],
  other: []
} as const satisfies Record<string, readonly RelType[]>;
export type RelGroup = keyof typeof REL_GROUPS;
export const relGroup = (kind: string): RelGroup =>
  (Object.keys(REL_GROUPS) as RelGroup[]).find(g => (REL_GROUPS[g] as readonly string[]).includes(kind)) ?? 'other';

/** "Kim Đan, trung kỳ": a level or a character's current level with its stage. */
export function metaStage(m: WikiMeta | undefined, stageWord: (n: number) => string): string {
  if (!m) return '';
  return m.stageName || (m.stage != null ? stageWord(m.stage) : '');
}

export interface WikiRelation {
  uid: string; fromUid: string; toUid: string;
  /** A `RelType`, or a free relation's words as first written. */
  kind: string;
  /** A free relation's words per language. */
  labels: Record<string, string>;
  note: string; firstChapter: number | null; firstChapterShown?: number | null;
}

export interface Wiki { entries: WikiEntry[]; relations: WikiRelation[]; auto: boolean; lastError: string | null }

export interface WikiEntryInput {
  uid?: string; kind: WikiKind; name: string; names: Record<string, string>; aliases: string[];
  summaries: Record<string, string>; rank: number | null; meta?: WikiMeta;
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
/** Writes the wiki's names into the book's translated chapters; returns the paragraphs changed. */
export const applyWikiToText = (user: string, novelId: number) => invoke<number>('wiki_apply_to_text', { user, novelId });
/** Sets a smart merge suggestion (the entries it holds) aside as already right; it is not suggested again. */
export const skipWikiMerge = (user: string, novelId: number, uids: string[]) => invoke<void>('wiki_merge_skip', { user, novelId, uids });
/** Removes the selected entries at once; returns how many went. */
export const removeWikiEntries = (user: string, uids: string[]) => invoke<number>('wiki_remove_many', { user, uids });
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

/** Why smart merge thinks two entries are one. */
export type MergeReason = 'same_name' | 'same_raw' | 'near_name';

/** A pair smart merge found, for the user to review: `from` goes into `into`, with the replace rules that adds. */
export interface MergeSuggestion {
  from: WikiEntry;
  into: WikiEntry;
  reason: MergeReason;
  rules: { find: string; replace: string }[];
  note: string;
}

/** A group the AI found: `members` go into `into`, which takes the final `name` (no members: only a better name). */
export interface MergeGroup {
  members: WikiEntry[];
  into: WikiEntry;
  name: string;
  /** The right original name for `into` when its own is wrong (often the Vietnamese name written as the original); '' when it is right. */
  raw: string;
  rules: { find: string; replace: string }[];
  note: string;
}

/** The book's entries that look like one written two ways (nothing is merged yet). */
export async function suggestWikiMerges(user: string, novelId: number) {
  try {
    return await invoke<MergeSuggestion[]>('wiki_merge_suggest', { user, novelId });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The AI's groups for the whole wiki, block by block (about 10k letters each, never cutting an entry), with their final names. */
export async function suggestWikiMergesAi(user: string, novelId: number) {
  try {
    return await invoke<MergeGroup[]>('wiki_merge_suggest_ai', { user, novelId });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The AI step of smart merge for the whole wiki, run in the background like the translation queue: its blocks by state and the groups found so far. */
export interface MergeRun {
  novelId: number;
  total: number;
  done: number;
  failed: number;
  queued: number;
  running: number;
  paused: number;
  /** Why the run waits or a block failed. */
  error: string | null;
  groups: MergeGroup[];
}

/** Starts the AI step in the background (or carries on the run left); `fresh` starts over. */
export async function startWikiMergeAi(user: string, novelId: number, fresh = false) {
  try {
    return await invoke<MergeRun>('wiki_merge_ai_start', { user, novelId, fresh });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The book's AI run with the groups found so far (they survive closing the app). */
export async function wikiMergeAiStatus(user: string, novelId: number) {
  try {
    return await invoke<MergeRun>('wiki_merge_ai_status', { user, novelId });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Pauses, carries on, tries the failed blocks again or clears the book's AI run. */
export async function controlWikiMergeAi(user: string, novelId: number, action: 'pause' | 'resume' | 'retry' | 'clear') {
  try {
    return await invoke<MergeRun>('wiki_merge_ai_control', { user, novelId, action });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** An original name the translation wrote as an entry's name (or close to it). */
export interface RawName {
  raw: string;
  translated: string;
  /** Chapters where it was written so, the first ones. */
  chapters: number[];
  /** The entry whose name or alias it is, if any. */
  entryUid: string | null;
  exact: boolean;
}

/** Smart merge for one entry: the original names translated as its name, and the entries that look like it. */
export async function entryWikiMerges(user: string, novelId: number, uid: string) {
  try {
    return await invoke<{ raws: RawName[]; pairs: MergeSuggestion[] }>('wiki_merge_entry', { user, novelId, uid });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The AI's group for one entry, with the final name, from everything known about it and its candidates. */
export async function entryWikiMergesAi(user: string, novelId: number, uid: string) {
  try {
    return await invoke<MergeGroup[]>('wiki_merge_entry_ai', { user, novelId, uid });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Merges the pairs the user kept; each merge also adds its line to the book's replace rules. */
export async function applyWikiMerges(user: string, novelId: number, picks: { fromUid: string; intoUid: string }[],
  groups: { intoUid: string; fromUids: string[]; name: string; raw?: string }[] = []) {
  try {
    return await invoke<{ merged: number; rules: number; renamed: number; fixed: number }>('wiki_merge_apply', { user, novelId, picks, groups });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Makes entry `fromUid` part of `intoUid` (one character written two ways): its names become aliases, its relations move over, and its old name goes in the replace rules. */
export async function mergeWikiEntry(user: string, novelId: number, fromUid: string, intoUid: string) {
  try {
    return await invoke<WikiEntry>('wiki_merge', { user, novelId, fromUid, intoUid });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** What smart merge finds for one entry: the original names translated as its name, and the entries that look like it (all into one kept entry). */
export type EntryMerge = { raws: RawName[]; pairs: MergeSuggestion[] };

/**
 * What one entry's check found to merge, many into one: the entry everything goes into,
 * the entries that go, and the original names with no entry of their own (they become
 * its aliases). Null when there is nothing to merge.
 */
export function entryMergeFound(m: EntryMerge | null | undefined, uid: string) {
  if (!m) return null;
  const into = m.pairs[0]?.into ?? null;
  const from = m.pairs.map(p => p.from);
  const loose = m.raws.filter(r => !r.entryUid && r.raw.trim());
  if (!from.length && !loose.length) return null;
  return { intoUid: into?.uid ?? uid, into, from, loose, pairs: m.pairs };
}

/** The user allowed the merge one entry's check found: `fromUids` go into `intoUid`, and the original names `raws` become its aliases. */
export async function allowWikiMerge(user: string, novelId: number, intoUid: string, fromUids: string[], raws: RawName[]) {
  try {
    return await invoke<{ entry: WikiEntry; applied: { merged: number; rules: number; renamed: number; fixed: number }; adopted: number }>(
      'wiki_merge_entry_allow', { user, novelId, intoUid, fromUids, raws });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Where a name stands in the book: the sentences around it, original and translation. */
export interface RetranslateContext { chapter: number; raw: string; translated: string }

/** The AI's new translation of one name, for the user to keep, edit or drop. */
export interface RetranslateProposal {
  uid: string | null;
  raw: string;
  current: string;
  name: string;
  kind: string;
  why: string;
  contexts: RetranslateContext[];
}

/** What to translate again: an entry, or words selected in a chapter (0-based). */
export type RetranslateTarget = { uid?: string; text?: string; chapterIndex?: number };

/** Asks the AI to translate one name again, with the sentences around it in the book; nothing changes yet. */
export async function askRetranslate(user: string, novelId: number, target: RetranslateTarget) {
  try {
    return await invoke<RetranslateProposal>('wiki_retranslate_ask', { user, novelId, target });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Keeps the name chosen: the entry takes it, the old spellings are replaced in the translations, and smart merge looks again (`merge`). */
export async function applyRetranslate(user: string, novelId: number, keep: { uid: string | null; raw: string; name: string; kind: string; old: string[] }) {
  try {
    return await invoke<{ entry: WikiEntry; changed: number; merge: EntryMerge }>('wiki_retranslate_apply', { user, novelId, keep });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Makes `uids` (lowest first) the levels of ladder `system`, ranked from 1; they are the user's from then on. */
export async function setWikiLadder(user: string, novelId: number, system: string, uids: string[]) {
  try {
    return await invoke<number>('wiki_levels_set', { user, novelId, system, uids });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** The AI puts every level of the book in its ladder, lowest first, and they are set so. */
export async function orderWikiLevelsAi(user: string, novelId: number) {
  try {
    return await invoke<{ system: string; uids: string[] }[]>('wiki_levels_ai', { user, novelId });
  } catch (e) {
    throw new AiFailure(e);
  }
}
