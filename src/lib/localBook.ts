import { invoke, isTauri } from '@tauri-apps/api/core';
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
  format: 'epub' | 'pdf';
  title: string | null;
  sections: LocalSectionMeta[];
}

export interface LocalSection {
  index: number;
  paragraphs: string[];
}

/** Asks for an EPUB/PDF and opens it. Resolves to null when the user cancels. */
export async function pickLocalBook(): Promise<LocalBook | null> {
  const path = await open({ multiple: false, filters: [{ name: 'EPUB / PDF', extensions: ['epub', 'pdf'] }] });
  if (!path) return null;
  return invoke<LocalBook>('book_open', { path });
}

/** Text of one section; the app keeps only the sections around it in memory. */
export const getLocalSection = (id: number, index: number) => invoke<LocalSection>('book_section', { id, index });

export const closeLocalBook = (id: number) => invoke<void>('book_close', { id });
