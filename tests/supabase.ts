// Reads the same Supabase project the app uses, so tests compare the UI with live data
// instead of hard-coded copies of it.
import { createClient } from '@supabase/supabase-js';
import { loadEnv } from 'vite';
import type { Database } from '../src/data/database.types';

const env = loadEnv('development', process.cwd(), 'VITE_');
export const db = createClient<Database>(env.VITE_SUPABASE_URL, env.VITE_SUPABASE_PUBLISHABLE_KEY);

export async function mostReadNovels() {
  const { data, error } = await db.from('novels')
    .select('id, read_count, original_lang, authors(name), novel_translations(lang, title)')
    .order('read_count', { ascending: false });
  if (error) throw error;
  return data.map(n => ({
    id: n.id,
    title: n.novel_translations.find(t => t.lang === n.original_lang)!.title,
    author: n.authors!.name
  }));
}

export async function chapterTexts(novelId: number, lang: string) {
  const { data, error } = await db.from('chapter_translations')
    .select('title, content, chapters!inner(novel_id, number)')
    .eq('lang', lang)
    .eq('chapters.novel_id', novelId);
  if (error) throw error;
  return data.sort((a, b) => a.chapters.number - b.chapters.number).map(c => ({ number: c.chapters.number, title: c.title, paragraphs: c.content.split(/\n\s*\n/).map(p => p.trim()).filter(Boolean) }));
}

/** Replaces `[[key]]` glossary markers with the term's name in `lang`, the way the reader shows them. */
export async function withGlossary(novelId: number, lang: string, text: string) {
  const { data, error } = await db.from('glossary_entries').select('term_key, value').eq('novel_id', novelId).eq('lang', lang);
  if (error) throw error;
  const names = new Map(data.map(g => [g.term_key, g.value]));
  return text.replace(/\[\[([\w-]+)\]\]/g, (_, k: string) => names.get(k) ?? k);
}

export async function categoryNames(lang: string) {
  const { data, error } = await db.from('category_translations').select('name').eq('lang', lang);
  if (error) throw error;
  return data.map(c => c.name);
}
