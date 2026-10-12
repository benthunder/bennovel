import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { load, save } from '../lib/storage';
import type { Category, CollectionKey, ContentLang, HistoryEntry, User } from '../data/types';
import { findNovel, rememberNovels, type ListSource } from '../data/repository';
import { useCatalog } from '../data/useCatalog';
import { useT } from '../i18n';
import { canOpenLocalBooks, libraryUser, type LibraryPlace, type LocalBook } from '../lib/localBook';
import { loadReading, saveReading } from '../lib/readingStore';
import { getSyncStatus, syncLibrary, SyncFailure } from '../lib/sync';
import { canDownload, deleteDownload, downloadChapter, downloadLang, downloadNovel, listDownloads, setDownloads, type Download } from '../lib/offline';
import { claimGuestLibrary, listenForSignInLinks, signOut, takeOAuthProvider, takeRecovery, toUser } from '../lib/auth';
import { supabase } from '../lib/supabase';

export type Tab = 'home' | 'category' | 'library' | 'profile';

export type Route =
  /** `recovery`: signed in from a password reset email, so a new password is asked for. */
  | { s: 'login'; recovery?: boolean }
  | { s: 'home' }
  | { s: 'detail'; id: number }
  | { s: 'list'; src: ListSource }
  | { s: 'library' }
  | { s: 'profile' }
  | { s: 'reader'; id: number; ch: number; lang: ContentLang }
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

const tabRoot = (t: Tab): Route => (t === 'category' ? { s: 'list', src: { type: 'all' } } : { s: t });

function useStored<T>(key: string, init: T | (() => T)) {
  const [v, setV] = useState<T>(() => load(key, typeof init === 'function' ? (init as () => T)() : init));
  useEffect(() => save(key, v), [key, v]);
  return [v, setV] as const;
}

/**
 * Keeps favourites and history in the user's reading store (SQLite in the native app)
 * with a snapshot of each novel, so they show offline. Loads when the user changes and
 * after a sync (`reload`); a user with nothing saved yet starts from the current lists.
 */
function useReadingStore(
  user: string,
  favs: number[], setFavs: (v: number[]) => void,
  history: HistoryEntry[], setHistory: (v: HistoryEntry[]) => void,
  status: string
) {
  const loadedFor = useRef<string | null>(null);
  /** Version of the lists as loaded; saves from older lists are refused after a sync. */
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

function useAppState() {
  const catalog = useCatalog();
  const tr = useT();
  const [user, setUser] = useStored<User | null>('user', null);
  const [favs, setFavs] = useStored<number[]>('favs', [2, 11, 8]);
  const [history, setHistory] = useStored<HistoryEntry[]>('history', seedHistory);
  /** Reading language saved on the device. null = ask on each new chapter. */
  const [contentLang, setContentLang] = useStored<ContentLang | null>('defaultLang', null);
  const [readerPrefs, setReaderPrefs] = useStored<ReaderPrefs>('readerPrefs', DEFAULT_READER_PREFS);
  const [localLast, setLocalLast] = useStored<LocalLast | null>('localLast', null);
  const reloadReading = useReadingStore(libraryUser(user), favs, setFavs, history, setHistory, catalog.status);

  // The app opens on the library (books on the device and saved novels).
  const [stack, setStack] = useState<Route[]>(() => [load('onboarded', false) || user ? { s: 'library' } : { s: 'login' }]);
  const [tab, setTab] = useState<Tab | null>(() => (stack[0].s === 'library' ? 'library' : null));
  const [langDialog, setLangDialog] = useState<LangDialog | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const showToast = useCallback((msg: string) => {
    clearTimeout(toastTimer.current);
    setToast(msg);
    toastTimer.current = setTimeout(() => setToast(null), 2200);
  }, []);

  const push = useCallback((r: Route) => setStack(s => [...s, r]), []);
  const back = useCallback(() => setStack(s => (s.length > 1 ? s.slice(0, -1) : s)), []);
  const switchTab = useCallback((t: Tab) => { setTab(t); setStack([tabRoot(t)]); }, []);

  const downloads = useDownloads(libraryUser(user), showToast, tr);

  // Chapters come from the server, so a novel's page needs a connection unless it was downloaded.
  const openDetail = useCallback((id: number) => {
    // While the catalog is still loading the page waits for it (see App).
    if (catalog.status === 'offline' && !downloads.byId.has(id)) showToast(tr('load.needsConnection'));
    else push({ s: 'detail', id });
  }, [catalog.status, downloads.byId, push, showToast, tr]);
  const openCategory = useCallback((value: Category) => push({ s: 'list', src: { type: 'category', value } }), [push]);
  const openAuthor = useCallback((value: string) => push({ s: 'list', src: { type: 'author', value } }), [push]);
  const openCollection = useCallback((value: CollectionKey) => push({ s: 'list', src: { type: 'collection', value } }), [push]);

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

  /** After signing in: the guest's library goes to the account the first time, then back to where the user was. */
  const signedIn = useCallback(async (u: User, welcome: string) => {
    await claimGuestLibrary(u);
    setUser(u);
    save('onboarded', true);
    setStack(s => (s.length > 1 ? s.slice(0, -1) : [{ s: 'library' }]));
    setTab(t => t ?? 'library');
    showToast(welcome);
  }, [setUser, showToast]);

  /** Back to a guest; the account's lists stay in its own library file. */
  const resetToGuest = useCallback(() => {
    setFavs([]);
    setHistory([]);
    setUser(null);
  }, [setFavs, setHistory, setUser]);

  const userRef = useRef(user);
  userRef.current = user;
  // Auth listeners are registered once; they call the latest callbacks.
  const signedInRef = useRef(signedIn);
  signedInRef.current = signedIn;
  const trRef = useRef(tr);
  trRef.current = tr;
  useEffect(() => {
    const { data } = supabase.auth.onAuthStateChange((event, session) => {
      if (event === 'SIGNED_OUT') resetToGuest();
      else if (event === 'PASSWORD_RECOVERY' || (event === 'INITIAL_SESSION' && takeRecovery())) {
        takeRecovery();
        // Opened from the reset email (link or code): ask for the new password.
        const route: Route = { s: 'login', recovery: true };
        setStack(s => (s[s.length - 1].s === 'login' ? [...s.slice(0, -1), route] : [...s, route]));
      } else if (event === 'INITIAL_SESSION' && !session) {
        // Without a session only sign-ins saved before real accounts existed are dropped:
        // an expired session that can't be refreshed offline still counts as signed in.
        setUser(u => (u && !u.id ? null : u));
      } else if (session && (event === 'INITIAL_SESSION' || event === 'USER_UPDATED')) {
        // Back from Google or an email link (web): take the session like a normal sign-in.
        if (event === 'INITIAL_SESSION' && !userRef.current) {
          const u = toUser(session.user, takeOAuthProvider() ?? 'email');
          signedInRef.current(u, trRef.current('auth.welcome', { name: u.name.split(' ')[0] }));
        } else setUser(u => (u?.id === session.user.id ? { ...toUser(session.user), provider: u.provider } : u));
      }
    });
    return () => data.subscription.unsubscribe();
  }, [resetToGuest, setUser]);

  // Native app: Google sends the reader back through a bennovel:// link.
  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    let stop: (() => void) | undefined;
    let gone = false;
    listenForSignInLinks(
      u => signedInRef.current(u, trRef.current('auth.welcome', { name: u.name.split(' ')[0] })),
      key => showToast(trRef.current(key))
    ).then(un => { if (gone) un(); else stop = un; }, console.error);
    return () => { gone = true; stop?.(); };
  }, []);

  const skipLogin = useCallback(() => {
    save('onboarded', true);
    if (stack.length > 1) back();
    else switchTab('library');
  }, [stack.length, back, switchTab]);

  const logout = useCallback(() => {
    resetToGuest();
    signOut();
  }, [resetToGuest]);

  const toggleFav = useCallback((id: number) => {
    const was = favs.includes(id);
    setFavs(f => (was ? f.filter(x => x !== id) : [...f, id]));
    return !was;
  }, [favs, setFavs]);

  const [syncing, setSyncing] = useState(false);
  /** Epoch ms of this library's last sync, null before the first. */
  const [syncedAt, setSyncedAt] = useState<number | null>(null);
  /** Bumped after a sync, so screens list the library books again. */
  const [libraryRev, setLibraryRev] = useState(0);
  /** Sync needs an account and the library file of the native app. */
  const canSync = !!user?.id && canOpenLocalBooks();
  const libKey = libraryUser(user);
  useEffect(() => {
    setSyncedAt(null);
    if (!canSync) return;
    let gone = false;
    getSyncStatus(libKey).then(s => { if (!gone) setSyncedAt(s.syncedAt); }, console.error);
    return () => { gone = true; };
  }, [canSync, libKey]);

  const syncNow = useCallback(async () => {
    if (!canSync || !user?.id || syncing) return;
    setSyncing(true);
    try {
      setSyncedAt(await syncLibrary(libKey, user.id));
      showToast(tr('sync.done'));
    } catch (e) {
      showToast(tr(e instanceof SyncFailure ? e.key : 'sync.err.generic'));
    } finally {
      // A sync that failed after merging still changed the lists here.
      reloadReading();
      setLibraryRev(n => n + 1);
      setSyncing(false);
    }
  }, [canSync, user?.id, syncing, libKey, showToast, tr, reloadReading]);

  const markLocalRead = useCallback((e: Omit<LocalLast, 'at'>) => setLocalLast({ ...e, at: Date.now() }), [setLocalLast]);
  const forgetLocal = useCallback((novelId: number) => setLocalLast(l => (l?.novelId === novelId ? null : l)), [setLocalLast]);

  const last = history[0] ?? null;
  /** The imported book, when it was read more recently than any online novel. */
  const lastLocal = localLast && localLast.user === libraryUser(user) && (!last || localLast.at >= last.at) ? localLast : null;

  return {
    user, favs, history, contentLang, readerPrefs, stack, tab, langDialog, toast, last, lastLocal,
    top: stack[stack.length - 1], canBack: stack.length > 1,
    push, back, switchTab, openDetail, openCategory, openAuthor, openCollection,
    openChapter, goReader, setReaderLang, setContentLang, setLangDialog, setReaderPrefs,
    signedIn, skipLogin, logout, toggleFav, showToast, markLocalRead, forgetLocal,
    canSync, syncing, syncedAt, libraryRev, syncNow, downloads
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
