import type { ReactNode } from 'react';
import { messageIn, useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import { RAW, isRelType, wikiLabel, wikiSummary, type Wiki, type WikiEntry, type WikiRelation } from '../lib/wiki';
import { Modal } from './Modal';

/**
 * The words of the app in the wiki's language: relation types and such follow the
 * reading language, not the app's own.
 */
export function useWikiT(lang: string) {
  const { t } = useI18n();
  return (key: MessageKey) => messageIn(lang, key) ?? t(key);
}

/** How relation `r` reads from entry `uid`'s side ("master of", or "disciple of" from the other end). */
export function relWords(r: WikiRelation, uid: string, lang: string, wt: (k: MessageKey) => string) {
  if (isRelType(r.kind)) return wt(`wiki.rel.${r.kind}${r.fromUid === uid ? '' : '.rev'}` as MessageKey);
  return r.labels[lang] ?? Object.values(r.labels)[0] ?? r.kind;
}

/** The level a character is at: the one it reached last. */
export function levelOf(wiki: Wiki, uid: string) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const reached = wiki.relations
    .filter(r => r.kind === 'has_level' && r.fromUid === uid && byUid.get(r.toUid)?.kind === 'level')
    .sort((a, b) => (b.firstChapter ?? 0) - (a.firstChapter ?? 0));
  return reached.length ? byUid.get(reached[0].toUid) ?? null : null;
}

/** A way to find the wiki's entries in a chapter's text. */
export interface WikiTerms { re: RegExp | null; byTerm: Map<string, WikiEntry> }

/**
 * The names to look for in text shown in `lang`: the raw name and aliases in a raw text,
 * the entry's name in that language in a translation. Longest first, so "Lâm Động Thiên"
 * wins over "Lâm Động"; one letter is too common to mark.
 */
export function wikiTerms(wiki: Wiki | null, lang: string): WikiTerms {
  const byTerm = new Map<string, WikiEntry>();
  for (const e of wiki?.entries ?? []) {
    const terms = lang === RAW ? [e.name, ...e.aliases] : [e.names[lang], e.name];
    for (const term of terms) if (term && [...term.trim()].length >= 2 && !byTerm.has(term.trim())) byTerm.set(term.trim(), e);
  }
  if (!byTerm.size) return { re: null, byTerm };
  const alts = [...byTerm.keys()].sort((a, b) => b.length - a.length).map(s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return { re: new RegExp(alts.join('|'), 'g'), byTerm };
}

/** `text` with the wiki's names bold and tappable. */
export function markWikiTerms(text: string, terms: WikiTerms, onPick: (e: WikiEntry) => void): ReactNode {
  if (!terms.re) return text;
  const out: ReactNode[] = [];
  let at = 0;
  for (const m of text.matchAll(terms.re)) {
    const e = terms.byTerm.get(m[0]);
    if (!e || m.index === undefined) continue;
    if (m.index > at) out.push(text.slice(at, m.index));
    out.push(
      <button key={m.index} type="button" className="wiki-term" data-wiki-term={e.uid}
        onClick={ev => { ev.stopPropagation(); onPick(e); }}>
        {m[0]}
      </button>
    );
    at = m.index + m[0].length;
  }
  if (!out.length) return text;
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/**
 * One wiki entry over the reader, as known by the chapter being read: what the wiki held
 * by then (`wiki` is that chapter's view), so nothing from later chapters shows.
 */
export function WikiPopup({ wiki, entry, lang, chapter, onPick, onOpenWiki, onClose }: {
  wiki: Wiki; entry: WikiEntry; lang: string; chapter: string;
  onPick: (e: WikiEntry) => void; onOpenWiki: () => void; onClose: () => void;
}) {
  const { t } = useI18n();
  const wt = useWikiT(lang);
  const label = wikiLabel(entry, lang);
  const sum = wikiSummary(entry, lang);
  const level = entry.kind === 'character' ? levelOf(wiki, entry.uid) : null;
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const rels = wiki.relations.filter(r => (r.fromUid === entry.uid || r.toUid === entry.uid) && !(r.kind === 'has_level' && level && r.toUid !== level.uid));
  return (
    <Modal onClose={onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={label} onClick={e => e.stopPropagation()}>
        <div>
          <div className="dialog-title" style={{ fontSize: 22 }}>{label}</div>
          <div className="muted" style={{ fontSize: 12, display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center', marginTop: 4 }}>
            <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${entry.kind}`)}</span>
            {label !== entry.name && <span>{entry.name}</span>}
            {entry.aliases.length > 0 && <span>· {entry.aliases.join(', ')}</span>}
          </div>
        </div>
        {level && <div style={{ fontSize: 14 }}><b>{t('wiki.currentLevel')}:</b> {wikiLabel(level, lang)}</div>}
        {entry.kind === 'level' && entry.rank != null && <div style={{ fontSize: 14 }}>{t('wiki.rankN', { n: entry.rank })}</div>}
        {sum.text
          ? <p style={{ margin: 0, fontSize: 15, lineHeight: 1.5 }}>{sum.text}</p>
          : <p className="muted" style={{ margin: 0, fontSize: 14 }}>{t('wiki.noSummary')}</p>}
        {rels.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.relations')}</div>
            {rels.map(r => {
              const other = byUid.get(r.fromUid === entry.uid ? r.toUid : r.fromUid);
              return other && (
                <button key={r.uid} type="button" data-relation={r.kind} onClick={() => onPick(other)}
                  style={{ display: 'flex', gap: 6, alignItems: 'baseline', padding: '8px 12px', borderRadius: 16, border: 'none', background: 'var(--color-bg)', color: 'inherit', font: 'inherit', fontSize: 14, textAlign: 'left', cursor: 'pointer' }}>
                  <span>{relWords(r, entry.uid, lang, wt)}</span><b>{wikiLabel(other, lang)}</b>
                </button>
              );
            })}
          </div>
        )}
        <div className="muted" style={{ fontSize: 12 }}>{t('wiki.asOf', { chapter })}</div>
        <div className="dialog-actions">
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={onOpenWiki}>{t('wiki.open')}</button>
          <button type="button" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={onClose}>{t('common.close')}</button>
        </div>
      </div>
    </Modal>
  );
}
