import { invoke } from '@tauri-apps/api/core';
import type { HistoryEntry, Novel } from '../data/types';
import { canOpenLocalBooks } from './localBook';
import { load, save } from './storage';

/**
 * Favourites and history of online novels, with a snapshot of each novel so they
 * show without a connection. The native app keeps them in the user's SQLite library
 * file; the web build keeps the snapshots in localStorage next to the lists.
 */
export interface ReadingState {
  favs: number[];
  /** Most recent first. */
  history: HistoryEntry[];
  /** `data` is null for a novel not seen in the catalog yet. */
  novels: { id: number; data: Novel | null }[];
}

const CACHE_KEY = 'novelCache';

/** The saved state, or null when nothing was saved for this user yet. */
export async function loadReading(user: string): Promise<ReadingState | null> {
  if (canOpenLocalBooks()) return invoke<ReadingState | null>('reading_load', { user });
  // The web build's lists already live in localStorage; only the snapshots are added.
  return { favs: load('favs', []), history: load('history', []), novels: load(CACHE_KEY, []) };
}

export async function saveReading(user: string, state: ReadingState): Promise<void> {
  if (canOpenLocalBooks()) return invoke<void>('reading_save', { user, state });
  save(CACHE_KEY, state.novels);
}
