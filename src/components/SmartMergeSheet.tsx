import { useCallback, useEffect, useRef, useState } from 'react';
import { Modal } from './Modal';
import { Checkbox, Spinner } from './ui';
import { SparklesIcon } from './Icons';
import { useI18n } from '../i18n';
import { AiFailure, aiErrorDetail, aiErrorKey } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import {
  applyWikiMerges, controlWikiMergeAi, entryWikiMerges, entryWikiMergesAi, startWikiMergeAi, suggestWikiMerges, wikiLabel,
  wikiMergeAiStatus, type MergeGroup, type MergeRun, type MergeSuggestion, type RawName, type WikiEntry } from '../lib/wiki';
import { useApp } from '../store/AppStore';

/**
 * Smart merge, offline first and then online. The device's candidates show at once for
 * review: same names ticked, near ones not; for one entry, also the original names the
 * translations wrote as its name. "Ask the AI" sends everything known about them (for
 * the whole wiki, in blocks of about 10k letters run in parallel) and brings back groups
 * with the final name (and the right original name) to use, which can be edited. For the
 * whole wiki the AI runs in the background like the translation queue: closing the
 * sheet or the app keeps the run, a lost connection pauses it to carry on, and the
 * groups found so far can be reviewed while the rest is read. Each merge writes the final name
 * over its old names in the chapters translated before; the user's replace list is not touched.
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
  // One entry's groups come back from one request; the whole wiki's from its run.
  const [entryGroups, setEntryGroups] = useState<MergeGroup[]>([]);
  const [run, setRun] = useState<MergeRun | null>(null);
  const groups = entry ? entryGroups : run?.groups ?? [];
  // The final name per group, as the user may have changed it.
  const [names, setNames] = useState<Record<string, string>>({});
  // The right original name per group, as the user may have changed it.
  const [raws2, setRaws] = useState<Record<string, string>>({});
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
  const fail = (e: unknown) => {
    console.error(e);
    const f = e instanceof AiFailure ? e : null;
    app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}` : String(e));
  };
  // Groups seen before keep the user's ticks and edits; new ones get the AI's names, and
  // the ones that merge (or only put a wrong original name right) are ticked.
  const seen = useRef(new Set<string>());
  const take = useCallback((found: MergeGroup[]) => {
    const fresh = found.filter(g => !seen.current.has(gid(g)));
    if (!fresh.length) return 0;
    fresh.forEach(g => seen.current.add(gid(g)));
    setNames(n => ({ ...n, ...Object.fromEntries(fresh.map(g => [gid(g), g.name])) }));
    setRaws(n => ({ ...n, ...Object.fromEntries(fresh.map(g => [gid(g), g.raw])) }));
    setPicked(p => new Set([...(p ?? []), ...fresh.filter(g => g.members.length > 0 || entry || (g.raw && g.name === wikiLabel(g.into, lang))).map(gid)]));
    return fresh.length;
  }, [entry, lang]);
  const loadRun = useCallback(async () => {
    const r = await wikiMergeAiStatus(user, novelId);
    take(r.groups);
    setRun(r);
  }, [user, novelId, take]);
  // The whole wiki's run, as left (it may be running in the background), and as it goes.
  useEffect(() => {
    if (entry) return;
    loadRun().catch(console.error);
    let stop: (() => void) | undefined;
    let gone = false;
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen<{ user: string; novelId: number }>('wiki-merge-progress', e => {
        if (e.payload.novelId === novelId && e.payload.user === user) loadRun().catch(console.error);
      })).then(un => { if (gone) un(); else stop = un; }).catch(() => undefined);
    return () => { gone = true; stop?.(); };
  }, [entry, novelId, user, loadRun]);
  const askAi = async (fresh = false) => {
    if (asking) return;
    setAsking(true);
    try {
      if (entry) {
        const found = await entryWikiMergesAi(user, novelId, entry.uid);
        const n = take(found);
        setEntryGroups(gs => [...gs, ...found.filter(g => !gs.some(x => gid(x) === gid(g)))]);
        app.showToast(t('wiki.smartMergeAiFound', { n }));
      } else {
        const r = await startWikiMergeAi(user, novelId, fresh);
        setRun(cur => ({ ...r, groups: fresh ? [] : cur?.groups ?? [] }));
        await loadRun();
      }
    } catch (e) {
      fail(e);
    } finally {
      setAsking(false);
    }
  };
  const control = async (action: 'pause' | 'resume' | 'retry') => {
    try {
      const r = await controlWikiMergeAi(user, novelId, action);
      setRun(cur => ({ ...r, groups: cur?.groups ?? [] }));
    } catch (e) {
      fail(e);
    }
  };
  const going = !!run && run.running + run.queued > 0;
  const runError = run?.error ? `${t(aiErrorKey(run.error))} ${aiErrorDetail(run.error)}`.trim() : '';

  const [busy, setBusy] = useState(false);
  const chosenPairs = pairs?.filter(s => picked?.has(pid(s))) ?? [];
  const chosenGroups = groups.filter(g => picked?.has(gid(g)) && (names[gid(g)] ?? g.name).trim());
  const count = chosenPairs.length + chosenGroups.length;
  const allKeys = [...(pairs ?? []).map(pid), ...groups.map(gid)];
  const allPicked = allKeys.length > 0 && allKeys.every(k => picked?.has(k));
  const pickAll = () => setPicked(allPicked ? new Set() : new Set(allKeys));
  const apply = async () => {
    if (!count || busy) return;
    setBusy(true);
    try {
      const done = await applyWikiMerges(user, novelId,
        chosenPairs.map(s => ({ fromUid: s.from.uid, intoUid: s.into.uid })),
        chosenGroups.map(g => ({ intoUid: g.into.uid, fromUids: g.members.map(m => m.uid), name: (names[gid(g)] ?? g.name).trim(), raw: (raws2[gid(g)] ?? g.raw).trim() })));
      app.showToast(t('wiki.smartMerged', { n: done.merged, rules: done.rules })
        + (done.renamed ? ` ${t('wiki.smartRenamed', { n: done.renamed })}` : '')
        + (done.fixed ? ` ${t('wiki.smartFixed', { n: done.fixed })}` : ''));
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
        {pairs && (entry || !run || run.total === 0) && (
          <button type="button" className="btn btn-secondary" data-smart-merge-ai style={{ height: 38, padding: '0 14px', gap: 6, alignSelf: 'flex-start' }} disabled={asking || busy} onClick={() => askAi()}>
            {asking ? <Spinner size={13} /> : <SparklesIcon size={14} />}
            {asking ? t('wiki.smartMergeAiAsking') : t('wiki.smartMergeAi')}
          </button>
        )}
        {!entry && run && run.total > 0 && (
          <section data-merge-run style={{ display: 'flex', flexDirection: 'column', gap: 6, ...card }}>
            <span role="status" style={{ display: 'flex', gap: 6, alignItems: 'center', fontSize: 13, overflowWrap: 'anywhere' }}>
              {going && <Spinner size={13} />}
              {going ? t('wiki.smartMergeRunning', { done: run.done, total: run.total })
                : run.paused > 0 ? (run.error ? t('wiki.smartMergeWaiting', { done: run.done, total: run.total, error: runError }) : t('wiki.smartMergePaused', { done: run.done, total: run.total }))
                  : t('wiki.smartMergeFinished', { total: run.total })}
              {run.failed > 0 && <span className="muted">{t('wiki.smartMergeFailedBlocks', { n: run.failed })}</span>}
            </span>
            <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {going && <button type="button" className="btn btn-secondary" data-merge-run-pause style={{ height: 34, padding: '0 12px' }} onClick={() => control('pause')}>{t('wiki.smartMergePause')}</button>}
              {!going && run.paused > 0 && <button type="button" className="btn btn-secondary" data-merge-run-resume style={{ height: 34, padding: '0 12px' }} onClick={() => control('resume')}>{t('wiki.smartMergeResume')}</button>}
              {run.failed > 0 && <button type="button" className="btn btn-secondary" data-merge-run-retry style={{ height: 34, padding: '0 12px' }} onClick={() => control('retry')}>{t('wiki.smartMergeRetry')}</button>}
              {!going && run.paused === 0 && <button type="button" className="btn btn-secondary" data-merge-run-restart style={{ height: 34, padding: '0 12px', gap: 6 }} disabled={asking} onClick={() => askAi(true)}><SparklesIcon size={13} />{t('wiki.smartMergeRestart')}</button>}
            </span>
          </section>
        )}
        {allKeys.length > 1 && (
          <button type="button" className="btn btn-secondary" data-merge-pick-all style={{ height: 34, padding: '0 12px', alignSelf: 'flex-start' }} onClick={pickAll}>
            {t(allPicked ? 'wiki.unselectAll' : 'wiki.selectAll')}
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
                        <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t(g.members.length ? 'wiki.reason.ai' : g.raw && g.name === wikiLabel(g.into, lang) ? 'wiki.reason.aiRaw' : 'wiki.reason.aiName')}</span>
                        {g.members.length > 0 && g.raw && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.reason.aiRaw')}</span>}
                      </span>
                    </span>
                  </Checkbox>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8, paddingLeft: 30 }}>
                    <label style={{ fontSize: 12, fontWeight: 700 }} htmlFor={`final-${k}`}>{t('wiki.finalName')}</label>
                    <input id={`final-${k}`} className="input" data-final-name style={{ height: 38 }} value={name}
                      onChange={e => setNames(n => ({ ...n, [k]: e.target.value }))} />
                    {g.raw && (
                      <>
                        <label style={{ fontSize: 12, fontWeight: 700 }} htmlFor={`raw-${k}`}>
                          {t('wiki.finalRaw')} <span className="muted" style={{ fontWeight: 400 }}>({t('wiki.finalRawWas', { raw: g.into.name })})</span>
                        </label>
                        <input id={`raw-${k}`} className="input" data-final-raw style={{ height: 38 }} value={raws2[k] ?? g.raw}
                          onChange={e => setRaws(n => ({ ...n, [k]: e.target.value }))} />
                      </>
                    )}
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
