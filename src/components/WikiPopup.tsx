import { useState, type ReactNode } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { messageIn, useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import {
  PROFILE_FIELDS, RAW, REL_GROUPS, SKILL_FIELDS, WIKI_LANG, isRelType, metaStage, relGroup, wikiChapter, wikiLabel, wikiSummary,
  removeWikiEntry, unlinkWikiWord, type RelGroup, type Wiki, type WikiEntry, type WikiRelation
} from '../lib/wiki';
import { useApp } from '../store/AppStore';
import { Modal } from './Modal';
import { SmartMergeSheet } from './SmartMergeSheet';
import { EntryMergeCheck, MergeIntoSheet, RetranslateSheet } from './WikiNameTools';
import { LanguagesIcon, XIcon } from './Icons';

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
  const words = r.labels[lang] ?? Object.values(r.labels)[0] ?? r.kind;
  // An English code the AI left ("ally_of", "other") never shows in the Vietnamese wiki.
  return lang === WIKI_LANG && /^[A-Za-z0-9_ /-]+$/.test(words) && /[A-Za-z]/.test(words) ? wt('wiki.relRelated') : words;
}

/** The level a character is at: the one it reached last. */
export function levelOf(wiki: Wiki, uid: string) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const reached = wiki.relations
    .filter(r => r.kind === 'has_level' && r.fromUid === uid && byUid.get(r.toUid)?.kind === 'level')
    .sort((a, b) => (b.firstChapter ?? 0) - (a.firstChapter ?? 0));
  return reached.length ? byUid.get(reached[0].toUid) ?? null : null;
}

/** The levels a character went through before its current one, oldest first. */
export function pastLevelsOf(wiki: Wiki, uid: string, current: WikiEntry | null) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  return wiki.relations
    .filter(r => r.kind === 'has_level' && r.fromUid === uid && r.toUid !== current?.uid)
    .sort((a, b) => (a.firstChapter ?? 0) - (b.firstChapter ?? 0))
    .map(r => byUid.get(r.toUid))
    .filter((e): e is WikiEntry => !!e && e.kind === 'level');
}

/**
 * An entry's relations sorted for reading: with other characters first, by kind
 * (master and disciples, family, love, friends, enemies, other), then the rest
 * (factions, places, items). A character's levels and skills are shown in its profile
 * instead, so `profiled` leaves them out.
 */
export function groupRelations(wiki: Wiki, uid: string, rels: WikiRelation[], profiled: boolean) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const other = (r: WikiRelation) => byUid.get(r.fromUid === uid ? r.toUid : r.fromUid);
  const people = new Map<RelGroup, WikiRelation[]>();
  const more: WikiRelation[] = [];
  for (const r of rels) {
    if (r.fromUid !== uid && r.toUid !== uid) continue;
    if (profiled && r.fromUid === uid && (r.kind === 'has_level' || r.kind === 'has_skill' || (r.kind === 'owns' && other(r)?.kind === 'item'))) continue;
    if (other(r)?.kind === 'character') {
      const g = relGroup(r.kind);
      people.set(g, [...(people.get(g) ?? []), r]);
    } else more.push(r);
  }
  const groups = (Object.keys(REL_GROUPS) as RelGroup[]).filter(g => people.has(g)).map(g => ({ group: g, rels: people.get(g)! }));
  return { groups, more };
}

/** A relation group's heading: "Sư môn", "Gia đình"… */
function GroupTitle({ children }: { children: ReactNode }) {
  return <div className="muted" style={{ fontSize: 11, fontWeight: 700, textTransform: 'uppercase', letterSpacing: 0.4, marginTop: 4 }}>{children}</div>;
}

/** The relations of `uid`, grouped, each drawn by `row`. */
export function GroupedRelations({ wiki, uid, rels, profiled, row, titled = true }: {
  wiki: Wiki; uid: string; rels: WikiRelation[]; profiled: boolean; row: (r: WikiRelation) => ReactNode;
  /** Off when a "Relationships" heading is already above: the rest then has none of its own unless groups come first. */
  titled?: boolean;
}) {
  const { t } = useI18n();
  const app = useApp();
  // A relation of no known type is hidden unless the settings say to show them.
  const shown = app.wikiUnknownRels ? rels : rels.filter(r => isRelType(r.kind));
  const { groups, more } = groupRelations(wiki, uid, shown, profiled);
  if (!groups.length && !more.length) return null;
  return (
    <>
      {groups.length > 0 && (
        <section data-wiki-rel-people style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.relChars')}</div>
          {groups.map(g => (
            <div key={g.group} data-rel-group={g.group} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <GroupTitle>{t(`wiki.relGroup.${g.group}`)}</GroupTitle>
              {g.rels.map(row)}
            </div>
          ))}
        </section>
      )}
      {more.length > 0 && (
        <section data-wiki-rel-more style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {(groups.length > 0 || profiled) ? <div style={{ fontSize: 13, fontWeight: 700 }}>{t(profiled ? 'wiki.relMore' : 'wiki.relOther')}</div>
            : titled && <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.relations')}</div>}
          {more.map(row)}
        </section>
      )}
    </>
  );
}

/**
 * A character's profile, one row each in this order: personality, age, background,
 * skills (its own words, then the named skills it knows) and level (with the ones before).
 */
export function CharacterProfile({ wiki, entry, onPick }: { wiki: Wiki; entry: WikiEntry; onPick?: (e: WikiEntry) => void }) {
  const { t } = useI18n();
  const m = entry.meta ?? {};
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const level = levelOf(wiki, entry.uid);
  const past = pastLevelsOf(wiki, entry.uid, level);
  const skills = wiki.relations
    .filter(r => r.kind === 'has_skill' && r.fromUid === entry.uid)
    .map(r => byUid.get(r.toUid))
    .filter((e): e is WikiEntry => !!e);
  const items = wiki.relations
    .filter(r => r.kind === 'owns' && r.fromUid === entry.uid)
    .map(r => byUid.get(r.toUid))
    .filter((e): e is WikiEntry => !!e && e.kind === 'item');
  const chip = (e: WikiEntry) => onPick
    ? <button key={e.uid} type="button" className="chip chip--sm" data-wiki-skill style={{ fontSize: 12 }} onClick={() => onPick(e)}>{wikiLabel(e, WIKI_LANG)}</button>
    : <span key={e.uid} className="chip chip--sm" data-wiki-skill style={{ fontSize: 12, pointerEvents: 'none' }}>{wikiLabel(e, WIKI_LANG)}</span>;
  const levelText = [level ? wikiLabel(level, WIKI_LANG) : m.level, metaStage(m, n => t('wiki.stageN', { n }))].filter(Boolean).join(', ');
  const rows: { key: string; value: ReactNode }[] = [
    ...PROFILE_FIELDS.map(f => ({
      key: f,
      value: f === 'abilities' && skills.length
        ? <>{m.abilities && <span>{m.abilities}</span>}<span style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: m.abilities ? 4 : 0 }}>{skills.map(chip)}</span></>
        : m[f] || null
    })),
    {
      key: 'level',
      value: levelText
        ? <>{levelText}{past.length > 0 && <span className="muted" style={{ display: 'block', fontSize: 12 }}>{t('wiki.profile.pastLevels', { levels: past.map(e => wikiLabel(e, WIKI_LANG)).join(' → ') })}</span>}</>
        : null
    },
    { key: 'items', value: items.length ? <span style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>{items.map(chip)}</span> : null }
  ];
  return (
    <section data-wiki-profile style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.profile')}</div>
      <dl style={{ margin: 0, display: 'grid', gridTemplateColumns: 'minmax(78px, auto) 1fr', borderRadius: 16, overflow: 'hidden', background: 'var(--color-bg)' }}>
        {rows.map((r, i) => (
          <div key={r.key} data-profile={r.key} style={{ display: 'contents' }}>
            <dt style={{ padding: '8px 10px 8px 12px', fontSize: 12, fontWeight: 700, color: 'var(--color-neutral-700)', borderTop: i ? '1px solid var(--color-neutral-200)' : 'none' }}>{t(`wiki.profile.${r.key}` as MessageKey)}</dt>
            <dd style={{ margin: 0, padding: '8px 12px 8px 0', fontSize: 14, lineHeight: 1.45, overflowWrap: 'anywhere', borderTop: i ? '1px solid var(--color-neutral-200)' : 'none' }}>
              {r.value ?? <span className="muted" style={{ fontSize: 13 }}>{t('wiki.profile.unknown')}</span>}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

/** A skill's grade and difficulty, beside its description; who knows it is not listed. */
export function SkillFacts({ entry }: { entry: WikiEntry }) {
  const { t } = useI18n();
  const m = entry.meta ?? {};
  return (
    <section data-wiki-skill-facts style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.skill')}</div>
      <dl style={{ margin: 0, display: 'grid', gridTemplateColumns: 'minmax(78px, auto) 1fr', borderRadius: 16, overflow: 'hidden', background: 'var(--color-bg)' }}>
        {SKILL_FIELDS.map((f, i) => (
          <div key={f} data-skill={f} style={{ display: 'contents' }}>
            <dt style={{ padding: '8px 10px 8px 12px', fontSize: 12, fontWeight: 700, color: 'var(--color-neutral-700)', borderTop: i ? '1px solid var(--color-neutral-200)' : 'none' }}>{t(`wiki.skill.${f}`)}</dt>
            <dd style={{ margin: 0, padding: '8px 12px 8px 0', fontSize: 14, lineHeight: 1.45, overflowWrap: 'anywhere', borderTop: i ? '1px solid var(--color-neutral-200)' : 'none' }}>
              {m[f] || <span className="muted" style={{ fontSize: 13 }}>{t('wiki.profile.unknown')}</span>}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

/** The levels of other ladders a level matches ("Kim Đan ≈ Tiên Thiên"), either way round. */
export function equivalentsOf(wiki: Wiki, uid: string) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const seen = new Set<string>();
  const out: WikiEntry[] = [];
  for (const r of wiki.relations) {
    if (r.kind !== 'equivalent_to' || (r.fromUid !== uid && r.toUid !== uid)) continue;
    const o = byUid.get(r.fromUid === uid ? r.toUid : r.fromUid);
    if (o && !seen.has(o.uid)) { seen.add(o.uid); out.push(o); }
  }
  return out;
}

/** A faction's leaders in the order the story names them (by the chapter first told), the last one current. */
export function leadersOf(wiki: Wiki, uid: string) {
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  return wiki.relations
    .filter(r => r.kind === 'leader_of' && r.toUid === uid && byUid.has(r.fromUid))
    .sort((a, b) => (a.firstChapter ?? Infinity) - (b.firstChapter ?? Infinity))
    .map(r => ({ rel: r, who: byUid.get(r.fromUid)! }));
}

/** Who leads a faction: each leader in turn, "the 2nd", from which chapter, the current one marked. */
export function FactionLeaders({ wiki, entry, onPick }: { wiki: Wiki; entry: WikiEntry; onPick?: (e: WikiEntry) => void }) {
  const { t } = useI18n();
  const leaders = leadersOf(wiki, entry.uid);
  return (
    <section data-wiki-leaders style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.leaders')}</div>
      {leaders.length === 0 && <div className="muted" style={{ fontSize: 13 }}>{t('wiki.profile.unknown')}</div>}
      <ol style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 6 }}>
        {leaders.map(({ rel, who }, i) => {
          const current = i === leaders.length - 1;
          const body = (
            <>
              <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
                {leaders.length > 1 && <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t('wiki.leaderN', { n: i + 1 })}</span>}
                <b>{wikiLabel(who, WIKI_LANG)}</b>
                {current && leaders.length > 1 && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.leaderNow')}</span>}
                {wikiChapter(rel) != null && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.fromChapter', { n: wikiChapter(rel)! })}</span>}
              </span>
              {rel.note && <span className="muted" data-relation-note style={{ fontSize: 12, lineHeight: 1.4 }}>{rel.note}</span>}
            </>
          );
          const style = { display: 'flex', flexDirection: 'column' as const, gap: 2, padding: '8px 12px', borderRadius: 16, border: 'none', background: 'var(--color-bg)', color: 'inherit', font: 'inherit', fontSize: 14, textAlign: 'left' as const, width: '100%' };
          return (
            <li key={rel.uid} data-leader={i + 1}>
              {onPick ? <button type="button" style={{ ...style, cursor: 'pointer' }} onClick={() => onPick(who)}>{body}</button> : <div style={style}>{body}</div>}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

/** A way to find the wiki's entries in a chapter's text. */
export interface WikiTerms { re: RegExp | null; byTerm: Map<string, WikiEntry> }

/** The words of a name, split at spaces and hyphens, in lower case: "Goo Jong-myung" → goo, jong, myung. */
const nameWords = (s: string) => s.toLocaleLowerCase().split(/[\s\-‐‑·・]+/u).filter(Boolean);

/**
 * An alias that is only one word of one of the entry's longer names ("Minh" of "Cố Tông
 * Minh", "Myung" of "Goo Jong-myung"): such a word stands for many others in the text
 * (a sect "hệ Minh", another "Minh"), so it is never marked (wiki.rs `name_piece`).
 */
export function namePiece(alias: string, full: (string | undefined)[]) {
  const a = nameWords(alias);
  if (a.length !== 1) return false;
  return full.some(n => { const ws = n ? nameWords(n) : []; return ws.length > 1 && ws.includes(a[0]); });
}

/** Whether the user unlinked `word` from `e` (wiki.rs `unlink`), whatever its case. */
export const wikiBlocked = (e: WikiEntry, word: string) => {
  const w = word.trim().toLocaleLowerCase();
  return !!w && (e.blocked ?? []).some(b => b.trim().toLocaleLowerCase() === w);
};

/**
 * The names to look for in text shown in `lang`: the raw name and aliases in a raw text,
 * the entry's name in that language in a translation. Longest first, so "Lâm Động Thiên"
 * wins over "Lâm Động"; one letter is too common to mark. A name only counts as a whole
 * word: "Từ" is not marked inside "Từng" (Chinese and other scripts without spaces have
 * no such letters around them, so their names are found anywhere).
 *
 * A word goes to the entry it names best: its name in that language first, then its
 * original name, then an alias. A word two entries share at that level is left unmarked
 * (it may be either), and so are an alias that is only one word of the entry's longer
 * name ("Minh" of "Cố Tông Minh") and a word the user unlinked from it.
 */
export function wikiTerms(wiki: Wiki | null, lang: string): WikiTerms {
  const best = new Map<string, { rank: number; entries: WikiEntry[] }>();
  const add = (term: string | undefined, e: WikiEntry, rank: number) => {
    const t = term?.trim();
    if (!t || [...t].length < 2 || wikiBlocked(e, t)) return;
    const had = best.get(t);
    if (!had || rank < had.rank) best.set(t, { rank, entries: [e] });
    else if (rank === had.rank && !had.entries.includes(e)) had.entries.push(e);
  };
  for (const e of wiki?.entries ?? []) {
    const full = [e.name, ...Object.values(e.names)];
    if (lang === RAW) add(e.name, e, 0);
    else { add(e.names[lang], e, 0); add(e.name, e, 1); }
    for (const a of e.aliases) if (!namePiece(a, full)) add(a, e, 2);
  }
  const byTerm = new Map<string, WikiEntry>();
  for (const [t, b] of best) if (b.entries.length === 1) byTerm.set(t, b.entries[0]);
  if (!byTerm.size) return { re: null, byTerm };
  const alts = [...byTerm.keys()].sort((a, b) => b.length - a.length).map(s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const word = '[\\p{Script=Latin}\\p{Script=Cyrillic}\\p{Script=Greek}\\p{M}\\p{Nd}]';
  return { re: new RegExp(`(?<!${word})(?:${alts.join('|')})(?!${word})`, 'gu'), byTerm };
}

/** `text` with the wiki's names bold and tappable; `onPick` gets the word as the text writes it. */
export function markWikiTerms(text: string, terms: WikiTerms, onPick: (e: WikiEntry, word: string) => void): ReactNode {
  if (!terms.re) return text;
  const out: ReactNode[] = [];
  let at = 0;
  for (const m of text.matchAll(terms.re)) {
    const e = terms.byTerm.get(m[0]);
    if (!e || m.index === undefined) continue;
    if (m.index > at) out.push(text.slice(at, m.index));
    const word = m[0];
    out.push(
      <button key={m.index} type="button" className="wiki-term" data-wiki-term={e.uid}
        onClick={ev => { ev.stopPropagation(); onPick(e, word); }}>
        {word}
      </button>
    );
    at = m.index + m[0].length;
  }
  if (!out.length) return text;
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/**
 * One wiki entry over the reader: everything the wiki knows of it, later chapters
 * included, and the entries it is linked to (tap one to go there).
 */
export function WikiPopup({ wiki, entry, word, onPick, onOpenWiki, onClose, novelId, onMerged }: {
  wiki: Wiki; entry: WikiEntry; lang: string;
  /** The word tapped in the text, which can be unlinked from this entry. */
  word?: string;
  onPick: (e: WikiEntry) => void; onOpenWiki: () => void; onClose: () => void;
  /** With the book's library id, smart merge for this entry is offered; `onMerged` follows a merge. */
  novelId?: number; onMerged?: () => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const [smart, setSmart] = useState(false);
  const [retr, setRetr] = useState(false);
  const [into, setInto] = useState(false);
  // Aliases unlinked here, gone at once while the wiki loads again.
  const [unlinked, setUnlinked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const aliases = entry.aliases.filter(a => !unlinked.includes(a));
  const wt = useWikiT(WIKI_LANG);
  const label = wikiLabel(entry, WIKI_LANG);
  const sum = wikiSummary(entry, WIKI_LANG);
  const level = entry.kind === 'character' ? levelOf(wiki, entry.uid) : null;
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const isChar = entry.kind === 'character';
  const isLevel = entry.kind === 'level';
  const isSkill = entry.kind === 'skill';
  const isFaction = entry.kind === 'faction';
  const equals = isLevel ? equivalentsOf(wiki, entry.uid) : [];
  // A level is only compared with the levels of other ladders, and a skill only
  // described: who reached or knows them is on the characters' own pages.
  const rels = wiki.relations.filter(r => (r.fromUid === entry.uid || r.toUid === entry.uid)
    && !(r.kind === 'has_level' && (isLevel || (level && r.toUid !== level.uid)))
    && !(isLevel && r.kind === 'equivalent_to')
    && !(isSkill && r.kind === 'has_skill')
    && !(isFaction && r.kind === 'leader_of' && r.toUid === entry.uid));
  // Smart merge takes the card's place (one sheet at a time), and Cancel brings it back.
  // A wrong alias, or the word tapped in the text: unlinked from this entry for good.
  const unlink = async (w: string, close: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      await unlinkWikiWord(app.libKey, entry.uid, w);
      setUnlinked(u => [...u, w]);
      app.showToast(t('wiki.unlinked', { word: w }));
      onMerged?.();
      if (close) onClose();
    } catch (e) {
      console.error(e);
      app.showToast(t('wiki.failed'));
    } finally {
      setBusy(false);
    }
  };
  const removeEntry = async () => {
    if (busy || !(await ask(t('wiki.removeAsk', { name: label }), { kind: 'warning' }))) return;
    setBusy(true);
    try {
      await removeWikiEntry(app.libKey, entry.uid);
      app.showToast(t('wiki.removed'));
      onMerged?.();
      onClose();
    } catch (e) {
      console.error(e);
      app.showToast(t('wiki.failed'));
      setBusy(false);
    }
  };
  const tapped = word?.trim();
  if (smart && novelId != null) {
    return (
      <SmartMergeSheet novelId={novelId} lang={WIKI_LANG} entry={entry} onClose={() => setSmart(false)}
        onMerged={() => { onMerged?.(); onClose(); }} />
    );
  }
  if (into && novelId != null) {
    return (
      <MergeIntoSheet novelId={novelId} entry={entry} entries={wiki.entries} lang={WIKI_LANG}
        onClose={() => setInto(false)} onMerged={() => { onMerged?.(); onClose(); }} />
    );
  }
  if (retr && novelId != null) {
    return (
      <RetranslateSheet novelId={novelId} lang={WIKI_LANG} target={{ uid: entry.uid, label }}
        onClose={() => { setRetr(false); onClose(); }} onDone={() => onMerged?.()} />
    );
  }
  return (
    <Modal onClose={onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={label} onClick={e => e.stopPropagation()}>
        <div>
          <div className="dialog-title" style={{ fontSize: 22 }}>{label}</div>
          <div className="muted" style={{ fontSize: 12, display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center', marginTop: 4 }}>
            <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${entry.kind}`)}</span>
            {label !== entry.name && <span>{entry.name}</span>}
            {novelId == null && aliases.length > 0 && <span>· {aliases.join(', ')}</span>}
          </div>
          {/* Each other name with its own ×: a wrong one is unlinked and never added back. */}
          {novelId != null && aliases.length > 0 && (
            <div data-wiki-aliases style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 8 }}>
              {aliases.map(a => (
                <span key={a} className="chip chip--sm" data-wiki-alias={a} style={{ fontSize: 12, gap: 2, paddingRight: 4, display: 'inline-flex', flexDirection: 'row', alignItems: 'center', whiteSpace: 'nowrap', height: 30 }}>
                  {a}
                  <button type="button" className="icon-btn" disabled={busy} aria-label={t('wiki.unlinkAlias', { word: a })} title={t('wiki.unlinkAlias', { word: a })}
                    style={{ width: 22, height: 22, minWidth: 22 }} onClick={() => unlink(a, false)}>
                    <XIcon size={12} />
                  </button>
                </span>
              ))}
            </div>
          )}
        </div>
        {/* The word tapped in the chapter: when it is not this one, it is unlinked from it. */}
        {novelId != null && tapped && (
          <div data-wiki-tapped style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap', padding: '8px 12px', borderRadius: 16, background: 'var(--color-bg)', fontSize: 13 }}>
            <span style={{ flex: '1 1 160px' }}>{t('wiki.wordLinked', { word: tapped })}</span>
            <button type="button" className="btn btn-secondary" data-wiki-unlink-word disabled={busy} style={{ height: 34, padding: '0 12px', fontSize: 13 }}
              onClick={() => unlink(tapped, true)}>
              {t('wiki.unlinkWord')}
            </button>
          </div>
        )}
        {/* Step 1: this name looked for in the book; a merge waits for the user's tap. */}
        {novelId != null && (
          <EntryMergeCheck novelId={novelId} entry={entry} lang={WIKI_LANG}
            onMerged={() => { onMerged?.(); onClose(); }} />
        )}
        {/* The two things to do with a name: translate it again, or merge it into the entry it really is. */}
        {novelId != null && (
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            <button type="button" className="btn btn-secondary" data-entry-retranslate style={{ height: 36, padding: '0 14px', gap: 6, fontSize: 13 }} onClick={() => setRetr(true)}>
              <LanguagesIcon size={13} />{t('wiki.retranslate')}
            </button>
            <button type="button" className="btn btn-secondary" data-entry-merge-into style={{ height: 36, padding: '0 14px', fontSize: 13 }} onClick={() => setInto(true)}>
              {t('wiki.mergeIntoBtn')}
            </button>
            <button type="button" className="btn btn-ghost" data-entry-remove disabled={busy} style={{ height: 36, padding: '0 14px', fontSize: 13, color: 'var(--color-accent-700)' }} onClick={removeEntry}>
              {t('wiki.removeEntry')}
            </button>
          </div>
        )}
        {!isChar && (level || entry.meta?.level) && (
          <div style={{ fontSize: 14 }}>
            <b>{t('wiki.currentLevel')}:</b> {[level ? wikiLabel(level, WIKI_LANG) : entry.meta?.level, metaStage(entry.meta, n => t('wiki.stageN', { n }))].filter(Boolean).join(', ')}
          </div>
        )}
        {entry.kind === 'level' && (entry.rank != null || entry.meta?.system) && (
          <div style={{ fontSize: 14 }}>{[entry.meta?.system, entry.rank != null ? t('wiki.rankN', { n: entry.rank }) : ''].filter(Boolean).join(' · ')}</div>
        )}
        {sum.text
          ? <p style={{ margin: 0, fontSize: 15, lineHeight: 1.5 }}>{sum.text}</p>
          : <p className="muted" style={{ margin: 0, fontSize: 14 }}>{t('wiki.noSummary')}</p>}
        {isChar && <CharacterProfile wiki={wiki} entry={entry} onPick={onPick} />}
        {isSkill && <SkillFacts entry={entry} />}
        {isFaction && <FactionLeaders wiki={wiki} entry={entry} onPick={onPick} />}
        {isLevel && equals.length > 0 && (
          <section data-wiki-equivalent style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.equivalent')}</div>
            <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {equals.map(e => (
                <button key={e.uid} type="button" className="chip chip--sm" style={{ fontSize: 12 }} onClick={() => onPick(e)}>
                  ≈ {wikiLabel(e, WIKI_LANG)}{e.meta?.system ? ` · ${e.meta.system}` : ''}
                </button>
              ))}
            </span>
          </section>
        )}
        <GroupedRelations wiki={wiki} uid={entry.uid} rels={rels} profiled={isChar} row={r => {
          const other = byUid.get(r.fromUid === entry.uid ? r.toUid : r.fromUid);
          return other && (
            <button key={r.uid} type="button" data-relation={r.kind} onClick={() => onPick(other)}
              style={{ display: 'flex', flexDirection: 'column', gap: 2, padding: '8px 12px', borderRadius: 16, border: 'none', background: 'var(--color-bg)', color: 'inherit', font: 'inherit', fontSize: 14, textAlign: 'left', cursor: 'pointer' }}>
              <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
                <span>{relWords(r, entry.uid, WIKI_LANG, wt)}</span><b>{wikiLabel(other, WIKI_LANG)}</b>
                {other.aliases.length > 0 && <span className="muted" style={{ fontSize: 12 }}>({other.aliases.slice(0, 3).join(', ')})</span>}
              </span>
              {r.note && <span className="muted" data-relation-note style={{ fontSize: 12, lineHeight: 1.4 }}>{r.note}</span>}
            </button>
          );
        }} />
        <div className="dialog-actions">
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={onOpenWiki}>{t('wiki.open')}</button>
          <button type="button" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={onClose}>{t('common.close')}</button>
        </div>
      </div>
    </Modal>
  );
}
