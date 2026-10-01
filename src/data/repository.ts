// Data access seam. Everything screens need from "the server" goes through here,
// so swapping the mock data for a real API later touches only this file.
import { CATEGORIES, COLLECTIONS, NOVELS, POPULAR_IDS } from './mock';
import type { Category, CollectionKey, Novel, NovelStatus } from './types';

const byId = new Map(NOVELS.map(n => [n.id, n]));

export const getNovel = (id: number): Novel => {
  const n = byId.get(id);
  if (!n) throw new Error(`Unknown novel ${id}`);
  return n;
};

export const getPopular = () => POPULAR_IDS.map(getNovel);
export const getCollections = () => COLLECTIONS.map(c => ({ key: c.key, items: c.ids.map(getNovel) }));
export const getCollection = (key: CollectionKey) => (COLLECTIONS.find(c => c.key === key)?.ids ?? []).map(getNovel);
export const getCategories = () => CATEGORIES;

export const matchesQuery = (n: Novel, q: string) => {
  const s = q.trim().toLowerCase();
  return !s || `${n.title} ${n.author} ${n.cat}`.toLowerCase().includes(s);
};

export const searchNovels = (q: string) => (q.trim() ? NOVELS.filter(n => matchesQuery(n, q)) : []);

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
  let base: Novel[];
  switch (src.type) {
    case 'category': base = NOVELS.filter(n => n.cat === src.value); break;
    case 'author': base = NOVELS.filter(n => n.author === src.value); break;
    case 'collection': base = getCollection(src.value); break;
    default: base = f.cat === 'All' ? NOVELS : NOVELS.filter(n => n.cat === f.cat);
  }
  return base
    .filter(n => (f.status === 'All' || n.status === f.status) && matchesQuery(n, f.query))
    .sort(SORTERS[f.sort]);
}

export const sameCategory = (n: Novel) => NOVELS.filter(x => x.cat === n.cat && x.id !== n.id);
export const sameAuthor = (n: Novel) => NOVELS.filter(x => x.author === n.author && x.id !== n.id);
