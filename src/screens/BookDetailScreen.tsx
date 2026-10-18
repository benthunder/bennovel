import { useState } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { AiTag, ChapterSheet, useSectionName } from '../components/ChapterSheet';
import { EditBookDialog } from '../components/EditBookDialog';
import { BookOpenIcon, ChevronLeftIcon, ChevronRightIcon, PencilIcon, PlayIcon, SparklesIcon, XIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { TranslateFields, defaultTranslation, type Translation } from '../components/ReaderTools';
import { Spinner } from '../components/ui';
import { useI18n, useT } from '../i18n';
import {
  AI_LANGS, AiFailure, aiTranslateBook, aiViewKey, cancelAiTranslation, getAiMarks, getAiProgress, getAiVersions, getLlmSettings, getReadingLang, styleOf,
  type AiChoice, type AiLang, type AiView, type AiMark, type AiProgress, type AiVersion
} from '../lib/ai';
import { save as store } from '../lib/storage';
import { deleteLibraryBook, getLibraryDetail, localCover, openLibraryBook, updateLibraryBook } from '../lib/localBook';
import { useAsync } from '../lib/useAsync';
import { getWiki } from '../lib/wiki';
import { useLayout } from '../lib/useLayout';
import { useApp } from '../store/AppStore';

/**
 * The page of a book in the on-device library, laid out like an online novel's:
 * cover, details, Start/Continue, description and chapters. Works offline.
 */
export function BookDetailScreen({ novelId }: { novelId: number }) {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const user = app.libKey;
  const sectionName = useSectionName();
  // Loaded again after saving the details.
  const [rev, setRev] = useState(0);
  const q = useAsync(() => getLibraryDetail(user, novelId), [user, novelId, rev, app.libraryRev]);
  // Which chapters have an AI version, marked in the chapter list.
  const marksQ = useAsync(async () => {
    const m = new Map<number, AiMark[]>();
    for (const mark of (await getAiMarks(user, novelId)) ?? []) m.set(mark.index, [...(m.get(mark.index) ?? []), mark]);
    return m;
  }, [user, novelId, app.aiRev]);
  const marks = marksQ.status === 'ready' ? marksQ.data : undefined;
  const [editing, setEditing] = useState(false);
  const [toc, setToc] = useState(false);
  const [opening, setOpening] = useState(false);

  const fail = (key: 'book.openFailed' | 'library.saveFailed', e: unknown) => {
    console.error(e);
    app.showToast(`${t(key)} ${String(e)}`);
  };

  if (q.status !== 'ready') {
    return (
      <div className="screen" style={{ padding: 'calc(var(--safe-top) + 12px) 20px' }}>
        <button className="icon-btn icon-btn--bg" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon /></button>
        <div style={{ marginTop: 24 }}>{q.status === 'failed' ? <p role="alert">{t('book.openFailed')} {String(q.error)}</p> : <Spinner size={18} />}</div>
      </div>
    );
  }

  const { book: b, sections } = q.data;
  const total = sections.length;
  const last = b.lastRead && b.lastRead.index < total ? b.lastRead.index : null;
  const pct = last == null ? 0 : Math.round(((last + 1) / Math.max(1, total)) * 100);

  const read = async (at?: number) => {
    if (opening) return;
    setOpening(true);
    try {
      const { book, place } = await openLibraryBook(user, novelId, at);
      app.push({ s: 'book', book, place });
    } catch (e) {
      fail('book.openFailed', e);
      setOpening(false);
    }
  };

  const remove = async () => {
    if (!(await ask(t('library.deleteAsk', { title: b.title }), { kind: 'warning' }))) return;
    try {
      await deleteLibraryBook(user, novelId);
      app.forgetLocal(novelId);
      app.back();
    } catch (e) {
      fail('book.openFailed', e);
    }
  };

  const startBtn = (
    <button className={`btn btn-primary${lay.wide ? ' btn-block' : ''}`} disabled={opening || total === 0}
      style={{ height: 52, fontSize: 16, gap: 10, marginTop: lay.wide ? 16 : 0 }} onClick={() => read()}>
      {opening ? <Spinner size={16} /> : <PlayIcon size={16} />}
      {last != null ? t('bookDetail.continue', { name: sectionName(sections[last], last, total) }) : t('bookDetail.start')}
    </button>
  );

  const shown = sections.slice(0, lay.chPreview);
  const meta = [b.format?.toUpperCase(), b.lang.toUpperCase()].filter(Boolean) as string[];

  return (
    <div className="screen" style={{ paddingBottom: 'calc(var(--nav-h) + 32px)' }}>
      <div style={{ display: 'grid', gridTemplateColumns: lay.detailCols, gap: lay.detailGap, padding: lay.wide ? `${lay.v(0, 44, 32)}px ${lay.px}px 0` : 0, alignItems: 'start' }}>
        <div style={{
          position: lay.wide ? 'sticky' : 'relative', top: lay.wide ? lay.v(0, 44, 32) : 'auto', background: 'var(--color-surface)',
          borderRadius: lay.wide ? 34 : '0 0 44px 44px', padding: lay.wide ? 20 : 'calc(var(--safe-top) + 12px) 20px 24px'
        }}>
          <div style={{ display: 'flex', gap: 8, marginBottom: 18 }}>
            <button className="icon-btn icon-btn--bg" aria-label={t('common.back')} onClick={app.back} style={{ marginRight: 'auto' }}><ChevronLeftIcon /></button>
            <button className="icon-btn icon-btn--bg" aria-label={t('library.edit')} title={t('library.edit')} onClick={() => setEditing(true)}><PencilIcon size={18} /></button>
            <button className="icon-btn icon-btn--bg" aria-label={t('library.delete')} title={t('library.delete')} onClick={remove}><XIcon /></button>
          </div>
          <div style={{ display: 'flex', flexDirection: lay.wide ? 'column' : 'row', gap: 18, alignItems: lay.wide ? 'stretch' : 'flex-end' }}>
            {lay.wide
              ? <NovelCover novel={{ title: b.title, cover: localCover(b.title) }} w="100%" h="auto" fs={lay.dCoverFs} style={{ maxWidth: lay.dCoverMax, aspectRatio: 0.7, boxShadow: 'var(--shadow-md)' }} />
              : <NovelCover novel={{ title: b.title, cover: localCover(b.title) }} w={124} h={178} fs={18} style={{ boxShadow: 'var(--shadow-md)' }} />}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('bookDetail.onDevice')}</span>
                {meta.map(m => <span key={m} className="tag tag-neutral">{m}</span>)}
              </div>
              <h3 style={{ margin: 0, fontSize: lay.dTitleFs, overflowWrap: 'anywhere' }}>{b.title}</h3>
              <button className="text-link" style={{ fontSize: 14, textAlign: 'left' }} onClick={() => setEditing(true)}>{b.author ?? t('library.fieldAuthorEmpty')}</button>
            </div>
          </div>
          <div style={{ display: 'flex', gap: 10, marginTop: 20 }}>
            <Stat value={total} label={t('bookDetail.chapters')} />
            <Stat value={`${pct}%`} label={t('bookDetail.progress')} />
            <Stat value={b.format?.toUpperCase() ?? '—'} label={t('bookDetail.format')} />
          </div>
          {lay.wide && startBtn}
          {lay.wide && <AiBox novelId={novelId} lang={b.lang} onRead={read} />}
          {lay.wide && <WikiButton novelId={novelId} title={b.title} />}
        </div>

        <div style={{ padding: lay.wide ? 0 : '20px 20px 0', display: 'flex', flexDirection: 'column', gap: lay.wide ? 22 : 20, minWidth: 0 }}>
          {lay.isPhone && startBtn}
          {!lay.wide && <AiBox novelId={novelId} lang={b.lang} onRead={read} />}
          {!lay.wide && <WikiButton novelId={novelId} title={b.title} />}

          <div>
            <h5 className="section-title" style={{ marginBottom: 6 }}>{t('bookDetail.synopsis')}</h5>
            {b.description.trim()
              ? <p style={{ margin: 0, fontSize: lay.synFs, lineHeight: 1.6, color: 'var(--color-neutral-800)', textWrap: 'pretty', maxWidth: 680, whiteSpace: 'pre-line' }}>{b.description}</p>
              : <button className="text-link" style={{ fontSize: 14 }} onClick={() => setEditing(true)}>{t('bookDetail.addDescription')}</button>}
            {b.sourceName && <div className="muted" style={{ fontSize: 12, marginTop: 8, overflowWrap: 'anywhere' }}>{t('bookDetail.file', { name: b.sourceName })}</div>}
          </div>

          <div>
            <h5 className="section-title" style={{ marginBottom: 8 }}>{t('bookDetail.chapters')}</h5>
            <div style={{ display: 'grid', gridTemplateColumns: lay.chCols, gap: '2px 12px' }}>
              {shown.map((s, i) => {
                const done = last != null && i <= last;
                const name = sectionName(s, i, total);
                return (
                  <button key={i} className="novel-row" style={{ alignItems: 'center', gap: 12, padding: '10px 8px', borderRadius: 20, minWidth: 0 }} onClick={() => read(i)}>
                    <span style={{ minWidth: 36, height: 36, padding: '0 6px', flex: 'none', borderRadius: 18, display: 'grid', placeItems: 'center', fontSize: 12, fontWeight: 700, background: done ? 'var(--color-accent-2-200)' : 'var(--color-surface)', color: done ? 'var(--color-accent-2-800)' : 'var(--color-text)' }}>{s.number ?? i + 1}</span>
                    <span style={{ flex: 1, minWidth: 0 }}>
                      <span style={{ display: 'block', fontSize: 14, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{s.label ?? name}</span>
                      <span className="muted" style={{ fontSize: 11 }}>
                        {done ? t(i === last ? 'bookDetail.lastRead' : 'bookDetail.read') : s.label ? name : s.pages ? t('book.pages', { from: s.pages[0], to: s.pages[1] }) : ''}
                      </span>
                    </span>
                    <AiTag marks={marks?.get(i)} />
                    <ChevronRightIcon size={16} style={{ flex: 'none', color: 'var(--color-neutral-500)' }} />
                  </button>
                );
              })}
            </div>
            {total > shown.length && (
              <button className="btn btn-secondary btn-block" style={{ marginTop: 6 }} onClick={() => setToc(true)}>
                {t('bookDetail.showMore', { n: total })}
              </button>
            )}
          </div>
        </div>
      </div>

      {toc && <ChapterSheet sections={sections} current={last ?? 0} marks={marks} onPick={i => read(i)} onClose={() => setToc(false)} />}
      {editing && (
        <EditBookDialog book={b} onClose={() => setEditing(false)} onSave={async details => {
          await updateLibraryBook(user, novelId, details);
          setEditing(false);
          app.showToast(t('library.saved'));
          setRev(n => n + 1);
        }} />
      )}
    </div>
  );
}

/** Opens the book's wiki, with how many entries it has. */
function WikiButton({ novelId, title }: { novelId: number; title: string }) {
  const t = useT();
  const app = useApp();
  const user = app.libKey;
  const q = useAsync(() => getWiki(user, novelId), [user, novelId, app.aiRev]);
  const n = q.status === 'ready' ? q.data?.entries.length ?? null : null;
  return (
    <button className="btn btn-secondary btn-block" style={{ gap: 8, marginTop: 8 }} onClick={() => app.push({ s: 'bookWiki', novelId, title, lang: getReadingLang(user, novelId) })}>
      <BookOpenIcon size={16} />{t('wiki.open')}
      {n != null && n > 0 && <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>· {t('wiki.count', { n })}</span>}
    </button>
  );
}

function Stat({ value, label }: { value: React.ReactNode; label: string }) {
  return (
    <div style={{ flex: 1, minWidth: 0, background: 'var(--color-bg)', borderRadius: 20, padding: '10px 12px' }}>
      <div className="display" style={{ fontSize: 18, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{value}</div>
      <div className="muted" style={{ fontSize: 11 }}>{label}</div>
    </div>
  );
}

/**
 * AI translation of the whole book with the user's own LLM: the chapters in the
 * background queue, the AI versions made so far, and a button to queue more.
 */
function AiBox({ novelId, lang, onRead }: { novelId: number; lang: string; onRead: (at?: number) => void }) {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const llmQ = useAsync(() => getLlmSettings(user), [user]);
  const progressQ = useAsync(() => getAiProgress(user, novelId), [user, novelId, app.aiRev]);
  const versionsQ = useAsync(() => getAiVersions(user, novelId), [user, novelId, app.aiRev]);
  // Keep the last numbers on screen while they reload.
  const [progress, setProgress] = useState<AiProgress[]>([]);
  if (progressQ.status === 'ready' && progressQ.data && progressQ.data !== progress) setProgress(progressQ.data);
  const [open, setOpen] = useState(false);
  const [tr, setTr] = useState<Translation | null>(null);
  const [busy, setBusy] = useState(false);

  const langName = (c: string) => AI_LANGS.find(l => l.code === c)?.native ?? c;
  const choice = tr ?? defaultTranslation(lang === 'vi' ? 'vi' : 'en');
  const active = progress.filter(p => p.queued + p.running + p.failed > 0);
  const versions = (versionsQ.status === 'ready' && versionsQ.data) || [];
  const hasLlm = llmQ.status === 'ready' && !!llmQ.data;

  const start = async () => {
    setBusy(true);
    try {
      const c: AiChoice = { lang: choice.lang as AiLang, style: choice.style, dict: choice.dict };
      const r = await aiTranslateBook(user, novelId, 0, c, uiLang);
      const vars = { n: r.queued, lang: langName(c.lang), style: t(`tr.style.${c.style}`) };
      app.showToast(t(r.queued === 0 ? 'ai.nothing' : r.mode === 'rewrite' ? 'ai.queuedRewrite' : 'ai.queued', vars));
      setOpen(false);
      setProgress((await getAiProgress(user, novelId)) ?? []);
    } catch (e) {
      console.error(e);
      app.showToast(t(e instanceof AiFailure ? e.key : 'ai.err.generic'));
    } finally {
      setBusy(false);
    }
  };
  const stop = async () => {
    await cancelAiTranslation(user, novelId).catch(console.error);
    app.showToast(t('ai.cancelled'));
    setProgress((await getAiProgress(user, novelId).catch(() => null)) ?? []);
  };
  const readVersion = (v: AiVersion) => {
    store(aiViewKey(user, novelId), { lang: v.lang, style: styleOf(v.style), dict: true, view: v.lang } satisfies AiView);
    onRead();
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 10 }}>
      {active.map(p => (
        <div key={`${p.lang}.${p.style}`} className="import-progress" role="status">
          {p.queued + p.running > 0 ? <Spinner size={14} /> : <SparklesIcon size={14} />}
          <span style={{ flex: 1 }}>
            {t('ai.progress', { lang: langName(p.lang), style: t(`tr.style.${styleOf(p.style)}`), done: p.done, total: p.total })}
            {p.queued + p.running > 0 && ` · ${t('tr.waiting', { n: p.queued + p.running })}`}
            {p.failed > 0 && ` · ${t('tr.failedCount', { n: p.failed })}`}
          </span>
          {p.queued > 0 && <button className="btn btn-ghost" style={{ height: 30, padding: '0 10px', fontSize: 12 }} onClick={stop}>{t('ai.cancel')}</button>}
          <div className="import-progress__bar"><div style={{ width: `${p.total ? Math.round((p.done / p.total) * 100) : 0}%` }} /></div>
        </div>
      ))}
      {versions.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div style={{ fontSize: 12, fontWeight: 700 }}>{t('ai.versions')}</div>
          {versions.map(v => (
            <div key={`${v.lang}.${v.style}`} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, padding: '6px 10px', borderRadius: 16, background: 'var(--color-bg)' }}>
              <SparklesIcon size={14} />
              <span style={{ flex: 1 }}>{t('ai.versionRow', { lang: langName(v.lang), style: t(`tr.style.${styleOf(v.style)}`), n: v.chapters })}</span>
              <button className="btn btn-ghost" style={{ height: 30, padding: '0 10px', fontSize: 12 }} onClick={() => readVersion(v)}>{t('ai.read')}</button>
            </div>
          ))}
        </div>
      )}
      {!open && (
        <button className="btn btn-secondary btn-block" style={{ gap: 8 }} onClick={() => (hasLlm ? setOpen(true) : app.switchTab('profile'))}>
          <SparklesIcon size={16} />{hasLlm ? t('tr.novel') : t('ai.setUp')}
        </button>
      )}
      {!hasLlm && llmQ.status === 'ready' && <div className="muted" style={{ fontSize: 12 }}>{t('ai.err.notSet')}</div>}
      {open && (
        <div role="group" aria-label={t('tr.novel')} style={{ display: 'flex', flexDirection: 'column', gap: 12, padding: 14, borderRadius: 26, background: 'var(--color-surface)', border: '2px solid var(--color-neutral-200)' }}>
          <div className="display" style={{ fontSize: 17, display: 'flex', alignItems: 'center', gap: 8 }}><SparklesIcon size={16} />{t('tr.novel')}</div>
          <div className="muted" style={{ fontSize: 12, marginTop: -4 }}>{t('ai.hint')}</div>
          <TranslateFields value={choice} onChange={setTr} langs={[...AI_LANGS]} glossaryCount={0} />
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn btn-ghost" style={{ flex: 1 }} disabled={busy} onClick={() => setOpen(false)}>{t('common.cancel')}</button>
            <button className="btn btn-primary" style={{ flex: 2, gap: 8 }} disabled={busy} onClick={start}>
              {busy && <Spinner size={14} />}{t('tr.novelStart')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
