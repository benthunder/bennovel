import { useEffect, useRef, useState } from 'react';
import { ChevronLeftIcon, SlidersIcon, SparklesIcon } from '../components/Icons';
import { ReaderTools, type Translation } from '../components/ReaderTools';
import { Spinner } from '../components/ui';
import { getChapterText, getContentLangs, getGlossary, getNovel } from '../data/repository';
import { useAsync } from '../lib/useAsync';
import type { ContentLang } from '../data/types';
import { useI18n } from '../i18n';
import { READER_THEMES, useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';

const GAP_PX = { tight: 6, normal: 16, airy: 28 };
const MARGIN_PX = { narrow: 16, normal: 24, wide: 36 };
const TR_STEP_MS = 550;

export function ReaderScreen({ id, ch, lang }: { id: number; ch: number; lang: ContentLang }) {
  const { t } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const novel = getNovel(id);
  const p = app.readerPrefs;
  const night = p.theme === 'night';
  const theme = READER_THEMES[p.theme];
  const line = night ? 'var(--color-neutral-700)' : 'color-mix(in srgb, currentColor 14%, transparent)';

  const [toolsOpen, setToolsOpen] = useState(false);
  const [translated, setTranslated] = useState<Translation | null>(null);
  const [trStep, setTrStep] = useState<number | null>(null);
  const [readPct, setReadPct] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);

  const dispLang = translated?.lang ?? lang;
  const glossaryQ = useAsync(() => getGlossary(id), [id]);
  const glossary = glossaryQ.status === 'ready' ? glossaryQ.data : {};
  const textQ = useAsync(() => getChapterText(id, ch, dispLang), [id, ch, dispLang]);
  const chapter = textQ.status === 'ready' ? textQ.data : null;
  const langName = (c: ContentLang) => getContentLangs().find(l => l.code === c)?.native ?? c;

  // Simulated AI translation until the translate API exists.
  const runTranslate = (tr: Translation) => {
    if (lay.isPhone) setToolsOpen(false);
    setTrStep(0);
    timers.current.push(
      setTimeout(() => setTrStep(1), TR_STEP_MS),
      setTimeout(() => setTrStep(2), TR_STEP_MS * 2),
      setTimeout(() => { setTrStep(null); setTranslated(tr); scrollRef.current?.scrollTo({ top: 0 }); }, TR_STEP_MS * 3)
    );
  };

  const paragraphs = (chapter?.paragraphs ?? []).map(para => para.split(/\[\[([\w-]+)\]\]/).map((seg, i) => {
    if (i % 2 === 0) return { text: seg, hl: false };
    const g = glossary[seg] ?? {};
    // Without the dictionary the translation keeps source-language names; with it, glossary names are used and highlighted.
    const useDict = !translated || translated.dict;
    return { text: (useDict ? g[dispLang] : g[lang]) ?? g[dispLang] ?? seg, hl: !!translated?.dict };
  }));

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget, max = el.scrollHeight - el.clientHeight;
    setReadPct(max > 0 ? el.scrollTop / max : 1);
  };

  return (
    <div className="screen" style={{ overflow: 'hidden', background: theme.bg, color: theme.fg }}>
      {/* On wide screens the open tools panel sits beside the chapter instead of over it. */}
      <div style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: lay.wide && toolsOpen ? lay.toolsW : 0, display: 'flex', flexDirection: 'column', transition: 'right .28s ease' }}>
        <div style={{ padding: `${lay.readerTop} ${lay.readerPx}px 8px`, display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="icon-btn icon-btn--plain" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon size={22} /></button>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase', opacity: .65, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{novel.title}</div>
            <div className="display" style={{ fontSize: 16 }}>{t('common.chapter', { n: ch })}</div>
          </div>
          <button onClick={() => app.setLangDialog({ mode: 'reader' })}
            style={{ height: 34, padding: '0 12px', borderRadius: 999, border: `2px solid ${line}`, background: 'none', color: theme.fg, font: 'inherit', fontSize: 12, fontWeight: 700, cursor: 'pointer' }}>
            {dispLang.toUpperCase()}
          </button>
          <button className="icon-btn icon-btn--accent" aria-label={t('reader.tools')} onClick={() => setToolsOpen(o => lay.wide ? !o : true)}><SlidersIcon /></button>
        </div>
        <div style={{ height: 5, margin: `0 ${lay.readerBarPx}px`, borderRadius: 999, background: line, overflow: 'hidden' }}>
          <div style={{ height: '100%', width: `${Math.round(readPct * 100)}%`, background: 'var(--color-accent)', borderRadius: 999 }} />
        </div>

        <div ref={scrollRef} className="nx-scroll" onScroll={onScroll}
          style={{ flex: 1, overflowY: 'auto', padding: `${lay.readerPadTop}px ${MARGIN_PX[p.margin]}px calc(env(safe-area-inset-bottom) + 40px)` }}>
          <div style={{ maxWidth: lay.readerMax, margin: '0 auto' }}>
            <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.1em', textTransform: 'uppercase', color: night ? 'var(--color-accent-400)' : 'var(--color-accent-700)' }}>
              {t('common.chapter', { n: ch })}
            </div>
            <h2 style={{ margin: '4px 0 18px', fontSize: lay.readerH, color: theme.fg }}>{chapter?.title}</h2>

            {translated && (
              <div style={{ display: 'flex', gap: 10, alignItems: 'flex-start', padding: '12px 14px', borderRadius: 22, background: night ? 'var(--color-neutral-800)' : 'var(--color-accent-100)', marginBottom: 18, fontSize: 12, lineHeight: 1.45 }}>
                <SparklesIcon size={16} style={{ flex: 'none', marginTop: 1 }} />
                <div style={{ flex: 1 }}>
                  {t('reader.banner', {
                    lang: langName(translated.lang),
                    style: t(`tr.style.${translated.style}`),
                    dict: translated.dict ? t('reader.dictKept', { n: Object.keys(glossary).length }) : t('reader.dictOff')
                  })}
                </div>
                <button onClick={() => setTranslated(null)} style={{ border: 'none', background: 'none', font: 'inherit', fontSize: 12, fontWeight: 700, textDecoration: 'underline', cursor: 'pointer', color: 'inherit', padding: 0 }}>
                  {t('reader.original')}
                </button>
              </div>
            )}

            {textQ.status === 'loading' && (
              <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: .75 }}><Spinner size={16} />{t('reader.loading')}</div>
            )}
            {textQ.status === 'failed' && <p role="alert">{t('load.failed')}</p>}
            {textQ.status === 'ready' && !chapter && <p role="alert">{t('reader.missing')}</p>}
            {paragraphs.map((segs, i) => (
              <p key={i} style={{ margin: `0 0 ${GAP_PX[p.gap]}px`, fontSize: p.fontSize, lineHeight: p.lineH, textAlign: p.align, textWrap: 'pretty' }}>
                {segs.map((s, j) => s.hl
                  ? <span key={j} style={{ background: night ? 'var(--color-accent-800)' : 'var(--color-accent-200)', borderRadius: 6, padding: '0 3px' }}>{s.text}</span>
                  : <span key={j}>{s.text}</span>)}
              </p>
            ))}

            <div style={{ display: 'flex', gap: 10, marginTop: 28 }}>
              <button className="btn btn-secondary" style={{ flex: 1, height: 48, color: theme.fg, borderColor: line }} disabled={ch <= 1} onClick={() => app.goReader(id, ch - 1, lang)}>{t('reader.prev')}</button>
              <button className="btn btn-primary" style={{ flex: 1, height: 48 }} disabled={ch >= novel.chapters} onClick={() => app.goReader(id, ch + 1, lang)}>{t('reader.next')}</button>
            </div>
          </div>
        </div>

        {trStep !== null && (
          <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', background: 'color-mix(in srgb, var(--color-neutral-900) 35%, transparent)', zIndex: 30 }}>
            <div style={{ background: 'var(--color-surface)', color: 'var(--color-text)', borderRadius: 30, padding: '22px 26px', display: 'flex', alignItems: 'center', gap: 14, boxShadow: 'var(--shadow-lg)' }}>
              <Spinner size={24} width={3} />
              <div>
                <div className="display" style={{ fontSize: 17 }}>{t('reader.translating')}</div>
                <div className="muted" style={{ fontSize: 12 }}>{t(`reader.step${trStep as 0 | 1 | 2}`)}</div>
              </div>
            </div>
          </div>
        )}
      </div>

      {toolsOpen && <ReaderTools glossary={glossary} onClose={() => setToolsOpen(false)} onTranslate={runTranslate} />}
    </div>
  );
}
