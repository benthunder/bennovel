import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { load, save } from '../lib/storage';
import type { Category, CollectionKey, ContentLang, HistoryEntry, User } from '../data/types';
import type { ListSource } from '../data/repository';
import { libraryUser, type LibraryPlace, type LocalBook } from '../lib/localBook';

export type Tab = 'home' | 'category' | 'library' | 'profile';

export type Route =
  | { s: 'login' }
  | { s: 'home' }
  | { s: 'detail'; id: number }
  | { s: 'list'; src: ListSource }
  | { s: 'library' }
  | { s: 'profile' }
  | { s: 'reader'; id: number; ch: number; lang: ContentLang }
  | { s: 'book'; book: LocalBook; place?: LibraryPlace };

export type LangDialog =
  | { mode: 'chapter'; id: number; ch: number }
  | { mode: 'reader' }
  | { mode: 'settings' };

export type ReaderTheme = 'cream' | 'paper' | 'sage' | 'night';
export interface ReaderPrefs {
  fontSize: number;
  theme: ReaderTheme;
  lineH: number;
  gap: 'tight' | 'normal' | 'airy';
  margin: 'narrow' | 'normal' | 'wide';
  align: 'left' | 'justify';
}
export const READER_THEMES: Record<ReaderTheme, { bg: string; fg: string }> = {
  cream: { bg: 'var(--color-bg)', fg: 'var(--color-text)' },
  paper: { bg: 'var(--color-neutral-100)', fg: 'var(--color-text)' },
  sage: { bg: 'var(--color-accent-2-200)', fg: 'var(--color-accent-2-900)' },
  night: { bg: 'var(--color-neutral-900)', fg: 'var(--color-neutral-200)' }
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

function useAppState() {
  const [user, setUser] = useStored<User | null>('user', null);
  const [favs, setFavs] = useStored<number[]>('favs', [2, 11, 8]);
  const [history, setHistory] = useStored<HistoryEntry[]>('history', seedHistory);
  /** Reading language saved on the device. null = ask on each new chapter. */
  const [contentLang, setContentLang] = useStored<ContentLang | null>('defaultLang', null);
  const [readerPrefs, setReaderPrefs] = useStored<ReaderPrefs>('readerPrefs', DEFAULT_READER_PREFS);
  const [localLast, setLocalLast] = useStored<LocalLast | null>('localLast', null);

  const [stack, setStack] = useState<Route[]>(() => [load('onboarded', false) || user ? { s: 'home' } : { s: 'login' }]);
  const [tab, setTab] = useState<Tab | null>(() => (stack[0].s === 'home' ? 'home' : null));
  const [langDialog, setLangDialog] = useState<LangDialog | null>(null);
  const [authLoading, setAuthLoading] = useState<string | null>(null);
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

  const openDetail = useCallback((id: number) => push({ s: 'detail', id }), [push]);
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
    if (contentLang) goReader(id, ch, contentLang);
    else setLangDialog({ mode: 'chapter', id, ch });
  }, [contentLang, goReader]);

  const setReaderLang = useCallback((lang: ContentLang) => {
    setStack(s => {
      const top = s[s.length - 1];
      return top.s === 'reader' ? [...s.slice(0, -1), { ...top, lang }] : s;
    });
  }, []);

  const login = useCallback((provider: string, name: string, email: string, welcome: string) => {
    setAuthLoading(provider);
    // Simulated network round-trip until the auth API exists.
    setTimeout(() => {
      setAuthLoading(null);
      setUser({ name, email, provider });
      save('onboarded', true);
      setStack(s => (s.length > 1 ? s.slice(0, -1) : [{ s: 'home' }]));
      setTab(t => t ?? 'home');
      showToast(welcome);
    }, 750);
  }, [setUser, showToast]);

  const skipLogin = useCallback(() => {
    save('onboarded', true);
    if (stack.length > 1) back();
    else switchTab('home');
  }, [stack.length, back, switchTab]);

  const logout = useCallback(() => setUser(null), [setUser]);

  const toggleFav = useCallback((id: number) => {
    const was = favs.includes(id);
    setFavs(f => (was ? f.filter(x => x !== id) : [...f, id]));
    return !was;
  }, [favs, setFavs]);

  const markLocalRead = useCallback((e: Omit<LocalLast, 'at'>) => setLocalLast({ ...e, at: Date.now() }), [setLocalLast]);
  const forgetLocal = useCallback((novelId: number) => setLocalLast(l => (l?.novelId === novelId ? null : l)), [setLocalLast]);

  const last = history[0] ?? null;
  /** The imported book, when it was read more recently than any online novel. */
  const lastLocal = localLast && localLast.user === libraryUser(user) && (!last || localLast.at >= last.at) ? localLast : null;

  return {
    user, favs, history, contentLang, readerPrefs, stack, tab, langDialog, authLoading, toast, last, lastLocal,
    top: stack[stack.length - 1], canBack: stack.length > 1,
    push, back, switchTab, openDetail, openCategory, openAuthor, openCollection,
    openChapter, goReader, setReaderLang, setContentLang, setLangDialog, setReaderPrefs,
    login, skipLogin, logout, toggleFav, showToast, markLocalRead, forgetLocal
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
