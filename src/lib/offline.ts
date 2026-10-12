import { invoke } from '@tauri-apps/api/core';
import type { ContentLang, Novel } from '../data/types';
import { canOpenLocalBooks } from './localBook';
import { supabase } from './supabase';

/*
 * Online novels downloaded to read offline, whole or chapter by chapter: each chapter
 * in every published language, and the glossary, copied from Supabase into the user's library file
 * (src-tauri/src/library/offline.rs). They stay on this device: sync leaves them out.
 * Native app only, like the library.
 */

export interface Download {
  id: number;
  /** The novel as the catalog showed it when downloaded. */
  data: Novel | null;
  /** Chapters stored. */
  chapters: number;
  /** Chapters the novel had when the download started. */
  total: number;
  langs: ContentLang[];
  /** Language of the original text. */
  origLang: ContentLang | null;
  /** False while downloading, or after a download was interrupted. */
  complete: boolean;
  downloadedAt: number | null;
}

export interface OfflineChapter {
  number: number;
  lang: ContentLang;
  source: 'original' | 'human' | 'ai';
  title: string;
  words: number;
}

export const canDownload = canOpenLocalBooks;

/** Downloads of the signed-in user (library key), read by the repository. */
let current = { user: '', byId: new Map<number, Download>() };

export function setDownloads(user: string, list: Download[]) {
  current = { user, byId: new Map(list.map(d => [d.id, d])) };
}

export const findDownload = (id: number): Download | undefined => current.byId.get(id);

export const listDownloads = async (user: string): Promise<Download[]> =>
  (canDownload() ? (await invoke<Download[] | null>('offline_list', { user })) ?? [] : []);

export const offlineChapters = (id: number) =>
  invoke<OfflineChapter[]>('offline_chapters', { user: current.user, novelId: id });

export const offlineChapter = (id: number, number: number, lang: ContentLang) =>
  invoke<{ title: string; content: string } | null>('offline_chapter', { user: current.user, novelId: id, number, lang });

export const offlineGlossary = (id: number) =>
  invoke<{ key: string; lang: string; value: string }[]>('offline_glossary', { user: current.user, novelId: id });

/** Numbers of the chapters of `id` stored on the device. */
export const offlineNumbers = (user: string, id: number) => invoke<number[]>('offline_numbers', { user, novelId: id });

export const deleteDownload = (user: string, id: number) => invoke<void>('offline_delete', { user, novelId: id });

/** The language to read a download in: the saved one when it was downloaded, else the original. */
export const downloadLang = (d: Download, saved: ContentLang | null): ContentLang | null =>
  (saved && d.langs.includes(saved) ? saved : d.origLang ?? d.langs[0] ?? null);

/** Chapters fetched and stored per step (each with its text in every language). */
const PAGE = 20;

type Translation = { lang: string; source: string; style?: string | null; title: string; content: string; word_count: number | null };

/** One text per language: an original or human translation over the AI ones. */
function onePerLang(trs: Translation[]) {
  const byLang = new Map<string, Translation>();
  for (const t of trs) {
    const had = byLang.get(t.lang);
    if (!had || (had.source === 'ai' && t.source !== 'ai')) byLang.set(t.lang, t);
  }
  return [...byLang.values()];
}

const must = <T>(res: { data: T | null; error: { message: string } | null }): T => {
  if (res.error) throw new Error(res.error.message);
  return res.data as T;
};

/**
 * Downloads every published chapter of `novel` (all languages) and its glossary into
 * the library of `user`. Downloading again updates it; what was there stays readable
 * until the new download completes.
 */
export async function downloadNovel(user: string, novel: Novel, onProgress: (done: number, total: number) => void) {
  const id = novel.id;
  const head = await supabase.from('chapters').select('id', { count: 'exact', head: true }).eq('novel_id', id);
  if (head.error) throw new Error(head.error.message);
  const total = head.count ?? 0;
  const started = await invoke<number>('offline_start', { user, novelId: id, data: novel, total });
  onProgress(0, total);

  await saveGlossary(user, id);

  let done = 0;
  for (let from = 0; ; from += PAGE) {
    const rows = must(await chapterQuery(id).order('number').range(from, from + PAGE - 1));
    if (!rows.length) break;
    await invoke('offline_add', { user, novelId: id, chapters: toRows(rows) });
    done += rows.length;
    onProgress(done, Math.max(total, done));
    if (rows.length < PAGE) break;
  }
  await invoke('offline_finish', { user, novelId: id, started });
}

/** Downloads one chapter of `novel` (all languages), and the glossary its text uses. */
export async function downloadChapter(user: string, novel: Novel, number: number) {
  const rows = must(await chapterQuery(novel.id).eq('number', number));
  if (!rows.length) throw new Error(`Chapter ${number} is not published`);
  await invoke('offline_ensure', { user, novelId: novel.id, data: novel, total: novel.chapters });
  await saveGlossary(user, novel.id);
  await invoke('offline_add', { user, novelId: novel.id, chapters: toRows(rows) });
}

const chapterQuery = (id: number) => supabase.from('chapters')
  .select('number, chapter_translations(lang, source, style, title, content, word_count)')
  .eq('novel_id', id);

const toRows = (rows: { number: number; chapter_translations: Translation[] }[]) =>
  rows.flatMap(c => onePerLang(c.chapter_translations).map(t => ({
    number: c.number, lang: t.lang, source: t.source, title: t.title, content: t.content, words: t.word_count ?? 0
  })));

async function saveGlossary(user: string, id: number) {
  const glossary = must(await supabase.from('glossary_entries').select('term_key, lang, value').eq('novel_id', id));
  await invoke('offline_set_glossary', { user, novelId: id, entries: glossary.map(g => ({ key: g.term_key, lang: g.lang, value: g.value })) });
}
