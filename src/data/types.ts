export type UiLang = 'en' | 'vi';
export type ContentLang = 'en' | 'vi' | 'es';
export type NovelStatus = 'Ongoing' | 'Completed';
export type Category = 'Wuxia' | 'Fantasy' | 'Romance' | 'Mystery' | 'Sci-Fi' | 'Slice of Life';

export interface Novel {
  id: number;
  title: string;
  author: string;
  cat: Category;
  status: NovelStatus;
  chapters: number;
  rating: number;
  /** Thousands of reads. */
  reads: number;
  year: number;
  desc: Record<UiLang, string>;
  /** Placeholder cover palette until real art arrives. */
  cover: { bg: string; fg: string; deco: string };
}

export type CollectionKey = 'trend' | 'new' | 'done' | 'picks';

export interface Collection {
  key: CollectionKey;
  ids: number[];
}

export interface GlossaryTerm {
  en: string;
  vi: string;
  es: string;
}

export interface DictEntry {
  pos: string;
  def: string;
  ex: string;
}

export type TranslateStyle = 'Literal' | 'Natural' | 'Literary' | 'Casual';

export interface User {
  name: string;
  email: string;
  provider: string;
}

export interface HistoryEntry {
  id: number;
  ch: number;
  /** Epoch ms of the last read. */
  at: number;
}
