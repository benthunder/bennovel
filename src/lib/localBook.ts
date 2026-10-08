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

/**
 * Android turns the extension filter into media types and greys out every file whose
 * type it doesn't know (MOBI, FB2, DjVu…), so there the picker shows all files and the
 * app tells the format from the file itself.
 */
export const isAndroid = () => /android/i.test(navigator.userAgent);

/** iPhone and iPad have no folder picker; there the chapter files are selected instead. */
export const canPickFolder = () => !/iphone|ipad/i.test(navigator.userAgent);

const pickBookFile = () =>
  open({ multiple: false, filters: isAndroid() ? undefined : [{ name: 'Books', extensions: BOOK_EXTENSIONS }] });

/** Opens a book file by path or `file:`/`content:` URL, without saving it. */
export const openBookFile = (path: string) => invoke<LocalBook>('book_open', { path });

/**
 * Asks for a book file and opens it. Resolves to null when the user cancels. The path
 * is kept so the book can still be imported from the reader.
 */
export async function pickLocalBook(): Promise<{ book: LocalBook; path: string } | null> {
  const path = await pickBookFile();
  if (!path) return null;
  return { book: await openBookFile(path), path };
}

/** Book files the system asked the app to open ("Open with"), oldest first. */
export const takeOpenedFiles = () => invoke<string[]>('take_opened_files');

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
  /** Language guessed from the text: en, vi, zh or ko. */
  lang: string;
  index: number;
  progress: number;
}

/** The latest progress save, so the list read after leaving the reader includes it. */
let lastSave: Promise<unknown> = Promise.resolve();

export const listLibrary = (user: string) =>
  lastSave.catch(() => undefined).then(() => invoke<LibraryBook[]>('library_list', { user }));

export type ImportProgress = (done: number, total: number) => void;

/** Runs an import command, passing each stored chapter to `onProgress`. */
async function withProgress<T>(onProgress: ImportProgress | undefined, run: () => Promise<T>): Promise<T> {
  const { listen } = await import('@tauri-apps/api/event');
  const stop = await listen<{ done: number; total: number }>('library-import', e => onProgress?.(e.payload.done, e.payload.total));
  try {
    return await run();
  } finally {
    stop();
  }
}

/** Copies a book file into the library chapter by chapter. */
export const importBookPath = (user: string, path: string, onProgress?: ImportProgress) =>
  withProgress(onProgress, () => invoke<LibraryBook>('library_import', { user, path }));

/** Asks for a book file and imports it. Resolves to null when the user cancels. */
export async function importLocalBook(user: string, onProgress?: ImportProgress): Promise<LibraryBook | null> {
  const path = await pickBookFile();
  return path ? importBookPath(user, path, onProgress) : null;
}

/**
 * Imports a folder (with its subfolders) of chapter files as one book, a file per
 * chapter, ordered by the chapter number in each file name. On Android the folder is
 * picked and walked by the app (`library_import_android_folder`); iOS has no folder
 * picker, so there the chapter files are selected. Resolves to null when cancelled.
 */
export async function importLocalFolder(user: string, onProgress?: ImportProgress): Promise<LibraryBook | null> {
  if (isAndroid()) {
    return withProgress(onProgress, () => invoke<LibraryBook | null>('library_import_android_folder', { user }));
  }
  let paths: string[];
  if (!canPickFolder()) {
    paths = (await open({ multiple: true, filters: [{ name: 'Books', extensions: BOOK_EXTENSIONS }] })) ?? [];
  } else {
    const dir = await open({ directory: true, multiple: false });
    paths = dir ? [dir] : [];
  }
  if (!paths.length) return null;
  return withProgress(onProgress, () => invoke<LibraryBook>('library_import_folder', { user, paths }));
}

export async function openLibraryBook(user: string, novelId: number): Promise<{ book: LocalBook; place: LibraryPlace }> {
  const r = await invoke<{ book: LocalBook; novelId: number; lang: string; index: number; progress: number }>('library_open', { user, novelId });
  return { book: r.book, place: { user, novelId, lang: r.lang, index: r.index, progress: r.progress } };
}

export function saveLibraryProgress(user: string, novelId: number, index: number, progress: number) {
  const p = invoke<void>('library_save_progress', { user, novelId, index, progress });
  lastSave = p;
  return p;
}

export const deleteLibraryBook = (user: string, novelId: number) => invoke<void>('library_delete', { user, novelId });

/* ---------- Covers for imported books ---------- */

const COVER_PALETTES = ['accent', 'accent-2'] as const;
const COVER_SHADES = [[200, 800, 400], [300, 900, 500], [100, 700, 300]] as const;

/** A placeholder cover like the online novels have, picked from the title so it stays the same. */
export function localCover(title: string) {
  let h = 0;
  for (const c of title) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const pal = COVER_PALETTES[h % COVER_PALETTES.length];
  const [bg, fg, deco] = COVER_SHADES[(h >>> 3) % COVER_SHADES.length];
  return { bg: `var(--color-${pal}-${bg})`, fg: `var(--color-${pal}-${fg})`, deco: `var(--color-${pal}-${deco})` };
}
