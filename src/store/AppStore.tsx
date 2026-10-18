import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { load, save } from '../lib/storage';
import type { ContentLang, User } from '../data/types';
import { useI18n, useT } from '../i18n';
import { aiErrorKey, resumeAiQueue, type AiProgress } from '../lib/ai';
import { canOpenLocalBooks, libraryUser, type LibraryPlace, type LocalBook } from '../lib/localBook';
import { runOcr } from '../lib/ocr';

export type Tab = 'library' | 'profile';

export type Route =
  | { s: 'library' }
  | { s: 'profile' }
  /** A book in the on-device library. */
  | { s: 'bookDetail'; novelId: number }
  /** The wiki of a book in the on-device library. */
  | { s: 'bookWiki'; novelId: number; title: string; lang?: string }
  /** `path`: a file opened without saving, which can still be imported from the reader. */
  | { s: 'book'; book: LocalBook; place?: LibraryPlace; path?: string };

export type ReaderTheme = 'cream' | 'paper' | 'sage' | 'night' | 'eink';
export interface ReaderPrefs {
  fontSize: number;
  theme: ReaderTheme;
  lineH: number;
  gap: 'tight' | 'normal' | 'airy';
  margin: 'narrow' | 'normal' | 'wide';
  align: 'left' | 'justify';
}
export interface ReaderThemeStyle {
  /** CSS background (may include a texture). */
  bg: string;
  fg: string;
  /** Text face for the chapter, when the theme has its own. */
  font?: string;
  /** Design tokens overridden inside the reader (the e-ink theme drops the colour accent). */
  vars?: Record<string, string>;
  /** No motion: e-ink screens don't animate. */
  still?: boolean;
}

/** Faint grain so the e-ink page reads as paper rather than a flat grey. */
const PAPER_GRAIN = `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='180' height='180'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix values='0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 .09 0 0 0 0'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E")`;

export const READER_THEMES: Record<ReaderTheme, ReaderThemeStyle> = {
  cream: { bg: 'var(--color-bg)', fg: 'var(--color-text)' },
  paper: { bg: 'var(--color-neutral-100)', fg: 'var(--color-text)' },
  sage: { bg: 'var(--color-accent-2-200)', fg: 'var(--color-accent-2-900)' },
  night: { bg: 'var(--color-neutral-900)', fg: 'var(--color-neutral-200)' },
  eink: {
    bg: `${PAPER_GRAIN} #ecebe6`,
    fg: '#1c1c1c',
    font: "Literata, Georgia, 'Noto Serif', 'Times New Roman', serif",
    still: true,
    vars: {
      '--color-accent': '#2a2a2a', '--color-accent-600': '#3a3a3a', '--color-accent-700': '#3a3a3a', '--color-accent-800': '#1c1c1c',
      '--color-accent-400': '#6b6b6b', '--color-accent-200': '#d4d2cb', '--color-accent-100': '#e0dfd9'
    }
  }
};

export const DEFAULT_READER_PREFS: ReaderPrefs = { fontSize: 18, theme: 'cream', lineH: 1.75, gap: 'normal', margin: 'normal', align: 'left' };

/** The imported book read last, so Continue can reopen it. */
export interface LocalLast {
  user: string;
  novelId: number;
  title: string;
  index: number;
  total: number;
  /** Epoch ms of the last read. */
  at: number;
}

/** Search fires at most once per this window while typing. */
export const SEARCH_THROTTLE_MS = 400;

const tabRoot = (t: Tab): Route => ({ s: t });

function useStored<T>(key: string, init: T | (() => T)) {
  const [v, setV] = useState<T>(() => load(key, typeof init === 'function' ? (init as () => T)() : init));
  useEffect(() => save(key, v), [key, v]);
  return [v, setV] as const;
}

/**
 * The background AI translation of library books (lib/ai.ts): picks up a queue left
 * by the last run, tells when a book is done or the queue paused, and returns a
 * counter bumped on every change so screens reload their progress.
 */
function useAiQueue(user: string, showToast: (msg: string) => void) {
  const { t, uiLang } = useI18n();
  const [rev, setRev] = useState(0);
  const tRef = useRef(t);
  tRef.current = t;
  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    resumeAiQueue(user, uiLang).catch(console.error);
  }, [user, uiLang]);
  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    let gone = false;
    const stops: (() => void)[] = [];
    void import('@tauri-apps/api/event').then(async ({ listen }) => {
      const mine = (p: { user: string }) => p.user === user;
      const all = await Promise.all([
        listen<AiProgress & { user: string }>('ai-progress', e => { if (mine(e.payload)) setRev(n => n + 1); }),
        listen<AiProgress & { user: string; title: string }>('ai-done', e => {
          if (mine(e.payload)) showToast(tRef.current('ai.done', { title: e.payload.title, n: e.payload.done }));
        }),
        listen<{ user: string; error: string }>('ai-paused', e => {
          if (!mine(e.payload)) return;
          setRev(n => n + 1);
          showToast(tRef.current('ai.paused', { reason: tRef.current(aiErrorKey(e.payload.error)) }));
        })
      ]);
      if (gone) all.forEach(stop => stop());
      else stops.push(...all);
    });
    return () => { gone = true; stops.forEach(stop => stop()); };
  }, [user, showToast]);
  return rev;
}

function useAppState() {
  const tr = useT();
  // No accounts: the library is the device's own. A library made while signed in to an
  // earlier version stays the one used, so its books don't disappear.
  const [libKey] = useState(() => libraryUser(load<User | null>('user', null)));
  /** Reading language saved on the device. null = ask on each new book. */
  const [contentLang, setContentLang] = useStored<ContentLang | null>('defaultLang', null);
  const [readerPrefs, setReaderPrefs] = useStored<ReaderPrefs>('readerPrefs', DEFAULT_READER_PREFS);
  const [localLast, setLocalLast] = useStored<LocalLast | null>('localLast', null);

  // The app opens on the library (books on the device).
  const [stack, setStack] = useState<Route[]>([{ s: 'library' }]);
  const [tab, setTab] = useState<Tab | null>('library');
  const [langDialogOpen, setLangDialogOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const showToast = useCallback((msg: string) => {
    clearTimeout(toastTimer.current);
    setToast(msg);
    toastTimer.current = setTimeout(() => setToast(null), 2200);
  }, []);
  // Started again after a backup brought settings back (see ProfileScreen).
  useEffect(() => {
    if (!load('restoredNotice', false)) return;
    save('restoredNotice', null);
    showToast(tr('backup.restored'));
  }, [showToast, tr]);

  const push = useCallback((r: Route) => setStack(s => [...s, r]), []);
  const back = useCallback(() => setStack(s => (s.length > 1 ? s.slice(0, -1) : s)), []);
  const switchTab = useCallback((t: Tab) => { setTab(t); setStack([tabRoot(t)]); }, []);

  // Scanned pages left unread by an earlier run (the app was closed) are picked up again.
  useEffect(() => runOcr(libKey), [libKey]);

  /** Bumped after a restore, so screens list the library books again. */
  const [libraryRev, setLibraryRev] = useState(0);
  const aiRev = useAiQueue(libKey, showToast);
  /** Lists everything again after the library file changed (a backup brought back). */
  const reloadLibrary = useCallback(() => setLibraryRev(n => n + 1), []);

  const markLocalRead = useCallback((e: Omit<LocalLast, 'at'>) => setLocalLast({ ...e, at: Date.now() }), [setLocalLast]);
  const forgetLocal = useCallback((novelId: number) => setLocalLast(l => (l?.novelId === novelId ? null : l)), [setLocalLast]);

  const lastLocal = localLast && localLast.user === libKey ? localLast : null;

  return {
    libKey, contentLang, readerPrefs, stack, tab, langDialogOpen, toast, lastLocal,
    top: stack[stack.length - 1], canBack: stack.length > 1,
    push, back, switchTab,
    setContentLang, openLangDialog: () => setLangDialogOpen(true), closeLangDialog: () => setLangDialogOpen(false), setReaderPrefs,
    showToast, markLocalRead, forgetLocal,
    libraryRev, aiRev, reloadLibrary
  };
}

export type AppStore = ReturnType<typeof useAppState>;
const Ctx = createContext<AppStore | null>(null);

export function AppStoreProvider({ children }: { children: ReactNode }) {
  return <Ctx.Provider value={useAppState()}>{children}</Ctx.Provider>;
}

export function useApp() {
  const c = useContext(Ctx);
  if (!c) throw new Error('useApp must be used inside AppStoreProvider');
  return c;
}
