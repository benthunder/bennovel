import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { load, save } from '../lib/storage';
import type { ContentLang, HistoryEntry, User } from '../data/types';
import { findNovel, rememberNovels } from '../data/repository';
import { useCatalog } from '../data/useCatalog';
import { useI18n, useT } from '../i18n';
import { aiErrorKey, resumeAiQueue, type AiProgress } from '../lib/ai';
import { canOpenLocalBooks, libraryUser, type LibraryPlace, type LocalBook } from '../lib/localBook';
import { runOcr } from '../lib/ocr';
import { loadReading, saveReading } from '../lib/readingStore';
import { canDownload, deleteDownload, downloadChapter, downloadLang, downloadNovel, listDownloads, setDownloads, type Download } from '../lib/offline';

export type Tab = 'library' | 'profile';

export type Route =
  | { s: 'detail'; id: number }
  | { s: 'library' }
  | { s: 'profile' }
  | { s: 'reader'; id: number; ch: number; lang: ContentLang }
  /** A book in the on-device library. */
  | { s: 'bookDetail'; novelId: number }
  /** The wiki of a book in the on-device library. */
  | { s: 'bookWiki'; novelId: number; title: string }
  /** `path`: a file opened without saving, which can still be imported from the reader. */
  | { s: 'book'; book: LocalBook; place?: LibraryPlace; path?: string };

export type LangDialog =
  | { mode: 'chapter'; id: number; ch: number }
  | { mode: 'reader' }
  | { mode: 'settings' };

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

const HOUR = 3600_000;
const seedHistory = (): HistoryEntry[] => {
  const now = Date.now();
  return [{ id: 1, ch: 12, at: now - 2 * HOUR }, { id: 3, ch: 4, at: now - 26 * HOUR }, { id: 5, ch: 17, at: now - 72 * HOUR }];
};

const tabRoot = (t: Tab): Route => ({ s: t });

function useStored<T>(key: string, init: T | (() => T)) {
  const [v, setV] = useState<T>(() => load(key, typeof init === 'function' ? (init as () => T)() : init));
  useEffect(() => save(key, v), [key, v]);
  return [v, setV] as const;
}

/**
 * Keeps favourites and history in the user's reading store (SQLite in the native app)
 * with a snapshot of each novel, so they show offline. Loads when the user changes and
 * after a backup was brought back (`reload`); a user with nothing saved yet starts from the current lists.
 */
function useReadingStore(
  user: string,
  favs: number[], setFavs: (v: number[]) => void,
  history: HistoryEntry[], setHistory: (v: HistoryEntry[]) => void,
  status: string
) {
  const loadedFor = useRef<string | null>(null);
  /** Version of the lists as loaded; saves from older lists are refused after a restore. */
  const rev = useRef(0);
  const [loads, setLoads] = useState(0);
  const [reloads, setReloads] = useState(0);
  const reload = useCallback(() => setReloads(n => n + 1), []);

  useEffect(() => {
    let gone = false;
    loadedFor.current = null;
    const done = () => {
      if (gone) return;
      loadedFor.current = user;
      setLoads(n => n + 1);
    };
    loadReading(user).then(state => {
      if (gone) return;
      rev.current = state?.rev ?? 0;
      if (state) {
        rememberNovels(state.novels);
        setFavs(state.favs);
        setHistory(state.history);
      }
      done();
    }, e => { console.error(e); done(); });
    return () => { gone = true; };
  }, [user, reloads]);

  // Saves after every change, and again once the catalog has loaded to refresh the snapshots.
  useEffect(() => {
    if (loadedFor.current !== user) return;
    const ids = [...new Set([...favs, ...history.map(h => h.id)])];
    const novels = ids.map(id => ({ id, data: findNovel(id) ?? null }));
    saveReading(user, { favs, history, novels, rev: rev.current })
      .then(saved => { if (saved === false) reload(); })
      .catch(console.error);
  }, [user, favs, history, status, loads, reload]);

  return reload;
}

/**
 * Online novels downloaded to read offline (native app): the list of `user`, the
 * progress of running downloads, and actions to start or remove one.
 */
function useDownloads(user: string, showToast: (msg: string) => void, tr: ReturnType<typeof useT>) {
  const [list, setList] = useState<Download[]>([]);
  const [progress, setProgress] = useState<Record<number, { done: number; total: number }>>({});

  const refresh = useCallback(async () => {
    try {
      const got = await listDownloads(user);
      setDownloads(user, got);
      rememberNovels(got);
      setList(got);
    } catch (e) {
      console.error(e);
    }
  }, [user]);
  useEffect(() => {
    setDownloads(user, []);
    setList([]);
    refresh();
  }, [refresh, user]);

  const start = useCallback(async (id: number) => {
    const novel = findNovel(id);
    if (!novel || progress[id]) return;
    setProgress(p => ({ ...p, [id]: { done: 0, total: novel.chapters } }));
    try {
      await downloadNovel(user, novel, (done, total) => setProgress(p => ({ ...p, [id]: { done, total } })));
      showToast(tr('download.done', { title: novel.title }));
    } catch (e) {
      console.error(e);
      showToast(tr('download.failed'));
    } finally {
      await refresh();
      setProgress(({ [id]: _, ...rest }) => rest);
    }
  }, [progress, refresh, showToast, tr, user]);

  /** Chapters downloading on their own, as `novelId:number`. */
  const [chapterBusy, setChapterBusy] = useState<string[]>([]);
  const startChapter = useCallback(async (id: number, number: number) => {
    const novel = findNovel(id), key = `${id}:${number}`;
    if (!novel || chapterBusy.includes(key)) return;
    setChapterBusy(b => [...b, key]);
    try {
      await downloadChapter(user, novel, number);
      showToast(tr('download.chapterDone', { n: number }));
    } catch (e) {
      console.error(e);
      showToast(tr('download.failed'));
    } finally {
      await refresh();
      setChapterBusy(b => b.filter(k => k !== key));
    }
  }, [chapterBusy, refresh, showToast, tr, user]);

  const remove = useCallback(async (id: number) => {
    try {
      await deleteDownload(user, id);
      showToast(tr('download.removed'));
    } catch (e) {
      console.error(e);
    }
    await refresh();
  }, [refresh, showToast, tr, user]);

  const byId = useMemo(() => new Map(list.map(d => [d.id, d])), [list]);
  return { enabled: canDownload(), user, list, byId, progress, start, chapterBusy, startChapter, remove };
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
  const catalog = useCatalog();
  const tr = useT();
  // No accounts: the library is the device's own. A library made while signed in to an
  // earlier version stays the one used, so its books don't disappear.
  const [libKey] = useState(() => libraryUser(load<User | null>('user', null)));
  const [favs, setFavs] = useStored<number[]>('favs', [2, 11, 8]);
  const [history, setHistory] = useStored<HistoryEntry[]>('history', seedHistory);
  /** Reading language saved on the device. null = ask on each new chapter. */
  const [contentLang, setContentLang] = useStored<ContentLang | null>('defaultLang', null);
  const [readerPrefs, setReaderPrefs] = useStored<ReaderPrefs>('readerPrefs', DEFAULT_READER_PREFS);
  const [localLast, setLocalLast] = useStored<LocalLast | null>('localLast', null);
  const reloadReading = useReadingStore(libKey, favs, setFavs, history, setHistory, catalog.status);

  // The app opens on the library (books on the device and saved novels).
  const [stack, setStack] = useState<Route[]>([{ s: 'library' }]);
  const [tab, setTab] = useState<Tab | null>('library');
  const [langDialog, setLangDialog] = useState<LangDialog | null>(null);
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

  const downloads = useDownloads(libKey, showToast, tr);
  // Scanned pages left unread by an earlier run (the app was closed) are picked up again.
  useEffect(() => runOcr(libKey), [libKey]);

  // Chapters come from the server, so a novel's page needs a connection unless it was downloaded.
  const openDetail = useCallback((id: number) => {
    // While the catalog is still loading the page waits for it (see App).
    if (catalog.status === 'offline' && !downloads.byId.has(id)) showToast(tr('load.needsConnection'));
    else push({ s: 'detail', id });
  }, [catalog.status, downloads.byId, push, showToast, tr]);

  const goReader = useCallback((id: number, ch: number, lang: ContentLang) => {
    const entry: Route = { s: 'reader', id, ch, lang };
    setHistory(h => [{ id, ch, at: Date.now() }, ...h.filter(x => x.id !== id)]);
    setStack(s => (s[s.length - 1].s === 'reader' ? [...s.slice(0, -1), entry] : [...s, entry]));
  }, [setHistory]);

  /** Opens a chapter in the saved language, or asks first when none is saved. */
  const openChapter = useCallback((id: number, ch: number) => {
    const dl = downloads.byId.get(id);
    // Offline, a download opens in a language it has (the language list needs a connection).
    const offlineLang = dl && catalog.status === 'offline' ? downloadLang(dl, contentLang) : null;
    if (offlineLang) goReader(id, ch, offlineLang);
    else if (catalog.status === 'offline') showToast(tr('load.needsConnection'));
    else if (contentLang) goReader(id, ch, contentLang);
    else setLangDialog({ mode: 'chapter', id, ch });
  }, [catalog.status, contentLang, downloads.byId, goReader, showToast, tr]);

  const setReaderLang = useCallback((lang: ContentLang) => {
    setStack(s => {
      const top = s[s.length - 1];
      return top.s === 'reader' ? [...s.slice(0, -1), { ...top, lang }] : s;
    });
  }, []);

  const toggleFav = useCallback((id: number) => {
    const was = favs.includes(id);
    setFavs(f => (was ? f.filter(x => x !== id) : [...f, id]));
    return !was;
  }, [favs, setFavs]);

  /** Bumped after a restore, so screens list the library books again. */
  const [libraryRev, setLibraryRev] = useState(0);
  const aiRev = useAiQueue(libKey, showToast);
  /** Lists everything again after the library file changed (a backup brought back). */
  const reloadLibrary = useCallback(() => { reloadReading(); setLibraryRev(n => n + 1); }, [reloadReading]);

  const markLocalRead = useCallback((e: Omit<LocalLast, 'at'>) => setLocalLast({ ...e, at: Date.now() }), [setLocalLast]);
  const forgetLocal = useCallback((novelId: number) => setLocalLast(l => (l?.novelId === novelId ? null : l)), [setLocalLast]);

  const last = history[0] ?? null;
  /** The imported book, when it was read more recently than any online novel. */
  const lastLocal = localLast && localLast.user === libKey && (!last || localLast.at >= last.at) ? localLast : null;

  return {
    libKey, favs, history, contentLang, readerPrefs, stack, tab, langDialog, toast, last, lastLocal,
    top: stack[stack.length - 1], canBack: stack.length > 1,
    push, back, switchTab, openDetail,
    openChapter, goReader, setReaderLang, setContentLang, setLangDialog, setReaderPrefs,
    toggleFav, showToast, markLocalRead, forgetLocal,
    libraryRev, downloads, aiRev, reloadLibrary
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
