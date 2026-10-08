import { useEffect, useMemo, useRef, useState } from 'react';
import { useT } from '../i18n';
import type { LocalSectionMeta } from '../lib/localBook';
import { SearchIcon } from './Icons';

/** "Chương 12" for a numbered section, else "Phần 3 / 40" (or the pages of a PDF part). */
export function useSectionName() {
  const t = useT();
  return (meta: LocalSectionMeta | undefined, index: number, total: number) =>
    meta?.number != null
      ? t('common.chapter', { n: meta.number })
      : meta?.pages && !meta.label
        ? t('book.pages', { from: meta.pages[0], to: meta.pages[1] })
        : t('book.part', { n: index + 1, total });
}

/** The book's chapters as a sheet: the current one is marked and scrolled into view. */
export function ChapterSheet({ sections, current, onPick, onClose }: {
  sections: LocalSectionMeta[];
  current: number;
  onPick: (index: number) => void;
  onClose: () => void;
}) {
  const t = useT();
  const name = useSectionName();
  const [q, setQ] = useState('');
  const currentRef = useRef<HTMLButtonElement>(null);
  useEffect(() => { currentRef.current?.scrollIntoView({ block: 'center' }); }, []);

  const rows = useMemo(() => {
    const all = sections.map((meta, i) => ({ i, name: name(meta, i, sections.length), label: meta.label }));
    const query = q.trim().toLowerCase();
    if (!query) return all;
    return all.filter(r => {
      const n = sections[r.i].number;
      return (n != null && String(n) === query) || `${r.name} ${r.label ?? ''}`.toLowerCase().includes(query);
    });
  }, [sections, q]);

  return (
    <div className="dialog-backdrop dialog-backdrop--sheet" onClick={onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('book.toc')} onClick={e => e.stopPropagation()}
        style={{ maxHeight: '82vh', gap: 12 }}>
        <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 12 }}>
          <div className="dialog-title" style={{ fontSize: 22 }}>{t('book.toc')}</div>
          <span className="muted" style={{ fontSize: 13 }}>{t('common.chapters', { n: sections.length })}</span>
        </div>
        {sections.length > 8 && (
          <div className="search" style={{ position: 'relative' }}>
            <span style={{ position: 'absolute', left: 14, top: 13, opacity: .6, display: 'flex' }}><SearchIcon size={20} /></span>
            <input className="input" value={q} onChange={e => setQ(e.target.value)} placeholder={t('book.tocFilter')} aria-label={t('book.tocFilter')} />
          </div>
        )}
        <div className="nx-scroll" style={{ overflowY: 'auto', minHeight: 0, margin: '0 -8px', padding: '0 8px', display: 'flex', flexDirection: 'column', gap: 2 }}>
          {rows.map(r => {
            const on = r.i === current;
            return (
              <button key={r.i} ref={on ? currentRef : undefined} aria-current={on || undefined} onClick={() => { onPick(r.i); onClose(); }}
                style={{
                  display: 'flex', flexShrink: 0, alignItems: 'baseline', gap: 12, width: '100%', padding: '11px 14px', border: 'none', borderRadius: 18,
                  background: on ? 'var(--color-accent-100)' : 'none', color: 'inherit', font: 'inherit', textAlign: 'left', cursor: 'pointer'
                }}>
                <span style={{ flex: '0 0 auto', fontWeight: 700, fontSize: 14, color: on ? 'var(--color-accent-700)' : undefined }}>{r.name}</span>
                {r.label && <span className="muted" style={{ flex: 1, minWidth: 0, fontSize: 14, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{r.label}</span>}
              </button>
            );
          })}
          {rows.length === 0 && <p className="muted" style={{ margin: '8px 6px', fontSize: 14 }}>{t('book.tocNone')}</p>}
        </div>
      </div>
    </div>
  );
}
