import { useEffect, useRef, useState } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { useI18n, useT } from '../i18n';
import {
  AI_LANGS, aiErrorDetail, aiErrorKey, aiTranslateChapter, clearAiTranslation, getAiChapter, getAiDraft,
  getAiStatus, onAiMoved, pauseAiTranslation, resumeAiTranslation, stopAiChapter, styleOf,
  type AiBookProgress, type AiLang, type AiLive, type AiManual, type AiStage, type AiStatus
} from '../lib/ai';
import { IMAGE_MARK, canOpenLocalBooks } from '../lib/localBook';
import { useApp } from '../store/AppStore';
import { Modal } from './Modal';
import { BookOpenIcon, ChevronRightIcon, EyeIcon, RefreshIcon, SparklesIcon, XIcon } from './Icons';
import { Spinner } from './ui';

/** The chapter whose parts the "watch live" sheet shows. */
interface Watch { novelId: number; index: number; lang: string; style: string; title: string }

/**
 * The translation pipeline, on every screen: each book in the queue and each chapter
 * translated from the reader, the node of the graph it is in, and what can be done
 * with it (pause, carry on, try again, clear, watch the text as it comes). A pill while
 * folded; it reads the status again when a translation moves (at most a few times a
 * second), and only this panel renders again.
 */
export function AiDock({ bottom }: { bottom: number }) {
  const app = useApp();
  const t = useT();
  const user = app.libKey;
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [open, setOpen] = useState(false);
  const [watch, setWatch] = useState<Watch | null>(null);

  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    let gone = false;
    const load = () => { getAiStatus(user).then(s => { if (!gone) setStatus(s); }).catch(e => console.warn(e)); };
    load();
    const stop = onAiMoved(load);
    return () => { gone = true; stop(); };
  }, [user, app.aiRev]);

  const books = status?.books ?? [];
  const chapters = status?.chapters ?? [];
  const live = status?.live ?? null;
  if (!books.length && !chapters.length) return watch ? <DraftSheet user={user} watch={watch} onClose={() => setWatch(null)} /> : null;

  const running = books.filter(b => b.queued + b.running > 0).length + chapters.filter(c => !c.error).length;
  const trouble = books.some(b => b.paused > 0 || (b.failed > 0 && b.queued + b.running === 0)) || chapters.some(c => c.error);
  const reload = () => getAiStatus(user).then(setStatus).catch(console.warn);

  return (
    <>
      <div className="ai-dock" style={{ bottom }} data-open={open || undefined}>
        {open ? (
          <div className="ai-dock__panel glass" role="region" aria-label={t('pipe.title')}>
            <div className="ai-dock__head">
              <SparklesIcon size={16} />
              <span style={{ flex: 1, fontWeight: 800 }}>{t('pipe.title')}</span>
              <button className="icon-btn icon-btn--plain" aria-label={t('pipe.collapse')} onClick={() => setOpen(false)}><XIcon size={16} /></button>
            </div>
            <div className="ai-dock__list">
              {chapters.map(c => (
                <ChapterRow key={`c${c.novelId}.${c.index}`} c={c} user={user} onChange={reload} onWatch={() => setWatch(watchOf(c, c.title))} />
              ))}
              {books.map(b => (
                <BookRow key={`b${b.novelId}.${b.lang}.${b.style}`} b={b} user={user}
                  live={live && live.novelId === b.novelId && live.lang === b.lang && live.style === b.style ? live : null}
                  onChange={reload}
                  onWatch={l => setWatch(watchOf(l, b.title))}
                  onOpen={() => { setOpen(false); app.push({ s: 'bookDetail', novelId: b.novelId }); }} />
              ))}
            </div>
          </div>
        ) : (
          <button className={`ai-dock__pill glass${trouble ? ' ai-dock__pill--warn' : ''}`} onClick={() => setOpen(true)} aria-expanded={false}>
            {running > 0 ? <Spinner size={13} /> : <SparklesIcon size={14} />}
            <span>{running > 0 ? t('pipe.running', { n: running }) : trouble ? t('pipe.attention') : t('pipe.idle')}</span>
            {running > 0 && live && <span className="muted">· {t(`pipe.stage.${live.stage}`)}</span>}
            {trouble && running > 0 && <span className="ai-dock__dot" aria-hidden="true" />}
          </button>
        )}
      </div>
      {watch && <DraftSheet user={user} watch={watch} onClose={() => setWatch(null)} />}
    </>
  );
}

const watchOf = (l: AiLive, title: string): Watch => ({ novelId: l.novelId, index: l.index, lang: l.lang, style: l.style, title });

function useLangName() {
  return (c: string) => AI_LANGS.find(l => l.code === c)?.native ?? c;
}

/** The graph's nodes in order, the one being worked on marked and the ones before it done. */
function Stages({ live }: { live: AiLive }) {
  const t = useT();
  const rewrite = live.stage === 'rewrite';
  const order: AiStage[] = ['detect', 'recall', rewrite ? 'rewrite' : 'translate', 'save', 'learn'];
  const at = order.indexOf(live.stage);
  return (
    <ol className="ai-stages" aria-label={t('pipe.title')}>
      {order.map((s, i) => (
        <li key={s} data-state={i < at ? 'done' : i === at ? 'now' : 'todo'}>
          {t(`pipe.stage.${s}`)}
          {i === at && live.partsTotal > 0 && ` ${live.partsDone}/${live.partsTotal}`}
        </li>
      ))}
    </ol>
  );
}

function errorText(t: ReturnType<typeof useT>, e: string) {
  const key = aiErrorKey(e);
  const detail = key === 'ai.err.http' || key === 'ai.err.generic' ? ` ${aiErrorDetail(e)}` : '';
  return t(key) + detail;
}

function BookRow({ b, user, live, onChange, onWatch, onOpen }: {
  b: AiBookProgress; user: string; live: AiLive | null; onChange: () => void; onWatch: (l: AiLive) => void; onOpen: () => void;
}) {
  const t = useT();
  const { uiLang } = useI18n();
  const app = useApp();
  const langName = useLangName();
  const [busy, setBusy] = useState(false);
  const working = b.queued + b.running > 0;
  const act = (f: () => Promise<unknown>, toast?: string) => async () => {
    setBusy(true);
    try {
      await f();
      if (toast) app.showToast(toast);
    } catch (e) {
      app.showToast(errorText(t, String(e)));
    } finally {
      setBusy(false);
      onChange();
    }
  };
  const clear = async () => {
    if (!(await ask(t('pipe.clearAsk', { title: b.title }), { kind: 'warning' }).catch(() => false))) return;
    await act(() => clearAiTranslation(user, b.novelId), t('pipe.cleared'))();
  };
  const small = { height: 30, padding: '0 10px', fontSize: 12 };
  return (
    <div className="ai-dock__item">
      <div className="ai-dock__title">
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{b.title}</span>
        <button className="icon-btn icon-btn--plain" aria-label={t('pipe.open')} title={t('pipe.open')} onClick={onOpen}><ChevronRightIcon size={16} /></button>
      </div>
      <div className="muted" style={{ fontSize: 12 }}>
        {t('pipe.book', { done: b.done, total: b.total })} · {langName(b.lang)} · {t(`tr.style.${styleOf(b.style)}`)}
      </div>
      <div className="import-progress__bar" style={{ marginTop: 6 }}><div style={{ width: `${b.total ? Math.round((b.done / b.total) * 100) : 0}%` }} /></div>
      {live ? (
        <>
          <div style={{ fontSize: 12, marginTop: 6 }}>{t('common.chapter', { n: live.index + 1 })}</div>
          <Stages live={live} />
        </>
      ) : working ? (
        <div className="muted" style={{ fontSize: 12, marginTop: 6 }}>{t('pipe.waiting')}</div>
      ) : null}
      {b.paused > 0 && (
        <div className={b.error ? 'ai-dock__error' : 'muted'} style={{ fontSize: 12, marginTop: 6 }}>
          {b.error ? t('ai.pausedError', { reason: errorText(t, b.error) }) : t('ai.pausedWaiting')}
        </div>
      )}
      {b.failed > 0 && (
        <div className="ai-dock__error" style={{ fontSize: 12, marginTop: 6 }}>
          {t('pipe.failedN', { n: b.failed })}{b.paused === 0 && b.error ? ` · ${errorText(t, b.error)}` : ''}
        </div>
      )}
      <div className="ai-dock__actions">
        {live && <button className="btn btn-ghost" style={{ ...small, gap: 6 }} onClick={() => onWatch(live)}><EyeIcon size={14} />{t('pipe.watch')}</button>}
        {working && <button className="btn btn-ghost" style={small} disabled={busy} onClick={act(() => pauseAiTranslation(user, b.novelId), t('ai.pausedByUser'))}>{t('ai.pause')}</button>}
        {!working && b.paused > 0 && (
          <button className="btn btn-primary" style={small} disabled={busy} onClick={act(() => resumeAiTranslation(user, b.novelId, b.failed > 0, uiLang))}>{t('ai.resume')}</button>
        )}
        {!working && b.paused === 0 && b.failed > 0 && (
          <button className="btn btn-primary" style={small} disabled={busy} onClick={act(() => resumeAiTranslation(user, b.novelId, true, uiLang))}>{t('ai.retry')}</button>
        )}
        <button className="btn btn-ghost ai-dock__danger" style={small} disabled={busy} onClick={clear}>{t('pipe.clear')}</button>
      </div>
    </div>
  );
}

function ChapterRow({ c, user, onChange, onWatch }: { c: AiManual; user: string; onChange: () => void; onWatch: () => void }) {
  const t = useT();
  const app = useApp();
  const langName = useLangName();
  const [busy, setBusy] = useState(false);
  const small = { height: 30, padding: '0 10px', fontSize: 12 };
  const stop = async () => {
    setBusy(true);
    await stopAiChapter(user, c.novelId, c.index).catch(console.warn);
    setBusy(false);
    onChange();
  };
  // Tried again from here: the reader shows it once it is saved (marks reload with aiRev).
  const retry = () => {
    void aiTranslateChapter(user, c.novelId, c.index, { lang: c.lang as AiLang, style: styleOf(c.style), dict: c.glossary }, true)
      .then(() => { app.showToast(t('ai.chapterReady', { name: t('common.chapter', { n: c.index + 1 }) })); app.reloadAi(); })
      .catch(e => app.showToast(errorText(t, String(e))));
    setTimeout(onChange, 300);
  };
  return (
    <div className="ai-dock__item">
      <div className="ai-dock__title">
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.title || t('book.untitled')}</span>
      </div>
      <div className="muted" style={{ fontSize: 12 }}>
        {t('common.chapter', { n: c.index + 1 })} · {t('pipe.fromReader')} · {langName(c.lang)} · {t(`tr.style.${styleOf(c.style)}`)}
      </div>
      <Stages live={c} />
      {c.error && <div className="ai-dock__error" style={{ fontSize: 12, marginTop: 6 }}>{t('pipe.failedError', { reason: errorText(t, c.error) })}</div>}
      <div className="ai-dock__actions">
        {!c.error && <button className="btn btn-ghost" style={{ ...small, gap: 6 }} onClick={onWatch}><EyeIcon size={14} />{t('pipe.watch')}</button>}
        {c.error && <button className="btn btn-primary" style={{ ...small, gap: 6 }} onClick={retry}><RefreshIcon size={14} />{t('pipe.retry')}</button>}
        <button className={`btn btn-ghost${c.error ? ' ai-dock__danger' : ''}`} style={small} disabled={busy} onClick={stop}>{c.error ? t('pipe.clear') : t('pipe.stop')}</button>
      </div>
    </div>
  );
}

/**
 * A chapter's translation as its parts come back: the draft's paragraphs, read again
 * each time the pipeline moves, and the finished chapter once it is saved.
 */
export function DraftSheet({ user, watch, onClose }: { user: string; watch: Watch; onClose: () => void }) {
  const t = useT();
  const [text, setText] = useState<{ title: string; paragraphs: string[]; over: boolean } | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    let gone = false;
    const load = async () => {
      try {
        const d = await getAiDraft(user, watch.novelId, watch.index, watch.lang, watch.style);
        if (gone) return;
        if (d) { setText({ ...d, over: false }); return; }
        const done = await getAiChapter(user, watch.novelId, watch.index, watch.lang as AiLang, styleOf(watch.style));
        if (!gone) setText(done ? { title: done.title, paragraphs: done.paragraphs, over: true } : null);
      } catch (e) {
        console.warn(e);
      }
    };
    void load();
    const stop = onAiMoved(() => { void load(); }, 600);
    return () => { gone = true; stop(); };
  }, [user, watch]);
  // Follows the new text while the reader is at the bottom; leaves the page alone otherwise.
  useEffect(() => {
    const el = bodyRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [text?.paragraphs.length]);
  return (
    <Modal onClose={onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('pipe.watch')} onClick={e => e.stopPropagation()}
        style={{ display: 'flex', flexDirection: 'column', maxHeight: '86vh' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <BookOpenIcon size={18} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="dialog-title" style={{ fontSize: 18 }}>{text?.title || watch.title}</div>
            <div className="muted" style={{ fontSize: 12, display: 'flex', gap: 6, alignItems: 'center' }}>
              {!text?.over && <Spinner size={11} />}
              {t('common.chapter', { n: watch.index + 1 })} · {t('pipe.watch')}
            </div>
          </div>
          <button className="icon-btn icon-btn--plain" aria-label={t('common.close')} onClick={onClose}><XIcon size={18} /></button>
        </div>
        <div ref={bodyRef} className="ai-draft" onScroll={e => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}>
          {!text || !text.paragraphs.length
            ? <p className="muted">{t('pipe.draftEmpty')}</p>
            : text.paragraphs.filter(p => !p.startsWith(IMAGE_MARK)).map((p, i) => <p key={i}>{p}</p>)}
          {text?.over && <p className="muted" style={{ fontSize: 12 }}>{t('pipe.draftOver')}</p>}
        </div>
      </div>
    </Modal>
  );
}
