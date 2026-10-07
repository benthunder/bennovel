export type UiLang = 'en' | 'vi';
export type ContentLang = 'en' | 'vi' | 'es';
export type NovelStatus = 'Ongoing' | 'Completed';
/** Category slug, e.g. `wuxia`. */
export type Category = string;

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

/** Collection key, e.g. `trend`. */
export type CollectionKey = string;

export interface Collection {
  key: CollectionKey;
  ids: number[];
}

/** One glossary term's name in each content language. */
export type GlossaryTerm = Partial<Record<ContentLang, string>>;

export interface ContentLangInfo {
  code: ContentLang;
  name: string;
  native: string;
}

export interface ChapterInfo {
  number: number;
  title: string;
  words: number;
}

export interface ChapterText {
  title: string;
  /** Paragraphs; `[[key]]` marks a glossary term. */
  paragraphs: string[];
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
