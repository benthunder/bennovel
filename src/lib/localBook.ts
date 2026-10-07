import { convertFileSrc, invoke, isTauri } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

/** Opening local files needs the native app; the web build hides the option. */
export const canOpenLocalBooks = () => isTauri();

export interface LocalSectionMeta {
  /** Title from the book's table of contents, if any. */
  label: string | null;
  /** First and last page (1-based) for PDF sections. */
  pages: [number, number] | null;
}

export interface LocalBook {
  id: number;
  format: 'epub' | 'pdf' | 'txt' | 'html' | 'mht' | 'markdown' | 'fb2' | 'docx' | 'odt' | 'rtf' | 'mobi' | 'chm' | 'djvu';
  title: string | null;
  sections: LocalSectionMeta[];
  /** Whole pages can be shown as pictures (PDF, DjVu). */
  pageImages: boolean;
}

export interface LocalSection {
  index: number;
  paragraphs: string[];
}

/** File types the reader opens (UMD is listed so the app can say it is not supported yet). */
export const BOOK_EXTENSIONS = [
  'epub', 'pdf', 'txt', 'mobi', 'azw3', 'azw', 'prc', 'fb2', 'djvu', 'chm', 'umd',
  'docx', 'odt', 'rtf', 'html', 'htm', 'xhtml', 'mht', 'mhtml', 'md', 'markdown'
];

/** Asks for a book file and opens it. Resolves to null when the user cancels. */
export async function pickLocalBook(): Promise<LocalBook | null> {
  const path = await open({ multiple: false, filters: [{ name: 'Books', extensions: BOOK_EXTENSIONS }] });
  if (!path) return null;
  return invoke<LocalBook>('book_open', { path });
}

/** A paragraph that is this character followed by a key stands for a picture. */
export const IMAGE_MARK = '\uFFFC';

export const imageKey = (paragraph: string) => (paragraph.startsWith(IMAGE_MARK) ? paragraph.slice(1) : null);

/**
 * Pictures are served by the app's `bookimg:` scheme and read from the book only when
 * the <img> is about to be shown, so they never sit in memory with the text.
 */
export const bookImageUrl = (id: number, key: string) => convertFileSrc(`${id}/i/${key}`, 'bookimg');

/** Page `page` (1-based) of a PDF/DjVu drawn `width` pixels wide. */
export const bookPageUrl = (id: number, page: number, width: number) => convertFileSrc(`${id}/p/${page}/${width}`, 'bookimg');

/** Text of one section; the app keeps only the sections around it in memory. */
export const getLocalSection = (id: number, index: number) => invoke<LocalSection>('book_section', { id, index });

export const closeLocalBook = (id: number) => invoke<void>('book_close', { id });

/* ---------- On-device library (one SQLite file per user, see src-tauri/src/library) ---------- */

/** Key of the library file: the signed-in email, or '' for a guest. */
export const libraryUser = (user: { email: string } | null) => user?.email ?? '';

export interface LibraryBook {
  novelId: number;
  title: string;
  /** Language guessed from the text: en, vi, zh or ko. */
  lang: string;
  format: LocalBook['format'] | null;
  sourceName: string | null;
  chapterCount: number;
  createdAt: string;
  lastRead: { index: number; progress: number; lastReadAt: string } | null;
}

/** Where a library book is read from and where to resume it. */
export interface LibraryPlace {
  user: string;
  novelId: number;
  index: number;
  progress: number;
}

/** The latest progress save, so the list read after leaving the reader includes it. */
let lastSave: Promise<unknown> = Promise.resolve();

export const listLibrary = (user: string) =>
  lastSave.catch(() => undefined).then(() => invoke<LibraryBook[]>('library_list', { user }));

/**
 * Asks for a book file and copies it into the library chapter by chapter.
 * `onProgress` gets each stored chapter. Resolves to null when the user cancels.
 */
export async function importLocalBook(user: string, onProgress?: (done: number, total: number) => void): Promise<LibraryBook | null> {
  const path = await open({ multiple: false, filters: [{ name: 'Books', extensions: BOOK_EXTENSIONS }] });
  if (!path) return null;
  const { listen } = await import('@tauri-apps/api/event');
  const stop = await listen<{ done: number; total: number }>('library-import', e => onProgress?.(e.payload.done, e.payload.total));
  try {
    return await invoke<LibraryBook>('library_import', { user, path });
  } finally {
    stop();
  }
}

export async function openLibraryBook(user: string, novelId: number): Promise<{ book: LocalBook; place: LibraryPlace }> {
  const r = await invoke<{ book: LocalBook; novelId: number; index: number; progress: number }>('library_open', { user, novelId });
  return { book: r.book, place: { user, novelId, index: r.index, progress: r.progress } };
}

export function saveLibraryProgress(user: string, novelId: number, index: number, progress: number) {
  const p = invoke<void>('library_save_progress', { user, novelId, index, progress });
  lastSave = p;
  return p;
}

export const deleteLibraryBook = (user: string, novelId: number) => invoke<void>('library_delete', { user, novelId });
