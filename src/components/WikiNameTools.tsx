import { useEffect, useState } from 'react';
import { Modal } from './Modal';
import { Spinner } from './ui';
import { LanguagesIcon, SparklesIcon } from './Icons';
import { useI18n } from '../i18n';
import { AiFailure, aiErrorDetail } from '../lib/ai';
import {
  allowWikiMerge, applyRetranslate, askRetranslate, entryMergeFound, entryWikiMerges, skipWikiMerge, wikiLabel,
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
export function EntryMergeCheck({ novelId, entry, lang, initial, onMerged, onDetails }: {
  novelId: number; entry: WikiEntry; lang: string; initial?: EntryMerge;
  onMerged: (kept: WikiEntry) => void;
  /** Opens smart merge for the entry, to pick one by one. */
  onDetails?: () => void;
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
      <div data-merge-check="loading" className="muted" style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12 }}>
        <Spinner size={12} />{t('wiki.checkLooking')}
      </div>
    );
  }
  if (state === 'failed' || gone) return null;
  if (!f) {
    return <div data-merge-check="none" className="muted" style={{ fontSize: 12 }}>{t('wiki.checkNone')}</div>;
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
      style={{ display: 'flex', flexDirection: 'column', gap: 8, padding: '10px 12px', borderRadius: 16, background: 'color-mix(in srgb, var(--color-accent) 12%, transparent)' }}>
      <div style={{ fontSize: 13, fontWeight: 700 }}>
        {t('wiki.checkFound', { n: f.from.length + f.loose.length })}
      </div>
      <ul style={{ margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: 4, fontSize: 13 }}>
        {f.from.map(e => (
          <li key={e.uid} data-merge-check-from style={{ overflowWrap: 'anywhere' }}>
            {name(e)} <span className="tag tag-neutral" style={{ fontSize: 10 }}>{t('wiki.checkEntry')}</span>
          </li>
        ))}
        {f.loose.map(r => (
          <li key={`${r.raw}>${r.translated}`} data-merge-check-raw style={{ overflowWrap: 'anywhere' }}>
            <b>{r.raw}</b> <span className="muted">→ {r.translated}</span>{' '}
            <span className="tag tag-neutral" style={{ fontSize: 10 }}>{t('wiki.rawLoose')}</span>
            {r.chapters.length > 0 && <span className="muted" style={{ fontSize: 11 }}> · {t('wiki.rawChapters', { list: r.chapters.join(', ') })}</span>}
          </li>
        ))}
      </ul>
      <div style={{ fontSize: 13 }}>
        {t('wiki.checkInto')} <b data-merge-check-into>{name(into)}</b>
      </div>
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
        <button type="button" className="btn btn-primary" data-merge-allow style={{ height: 36, padding: '0 14px' }} disabled={busy} onClick={allow}>
          {busy ? <Spinner size={13} /> : t('wiki.checkAllow')}
        </button>
        {f.pairs.length > 0 && (
          <button type="button" className="btn btn-secondary" data-merge-not-one style={{ height: 36, padding: '0 12px' }} disabled={busy} onClick={notOne}>{t('wiki.checkNotOne')}</button>
        )}
        {onDetails && (
          <button type="button" className="btn btn-ghost" style={{ height: 36, padding: '0 10px' }} disabled={busy} onClick={onDetails}>{t('wiki.checkDetails')}</button>
        )}
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
  useEffect(() => {
    let live = true;
    setAsking(true);
    askRetranslate(user, novelId, target).then(r => {
      if (!live) return;
      setP(r);
      setName(r.name);
      setRaw(r.raw);
    }, e => { if (live) fail(e); }).finally(() => { if (live) setAsking(false); });
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user, novelId, target.uid, target.text, target.chapterIndex, round]);
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
            <button type="button" className="btn btn-ghost" data-retr-again style={{ height: 44, padding: '0 12px', marginRight: 'auto', gap: 6 }} disabled={asking || busy} onClick={() => setRound(n => n + 1)}>
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
