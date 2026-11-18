import { useEffect, useState } from 'react';
import { Modal } from './Modal';
import { Checkbox, Spinner } from './ui';
import { SparklesIcon } from './Icons';
import { useI18n } from '../i18n';
import { AiFailure, aiErrorDetail } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import {
  applyWikiMerges, entryWikiMerges, entryWikiMergesAi, suggestWikiMerges, suggestWikiMergesAi, wikiLabel,
  type MergeGroup, type MergeSuggestion, type RawName, type WikiEntry } from '../lib/wiki';
import { useApp } from '../store/AppStore';

/**
 * Smart merge, offline first and then online. The device's candidates show at once for
 * review: same names ticked, near ones not; for one entry, also the original names the
 * translations wrote as its name. "Ask the AI" sends everything known about them (for
 * the whole wiki, in blocks of about 10k letters run in parallel) and brings back groups
 * with the final name to use, which can be edited. Each merge adds its old names to the
 * replace rules, so chapters translated before show the final name.
 */
export function SmartMergeSheet({ novelId, lang, entry, onClose, onMerged }: {
  novelId: number; lang: string; entry?: WikiEntry; onClose: () => void; onMerged: () => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const q = useAsync(
    () => (entry ? entryWikiMerges(user, novelId, entry.uid) : suggestWikiMerges(user, novelId).then(pairs => ({ raws: [] as RawName[], pairs }))),
    [user, novelId, entry?.uid]
  );
  const pairsAll = q.status === 'ready' ? q.data.pairs : null;
  const raws = q.status === 'ready' ? q.data.raws : [];
  const [groups, setGroups] = useState<MergeGroup[]>([]);
  // The final name per group, as the user may have changed it.
  const [names, setNames] = useState<Record<string, string>>({});
  const gid = (g: MergeGroup) => [g.into.uid, ...g.members.map(m => m.uid)].join('+');
  const pid = (s: MergeSuggestion) => `${s.from.uid}>${s.into.uid}`;
  // A pair the AI put in one of its groups is shown there only.
  const inGroup = (s: MergeSuggestion) => groups.some(g => {
    const u = [g.into.uid, ...g.members.map(m => m.uid)];
    return u.includes(s.from.uid) && u.includes(s.into.uid);
  });
  const pairs = pairsAll?.filter(s => !inGroup(s)) ?? null;
  const [picked, setPicked] = useState<Set<string> | null>(null);
  useEffect(() => {
    if (pairsAll && !picked) setPicked(new Set(pairsAll.filter(s => s.reason !== 'near_name').map(pid)));
  }, [pairsAll, picked]);
  const toggle = (k: string) => setPicked(p => {
    const n = new Set(p ?? []);
    if (n.has(k)) n.delete(k); else n.add(k);
    return n;
  });

  const [asking, setAsking] = useState(false);
  const [step, setStep] = useState<{ done: number; total: number } | null>(null);
  useEffect(() => {
    if (!asking || entry) return;
    let stop: (() => void) | undefined;
    let gone = false;
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen<{ user: string; novelId: number; done: number; total: number }>('wiki-merge-progress', e => {
        if (e.payload.novelId === novelId) setStep({ done: e.payload.done, total: e.payload.total });
      })).then(un => { if (gone) un(); else stop = un; }).catch(() => undefined);
    return () => { gone = true; stop?.(); };
  }, [asking, entry, novelId]);
  const askAi = async () => {
    if (asking) return;
    setAsking(true);
    setStep(null);
    try {
      const found = await (entry ? entryWikiMergesAi(user, novelId, entry.uid) : suggestWikiMergesAi(user, novelId));
      const fresh = found.filter(g => !groups.some(x => gid(x) === gid(g)));
      setGroups(gs => [...gs, ...fresh]);
      setNames(n => ({ ...n, ...Object.fromEntries(fresh.map(g => [gid(g), g.name])) }));
      // A better name alone is ticked for one entry; across the whole wiki it waits for the user.
      setPicked(p => new Set([...(p ?? []), ...fresh.filter(g => g.members.length > 0 || entry).map(gid)]));
      app.showToast(t('wiki.smartMergeAiFound', { n: fresh.length }));
    } catch (e) {
      console.error(e);
      const f = e instanceof AiFailure ? e : null;
      app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}` : String(e));
    } finally {
      setAsking(false);
      setStep(null);
    }
  };

  const [busy, setBusy] = useState(false);
  const chosenPairs = pairs?.filter(s => picked?.has(pid(s))) ?? [];
  const chosenGroups = groups.filter(g => picked?.has(gid(g)) && (names[gid(g)] ?? g.name).trim());
  const count = chosenPairs.length + chosenGroups.length;
  const apply = async () => {
    if (!count || busy) return;
    setBusy(true);
    try {
      const done = await applyWikiMerges(user, novelId,
        chosenPairs.map(s => ({ fromUid: s.from.uid, intoUid: s.into.uid })),
        chosenGroups.map(g => ({ intoUid: g.into.uid, fromUids: g.members.map(m => m.uid), name: (names[gid(g)] ?? g.name).trim() })));
      app.showToast(t('wiki.smartMerged', { n: done.merged, rules: done.rules })
        + (done.renamed ? ` ${t('wiki.smartRenamed', { n: done.renamed })}` : ''));
      onMerged();
      onClose();
    } catch (e) {
      console.error(e);
      app.showToast(e instanceof AiFailure ? t(e.key) : String(e));
    } finally {
      setBusy(false);
    }
  };
  const label = (e: WikiEntry) => {
    const l = wikiLabel(e, lang);
    return l !== e.name ? `${l} (${e.name})` : l;
  };
  const card = { padding: '10px 12px', borderRadius: 16, background: 'var(--frost)' } as const;
  return (
    <Modal onClose={busy ? undefined : onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('wiki.smartMerge')} data-wiki-smart-merge-sheet onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{entry ? `${t('wiki.smartMerge')}: ${wikiLabel(entry, lang)}` : t('wiki.smartMerge')}</div>
        <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>{t(entry ? 'wiki.smartMergeEntrySub' : 'wiki.smartMergeSub')}</div>
        {entry && q.status === 'ready' && (
          <section data-merge-raws style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.rawsTitle', { name: wikiLabel(entry, lang) })}</div>
            {raws.length === 0 && <span className="muted" style={{ fontSize: 12 }}>{t('wiki.rawsNone')}</span>}
            {raws.map(r => (
              <div key={`${r.raw}>${r.translated}`} data-merge-raw style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'baseline', fontSize: 13 }}>
                <b>{r.raw}</b><span className="muted">→</span><span>{r.translated}</span>
                <span className={`tag ${r.entryUid && r.entryUid !== entry.uid ? 'tag-accent-2' : 'tag-neutral'}`} style={{ fontSize: 11 }}>
                  {r.entryUid === entry.uid ? t('wiki.rawOwn') : r.entryUid ? t('wiki.rawOther') : t('wiki.rawLoose')}
                </span>
                {r.chapters.length > 0 && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.rawChapters', { list: r.chapters.join(', ') })}</span>}
              </div>
            ))}
          </section>
        )}
        {q.status === 'failed' && <p role="alert">{String(q.error)}</p>}
        {!pairs && q.status === 'loading' && <Spinner size={18} />}
        {pairs && (
          <button type="button" className="btn btn-secondary" data-smart-merge-ai style={{ height: 38, padding: '0 14px', gap: 6, alignSelf: 'flex-start' }} disabled={asking || busy} onClick={askAi}>
            {asking ? <Spinner size={13} /> : <SparklesIcon size={14} />}
            {asking ? (step ? t('wiki.smartMergeAiStep', step) : t('wiki.smartMergeAiAsking')) : t('wiki.smartMergeAi')}
          </button>
        )}
        {pairs && pairs.length === 0 && groups.length === 0 && <p className="muted" style={{ fontSize: 14 }}>{t('wiki.smartMergeNone')}</p>}
        {pairs && (pairs.length > 0 || groups.length > 0) && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxHeight: '55vh', overflowY: 'auto' }}>
            {groups.map(g => {
              const k = gid(g);
              const name = names[k] ?? g.name;
              return (
                <div key={k} data-merge-group style={card}>
                  <Checkbox checked={!!picked?.has(k)} onChange={() => toggle(k)} size={20}>
                    <span style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
                      <span style={{ fontSize: 14, overflowWrap: 'anywhere' }}>
                        {[...g.members, g.into].map(label).join(', ')}
                      </span>
                      <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                        <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${g.into.kind}`)}</span>
                        <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t(g.members.length ? 'wiki.reason.ai' : 'wiki.reason.aiName')}</span>
                      </span>
                    </span>
                  </Checkbox>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8, paddingLeft: 30 }}>
                    <label style={{ fontSize: 12, fontWeight: 700 }} htmlFor={`final-${k}`}>{t('wiki.finalName')}</label>
                    <input id={`final-${k}`} className="input" data-final-name style={{ height: 38 }} value={name}
                      onChange={e => setNames(n => ({ ...n, [k]: e.target.value }))} />
                    {g.note && <span style={{ fontSize: 12, overflowWrap: 'anywhere' }}>{g.note}</span>}
                    {g.rules.length > 0 && (
                      <span className="muted" style={{ fontSize: 12, overflowWrap: 'anywhere' }}>
                        {t('wiki.smartMergeRules')}: {g.rules.filter(r => r.find !== name.trim()).map(r => `${r.find} → ${name.trim()}`).join(', ')}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
            {pairs.map(s => (
              <div key={pid(s)} data-merge-pair style={card}>
                <Checkbox checked={!!picked?.has(pid(s))} onChange={() => toggle(pid(s))} size={20}>
                  <span style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                    <span style={{ fontSize: 14, overflowWrap: 'anywhere' }}>
                      {label(s.from)} <span className="muted">→</span> <b>{label(s.into)}</b>
                    </span>
                    <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                      <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${s.into.kind}`)}</span>
                      <span className={`tag ${s.reason === 'near_name' ? 'tag-neutral' : 'tag-accent-2'}`} style={{ fontSize: 11 }}>{t(`wiki.reason.${s.reason}`)}</span>
                    </span>
                    {s.rules.length > 0 && (
                      <span className="muted" style={{ fontSize: 12, overflowWrap: 'anywhere' }}>
                        {t('wiki.smartMergeRules')}: {s.rules.map(r => `${r.find} → ${r.replace}`).join(', ')}
                      </span>
                    )}
                  </span>
                </Checkbox>
              </div>
            ))}
          </div>
        )}
        <div className="dialog-actions">
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
          <button type="button" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={busy || asking || !count} onClick={apply}>
            {busy ? <Spinner size={14} /> : t('wiki.smartMergeApply', { n: count })}
          </button>
        </div>
      </div>
    </Modal>
  );
}
