import { useEffect, useRef, useState } from 'react';
import { ChapterSheet, useSectionName } from '../components/ChapterSheet';
import { ChevronLeftIcon, DownloadIcon, ListIcon, SlidersIcon } from '../components/Icons';
import { ReaderTools, type ToolsTab } from '../components/ReaderTools';
import { Segmented, Spinner } from '../components/ui';
import { applyReplaceRules } from '../data/repository';
import { CONTENT_LANGS, type ContentLang, type ReplaceRule } from '../data/types';
import { useT } from '../i18n';
import {
  bookImageUrl, bookPageUrl, closeLocalBook, getLocalSection, imageKey, importBookPath, libraryUser, openLibraryBook, saveLibraryProgress,
  type LibraryPlace, type LocalBook
} from '../lib/localBook';
import { load, save as store } from '../lib/storage';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { READER_THEMES, useApp } from '../store/AppStore';

const GAP_PX = { tight: 6, normal: 16, airy: 28 };
const MARGIN_PX = { narrow: 16, normal: 24, wide: 36 };

type View = 'pages' | 'text';
const VIEW_KEY = 'bennovel.bookView';
function loadView(): View {
  try {
    return localStorage.getItem(VIEW_KEY) === 'text' ? 'text' : 'pages';
  } catch {
    return 'pages';
  }
}

/** Page pictures are drawn at the screen's pixel width (in steps, so the cache hits). */
const PAGE_WIDTH = Math.min(2400, Math.ceil((window.innerWidth * (window.devicePixelRatio || 1)) / 400) * 400);

const NO_GLOSSARY = {};

/**
 * Reads a book file opened from the device, or a book from the on-device library
 * (`place` set: it resumes where it was left and saves the position as you read).
 * Same layout and tools as the online reader; the replace list is kept on the device.
 * A file opened without saving (`path` set) can be imported from here; the reader then
 * switches to the library copy at the same place.
 */
export function LocalBookScreen({ book, place, path }: { book: LocalBook; place?: LibraryPlace; path?: string }) {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const p = app.readerPrefs;
  const night = p.theme === 'night';
  const theme = READER_THEMES[p.theme];
  const line = night ? 'var(--color-neutral-700)' : 'color-mix(in srgb, currentColor 14%, transparent)';
  const scrollRef = useRef<HTMLDivElement>(null);

  const [index, setIndex] = useState(place?.index ?? 0);
  const [tools, setTools] = useState<ToolsTab | null>(null);
  const [toc, setToc] = useState(false);
  const sectionName = useSectionName();
  const [readPct, setReadPct] = useState(0);
  // Library books know their language; a file opened once falls back to the reading language.
  const known = CONTENT_LANGS.find(l => l === place?.lang);
  const lang: ContentLang = known ?? app.contentLang ?? 'vi';
  const rulesKey = `localRules.${place ? `${place.user}:${place.novelId}` : `file:${book.title ?? ''}`}.${lang}`;
  const [rules, setRules] = useState<ReplaceRule[]>(() => load(rulesKey, []));
  const changeRules = (next: ReplaceRule[]) => {
    setRules(next);
    store(rulesKey, next.length ? next : null);
  };
  const fix = (text: string) => applyReplaceRules(text, rules);
  // PDF/DjVu: the original pages as pictures, or the extracted text.
  const [view, setViewState] = useState<View>(() => (book.pageImages ? loadView() : 'text'));
  const setView = (v: View) => {
    setViewState(v);
    try { localStorage.setItem(VIEW_KEY, v); } catch { /* private mode */ }
  };
  const showPages = book.pageImages && view === 'pages';
  const total = book.sections.length;
  const meta = book.sections[index];
  const sectionQ = useAsync(() => getLocalSection(book.id, index), [book.id, index]);
  // Leaving the reader frees the file and every cached section.
  useEffect(() => () => { closeLocalBook(book.id).catch(console.error); }, [book.id]);

  // Scroll position to restore once the first section has rendered (library books only).
  const restore = useRef(place?.progress ?? 0);
  const ready = sectionQ.status === 'ready';
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !ready) return;
    const max = el.scrollHeight - el.clientHeight;
    el.scrollTo({ top: restore.current > 0 && max > 0 ? restore.current * max : 0 });
    restore.current = 0;
  }, [index, ready]);

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
  const save = (progress: number) => {
    if (!place) return;
    pending.current = { index, progress };
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(flush, 600);
  };
  // The bar shows how far into this chapter, like the online reader.
  useEffect(() => {
    if (!ready) return;
    const progress = currentProgress(scrollRef.current);
    setReadPct(progress);
    save(progress);
  }, [index, ready]);
  // Continue (bottom bar / sidebar) reopens the library book read last.
  useEffect(() => {
    if (place) app.markLocalRead({ user: place.user, novelId: place.novelId, title: book.title ?? t('book.untitled'), index, total });
  }, [index]);
  useEffect(() => () => flush(), []);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const progress = currentProgress(e.currentTarget);
    setReadPct(progress);
    if (place && ready) save(progress);
  };
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);
  const importHere = async () => {
    if (!path || importing) return;
    setImporting({ done: 0, total: 0 });
    try {
      const user = libraryUser(app.user);
      const saved = await importBookPath(user, path, (done, total) => setImporting({ done, total }));
      // Chapters are the file's sections, so the place carries over as is.
      await saveLibraryProgress(user, saved.novelId, index, currentProgress(scrollRef.current));
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


  // "Chương 12" above the chapter's own title; a chapter without one shows "Chương 12" once.
  const name = sectionName(meta, index, total);
  const heading = meta?.label ?? name;

  return (
    <div className="screen" style={{ overflow: 'hidden', background: theme.bg, color: theme.fg }}>
      {/* On wide screens the open tools panel sits beside the text instead of over it. */}
      <div style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: lay.wide && tools ? lay.toolsW : 0, display: 'flex', flexDirection: 'column', transition: 'right .28s ease' }}>
      <div style={{ padding: `${lay.readerTop} ${lay.readerPx}px 8px`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <button className="icon-btn icon-btn--plain" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon size={22} /></button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase', opacity: .65, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{book.title ?? t('book.untitled')}</div>
          <div className="display" style={{ fontSize: 16, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {name}{meta?.number != null && !lay.isPhone && <span style={{ opacity: .55, fontSize: 13 }}> · {index + 1}/{total}</span>}
          </div>
        </div>
        {book.pageImages && (
          <Segmented<View> value={view} onChange={setView} style={{ flex: '0 0 auto' }} optStyle={{ padding: '6px 10px', fontSize: 13 }}
            options={[{ value: 'pages', label: t('book.viewPages') }, { value: 'text', label: t('book.viewText') }]} />
        )}
        {path && !place && (
          <button onClick={importHere} disabled={!!importing} aria-label={t('book.importAria')} title={t('book.importAria')}
            style={{ height: 38, padding: '0 14px', display: 'inline-flex', alignItems: 'center', gap: 6, borderRadius: 999, border: 'none', background: 'var(--color-accent)', color: 'var(--color-bg)', font: 'inherit', fontSize: 12, fontWeight: 700, cursor: importing ? 'default' : 'pointer', flex: '0 0 auto', whiteSpace: 'nowrap' }}>
            {importing ? <Spinner size={13} /> : <DownloadIcon size={15} />}
            {importing?.total ? `${importing.done}/${importing.total}` : (!lay.isPhone || !book.pageImages) && t('book.import')}
          </button>
        )}
        {known && (
          <button onClick={() => setTools('ai')}
            style={{ height: 34, padding: '0 12px', borderRadius: 999, border: `2px solid ${line}`, background: 'none', color: theme.fg, font: 'inherit', fontSize: 12, fontWeight: 700, cursor: 'pointer' }}>
            {lang.toUpperCase()}
          </button>
        )}
        {total > 1 && (
          <button className="icon-btn icon-btn--plain" aria-label={t('book.tocAria')} title={t('book.tocAria')} onClick={() => setToc(true)}><ListIcon size={22} /></button>
        )}
        <button className="icon-btn icon-btn--accent" aria-label={t('reader.tools')} onClick={() => setTools(o => (lay.wide && o ? null : 'reading'))}><SlidersIcon /></button>
      </div>
      <div style={{ height: 5, margin: `0 ${lay.readerBarPx}px`, borderRadius: 999, background: line, overflow: 'hidden' }}>
        <div style={{ height: '100%', width: `${Math.round(readPct * 100)}%`, background: 'var(--color-accent)', borderRadius: 999 }} />
      </div>

      <div ref={scrollRef} className="nx-scroll" onScroll={onScroll}
        style={{ flex: 1, overflowY: 'auto', padding: `${lay.readerPadTop}px ${MARGIN_PX[p.margin]}px calc(env(safe-area-inset-bottom) + 40px)` }}>
        <div style={{ maxWidth: lay.readerMax, margin: '0 auto' }}>
          {heading !== name && (
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.1em', textTransform: 'uppercase', color: night ? 'var(--color-accent-400)' : 'var(--color-accent-700)' }}>
              {name}
            </div>
          )}
          <h2 style={{ margin: '4px 0 18px', fontSize: lay.readerH, color: theme.fg }}>{fix(heading)}</h2>
          {showPages && meta?.pages && pageRange(meta.pages).map(n => (
            <img key={n} src={bookPageUrl(book.id, n, PAGE_WIDTH)} alt={t('book.page', { n })} loading="lazy" decoding="async"
              onLoad={e => { const i = e.currentTarget; i.style.aspectRatio = `${i.naturalWidth} / ${i.naturalHeight}`; }}
              style={{ display: 'block', width: '100%', aspectRatio: '1 / 1.414', margin: `0 0 ${GAP_PX[p.gap]}px`, background: '#fff', borderRadius: 4, boxShadow: `0 0 0 1px ${line}`, filter: night ? 'brightness(.82)' : undefined }} />
          ))}
          {!showPages && sectionQ.status === 'loading' && (
            <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: .75 }}><Spinner size={16} />{t('book.loading')}</div>
          )}
          {!showPages && sectionQ.status === 'failed' && <p role="alert">{String(sectionQ.error)}</p>}
          {!showPages && sectionQ.status === 'ready' && sectionQ.data.paragraphs.length === 0 && <p style={{ opacity: .7 }}>{t('book.empty')}</p>}
          {!showPages && sectionQ.status === 'ready' && sectionQ.data.paragraphs.map((para, i) => {
            // The chapter's own "Chương 629." line repeats the heading shown above.
            if (i === 0 && meta?.number != null && isHeadingLine(para, meta.number, meta.label)) return null;
            const key = imageKey(para);
            return key !== null ? (
              // A picture the book can't provide just disappears.
              <img key={i} src={bookImageUrl(book.id, key)} alt="" loading="lazy" decoding="async"
                onError={e => { e.currentTarget.style.display = 'none'; }}
                style={{ display: 'block', maxWidth: '100%', height: 'auto', margin: `0 auto ${GAP_PX[p.gap]}px`, borderRadius: 4 }} />
            ) : (
              <p key={i} style={{ margin: `0 0 ${GAP_PX[p.gap]}px`, fontSize: p.fontSize, lineHeight: p.lineH, textAlign: p.align, textWrap: 'pretty' }}>{fix(para)}</p>
            );
          })}

          <div style={{ display: 'flex', gap: 10, marginTop: 28 }}>
            <button className="btn btn-secondary" style={{ flex: 1, height: 48, color: theme.fg, borderColor: line }} disabled={index <= 0} onClick={() => setIndex(i => i - 1)}>{t('book.prev')}</button>
            <button className="btn btn-primary" style={{ flex: 1, height: 48 }} disabled={index >= total - 1} onClick={() => setIndex(i => i + 1)}>{t('book.next')}</button>
          </div>
        </div>
      </div>
      </div>

      {toc && <ChapterSheet sections={book.sections} current={index} onPick={setIndex} onClose={() => setToc(false)} />}

      {tools && (
        <ReaderTools key={tools} initialTab={tools} glossary={NO_GLOSSARY} lang={lang} rules={rules} onRulesChange={changeRules}
          onClose={() => setTools(null)} onTranslate={() => app.showToast(t('book.translateSoon'))} />
      )}
    </div>
  );
}

/** Whether `line` is just "Chương 12", "Chapter 12: Title"… for chapter `n` titled `label`. */
function isHeadingLine(line: string, n: number, label: string | null) {
  const m = /^\s*(?:chương|chuong|chapter|chap|ch|hồi|hoi|第)\s*[.:#_-]?\s*(\d+(?:\.\d+)?)\s*[章回话話节節集]?(.*)$/i.exec(line);
  if (!m || Number(m[1]) !== n) return false;
  const rest = m[2].replace(/^[\s:：\-–—.、,)\]]+/, '').replace(/[\s.]+$/, '').trim();
  return !rest || rest === label?.trim();
}

function currentProgress(el: HTMLElement | null) {
  if (!el) return 0;
  const max = el.scrollHeight - el.clientHeight;
  return max > 0 ? Math.min(1, Math.max(0, el.scrollTop / max)) : 0;
}

function pageRange([from, to]: [number, number]) {
  return Array.from({ length: Math.max(0, to - from + 1) }, (_, i) => from + i);
}
