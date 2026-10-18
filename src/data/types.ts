export type UiLang = 'en' | 'vi';
/** Content is offered in these languages only (imported books may be in others too). */
export const CONTENT_LANGS = ['en', 'vi', 'zh', 'ko'] as const;
export type ContentLang = (typeof CONTENT_LANGS)[number];

export interface ContentLangInfo {
  code: ContentLang;
  name: string;
  native: string;
}

/** Reading languages offered for AI translation and the default-language setting. */
export const CONTENT_LANG_INFO: ContentLangInfo[] = [
  { code: 'en', name: 'English', native: 'English' },
  { code: 'vi', name: 'Vietnamese', native: 'Tiếng Việt' },
  { code: 'zh', name: 'Chinese', native: '中文' },
  { code: 'ko', name: 'Korean', native: '한국어' }
];

/** One glossary term's name in each content language. */
export type GlossaryTerm = Partial<Record<ContentLang, string>>;

/** A reader's "find → replace" for one book in one language, applied to the chapter text shown. */
export interface ReplaceRule {
  find: string;
  replace: string;
}

export interface DictEntry {
  pos: string;
  def: string;
  ex: string;
}

export type TranslateStyle = 'Literal' | 'Natural' | 'Literary' | 'Casual';

export interface User {
  id?: string;
  name: string;
  email: string;
  provider: string;
}
