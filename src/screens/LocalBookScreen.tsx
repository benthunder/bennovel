import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { ChapterSheet, useSectionName } from '../components/ChapterSheet';
import { BookOpenIcon, ChevronLeftIcon, DownloadIcon, ListIcon, SlidersIcon, SparklesIcon } from '../components/Icons';
import { ReaderChrome, ReaderStatusBar, useImmersive } from '../components/ReaderChrome';
import { ReaderTools, type ToolsTab, type Translation } from '../components/ReaderTools';
import { Spinner } from '../components/ui';
import { WikiPopup, markWikiTerms, wikiTerms } from '../components/WikiPopup';
import { applyReplaceRules, getBookRules, saveBookRules } from '../data/repository';
import { CONTENT_LANGS, type ContentLang, type ReplaceRule } from '../data/types';
import { useI18n } from '../i18n';
import {
  AI_LANGS, AiFailure, aiErrorDetail, aiTranslateBook, aiTranslateChapter, aiViewKey, getAiChapter, getAiDraft, getAiMarks, getAiStatus,
  onAiMoved, onWatchLive, readingLangOf, styleOf,
  type AiChoice, type AiLang, type AiMark, type AiView, type LiveWatch
} from '../lib/ai';
import {
  bookImageUrl, closeLocalBook, getLocalSection, imageKey, importBookPath, openLibraryBook, saveLibraryProgress,
  type LibraryPlace, type LocalBook, type LocalSectionMeta
} from '../lib/localBook';
import { runOcr } from '../lib/ocr';
import { load, save as store } from '../lib/storage';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { getWiki, type Wiki, type WikiEntry } from '../lib/wiki';
import { READER_THEMES, useApp } from '../store/AppStore';

const GAP_PX = { tight: 6, normal: 16, airy: 28 };
const MARGIN_PX = { narrow: 16, normal: 24, wide: 36 };

const NO_GLOSSARY = {};
const NO_RULES: ReplaceRule[] = [];

/** Parts kept on screen at once; the reader adds the next one as you near the end. */
const MAX_PARTS = 6;

/**
 * Reads a book file opened from the device, or a book from the on-device library
 * (`place` set: it resumes where it was left and saves the position as you read).
 * The reader for an imported book; the replace list is kept on the device.
 * A file opened without saving (`path` set) can be imported from here; the reader then
 * switches to the library copy at the same place.
 */
/** Where a library book's last AI choice (language, style) is remembered: the version shown when a chapter has several. */
const LANG_NATIVE: Record<string, string> = { en: 'English', vi: 'Tiếng Việt', zh: '中文', ko: '한국어' };
const langLabel = (code: string) => `${code.toUpperCase()} · ${LANG_NATIVE[code] ?? code}`;

export { aiViewKey };

export function LocalBookScreen({ book, place, path, live: liveStart }: { book: LocalBook; place?: LibraryPlace; path?: string; live?: LiveWatch }) {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const p = app.readerPrefs;
  const night = p.theme === 'night';
  const theme = READER_THEMES[p.theme];
  const line = night ? 'var(--color-neutral-700)' : 'color-mix(in srgb, currentColor 14%, transparent)';
  const scrollRef = useRef<HTMLDivElement>(null);

  // The book reads as one long scroll: `span` holds the parts on screen, `index` the
  // one at the top of the view (shown in the header, saved as the reading place).
  const start = place?.index ?? 0;
  const [span, setSpan] = useState<[number, number]>([start, start]);
  const spanRef = useRef(span);
  spanRef.current = span;
  const [index, setIndex] = useState(start);
  // Bumped on a jump so every part remounts and the target part scrolls into place.
  const [gen, setGen] = useState(0);
  const [tools, setTools] = useState<ToolsTab | null>(null);
  const [toc, setToc] = useState(false);
  const sectionName = useSectionName();
  const [readPct, setReadPct] = useState(0);
  const [chromeH, setChromeH] = useState(0);
  const immersive = useImmersive(!!tools || toc);
  // Library books know their language; a file opened once falls back to the reading language.
  const known = CONTENT_LANGS.find(l => l === place?.lang);
  const lang: ContentLang = known ?? app.contentLang ?? 'vi';
  const rulesKey = `localRules.${place ? `${place.user}:${place.novelId}` : `file:${book.title ?? ''}`}.${lang}`;
  // Library books keep their list in the library file (synced); a file opened once, on the device.
  const rulesQ = useAsync(
    () => (place ? getBookRules(place.user, place.novelId, lang, rulesKey) : Promise.resolve(load<ReplaceRule[]>(rulesKey, []))),
    [rulesKey]
  );
  // Edits made here win over what was loaded until the book or language changes.
  const [edited, setEdited] = useState<{ key: string; rules: ReplaceRule[] } | null>(null);
  const rules = edited?.key === rulesKey ? edited.rules : rulesQ.status === 'ready' ? rulesQ.data : NO_RULES;
  const changeRules = (next: ReplaceRule[]) => {
    setEdited({ key: rulesKey, rules: next });
    if (place) saveBookRules(place.user, place.novelId, lang, next).catch(() => app.showToast(t('replace.saveFailed')));
    else store(rulesKey, next.length ? next : null);
  };
  // Kept the same while the rules are, so the parts on screen don't lay out their text again.
  const fix = useCallback((text: string) => applyReplaceRules(text, rules), [rules]);

  // AI versions are marked per chapter: a chapter with one shows it (the last language
  // and style chosen when it has several), the others their original. Each chapter can
  // be switched back to the original on its own.
  const viewKey = place ? aiViewKey(place.user, place.novelId) : null;
  // `view`: the language the chapters are read in (the language button): `raw`, the text
  // as imported (the first time, always), or an AI language. A chapter without a version
  // in it shows the raw text with a button to translate it; so does one whose raw text
  // already is in that language, without the button.
  const [ai, setAiState] = useState<AiView | null>(() => (viewKey ? load<AiView | null>(viewKey, null) : null));
  const setAi = (v: AiView) => {
    setAiState(v);
    if (viewKey) store(viewKey, v);
  };
  const viewLang = readingLangOf(ai);
  const [langMenu, setLangMenu] = useState<DOMRect | null>(null);
  const viewLangs = ['raw', ...AI_LANGS.map(l => l.code as string)];
  const pickView = (l: string) => {
    setLangMenu(null);
    const aiLang = AI_LANGS.find(x => x.code === l)?.code;
    setAi({ lang: aiLang ?? ai?.lang ?? (lang === 'vi' ? 'en' : 'vi'), style: ai?.style ?? 'Natural', dict: ai?.dict ?? true, view: l });
    setOriginal(new Set());
  };
  /**
   * Reading Raw, so nothing needs an AI version. A language is always an AI version, even
   * the raw text's own: then the AI rewrites its bad sentences.
   */
  const rawIsView = viewLang === 'raw';
  const [marksRev, setMarksRev] = useState(0);
  const marksQ = useAsync(
    () => (place ? getAiMarks(place.user, place.novelId) : Promise.resolve([] as AiMark[])),
    [place?.user, place?.novelId, marksRev, app.aiRev]
  );
  // Keep the last marks while they reload, so the chapters on screen don't flicker.
  const [marks, setMarks] = useState<Map<number, AiMark[]> | null>(place ? null : new Map());
  const [marksData, setMarksData] = useState<AiMark[] | null | undefined>(undefined);
  if (marksQ.status === 'ready' && marksQ.data !== marksData) {
    setMarksData(marksQ.data);
    const m = new Map<number, AiMark[]>();
    for (const mark of marksQ.data ?? []) m.set(mark.index, [...(m.get(mark.index) ?? []), mark]);
    setMarks(m);
  }
  if (marksQ.status === 'failed' && !marks) setMarks(new Map());
  const [original, setOriginal] = useState<ReadonlySet<number>>(new Set());
  /** The wiki entry tapped in a chapter, with that chapter's view of the wiki. */
  const [wikiPop, setWikiPop] = useState<{ wiki: Wiki; entry: WikiEntry; lang: string; at: number } | null>(null);
  const showOriginal = (i: number, on: boolean) => setOriginal(o => {
    const next = new Set(o);
    if (on) next.add(i); else next.delete(i);
    return next;
  });
  /** The AI version chapter `i` shows: one in the reading language (the chosen style first), else none. */
  const versionOf = (i: number): AiMark | null => {
    const list = marks?.get(i)?.filter(m => m.lang === viewLang);
    if (!list?.length) return null;
    return list.find(m => ai && styleOf(m.style) === ai.style) ?? list[0];
  };
  /**
   * This book's chapters being translated now (the queue's and the reader's), by index.
   * One being translated into the reading language shows its raw text with each
   * paragraph replaced by its translation as the part it is in comes back.
   */
  const [running, setRunning] = useState<ReadonlyMap<number, LiveWatch>>(new Map());
  const aiRunning = useRef(false);
  /** Bumped per chapter each time it gets a new AI text, so its part loads it again. */
  const [aiRev, setAiRev] = useState<Record<number, number>>({});
  const aiLangName = (l: string) => AI_LANGS.find(x => x.code === l)?.native ?? l;
  const aiFail = (e: unknown) => {
    console.error(e);
    if (e instanceof AiFailure) {
      const detail = e.key === 'ai.err.http' || e.key === 'ai.err.generic' ? ` ${aiErrorDetail(e.message)}` : '';
      app.showToast(t(e.key) + detail);
    } else app.showToast(`${t('ai.err.generic')} ${String(e)}`);
  };
  const asChoice = (tr: Translation): AiChoice | null =>
    AI_LANGS.some(l => l.code === tr.lang) ? { lang: tr.lang as AiLang, style: tr.style, dict: tr.dict } : null;
  /** What "translate this chapter" in a chapter uses: into the reading language, in the last style. */
  const lastChoice = (): AiChoice => {
    const into = AI_LANGS.find(x => x.code === viewLang)?.code ?? ai?.lang ?? (lang === 'vi' ? 'en' : 'vi');
    return { lang: into, style: ai?.style ?? 'Natural', dict: ai?.dict ?? true };
  };

  /** One chapter now, made fresh even when it had this version: a loader over the text, then the AI version in its place. */
  const translateChapter = async (tr: Translation, at = index) => {
    const c = asChoice(tr);
    if (!c) return;
    if (!place) { app.showToast(t('ai.needsImport')); return; }
    if (aiRunning.current) { app.showToast(t('ai.busyChapter')); return; }
    aiRunning.current = true;
    if (lay.isPhone) setTools(null);
    // Its progress shows on the pipeline button; the paragraphs come in place as they are translated.
    const novelId = place.novelId;
    try {
      await aiTranslateChapter(place.user, novelId, at, c, true);
      setAi({ ...c, view: c.lang });
      showOriginal(at, false);
      setMarksRev(n => n + 1);
      setAiRev(r => ({ ...r, [at]: (r[at] ?? 0) + 1 }));
      // The reader went on meanwhile: the new version shows in place, without moving the page.
      app.showToast(t('ai.chapterReady', { name: sectionName(book.sections[at], at, book.sections.length) }));
    } catch (e) {
      aiFail(e);
    } finally {
      aiRunning.current = false;
    }
  };

  /** This chapter and the ones after it, in the background queue. */
  const translateFrom = async (tr: Translation) => {
    const c = asChoice(tr);
    if (!c) return;
    if (!place) { app.showToast(t('ai.needsImport')); return; }
    try {
      const r = await aiTranslateBook(place.user, place.novelId, index, c, uiLang);
      const vars = { n: r.queued, lang: aiLangName(c.lang), style: t(`tr.style.${c.style}`) };
      app.showToast(t(r.queued === 0 ? 'ai.nothing' : r.mode === 'rewrite' ? 'ai.queuedRewrite' : 'ai.queued', vars));
      setAi({ ...c, view: viewLang });
    } catch (e) {
      aiFail(e);
    }
  };
  const total = book.sections.length;
  const meta = book.sections[index];
  // Leaving the reader frees the file and every cached section.
  useEffect(() => () => { closeLocalBook(book.id).catch(console.error); }, [book.id]);

  // Where to land once part `target` has rendered: `progress` through it (library books
  // resume where they were left; a jump starts at the top).
  const landing = useRef<{ target: number; progress: number } | null>({ target: start, progress: place?.progress ?? 0 });

  // What the reader is looking at: the element (paragraph, picture, heading) at the top of
  // the view in part `i`, and where it was. Whatever changes size above it (a part added,
  // a part loading its text, a picture, a new font size), the page is moved by the same
  // amount so that element stays put; only the reader's own scrolling moves the text.
  // `frac` (how far into the part) is the fallback when the part's text was replaced.
  const anchor = useRef<{ i: number; node: HTMLElement; top: number; frac: number } | null>(null);
  // The scroll position the reader set itself, so its own scroll events aren't taken for the reader's.
  const selfTop = useRef(-1);
  // Heights of the parts dropped above the view: a spacer of that height takes their place,
  // so dropping them never moves the page (and the browser never has to clamp it).
  const dropped = useRef(new Map<number, number>());
  const partEl = (i: number) => scrollRef.current?.querySelector<HTMLElement>(`[data-part="${i}"]`) ?? null;

  /**
   * The part under the header and how far through it, as the bar shows it. Only parts
   * whose text is in count: a chapter still loading above the one being read (scrolled
   * up to its top quickly) is a placeholder, and taking it for the place read would
   * anchor the page to it and, once its text came in, land at its start instead of
   * staying where the reader was (and save that chapter as the reading place).
   */
  const measure = () => {
    const el = scrollRef.current;
    if (!el) return null;
    const parts = el.querySelectorAll<HTMLElement>('[data-part][data-ready]');
    let part: HTMLElement | undefined;
    for (const p of parts) {
      if (!part || p.offsetTop <= el.scrollTop + chromeH + 1) part = p;
      else break;
    }
    if (!part) return null;
    const top = part.offsetTop, h = part.offsetHeight;
    const progress = Math.min(1, Math.max(0, (el.scrollTop - top) / Math.max(1, h - el.clientHeight)));
    return { i: Number(part.dataset.part), el: part, top, h, progress };
  };

  /** Remembers the element at the top of the view as the one to keep in place. */
  const reAnchor = () => {
    const el = scrollRef.current, m = measure();
    if (!el || !m) return;
    const y = el.scrollTop + chromeH;
    // The part's children are in order; the first one not yet scrolled past.
    const kids = m.el.children;
    let lo = 0, hi = kids.length - 1, node: HTMLElement | null = null;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1, k = kids[mid] as HTMLElement;
      if (k.offsetTop + k.offsetHeight > y) { node = k; hi = mid - 1; } else lo = mid + 1;
    }
    // Past its last line (its bottom margin): the part itself.
    node ??= m.el;
    anchor.current = { i: m.i, node, top: node.offsetTop, frac: (el.scrollTop - m.top) / Math.max(1, m.h) };
  };

  const scrollTo = (el: HTMLElement, top: number) => {
    el.scrollTop = top;
    selfTop.current = el.scrollTop;
  };

  /**
   * Moves the page by however much the anchored element moved since it was remembered.
   * `scrolled`: the reader scrolled meanwhile, which a replaced text must not undo.
   */
  const keep = (scrolled = false) => {
    const el = scrollRef.current, a = anchor.current;
    const part = a && partEl(a.i);
    if (!el || !a || !part) return;
    if (a.node.isConnected && (a.node === part || a.node.parentElement === part)) {
      const moved = a.node.offsetTop - a.top;
      if (moved !== 0) scrollTo(el, el.scrollTop + moved);
      a.top = a.node.offsetTop;
      return;
    }
    if (scrolled) return;
    // The part's text was replaced (loading, another version): the same share of it.
    const top = part.offsetTop + a.frac * part.offsetHeight;
    if (Math.abs(el.scrollTop - top) > 1) scrollTo(el, top);
    if (part.dataset.ready) reAnchor();
  };

  /** Adds the next part when the end is near, the previous one near the top. */
  const grow = () => {
    const el = scrollRef.current;
    if (!el || landing.current) return;
    const [a, b] = spanRef.current;
    let next: [number, number] | null = null;
    const first = partEl(a);
    if (b < total - 1 && partEl(b)?.dataset.ready && el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 1.5) {
      next = [Math.max(a, b + 2 - MAX_PARTS), b + 1];
      // The parts dropped above leave their height behind.
      for (let i = a; i < next[0]; i++) {
        const cur = partEl(i), after = partEl(i + 1);
        if (cur && after) dropped.current.set(i, after.offsetTop - cur.offsetTop);
      }
    } else if (a > 0 && first?.dataset.ready && el.scrollTop < first.offsetTop + el.clientHeight) {
      next = [a - 1, Math.min(b, a - 2 + MAX_PARTS)];
    }
    if (next) {
      spanRef.current = next;
      setSpan(next);
    }
  };

  const onPartReady = (i: number) => {
    const el = scrollRef.current, l = landing.current, part = partEl(i);
    if (el && l && l.target === i && part) {
      landing.current = null;
      scrollTo(el, part.offsetTop + l.progress * Math.max(0, part.offsetHeight - el.clientHeight));
      reAnchor();
      setReadPct(l.progress);
      save(i, l.progress);
    }
    grow();
  };

  /** Opens part `i` on its own, at its top (chapter list). */
  const jump = (i: number) => {
    landing.current = { target: i, progress: 0 };
    anchor.current = null;
    dropped.current.clear();
    spanRef.current = [i, i];
    setSpan([i, i]);
    setIndex(i);
    setGen(g => g + 1);
  };

  /** Goes to chapter `w` being translated, read in its language (the pipeline panel's "open in the reader"). */
  const goToLive = (w: LiveWatch) => {
    const into = AI_LANGS.find(x => x.code === w.lang)?.code;
    if (into && viewLang !== into) setAi({ lang: into, style: styleOf(w.style), dict: ai?.dict ?? true, view: into });
    if (w.index !== index) jump(w.index);
  };
  const goToLiveRef = useRef(goToLive);
  goToLiveRef.current = goToLive;
  useEffect(() => (place ? onWatchLive(place.novelId, w => goToLiveRef.current(w)) : undefined), [place?.novelId]);
  // Opened from the pipeline panel on a chapter being translated.
  useEffect(() => { if (liveStart) goToLive(liveStart); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Which chapters are being translated, read again each time a translation moves; one
  // that is done shows its saved version in place of the parts.
  const runningRef = useRef(running);
  useEffect(() => {
    if (!place) return;
    let gone = false;
    const load = () => {
      getAiStatus(place.user).then(s => {
        if (gone || !s) return;
        const next = new Map<number, LiveWatch>();
        const add = (c: { novelId: number; index: number; lang: string; style: string }) => {
          if (c.novelId === place.novelId) next.set(c.index, { index: c.index, lang: c.lang, style: c.style });
        };
        if (s.live) add(s.live);
        s.chapters.filter(c => !c.error && !c.stopped).forEach(add);
        const prev = runningRef.current;
        const over = [...prev.keys()].filter(i => !next.has(i));
        const same = over.length === 0 && next.size === prev.size
          && [...next].every(([i, w]) => prev.get(i)?.lang === w.lang && prev.get(i)?.style === w.style);
        if (same) return;
        runningRef.current = next;
        setRunning(next);
        if (over.length) {
          setMarksRev(n => n + 1);
          setAiRev(r => { const o = { ...r }; for (const i of over) o[i] = (o[i] ?? 0) + 1; return o; });
        }
      }).catch(e => console.warn(e));
    };
    load();
    const stop = onAiMoved(load, 600);
    return () => { gone = true; stop(); };
  }, [place?.user, place?.novelId]);
  /** The chapter `i` read as it is translated into the reading language, when it is and has no version in it yet. */
  const liveOf = (i: number) => {
    const w = running.get(i);
    return w && place && w.lang === viewLang && !versionOf(i) ? { user: place.user, novelId: place.novelId, watch: w } : null;
  };

  // Every change to the parts on screen is made up for before it is painted.
  const onLayout = useRef(() => {});
  onLayout.current = () => { if (!landing.current) keep(); };
  const partLayout = useCallback(() => onLayout.current(), []);
  useLayoutEffect(() => onLayout.current());

  // Watches the content's height once; the callback reads the latest render's helpers.
  const contentRef = useRef<HTMLDivElement>(null);
  const onResize = useRef(() => {});
  onResize.current = () => { if (!landing.current) keep(); grow(); };
  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const ro = new ResizeObserver(() => onResize.current());
    ro.observe(content);
    return () => ro.disconnect();
  }, []);


  // Library books remember the part and how far into it you scrolled. Saves are
  // batched while scrolling and flushed when you leave.
  const pending = useRef<{ index: number; progress: number } | null>(null);
  const saveTimer = useRef<number | undefined>(undefined);
  const flush = () => {
    window.clearTimeout(saveTimer.current);
    const p = pending.current;
    pending.current = null;
    if (place && p) saveLibraryProgress(place.user, place.novelId, p.index, p.progress).catch(console.error);
  };
  const save = (index: number, progress: number) => {
    if (!place) return;
    pending.current = { index, progress };
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(flush, 600);
  };
  // Continue (bottom bar / sidebar) reopens the library book read last.
  useEffect(() => {
    if (place) app.markLocalRead({ user: place.user, novelId: place.novelId, title: book.title ?? t('book.untitled'), index, total });
  }, [index]);
  useEffect(() => () => flush(), []);

  // The bar shows how far into the part at the top. Scroll events come faster than
  // frames on phones, and each one used to re-render the reader with every paragraph
  // on screen (a long PDF part holds hundreds); now the reading is taken once a frame.
  const scrollFrame = useRef(0);
  const onScrollFrame = useRef(() => {});
  onScrollFrame.current = () => {
    scrollFrame.current = 0;
    const el = scrollRef.current;
    if (!el || landing.current) return;
    // Whether the reader scrolled, or only the reader's own corrections moved the page.
    const scrolled = el.scrollTop !== selfTop.current;
    keep(scrolled);
    if (scrolled || !anchor.current) reAnchor();
    const m = measure();
    if (!m) return;
    if (m.i !== index) setIndex(m.i);
    // A tenth of a percent is finer than the bar can show.
    setReadPct(r => (Math.abs(r - m.progress) < 0.001 ? r : m.progress));
    save(m.i, m.progress);
    grow();
  };
  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    immersive.onScroll(e.currentTarget);
    if (!scrollFrame.current) scrollFrame.current = requestAnimationFrame(() => onScrollFrame.current());
  };
  useEffect(() => () => cancelAnimationFrame(scrollFrame.current), []);
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);
  const importHere = async () => {
    if (!path || importing) return;
    setImporting({ done: 0, total: 0 });
    try {
      const user = app.libKey;
      const saved = await importBookPath(user, path, (done, total) => setImporting({ done, total }));
      runOcr(user);
      // Chapters are the file's sections, so the place carries over as is.
      const m = measure();
      await saveLibraryProgress(user, saved.novelId, m?.i ?? index, m?.progress ?? 0);
      const opened = await openLibraryBook(user, saved.novelId);
      app.showToast(t('library.imported', { title: saved.title }));
      app.back();
      app.push({ s: 'book', book: opened.book, place: opened.place });
    } catch (e) {
      console.error(e);
      app.showToast(`${t('library.importFailed')} ${String(e)}`);
      setImporting(null);
    }
  };


  const name = sectionName(meta, index, total);
  const accent = night ? 'var(--color-accent-400)' : 'var(--color-accent-700)';
  const partStyle = useMemo<PartStyle>(() => ({
    accent, headingSize: lay.readerH, gap: GAP_PX[p.gap], fontSize: p.fontSize, lineH: p.lineH, align: p.align,
  }), [accent, lay.readerH, p.gap, p.fontSize, p.lineH, p.align]);

  return (
    <div className="screen" style={{ overflow: 'hidden', background: theme.bg, color: theme.fg, ...theme.vars }}>
      {/* On wide screens the open tools panel sits beside the text instead of over it. */}
      <div style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: lay.wide && tools ? lay.toolsW : 0, display: 'flex', flexDirection: 'column', transition: 'right .28s ease' }}>
      <ReaderChrome hidden={immersive.hidden} theme={theme} readPct={readPct} line={line} onHeight={setChromeH}>
        <button className="icon-btn icon-btn--plain" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon size={22} /></button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase', opacity: .65, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{book.title ?? t('book.untitled')}</div>
          <div className="display" style={{ fontSize: 16, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {name}{meta?.number != null && !lay.isPhone && <span style={{ opacity: .55, fontSize: 13 }}> · {index + 1}/{total}</span>}
          </div>
        </div>
        {path && !place && (
          <button onClick={importHere} disabled={!!importing} aria-label={t('book.importAria')} title={t('book.importAria')}
            style={{ height: 38, padding: '0 14px', display: 'inline-flex', alignItems: 'center', gap: 6, borderRadius: 999, border: 'none', background: 'var(--color-accent)', color: 'var(--color-bg)', font: 'inherit', fontSize: 12, fontWeight: 700, cursor: importing ? 'default' : 'pointer', flex: '0 0 auto', whiteSpace: 'nowrap' }}>
            {importing ? <Spinner size={13} /> : <DownloadIcon size={15} />}
            {importing?.total ? `${importing.done}/${importing.total}` : t('book.import')}
          </button>
        )}
        {known && place && (
          <button onClick={e => setLangMenu(e.currentTarget.getBoundingClientRect())} aria-haspopup="menu" aria-expanded={!!langMenu}
            aria-label={t('reader.viewLang')} title={t('reader.viewLang')}
            style={{ height: 34, padding: '0 12px', borderRadius: 999, border: `2px solid ${line}`, background: 'none', color: theme.fg, font: 'inherit', fontSize: 12, fontWeight: 700, cursor: 'pointer' }}>
            {viewLang === 'raw' ? t('reader.raw').toUpperCase() : viewLang.toUpperCase()}
          </button>
        )}
        {place && (
          <button className="icon-btn icon-btn--plain" aria-label={t('reader.wiki')} title={t('reader.wiki')}
            onClick={() => app.push({ s: 'bookWiki', novelId: place.novelId, title: book.title ?? t('book.untitled'), lang: viewLang })}>
            <BookOpenIcon size={21} />
          </button>
        )}
        {total > 1 && (
          <button className="icon-btn icon-btn--plain" aria-label={t('book.tocAria')} title={t('book.tocAria')} onClick={() => setToc(true)}><ListIcon size={22} /></button>
        )}
        <button className="icon-btn icon-btn--accent" aria-label={t('reader.tools')} onClick={() => setTools(o => (lay.wide && o ? null : 'reading'))}><SlidersIcon /></button>
      </ReaderChrome>

      <div ref={scrollRef} className="nx-scroll" onScroll={onScroll} onClick={immersive.onTap}
        style={{ position: 'relative', flex: 1, overflowY: 'auto', overflowAnchor: 'none', padding: `${chromeH + lay.readerPadTop}px ${MARGIN_PX[p.margin]}px 40px`, fontFamily: theme.font }}>
        <div ref={contentRef} style={{ maxWidth: lay.readerMax, margin: '0 auto' }}>
          {/* Stands in for the parts dropped above, at their height. */}
          <div aria-hidden style={{ height: [...dropped.current].reduce((sum, [i, h]) => (i < span[0] ? sum + h : sum), 0) }} />
          {Array.from({ length: span[1] - span[0] + 1 }, (_, k) => span[0] + k).map(i => (
            <BookPart key={`${gen}-${i}`} bookId={book.id} index={i} meta={book.sections[i]} name={sectionName(book.sections[i], i, total)}
              fix={fix} st={partStyle} color={theme.fg} onReady={onPartReady} onLayout={partLayout} lastH={dropped.current.get(i)}
              marksReady={!!marks} lang={lang} rev={aiRev[i] ?? 0} viewLang={viewLang}
              ai={(() => {
                const v = versionOf(i);
                return v && place ? { user: place.user, novelId: place.novelId, lang: v.lang, style: styleOf(v.style), others: (marks?.get(i)?.length ?? 1) - 1 } : null;
              })()}
              original={original.has(i)} onShowOriginal={on => showOriginal(i, on)}
              live={liveOf(i)}
              wiki={place ? { user: place.user, novelId: place.novelId, rev: app.aiRev } : null}
              onWiki={(wiki, entry, l) => setWikiPop({ wiki, entry, lang: l, at: i })}
              onTranslate={place && (versionOf(i) || !rawIsView) ? () => {
                const v = versionOf(i);
                const c = v && !original.has(i) ? { lang: v.lang, style: styleOf(v.style), dict: ai?.dict ?? true } : lastChoice();
                void translateChapter(c, i);
              } : undefined} />
          ))}
          {span[1] < total - 1 && (
            <div role="status" style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10, padding: '24px 0', opacity: .6 }}><Spinner size={16} /></div>
          )}
        </div>
      </div>
      <ReaderStatusBar theme={theme}>
      </ReaderStatusBar>

      </div>

      {langMenu && (
        <div onClick={() => setLangMenu(null)} style={{ position: 'fixed', inset: 0, zIndex: 40 }}>
          <div role="menu" aria-label={t('reader.viewLang')} onClick={e => e.stopPropagation()} className="glass"
            style={{ position: 'fixed', top: langMenu.bottom + 6, right: Math.max(8, window.innerWidth - langMenu.right), minWidth: 190, padding: 6, borderRadius: 16, background: 'var(--glass-bg-strong)', border: '1px solid var(--glass-edge)', color: 'var(--color-text)', boxShadow: 'var(--glass-shadow)' }}>
            {viewLangs.map(l => (
              <button key={l} role="menuitemradio" aria-checked={l === viewLang} onClick={() => pickView(l)}
                style={{ display: 'flex', width: '100%', alignItems: 'center', justifyContent: 'space-between', gap: 12, padding: '10px 12px', border: 'none', borderRadius: 10, background: l === viewLang ? 'color-mix(in srgb, var(--color-accent) 16%, transparent)' : 'none', color: 'inherit', font: 'inherit', fontSize: 14, fontWeight: l === viewLang ? 700 : 500, cursor: 'pointer', textAlign: 'left' }}>
                <span>{l === 'raw' ? t('reader.raw') : langLabel(l)}</span>
                <span style={{ fontSize: 11, opacity: .6 }}>{l === 'raw' ? t('reader.rawSub') : t('reader.viewAi')}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      {wikiPop && place && (
        <WikiPopup key={wikiPop.entry.uid} wiki={wikiPop.wiki} entry={wikiPop.entry} lang={wikiPop.lang}
          onPick={e => setWikiPop(w => w && { ...w, entry: e })} onClose={() => setWikiPop(null)}
          onOpenWiki={() => { setWikiPop(null); app.push({ s: 'bookWiki', novelId: place.novelId, title: book.title ?? t('book.untitled'), lang: wikiPop.lang }); }} />
      )}

      {toc && <ChapterSheet sections={book.sections} current={index} marks={marks ?? undefined} onPick={jump} onClose={() => setToc(false)} />}

      {tools && (
        <ReaderTools key={tools} initialTab={tools} glossary={NO_GLOSSARY} lang={lang} rules={rules} onRulesChange={changeRules}
          onClose={() => setTools(null)} onTranslate={tr => void translateChapter(tr)}
          onTranslateFrom={place ? translateFrom : undefined} aiLangs={[...AI_LANGS]} aiHint={t('ai.hint')}
          aiBook={place ? { user: place.user, novelId: place.novelId } : undefined} />
      )}
    </div>
  );
}

/**
 * Whether a chapter's first line only repeats the heading shown above it (its title or
 * name, as is). Nothing is guessed from the words: no "Chương 12" patterns.
 */
function repeatsHeading(line: string, names: (string | null | undefined)[]) {
  const norm = (x: string) => x.normalize('NFC').trim().replace(/\s+/g, ' ').toLocaleLowerCase();
  const l = norm(line);
  return !!l && names.some(n => !!n && norm(n) === l);
}

type PartStyle = {
  accent: string; headingSize: number | string; gap: number; fontSize: number; lineH: number;
  align: CSSProperties['textAlign'];
};

/**
 * One part of the book in the long scroll: its heading, then its text. Marked `data-ready` once its content is in, so the reader knows the
 * height is real before adding the next part.
 */
function BookPart({ bookId, index, meta, name, fix, st, color, onReady, onLayout, lastH, marksReady, lang, rev, viewLang, ai, original, onShowOriginal, onTranslate, wiki, onWiki, live }: {
  bookId: number; index: number; meta: LocalSectionMeta | undefined; name: string;
  fix: (s: string) => string; st: PartStyle; color: string;
  onReady: (index: number) => void;
  /** Called after each change to the part is laid out, before it is painted. */
  onLayout: () => void;
  /** Its height when it was last on screen: kept while its text loads again. */
  lastH: number | undefined;
  /** Whether the AI marks are in: until then the part waits, so it doesn't show the original first. */
  marksReady: boolean;
  /** The book's language: a version in it is a rewrite. */
  lang: string;
  /** Goes up when this chapter got a new AI text. */
  rev: number;
  /** The language picked to read in; a chapter without a version in it offers to translate. */
  viewLang: string;
  /** This chapter's AI version to show (`others`: how many more it has); null when it has none. */
  ai: { user: string; novelId: number; lang: AiLang; style: AiChoice['style']; others: number } | null;
  /** The reader asked for the original of this chapter. */
  original: boolean;
  onShowOriginal: (on: boolean) => void;
  /** Translates this chapter (again, when it has a version); library books only. */
  onTranslate?: () => void;
  /** A library book's wiki: its names in this chapter are marked, as known by this chapter. */
  wiki: { user: string; novelId: number; rev: number } | null;
  onWiki: (wiki: Wiki, entry: WikiEntry, lang: string) => void;
  /** This chapter is read live while it is translated (`watch`): the raw text, each paragraph replaced as its part comes back. */
  live: { user: string; novelId: number; watch: LiveWatch } | null;
}) {
  const { t } = useI18n();
  const showAi = !!ai && !original && !live;
  const aiKey = live ? 'live' : showAi ? `${ai.lang}.${ai.style}` : '';
  const draft = useLiveDraft(live);
  // Goes up when the reader asks to load a failed part again.
  const [retry, setRetry] = useState(0);
  const q = useAsync(async () => {
    if (!marksReady) return new Promise<never>(() => {});
    const own = showAi ? await getAiChapter(ai.user, ai.novelId, index, ai.lang, ai.style) : null;
    if (own) return { paragraphs: own.paragraphs, title: own.title || null, isAi: true };
    return { ...(await getLocalSection(bookId, index)), title: null, isAi: false };
  }, [bookId, index, aiKey, marksReady, rev, retry]);
  // "Chương 12" above the chapter's own title; a chapter without one shows "Chương 12" once.
  const heading = (q.status === 'ready' && q.data.title) || (meta?.label ?? name);
  const ready = q.status !== 'loading';
  useEffect(() => { if (ready) onReady(index); }, [ready]);
  useLayoutEffect(() => onLayout());
  // The text loaded can still be the last version's while a switch to another one loads.
  const isAi = q.status === 'ready' && q.data.isAi && !!ai;
  // The whole wiki, later chapters included: spoilers are fine, the links are what count.
  const wikiQ = useAsync(() => (wiki ? getWiki(wiki.user, wiki.novelId) : Promise.resolve(null)), [wiki?.user, wiki?.novelId, wiki?.rev]);
  const wikiAt = wikiQ.status === 'ready' ? wikiQ.data : null;
  // Names are found as the text shows them; the card speaks the reading language.
  const termsLang = isAi ? ai!.lang : 'raw';
  const cardLang = isAi ? ai!.lang : AI_LANGS.some(l => l.code === viewLang) ? viewLang : 'raw';
  const terms = useMemo(() => wikiTerms(wikiAt, termsLang), [wikiAt, termsLang]);
  const langName = (l: string) => LANG_NATIVE[l] ?? l;
  // Read in a language (the raw text's own too), and this chapter has no AI version in it.
  const missing = !ai && viewLang !== 'raw';
  // The paragraphs are made again only when what they show changes, not each time the
  // reader re-renders (scrolling, the bar, an AI job's progress…).
  const onWikiRef = useRef(onWiki);
  onWikiRef.current = onWiki;
  const loaded = q.status === 'ready' ? q.data : null;
  // Read live: each paragraph back from the AI in place of the raw one; the rest stay raw, dimmed.
  const data = useMemo(() => (loaded && draft
    ? { ...loaded, paragraphs: loaded.paragraphs.map((p, i) => draft.get(i) ?? p) }
    : loaded), [loaded, draft]);
  const rawLeft = useMemo(() => {
    if (!loaded || !draft) return null;
    const left = new Set<number>();
    loaded.paragraphs.forEach((p, i) => { if (!draft.has(i) && imageKey(p) === null) left.add(i); });
    return left;
  }, [loaded, draft]);
  const body = useMemo<ReactNode>(() => data?.paragraphs.map((para, i) => {
    // The chapter's own first line repeating the heading shown above.
    if (i === 0 && repeatsHeading(para, [heading, name])) return null;
    const key = imageKey(para);
    return key !== null ? (
      // A picture the book can't provide just disappears.
      <img key={i} src={bookImageUrl(bookId, key)} alt="" loading="lazy" decoding="async"
        onError={e => { e.currentTarget.style.display = 'none'; }}
        style={{ display: 'block', maxWidth: '100%', height: 'auto', margin: `0 auto ${st.gap}px`, borderRadius: 4 }} />
    ) : (
      <p key={i} data-live={rawLeft ? (rawLeft.has(i) ? 'raw' : 'done') : undefined}
        style={{ margin: `0 0 ${st.gap}px`, fontSize: st.fontSize, lineHeight: st.lineH, textAlign: st.align, textWrap: 'pretty', opacity: rawLeft?.has(i) ? .45 : undefined, transition: 'opacity .3s ease' }}>
        {wikiAt ? markWikiTerms(fix(para), terms, e => onWikiRef.current(wikiAt, e, cardLang)) : fix(para)}
      </p>
    );
  }), [data, rawLeft, heading, name, bookId, st.gap, st.fontSize, st.lineH, st.align, wikiAt, terms, cardLang, fix]);
  const link: CSSProperties = { border: 'none', background: 'none', font: 'inherit', fontSize: 12, fontWeight: 700, textDecoration: 'underline', cursor: 'pointer', color: 'inherit', padding: 0 };

  return (
    <section data-part={index} data-ready={ready ? 'true' : undefined} style={{ paddingBottom: 28, minHeight: ready ? undefined : lastH }}>
      {heading !== name && (
        <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.1em', textTransform: 'uppercase', color: st.accent }}>{name}</div>
      )}
      <h2 style={{ margin: '4px 0 18px', fontSize: st.headingSize, color }}>{fix(heading)}</h2>
      {q.status === 'loading' && (
        <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: .75 }}><Spinner size={16} />{t('book.loading')}</div>
      )}
      {/* The reason goes to the console; the page never shows an error string in place of the text. */}
      {q.status === 'failed' && (
        <div role="alert" style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap', opacity: .8 }}>
          <span>{t('book.loadFailed')}</span>
          <button onClick={() => setRetry(n => n + 1)}
            style={{ border: 'none', background: 'none', padding: 0, font: 'inherit', fontWeight: 700, textDecoration: 'underline', color: 'inherit', cursor: 'pointer' }}>
            {t('load.retry')}
          </button>
        </div>
      )}
      {q.status === 'ready' && !live && (onTranslate || ai) && (
        <div data-ai-mark={isAi ? `${ai!.lang}.${ai!.style}` : 'original'}
          style={{ display: 'flex', alignItems: 'center', gap: '6px 12px', flexWrap: 'wrap', fontSize: 12, marginBottom: 14 }}>
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 5, padding: '4px 10px', borderRadius: 999, fontWeight: 700,
            background: isAi ? 'color-mix(in srgb, var(--color-accent) 18%, transparent)' : 'color-mix(in srgb, currentColor 8%, transparent)',
            color: isAi ? st.accent : 'inherit', opacity: isAi ? 1 : .75
          }}>
            {isAi && <SparklesIcon size={12} />}
            {isAi
              ? t('ai.markAi', { lang: ai!.lang === lang ? `${langName(ai!.lang)} (${t('ai.markRewrite')})` : langName(ai!.lang), style: t(`tr.style.${ai!.style}`) })
              : missing ? t('ai.missingLang', { lang: langName(viewLang) }) : t(ai || viewLang === 'raw' ? 'ai.markOriginal' : 'ai.markNone')}
          </span>
          {isAi && <button style={link} onClick={() => onShowOriginal(true)}>{t('ai.showOriginal')}</button>}
          {!isAi && ai && <button style={link} onClick={() => onShowOriginal(false)}>{t('ai.showAi')}</button>}
          {onTranslate && missing && (
            <button onClick={onTranslate}
              style={{ display: 'inline-flex', alignItems: 'center', gap: 6, height: 30, padding: '0 12px', borderRadius: 999, border: 'none', background: st.accent, color: 'var(--color-bg)', font: 'inherit', fontSize: 12, fontWeight: 700, cursor: 'pointer' }}>
              <SparklesIcon size={13} />{t(viewLang === lang ? 'ai.rewriteInto' : 'ai.translateInto', { lang: langName(viewLang) })}
            </button>
          )}
          {onTranslate && !missing && <button style={link} onClick={onTranslate}>{t(isAi ? 'ai.again' : 'ai.translateHere')}</button>}
        </div>
      )}
      {q.status === 'ready' && q.data.paragraphs.length === 0 && <p style={{ opacity: .7 }}>{t('book.empty')}</p>}
      {body}
    </section>
  );
}

/**
 * The parts of a chapter back so far while it is translated, by the position of the raw
 * block each translates; read again each time the pipeline moves. Null when not live.
 */
function useLiveDraft(live: { user: string; novelId: number; watch: LiveWatch } | null): Map<number, string> | null {
  const [draft, setDraft] = useState<Map<number, string> | null>(null);
  useEffect(() => {
    if (!live) { setDraft(null); return; }
    let gone = false;
    const w = live.watch;
    const load = async () => {
      try {
        const d = await getAiDraft(live.user, live.novelId, w.index, w.lang, w.style);
        if (gone) return;
        const m = new Map<number, string>();
        d?.positions.forEach((at, k) => { const p = d.paragraphs[k]; if (p?.trim()) m.set(at, p); });
        setDraft(m);
      } catch (e) {
        console.warn(e);
      }
    };
    void load();
    const stop = onAiMoved(() => { void load(); }, 600);
    return () => { gone = true; stop(); };
  }, [live?.user, live?.novelId, live?.watch.index, live?.watch.lang, live?.watch.style]);
  return draft;
}
