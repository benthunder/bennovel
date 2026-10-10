// Data access seam. Everything screens need from "the server" goes through here.
// The catalog (novels, categories, collections, languages) is small, so it is loaded
// from Supabase once at startup and served synchronously; chapters and glossaries are
// fetched on demand. Until it has loaded (or when offline) the catalog is empty.
import { supabase } from '../lib/supabase';
import { load, save } from '../lib/storage';
import type {
  Category, ChapterInfo, ChapterText, CollectionKey, ContentLang, ContentLangInfo, GlossaryTerm, Novel, NovelStatus, ReplaceRule, UiLang
} from './types';
import { CONTENT_LANGS } from './types';

interface Catalog {
  novels: Novel[];
  byId: Map<number, Novel>;
  categories: Category[];
  categoryNames: Map<Category, Record<UiLang, string>>;
  collections: { key: CollectionKey; ids: number[] }[];
  collectionNames: Map<CollectionKey, Record<UiLang, { kicker: string; title: string }>>;
  contentLangs: ContentLangInfo[];
}

let catalog: Catalog | null = null;

const EMPTY: Catalog = {
  novels: [], byId: new Map(), categories: [], categoryNames: new Map(), collections: [], collectionNames: new Map(), contentLangs: []
};

const cat = (): Catalog => catalog ?? EMPTY;

const must = <T>(res: { data: T | null; error: { message: string } | null }, what: string): T => {
  if (res.error) throw new Error(`Loading ${what} failed: ${res.error.message}`);
  return res.data as T;
};

/** Most-read novels shown in the "popular" strip. */
const POPULAR_COUNT = 5;

export async function loadCatalog(): Promise<void> {
  const [novelsRes, catsRes, collsRes, langsRes] = await Promise.all([
    supabase.from('novels')
      .select('id, status, chapter_count, rating_avg, read_count, published_year, cover_palette, original_lang, category_slug, authors(name), novel_translations(lang, title, description)')
      .order('id'),
    supabase.from('categories').select('slug, name, sort_order, category_translations(lang, name)').order('sort_order'),
    supabase.from('collections').select('key, sort_order, collection_translations(lang, kicker, title), collection_items(novel_id, position)').order('sort_order'),
    supabase.from('languages').select('code, name, native_name').order('sort_order')
  ]);

  const novels: Novel[] = must(novelsRes, 'novels').map(r => {
    const tr = r.novel_translations;
    const orig = tr.find(t => t.lang === r.original_lang) ?? tr[0];
    const descIn = (l: UiLang) => tr.find(t => t.lang === l)?.description || orig?.description || '';
    const author = (Array.isArray(r.authors) ? r.authors[0] : r.authors) as { name: string } | null;
    return {
      id: r.id,
      title: orig?.title ?? '',
      author: author?.name ?? '',
      cat: r.category_slug,
      status: (r.status === 'completed' ? 'Completed' : 'Ongoing') as NovelStatus,
      chapters: r.chapter_count,
      rating: Number(r.rating_avg),
      reads: Math.round(Number(r.read_count) / 1000),
      year: r.published_year ?? 0,
      desc: { en: descIn('en'), vi: descIn('vi') },
      cover: (r.cover_palette as Novel['cover'] | null) ?? DEFAULT_COVER
    };
  });

  const cats = must(catsRes, 'categories');
  const colls = must(collsRes, 'collections');
  const ids = new Set(novels.map(n => n.id));

  catalog = {
    novels,
    byId: new Map(novels.map(n => [n.id, n])),
    categories: cats.map(c => c.slug),
    categoryNames: new Map(cats.map(c => {
      const name = (l: UiLang) => c.category_translations.find(t => t.lang === l)?.name ?? c.name;
      return [c.slug, { en: name('en'), vi: name('vi') }];
    })),
    collections: colls.map(c => ({
      key: c.key,
      // Items whose novel is hidden by row level security are dropped.
      ids: [...c.collection_items].sort((a, b) => a.position - b.position).map(i => i.novel_id).filter(id => ids.has(id))
    })),
    collectionNames: new Map(colls.map(c => {
      const pick = (l: UiLang) => {
        const t = c.collection_translations.find(x => x.lang === l) ?? c.collection_translations[0];
        return { kicker: t?.kicker ?? '', title: t?.title ?? c.key };
      };
      return [c.key, { en: pick('en'), vi: pick('vi') }];
    })),
    contentLangs: must(langsRes, 'languages')
      .filter(l => (CONTENT_LANGS as readonly string[]).includes(l.code))
      .map(l => ({ code: l.code as ContentLang, name: l.name, native: l.native_name }))
  };
}

const DEFAULT_COVER: Novel['cover'] = { bg: 'var(--color-accent-300)', fg: 'var(--color-accent-900)', deco: 'var(--color-accent-500)' };

/** Novels saved or read before, as last seen, so they show without a connection. */
const remembered = new Map<number, Novel>();

export function rememberNovels(novels: { id: number; data: Novel | null }[]) {
  for (const n of novels) if (n.data) remembered.set(n.id, n.data);
}

export const findNovel = (id: number): Novel | undefined => cat().byId.get(id) ?? remembered.get(id);

export const getNovel = (id: number): Novel => {
  const n = findNovel(id);
  if (!n) throw new Error(`Unknown novel ${id}`);
  return n;
};

/** Novels for saved ids, skipping any that are no longer in the catalog. */
export const getNovels = (ids: number[]): Novel[] => ids.flatMap(id => findNovel(id) ?? []);

const allNovels = () => cat().novels;

export const getPopular = () => [...allNovels()].sort((a, b) => b.reads - a.reads).slice(0, POPULAR_COUNT);
export const getTopRated = (n: number) => [...allNovels()].sort((a, b) => b.rating - a.rating).slice(0, n);
export const getCollections = () => cat().collections.map(c => ({ key: c.key, items: getNovels(c.ids) }));
export const getCollection = (key: CollectionKey) => getNovels(cat().collections.find(c => c.key === key)?.ids ?? []);
export const getCategories = () => cat().categories;
export const getContentLangs = () => cat().contentLangs;
export const categoryName = (c: Category, lang: UiLang) => cat().categoryNames.get(c)?.[lang] ?? c;
export const collectionName = (key: CollectionKey, lang: UiLang) => cat().collectionNames.get(key)?.[lang] ?? { kicker: '', title: key };

export const matchesQuery = (n: Novel, q: string) => {
  const s = q.trim().toLowerCase();
  const catNames = Object.values(cat().categoryNames.get(n.cat) ?? {}).join(' ');
  return !s || `${n.title} ${n.author} ${catNames}`.toLowerCase().includes(s);
};

export const searchNovels = (q: string) => (q.trim() ? allNovels().filter(n => matchesQuery(n, q)) : []);

export type ListSource =
  | { type: 'all' }
  | { type: 'category'; value: Category }
  | { type: 'author'; value: string }
  | { type: 'collection'; value: CollectionKey };

export type SortKey = 'Popular' | 'Newest' | 'Rating';

export interface ListFilter {
  query: string;
  status: 'All' | NovelStatus;
  sort: SortKey;
  /** Only used when browsing all novels. */
  cat: 'All' | Category;
}

const SORTERS: Record<SortKey, (a: Novel, b: Novel) => number> = {
  Popular: (a, b) => b.reads - a.reads,
  Newest: (a, b) => b.year - a.year,
  Rating: (a, b) => b.rating - a.rating
};

export function listNovels(src: ListSource, f: ListFilter): Novel[] {
  const all = allNovels();
  let base: Novel[];
  switch (src.type) {
    case 'category': base = all.filter(n => n.cat === src.value); break;
    case 'author': base = all.filter(n => n.author === src.value); break;
    case 'collection': base = getCollection(src.value); break;
    default: base = f.cat === 'All' ? all : all.filter(n => n.cat === f.cat);
  }
  return base
    .filter(n => (f.status === 'All' || n.status === f.status) && matchesQuery(n, f.query))
    .sort(SORTERS[f.sort]);
}

export const sameCategory = (n: Novel) => allNovels().filter(x => x.cat === n.cat && x.id !== n.id);
export const sameAuthor = (n: Novel) => allNovels().filter(x => x.author === n.author && x.id !== n.id);

/** Released chapters with titles in `lang`, falling back to the original text's title. */
export async function getChapterList(novelId: number, lang: ContentLang | null): Promise<ChapterInfo[]> {
  const rows = must(await supabase.from('chapters')
    .select('number, chapter_translations(lang, source, title, word_count)')
    .eq('novel_id', novelId)
    .order('number'), 'chapters');
  return rows.map(r => {
    const tr = r.chapter_translations;
    const t = tr.find(x => x.lang === lang) ?? tr.find(x => x.source === 'original') ?? tr[0];
    return { number: r.number, title: t?.title ?? '', words: t?.word_count ?? 0 };
  });
}

/** One chapter's text in `lang`, or null when that language has no published text. */
export async function getChapterText(novelId: number, number: number, lang: ContentLang): Promise<ChapterText | null> {
  const { data: t, error } = await supabase.from('chapter_translations')
    .select('title, content, chapters!inner(novel_id, number)')
    .eq('lang', lang)
    .eq('chapters.novel_id', novelId)
    .eq('chapters.number', number)
    .maybeSingle();
  if (error) throw new Error(`Loading chapter failed: ${error.message}`);
  if (!t) return null;
  return { title: t.title, paragraphs: t.content.split(/\n\s*\n/).map(p => p.trim()).filter(Boolean) };
}

/** A novel's glossary: term key → name in each content language. */
export async function getGlossary(novelId: number): Promise<Record<string, GlossaryTerm>> {
  const rows = must(await supabase.from('glossary_entries').select('term_key, lang, value').eq('novel_id', novelId), 'glossary');
  const out: Record<string, GlossaryTerm> = {};
  for (const r of rows) (out[r.term_key] ??= {})[r.lang as ContentLang] = r.value;
  return out;
}

// Replace rules belong to the reader. Signed in to Supabase they live in `replace_rules`
// (row level security keeps them private); otherwise they stay on this device.
const rulesKey = (novelId: number, lang: ContentLang) => `replaceRules.${novelId}.${lang}`;

async function signedInUserId(): Promise<string | null> {
  const { data } = await supabase.auth.getSession();
  return data.session?.user.id ?? null;
}

/** The reader's replace rules for a novel in one language, in the order they apply. */
export async function getReplaceRules(novelId: number, lang: ContentLang): Promise<ReplaceRule[]> {
  if (!(await signedInUserId())) return load<ReplaceRule[]>(rulesKey(novelId, lang), []);
  const rows = must(await supabase.from('replace_rules')
    .select('find, replace')
    .eq('novel_id', novelId)
    .eq('lang', lang)
    .order('position'), 'replace rules');
  return rows;
}

/** Replaces the reader's whole rule list for a novel in one language. */
export async function saveReplaceRules(novelId: number, lang: ContentLang, rules: ReplaceRule[]): Promise<void> {
  if (!(await signedInUserId())) return save(rulesKey(novelId, lang), rules.length ? rules : null);
  const del = await supabase.from('replace_rules').delete().eq('novel_id', novelId).eq('lang', lang);
  if (del.error) throw new Error(`Saving replace rules failed: ${del.error.message}`);
  if (!rules.length) return;
  const ins = await supabase.from('replace_rules')
    .insert(rules.map((r, position) => ({ novel_id: novelId, lang, find: r.find, replace: r.replace, position })));
  if (ins.error) throw new Error(`Saving replace rules failed: ${ins.error.message}`);
}

/** Applies each rule to `text` in order, replacing every occurrence (case-sensitive, plain text). */
export const applyReplaceRules = (text: string, rules: ReplaceRule[]) =>
  rules.reduce((s, r) => (r.find ? s.split(r.find).join(r.replace) : s), text);
