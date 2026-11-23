import { useEffect, useMemo, useState } from 'react';
import { Modal } from './Modal';
import { Spinner } from './ui';
import { LanguagesIcon, SparklesIcon } from './Icons';
import { useI18n } from '../i18n';
import { AiFailure, aiErrorDetail } from '../lib/ai';
import {
  allowWikiMerge, applyRetranslate, mergeWikiEntry, askRetranslate, entryMergeFound, entryWikiMerges, skipWikiMerge, wikiLabel,
  type EntryMerge, type RetranslateProposal, type RetranslateTarget, type WikiEntry
} from '../lib/wiki';
import { useApp } from '../store/AppStore';

/** The error of an AI call in the user's words. */
function useFail() {
  const { t } = useI18n();
  const app = useApp();
  return (e: unknown) => {
    console.error(e);
    const f = e instanceof AiFailure ? e : null;
    app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}`.trim() : String(e));
  };
}

/**
 * Step 1 for an entry the user opened: looks in the book (the original names the
 * chapters translated as its name, and the entries that look like it) and, when it finds
 * any, says so with a button. Nothing is merged until the user taps "Allow merge": then
 * everything found goes into one entry (many into one), and the original names with no
 * entry of their own become its aliases. `initial` is a check already made (after a new
 * translation of the name).
 */
export function EntryMergeCheck({ novelId, entry, lang, initial, onMerged }: {
  novelId: number; entry: WikiEntry; lang: string; initial?: EntryMerge;
  onMerged: (kept: WikiEntry) => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const fail = useFail();
  const [found, setFound] = useState<EntryMerge | null>(initial ?? null);
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>(initial ? 'ready' : 'loading');
  const [busy, setBusy] = useState(false);
  const [gone, setGone] = useState(false);
  useEffect(() => {
    if (initial) { setFound(initial); setState('ready'); setGone(false); return; }
    let live = true;
    setState('loading');
    setGone(false);
    entryWikiMerges(user, novelId, entry.uid).then(
      m => { if (live) { setFound(m); setState('ready'); } },
      e => { if (live) { console.error(e); setState('failed'); } });
    return () => { live = false; };
  }, [user, novelId, entry.uid, initial]);
  const f = entryMergeFound(found, entry.uid);
  if (state === 'loading') {
    return (
      <span data-merge-check="loading" hidden />
    );
  }
  if (state === 'failed' || gone) return null;
  if (!f) {
    // Nothing to say: the card stays as it was.
    return <span data-merge-check="none" hidden />;
  }
  const into = f.into ?? entry;
  const allow = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const done = await allowWikiMerge(user, novelId, f.intoUid, f.from.map(e => e.uid), f.loose);
      app.showToast(t('wiki.checkMerged', { into: wikiLabel(done.entry, lang), n: done.applied.merged + done.adopted, rules: done.applied.rules }));
      setGone(true);
      onMerged(done.entry);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  /** Not one: the pairs are set aside for good (auto merge leaves them too). */
  const notOne = async () => {
    if (busy) return;
    setBusy(true);
    try {
      for (const p of f.pairs) await skipWikiMerge(user, novelId, [p.from.uid, p.into.uid]);
      setGone(true);
      app.showToast(t('wiki.smartSkipped', { n: f.pairs.length || 1 }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const name = (e: WikiEntry) => {
    const l = wikiLabel(e, lang);
    return l !== e.name ? `${l} (${e.name})` : l;
  };
  return (
    <section data-merge-check="found" role="status"
      style={{ display: 'flex', flexDirection: 'column', gap: 6, padding: '10px 12px', borderRadius: 16, background: 'color-mix(in srgb, var(--color-accent) 12%, transparent)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <span style={{ fontSize: 14, flex: '1 1 160px', overflowWrap: 'anywhere' }}>
          {t('wiki.checkAsk')} <b data-merge-check-into>{wikiLabel(into, lang)}</b>?
        </span>
        <button type="button" className="btn btn-primary" data-merge-allow style={{ height: 34, padding: '0 14px' }} disabled={busy} onClick={allow}>
          {busy ? <Spinner size={13} /> : t('wiki.checkAllow')}
        </button>
        {f.pairs.length > 0 && (
          <button type="button" className="btn btn-ghost" data-merge-not-one style={{ height: 34, padding: '0 8px', fontSize: 13 }} disabled={busy} onClick={notOne}>{t('wiki.checkNotOne')}</button>
        )}
      </div>
      {/* What goes into it, in one line: other entries and original names the chapters wrote. */}
      <div className="muted" style={{ fontSize: 12, overflowWrap: 'anywhere' }}>
        {[
          ...f.from.filter(e => e.uid !== into.uid).map(e => <span key={e.uid} data-merge-check-from>{name(e)}</span>),
          ...f.loose.map(r => <span key={`${r.raw}>${r.translated}`} data-merge-check-raw>{r.raw} → {r.translated}</span>)
        ].flatMap((x, i) => (i ? [<span key={`s${i}`}>, </span>, x] : [x]))}
      </div>
    </section>
  );
}

/**
 * Translating one name again (step 2): an entry, or words selected in the reader. The
 * AI reads the sentences around it in the book (shown here) and gives a name, which the
 * user can edit before keeping it. Kept, the translations change, and step 1 runs again
 * on the entry: the new name may be another entry's, to merge.
 */
export function RetranslateSheet({ novelId, target, lang, onClose, onDone }: {
  novelId: number; target: RetranslateTarget & { label: string }; lang: string;
  onClose: () => void;
  /** After a name is kept (the translations changed) or a merge allowed. */
  onDone: () => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const fail = useFail();
  const [asking, setAsking] = useState(true);
  const [p, setP] = useState<RetranslateProposal | null>(null);
  const [name, setName] = useState('');
  const [raw, setRaw] = useState('');
  const [busy, setBusy] = useState(false);
  const [kept, setKept] = useState<{ entry: WikiEntry; merge: EntryMerge } | null>(null);
  const [round, setRound] = useState(0);
  // The names already given: "Ask again" wants another one.
  const [tried, setTried] = useState<string[]>([]);
  useEffect(() => {
    let live = true;
    setAsking(true);
    askRetranslate(user, novelId, { ...target, avoid: tried }).then(r => {
      if (!live) return;
      setP(r);
      setName(r.name);
      setRaw(r.raw);
    }, e => { if (live) fail(e); }).finally(() => { if (live) setAsking(false); });
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user, novelId, target.uid, target.text, target.chapterIndex, target.passage, round]);
  const keep = async () => {
    if (!p || !name.trim() || busy) return;
    setBusy(true);
    try {
      const old = [target.text ?? '', p.current].map(s => s.trim()).filter(s => s && s !== name.trim());
      const done = await applyRetranslate(user, novelId, { uid: p.uid, raw: raw.trim(), name: name.trim(), kind: p.kind, old: [...new Set(old)] });
      app.showToast(t('wiki.retrKept', { name: wikiLabel(done.entry, lang), n: done.changed }));
      setKept(done);
      onDone();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const mark = (text: string, words: string[]) => {
    const w = words.map(s => s.trim()).filter(s => s.length > 1).sort((a, b) => b.length - a.length);
    if (!w.length) return text;
    const re = new RegExp(`(${w.map(s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'g');
    return text.split(re).map((part, i) => (i % 2 ? <mark key={i} style={{ background: 'color-mix(in srgb, var(--color-accent) 28%, transparent)', color: 'inherit', borderRadius: 4, padding: '0 2px' }}>{part}</mark> : part));
  };
  return (
    <Modal onClose={busy ? undefined : onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('wiki.retrTitle')} data-retranslate-sheet onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('wiki.retrTitle')}: {target.label}</div>
        <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>{t('wiki.retrSub')}</div>
        {asking && (
          <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13 }}><Spinner size={14} />{t('wiki.retrAsking')}</div>
        )}
        {!asking && !p && <p className="muted" style={{ fontSize: 13, margin: 0 }}>{t('wiki.retrFailed')}</p>}
        {p && !kept && (
          <>
            <section data-retr-contexts style={{ display: 'flex', flexDirection: 'column', gap: 6, maxHeight: '32vh', overflowY: 'auto' }}>
              <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.retrContexts', { n: p.contexts.length })}</div>
              {p.contexts.length === 0 && <span className="muted" style={{ fontSize: 12 }}>{t('wiki.retrNoContext')}</span>}
              {p.contexts.map((c, i) => (
                <div key={i} data-retr-context style={{ padding: '8px 10px', borderRadius: 12, background: 'var(--frost)', fontSize: 13, lineHeight: 1.45, display: 'flex', flexDirection: 'column', gap: 3 }}>
                  <span className="muted" style={{ fontSize: 11 }}>{t('wiki.retrChapter', { n: c.chapter })}</span>
                  <span>{mark(c.raw, [p.raw, raw])}</span>
                  {c.translated && <span className="muted">{mark(c.translated, [p.current, target.text ?? ''])}</span>}
                </div>
              ))}
            </section>
            <div style={{ fontSize: 13 }}>{t('wiki.retrNow')}: <b>{p.current}</b></div>
            <div className="field">
              <label htmlFor="retr-name">{t('wiki.retrNew')}</label>
              <input id="retr-name" className="input" data-retr-name style={{ height: 42 }} value={name} onChange={e => setName(e.target.value)} />
            </div>
            <div className="field">
              <label htmlFor="retr-raw">{t('wiki.retrRaw')}</label>
              <input id="retr-raw" className="input" data-retr-raw style={{ height: 38 }} value={raw} onChange={e => setRaw(e.target.value)} />
            </div>
            {p.why && <div data-retr-why style={{ fontSize: 12, overflowWrap: 'anywhere' }}><SparklesIcon size={12} /> {p.why}</div>}
            {!p.uid && <div className="muted" style={{ fontSize: 12 }}>{t('wiki.retrNewEntry', { kind: t(`wiki.kind.${(p.kind || 'term') as 'term'}`) })}</div>}
          </>
        )}
        {kept && (
          <>
            <div data-retr-done style={{ fontSize: 14 }}>{t('wiki.retrDone', { name: wikiLabel(kept.entry, lang) })}</div>
            {/* Back to step 1: the new name may be another entry's. */}
            <EntryMergeCheck novelId={novelId} entry={kept.entry} lang={lang} initial={kept.merge}
              onMerged={e => { setKept(k => k && { ...k, entry: e, merge: { raws: [], pairs: [] } }); onDone(); }} />
          </>
        )}
        <div className="dialog-actions">
          {p && !kept && (
            <button type="button" className="btn btn-ghost" data-retr-again style={{ height: 44, padding: '0 12px', marginRight: 'auto', gap: 6 }} disabled={asking || busy} onClick={() => { if (p) setTried(x => [...new Set([...x, p.name, name.trim()].filter(Boolean))]); setRound(n => n + 1); }}>
              <LanguagesIcon size={14} />{t('wiki.retrAgain')}
            </button>
          )}
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t(kept ? 'common.close' : 'common.cancel')}</button>
          {!kept && (
            <button type="button" className="btn btn-primary" data-retr-keep style={{ height: 44, padding: '0 22px' }} disabled={!p || asking || busy || !name.trim()} onClick={keep}>
              {busy ? <Spinner size={14} /> : t('wiki.retrKeep')}
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
}

/**
 * "Merge into…": the user picks the entry this one really is (a nickname, a name
 * written another way) and it goes into it: its names become that entry's other names,
 * and the translations follow. Entries of the same kind come first; the search goes by
 * every name.
 */
export function MergeIntoSheet({ novelId, entry, entries, lang, onClose, onMerged }: {
  novelId: number; entry: WikiEntry; entries: WikiEntry[]; lang: string;
  onClose: () => void; onMerged: (kept: WikiEntry) => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const fail = useFail();
  const [q, setQ] = useState('');
  const [busy, setBusy] = useState(false);
  const list = useMemo(() => {
    const k = q.trim().toLocaleLowerCase();
    return entries
      .filter(e => e.uid !== entry.uid)
      .filter(e => !k || [wikiLabel(e, lang), e.name, ...e.aliases].some(n => n.toLocaleLowerCase().includes(k)))
      .sort((a, b) => Number(b.kind === entry.kind) - Number(a.kind === entry.kind))
      .slice(0, 40);
  }, [q, entries, entry, lang]);
  const pick = async (into: WikiEntry) => {
    if (busy) return;
    setBusy(true);
    try {
      const kept = await mergeWikiEntry(app.libKey, novelId, entry.uid, into.uid);
      app.showToast(t('wiki.merged', { into: wikiLabel(into, lang) }));
      onMerged(kept);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal onClose={busy ? undefined : onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('wiki.mergeIntoTitle')} data-merge-into-sheet onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('wiki.mergeIntoTitle')}</div>
        <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>{t('wiki.mergeIntoSub', { name: wikiLabel(entry, lang) })}</div>
        <input className="input" style={{ height: 42 }} autoFocus value={q} placeholder={t('wiki.mergeIntoSearch')} aria-label={t('wiki.mergeIntoSearch')} onChange={e => setQ(e.target.value)} />
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, maxHeight: '45vh', overflowY: 'auto' }}>
          {list.map(e => (
            <button key={e.uid} type="button" data-merge-into-option={e.uid} disabled={busy} onClick={() => pick(e)}
              style={{ display: 'flex', alignItems: 'baseline', gap: 8, padding: '10px 12px', borderRadius: 14, border: 'none', background: 'var(--frost)', color: 'inherit', font: 'inherit', fontSize: 14, textAlign: 'left', cursor: 'pointer' }}>
              <b style={{ overflowWrap: 'anywhere' }}>{wikiLabel(e, lang)}</b>
              <span className="muted" style={{ fontSize: 12, flex: 1, overflowWrap: 'anywhere' }}>{e.name}</span>
              {e.kind !== entry.kind && <span className="tag tag-neutral" style={{ fontSize: 10 }}>{t(`wiki.kind.${e.kind as 'term'}`)}</span>}
            </button>
          ))}
          {list.length === 0 && <span className="muted" style={{ fontSize: 13 }}>{t('wiki.mergeIntoNone')}</span>}
        </div>
        <div className="dialog-actions">
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
        </div>
      </div>
    </Modal>
  );
}
