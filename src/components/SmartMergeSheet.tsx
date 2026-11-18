import { useEffect, useState } from 'react';
import { Modal } from './Modal';
import { Checkbox, Spinner } from './ui';
import { SparklesIcon } from './Icons';
import { useI18n } from '../i18n';
import { AiFailure, aiErrorDetail } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import {
  applyWikiMerges, entryWikiMerges, entryWikiMergesAi, suggestWikiMerges, suggestWikiMergesAi, wikiLabel,
  type MergeSuggestion, type RawName, type WikiEntry } from '../lib/wiki';
import { useApp } from '../store/AppStore';

/**
 * Smart merge: lists the entries that look like one written two ways, for the user to
 * review first. Same names are ticked, near ones are not; each pair shows the line it
 * adds to the replace rules, so chapters translated before show the kept name.
 */
export function SmartMergeSheet({ novelId, lang, entry, onClose, onMerged }: {
  novelId: number; lang: string; entry?: WikiEntry; onClose: () => void; onMerged: () => void;
}) {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  // For one entry: the original names translated as its name, and only its pairs.
  const q = useAsync(
    () => (entry ? entryWikiMerges(user, novelId, entry.uid) : suggestWikiMerges(user, novelId).then(pairs => ({ raws: [] as RawName[], pairs }))),
    [user, novelId, entry?.uid]
  );
  const [fromAi, setFromAi] = useState<MergeSuggestion[]>([]);
  const [asking, setAsking] = useState(false);
  const list = q.status === 'ready' ? [...q.data.pairs, ...fromAi] : null;
  const raws = q.status === 'ready' ? q.data.raws : [];
  const id = (s: MergeSuggestion) => `${s.from.uid}>${s.into.uid}`;
  // The AI also reads the descriptions: a title and the name of the one who holds it.
  const askAi = async () => {
    if (asking) return;
    setAsking(true);
    try {
      const more = await (entry ? entryWikiMergesAi(user, novelId, entry.uid) : suggestWikiMergesAi(user, novelId));
      const fresh = more.filter(s => !list?.some(x => id(x) === id(s)));
      setFromAi(f => [...f, ...fresh]);
      setPicked(p => new Set([...(p ?? []), ...fresh.map(id)]));
      app.showToast(t('wiki.smartMergeAiFound', { n: fresh.length }));
    } catch (e) {
      console.error(e);
      const f = e instanceof AiFailure ? e : null;
      app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}` : String(e));
    } finally {
      setAsking(false);
    }
  };
  const [picked, setPicked] = useState<Set<string> | null>(null);
  useEffect(() => {
    if (list && !picked) setPicked(new Set(list.filter(s => s.reason !== 'near_name').map(id)));
  }, [list, picked]);
  const [busy, setBusy] = useState(false);
  const chosen = list?.filter(s => picked?.has(id(s))) ?? [];
  const toggle = (s: MergeSuggestion) => setPicked(p => {
    const n = new Set(p ?? []);
    if (n.has(id(s))) n.delete(id(s)); else n.add(id(s));
    return n;
  });
  const apply = async () => {
    if (!chosen.length || busy) return;
    setBusy(true);
    try {
      const done = await applyWikiMerges(user, novelId, chosen.map(s => ({ fromUid: s.from.uid, intoUid: s.into.uid })));
      app.showToast(t('wiki.smartMerged', { n: done.merged, rules: done.rules }));
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
  return (
    <Modal onClose={busy ? undefined : onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('wiki.smartMerge')} data-wiki-smart-merge-sheet onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{entry ? `${t('wiki.smartMerge')}: ${wikiLabel(entry, lang)}` : t('wiki.smartMerge')}</div>
        <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>{t('wiki.smartMergeSub')}</div>
        {entry && q.status === 'ready' && (
          <section data-merge-raws style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ fontSize: 13, fontWeight: 700 }}>{t('wiki.rawsTitle', { name: wikiLabel(entry, lang) })}</div>
            {raws.length === 0 && <span className="muted" style={{ fontSize: 12 }}>{t('wiki.rawsNone')}</span>}
            {raws.map(r => (
              <div key={`${r.raw}>${r.translated}`} data-merge-raw style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'baseline', fontSize: 13 }}>
                <b>{r.raw}</b><span className="muted">→</span><span>{r.translated}</span>
                <span className={`tag ${r.entryUid === entry.uid ? 'tag-neutral' : r.entryUid ? 'tag-accent-2' : 'tag-neutral'}`} style={{ fontSize: 11 }}>
                  {r.entryUid === entry.uid ? t('wiki.rawOwn') : r.entryUid ? t('wiki.rawOther') : t('wiki.rawLoose')}
                </span>
                {r.chapters.length > 0 && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.rawChapters', { list: r.chapters.join(', ') })}</span>}
              </div>
            ))}
          </section>
        )}
        {q.status === 'failed' && <p role="alert">{String(q.error)}</p>}
        {!list && q.status === 'loading' && <Spinner size={18} />}
        {list && (
          <button type="button" className="btn btn-secondary" data-smart-merge-ai style={{ height: 38, padding: '0 14px', gap: 6, alignSelf: 'flex-start' }} disabled={asking || busy} onClick={askAi}>
            {asking ? <Spinner size={13} /> : <SparklesIcon size={14} />}
            {asking ? t('wiki.smartMergeAiAsking') : t('wiki.smartMergeAi')}
          </button>
        )}
        {list && list.length === 0 && <p className="muted" style={{ fontSize: 14 }}>{t('wiki.smartMergeNone')}</p>}
        {list && list.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxHeight: '55vh', overflowY: 'auto' }}>
            {list.map(s => (
              <div key={id(s)} data-merge-pair style={{ padding: '10px 12px', borderRadius: 16, background: 'var(--frost)' }}>
                <Checkbox checked={!!picked?.has(id(s))} onChange={() => toggle(s)} size={20}>
                  <span style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                    <span style={{ fontSize: 14, overflowWrap: 'anywhere' }}>
                      {label(s.from)} <span className="muted">→</span> <b>{label(s.into)}</b>
                    </span>
                    <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                      <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${s.into.kind}`)}</span>
                      <span className={`tag ${s.reason === 'near_name' ? 'tag-neutral' : 'tag-accent-2'}`} style={{ fontSize: 11 }}>{t(`wiki.reason.${s.reason}`)}</span>
                    </span>
                    {s.note && <span style={{ fontSize: 12, overflowWrap: 'anywhere' }}>{s.note}</span>}
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
          <button type="button" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={busy || asking || !chosen.length} onClick={apply}>
            {busy ? <Spinner size={14} /> : t('wiki.smartMergeApply', { n: chosen.length })}
          </button>
        </div>
      </div>
    </Modal>
  );
}
