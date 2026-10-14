import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { ChapterSheet, useSectionName } from '../components/ChapterSheet';
import { ChevronLeftIcon, DownloadIcon, ListIcon, SlidersIcon, SparklesIcon } from '../components/Icons';
import { ReaderChrome, ReaderStatusBar, useImmersive } from '../components/ReaderChrome';
import { ReaderTools, type ToolsTab, type Translation } from '../components/ReaderTools';
import { Spinner } from '../components/ui';
import { applyReplaceRules, getBookRules, saveBookRules } from '../data/repository';
import { CONTENT_LANGS, type ContentLang, type ReplaceRule } from '../data/types';
import { useI18n } from '../i18n';
import { AI_LANGS, AiFailure, aiErrorDetail, aiTranslateBook, aiTranslateChapter, getAiChapter, type AiChoice, type AiLang } from '../lib/ai';
import {
  bookImageUrl, closeLocalBook, getLocalSection, imageKey, importBookPath, libraryUser, openLibraryBook, saveLibraryProgress,
  type LibraryPlace, type LocalBook, type LocalSectionMeta
} from '../lib/localBook';
import { runOcr } from '../lib/ocr';
import { load, save as store } from '../lib/storage';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { READER_THEMES, useApp } from '../store/AppStore';

const GAP_PX = { tight: 6, normal: 16, airy: 28 };
const MARGIN_PX = { narrow: 16, normal: 24, wide: 36 };

const NO_GLOSSARY = {};

/** Parts kept on screen at once; the reader adds the next one as you near the end. */
const MAX_PARTS = 6;

/**
 * Reads a book file opened from the device, or a book from the on-device library
 * (`place` set: it resumes where it was left and saves the position as you read).
 * Same layout and tools as the online reader; the replace list is kept on the device.
 * A file opened without saving (`path` set) can be imported from here; the reader then
 * switches to the library copy at the same place.
 */
/** Where a library book's AI version to read is remembered. */
export const aiViewKey = (user: string, novelId: number) => `aiView.${user}:${novelId}`;

export function LocalBookScreen({ book, place, path }: { book: LocalBook; place?: LibraryPlace; path?: string }) {
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
  const rules = edited?.key === rulesKey ? edited.rules : rulesQ.status === 'ready' ? rulesQ.data : [];
  const changeRules = (next: ReplaceRule[]) => {
    setEdited({ key: rulesKey, rules: next });
    if (place) saveBookRules(place.user, place.novelId, lang, next).catch(() => app.showToast(t('replace.saveFailed')));
    else store(rulesKey, next.length ? next : null);
  };
  const fix = (text: string) => applyReplaceRules(text, rules);

  // AI version on screen instead of the original (library books), kept per book.
  const viewKey = place ? aiViewKey(place.user, place.novelId) : null;
  const [ai, setAiState] = useState<AiChoice | null>(() => (viewKey ? load<AiChoice | null>(viewKey, null) : null));
  const setAi = (v: AiChoice | null) => {
    setAiState(v);
    if (viewKey) store(viewKey, v);
  };
  const [aiBusy, setAiBusy] = useState<'translate' | 'rewrite' | null>(null);
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

  /** One chapter now: a loader over the text, then the AI version in its place. */
  const translateChapter = async (tr: Translation, at = index) => {
    const c = asChoice(tr);
    if (!c) return;
    if (!place) { app.showToast(t('ai.needsImport')); return; }
    if (lay.isPhone) setTools(null);
    setAiBusy(c.lang === lang ? 'rewrite' : 'translate');
    try {
      await aiTranslateChapter(place.user, place.novelId, at, c);
      setAi(c);
      jump(at);
    } catch (e) {
      aiFail(e);
    } finally {
      setAiBusy(null);
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
      if (r.queued > 0) setAi(c);
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
  // The part at the top of the view and how far into it (share of its height), kept
  // while parts load, pictures size themselves or parts are added and dropped above.
  const anchor = useRef<{ i: number; frac: number } | null>(null);
  const partEl = (i: number) => scrollRef.current?.querySelector<HTMLElement>(`[data-part="${i}"]`) ?? null;

  /** The part under the header and how far through it, as the bar shows it. */
  const measure = () => {
    const el = scrollRef.current;
    if (!el) return null;
    const parts = el.querySelectorAll<HTMLElement>('[data-part]');
    let part: HTMLElement | undefined;
    for (const p of parts) {
      if (!part || p.offsetTop <= el.scrollTop + chromeH + 1) part = p;
      else break;
    }
    if (!part) return null;
    const top = part.offsetTop, h = part.offsetHeight;
    const progress = Math.min(1, Math.max(0, (el.scrollTop - top) / Math.max(1, h - el.clientHeight)));
    return { i: Number(part.dataset.part), top, h, progress };
  };

  /** Adds the next part when the end is near, the previous one near the top. */
  const grow = () => {
    const el = scrollRef.current;
    if (!el || landing.current) return;
    const [a, b] = spanRef.current;
    let next: [number, number] | null = null;
    if (b < total - 1 && partEl(b)?.dataset.ready && el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 1.5) {
      next = [Math.max(a, b + 2 - MAX_PARTS), b + 1];
    } else if (a > 0 && partEl(a)?.dataset.ready && el.scrollTop < el.clientHeight) {
      next = [a - 1, Math.min(b, a - 2 + MAX_PARTS)];
    }
    if (next) {
      spanRef.current = next;
      setSpan(next);
    }
  };

  /** Puts the anchored part back where it was after the content above it changed size. */
  const keep = () => {
    const el = scrollRef.current, a = anchor.current;
    const part = a && partEl(a.i);
    if (!el || !part) return;
    const top = part.offsetTop + a.frac * part.offsetHeight;
    if (Math.abs(el.scrollTop - top) > 1) el.scrollTop = top;
  };

  const onPartReady = (i: number) => {
    const el = scrollRef.current, l = landing.current, part = partEl(i);
    if (el && l && l.target === i && part) {
      landing.current = null;
      el.scrollTop = part.offsetTop + l.progress * Math.max(0, part.offsetHeight - el.clientHeight);
      anchor.current = { i, frac: (el.scrollTop - part.offsetTop) / Math.max(1, part.offsetHeight) };
      setReadPct(l.progress);
      save(i, l.progress);
    }
    grow();
  };

  /** Opens part `i` on its own, at its top (chapter list). */
  const jump = (i: number) => {
    landing.current = { target: i, progress: 0 };
    anchor.current = null;
    spanRef.current = [i, i];
    setSpan([i, i]);
    setIndex(i);
    setGen(g => g + 1);
  };

  // Watches the content's height once; the callback reads the latest render's helpers.
  const contentRef = useRef<HTMLDivElement>(null);
  const onResize = useRef(() => {});
  onResize.current = () => { keep(); grow(); };
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

  // The bar shows how far into the part at the top, like the online reader.
  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    immersive.onScroll(e.currentTarget);
    const m = measure();
    if (!m || landing.current) return;
    anchor.current = { i: m.i, frac: (e.currentTarget.scrollTop - m.top) / Math.max(1, m.h) };
    if (m.i !== index) setIndex(m.i);
    setReadPct(m.progress);
    save(m.i, m.progress);
    grow();
  };
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);
  const importHere = async () => {
    if (!path || importing) return;
    setImporting({ done: 0, total: 0 });
    try {
      const user = libraryUser(app.user);
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
  const partStyle = {
    accent: night ? 'var(--color-accent-400)' : 'var(--color-accent-700)',
    headingSize: lay.readerH, gap: GAP_PX[p.gap], fontSize: p.fontSize, lineH: p.lineH, align: p.align,
  };

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
      </ReaderChrome>

      <div ref={scrollRef} className="nx-scroll" onScroll={onScroll} onClick={immersive.onTap}
        style={{ position: 'relative', flex: 1, overflowY: 'auto', overflowAnchor: 'none', padding: `${chromeH + lay.readerPadTop}px ${MARGIN_PX[p.margin]}px 40px`, fontFamily: theme.font }}>
        <div ref={contentRef} style={{ maxWidth: lay.readerMax, margin: '0 auto' }}>
          {ai && (
            <div style={{ display: 'flex', gap: 10, alignItems: 'flex-start', padding: '12px 14px', borderRadius: 22, background: night ? 'var(--color-neutral-800)' : 'var(--color-accent-100)', marginBottom: 18, fontSize: 12, lineHeight: 1.45 }}>
              <SparklesIcon size={16} style={{ flex: 'none', marginTop: 1 }} />
              <div style={{ flex: 1 }}>
                {t(ai.lang === lang ? 'ai.bannerRewrite' : 'ai.bannerTranslate', { lang: aiLangName(ai.lang), style: t(`tr.style.${ai.style}`) })}
              </div>
              <button onClick={() => { setAi(null); jump(index); }} style={{ border: 'none', background: 'none', font: 'inherit', fontSize: 12, fontWeight: 700, textDecoration: 'underline', cursor: 'pointer', color: 'inherit', padding: 0 }}>
                {t('reader.original')}
              </button>
            </div>
          )}
          {Array.from({ length: span[1] - span[0] + 1 }, (_, k) => span[0] + k).map(i => (
            <BookPart key={`${gen}-${i}`} bookId={book.id} index={i} meta={book.sections[i]} name={sectionName(book.sections[i], i, total)}
              fix={fix} st={partStyle} color={theme.fg} onReady={onPartReady}
              ai={ai && place ? { user: place.user, novelId: place.novelId, lang: ai.lang, style: ai.style } : null}
              onTranslate={ai ? () => void translateChapter({ lang: ai.lang, style: ai.style, dict: ai.dict }, i) : undefined} />
          ))}
          {span[1] < total - 1 && (
            <div role="status" style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10, padding: '24px 0', opacity: .6 }}><Spinner size={16} /></div>
          )}
        </div>
      </div>
      <ReaderStatusBar theme={theme} />

      {aiBusy && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', background: 'color-mix(in srgb, var(--color-neutral-900) 35%, transparent)', zIndex: 30 }}>
          <div role="status" style={{ background: 'var(--color-surface)', color: 'var(--color-text)', borderRadius: 30, padding: '22px 26px', display: 'flex', alignItems: 'center', gap: 14, boxShadow: 'var(--shadow-lg)', margin: 20 }}>
            <Spinner size={24} width={3} />
            <div>
              <div className="display" style={{ fontSize: 17 }}>{t(aiBusy === 'rewrite' ? 'ai.rewriting' : 'reader.translating')}</div>
              <div className="muted" style={{ fontSize: 12 }}>{t('ai.working')}</div>
            </div>
          </div>
        </div>
      )}
      </div>

      {toc && <ChapterSheet sections={book.sections} current={index} onPick={jump} onClose={() => setToc(false)} />}

      {tools && (
        <ReaderTools key={tools} initialTab={tools} glossary={NO_GLOSSARY} lang={lang} rules={rules} onRulesChange={changeRules}
          onClose={() => setTools(null)} onTranslate={tr => void translateChapter(tr)}
          onTranslateFrom={place ? translateFrom : undefined} aiLangs={[...AI_LANGS]} aiHint={t('ai.hint')} />
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

type PartStyle = {
  accent: string; headingSize: number | string; gap: number; fontSize: number; lineH: number;
  align: CSSProperties['textAlign'];
};

/**
 * One part of the book in the long scroll: its heading, then its text. Marked `data-ready` once its content is in, so the reader knows the
 * height is real before adding the next part.
 */
function BookPart({ bookId, index, meta, name, fix, st, color, onReady, ai, onTranslate }: {
  bookId: number; index: number; meta: LocalSectionMeta | undefined; name: string;
  fix: (s: string) => string; st: PartStyle; color: string;
  onReady: (index: number) => void;
  /** Show this AI version of the chapter (the original when it has none yet). */
  ai: { user: string; novelId: number; lang: AiLang; style: AiChoice['style'] } | null;
  /** Translates this chapter, offered when its AI version is missing. */
  onTranslate?: () => void;
}) {
  const { t } = useI18n();
  const aiKey = ai ? `${ai.lang}.${ai.style}` : '';
  const q = useAsync(async () => {
    const own = ai ? await getAiChapter(ai.user, ai.novelId, index, ai.lang, ai.style) : null;
    if (own) return { paragraphs: own.paragraphs, title: own.title || null, missing: false };
    return { ...(await getLocalSection(bookId, index)), title: null, missing: !!ai };
  }, [bookId, index, aiKey]);
  // "Chương 12" above the chapter's own title; a chapter without one shows "Chương 12" once.
  const heading = (q.status === 'ready' && q.data.title) || (meta?.label ?? name);
  const ready = q.status !== 'loading';
  useEffect(() => { if (ready) onReady(index); }, [ready]);

  return (
    <section data-part={index} data-ready={ready ? 'true' : undefined} style={{ paddingBottom: 28 }}>
      {heading !== name && (
        <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.1em', textTransform: 'uppercase', color: st.accent }}>{name}</div>
      )}
      <h2 style={{ margin: '4px 0 18px', fontSize: st.headingSize, color }}>{fix(heading)}</h2>
      {q.status === 'loading' && (
        <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: .75 }}><Spinner size={16} />{t('book.loading')}</div>
      )}
      {q.status === 'failed' && <p role="alert">{String(q.error)}</p>}
      {q.status === 'ready' && q.data.missing && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap', fontSize: 12, opacity: .8, marginBottom: 14 }}>
          <span style={{ flex: 1, minWidth: 160 }}>{t('ai.missing')}</span>
          {onTranslate && (
            <button onClick={onTranslate} style={{ border: 'none', background: 'none', font: 'inherit', fontSize: 12, fontWeight: 700, textDecoration: 'underline', cursor: 'pointer', color: 'inherit', padding: 0 }}>
              {t('ai.translateHere')}
            </button>
          )}
        </div>
      )}
      {q.status === 'ready' && q.data.paragraphs.length === 0 && <p style={{ opacity: .7 }}>{t('book.empty')}</p>}
      {q.status === 'ready' && q.data.paragraphs.map((para, i) => {
        // The chapter's own "Chương 629." line repeats the heading shown above.
        if (i === 0 && meta?.number != null && isHeadingLine(para, meta.number, meta.label)) return null;
        const key = imageKey(para);
        return key !== null ? (
          // A picture the book can't provide just disappears.
          <img key={i} src={bookImageUrl(bookId, key)} alt="" loading="lazy" decoding="async"
            onError={e => { e.currentTarget.style.display = 'none'; }}
            style={{ display: 'block', maxWidth: '100%', height: 'auto', margin: `0 auto ${st.gap}px`, borderRadius: 4 }} />
        ) : (
          <p key={i} style={{ margin: `0 0 ${st.gap}px`, fontSize: st.fontSize, lineHeight: st.lineH, textAlign: st.align, textWrap: 'pretty' }}>{fix(para)}</p>
        );
      })}
    </section>
  );
}
