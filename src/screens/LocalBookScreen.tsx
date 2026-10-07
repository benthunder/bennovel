import { useEffect, useRef, useState } from 'react';
import { ChevronLeftIcon } from '../components/Icons';
import { Spinner } from '../components/ui';
import { useT } from '../i18n';
import { closeLocalBook, getLocalSection, type LocalBook } from '../lib/localBook';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { READER_THEMES, useApp } from '../store/AppStore';

const GAP_PX = { tight: 6, normal: 16, airy: 28 };
const MARGIN_PX = { narrow: 16, normal: 24, wide: 36 };

/** Reads a book file opened from the device, one section at a time. */
export function LocalBookScreen({ book }: { book: LocalBook }) {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const p = app.readerPrefs;
  const night = p.theme === 'night';
  const theme = READER_THEMES[p.theme];
  const line = night ? 'var(--color-neutral-700)' : 'color-mix(in srgb, currentColor 14%, transparent)';
  const scrollRef = useRef<HTMLDivElement>(null);

  const [index, setIndex] = useState(0);
  const total = book.sections.length;
  const meta = book.sections[index];
  const sectionQ = useAsync(() => getLocalSection(book.id, index), [book.id, index]);
  // Leaving the reader frees the file and every cached section.
  useEffect(() => () => { closeLocalBook(book.id).catch(console.error); }, [book.id]);
  useEffect(() => { scrollRef.current?.scrollTo({ top: 0 }); }, [index]);

  const heading = meta?.label ?? (meta?.pages ? t('book.pages', { from: meta.pages[0], to: meta.pages[1] }) : t('book.part', { n: index + 1, total }));

  return (
    <div className="screen" style={{ overflow: 'hidden', background: theme.bg, color: theme.fg, display: 'flex', flexDirection: 'column' }}>
      <div style={{ padding: `${lay.readerTop} ${lay.readerPx}px 8px`, display: 'flex', alignItems: 'center', gap: 10 }}>
        <button className="icon-btn icon-btn--plain" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon size={22} /></button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 11, fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase', opacity: .65, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{book.title ?? t('book.untitled')}</div>
          <div className="display" style={{ fontSize: 16 }}>{t('book.part', { n: index + 1, total })}</div>
        </div>
      </div>
      <div style={{ height: 5, margin: `0 ${lay.readerBarPx}px`, borderRadius: 999, background: line, overflow: 'hidden' }}>
        <div style={{ height: '100%', width: `${Math.round(((index + 1) / Math.max(total, 1)) * 100)}%`, background: 'var(--color-accent)', borderRadius: 999 }} />
      </div>

      <div ref={scrollRef} className="nx-scroll"
        style={{ flex: 1, overflowY: 'auto', padding: `${lay.readerPadTop}px ${MARGIN_PX[p.margin]}px calc(env(safe-area-inset-bottom) + 40px)` }}>
        <div style={{ maxWidth: lay.readerMax, margin: '0 auto' }}>
          <h2 style={{ margin: '4px 0 18px', fontSize: lay.readerH, color: theme.fg }}>{heading}</h2>
          {sectionQ.status === 'loading' && (
            <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, opacity: .75 }}><Spinner size={16} />{t('book.loading')}</div>
          )}
          {sectionQ.status === 'failed' && <p role="alert">{String(sectionQ.error)}</p>}
          {sectionQ.status === 'ready' && sectionQ.data.paragraphs.length === 0 && <p style={{ opacity: .7 }}>{t('book.empty')}</p>}
          {sectionQ.status === 'ready' && sectionQ.data.paragraphs.map((para, i) => (
            <p key={i} style={{ margin: `0 0 ${GAP_PX[p.gap]}px`, fontSize: p.fontSize, lineHeight: p.lineH, textAlign: p.align, textWrap: 'pretty' }}>{para}</p>
          ))}

          <div style={{ display: 'flex', gap: 10, marginTop: 28 }}>
            <button className="btn btn-secondary" style={{ flex: 1, height: 48, color: theme.fg, borderColor: line }} disabled={index <= 0} onClick={() => setIndex(i => i - 1)}>{t('book.prev')}</button>
            <button className="btn btn-primary" style={{ flex: 1, height: 48 }} disabled={index >= total - 1} onClick={() => setIndex(i => i + 1)}>{t('book.next')}</button>
          </div>
        </div>
      </div>
    </div>
  );
}
