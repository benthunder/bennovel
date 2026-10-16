import { useMemo, useState } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { Modal } from '../components/Modal';
import { ChevronLeftIcon, ChevronRightIcon, PencilIcon, XIcon } from '../components/Icons';
import { Checkbox, SearchField, Spinner } from '../components/ui';
import { useI18n } from '../i18n';
import { AI_LANGS, AiFailure, aiErrorDetail } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { useThrottledValue } from '../lib/useThrottledValue';
import {
  WIKI_KINDS, addWikiRelation, getWiki, removeWikiEntry, removeWikiRelation, saveWikiEntry, searchWiki, setWikiAuto, wikiLabel,
  type Wiki, type WikiEntry, type WikiKind
} from '../lib/wiki';
import { useApp } from '../store/AppStore';

/**
 * The wiki of a book on this device: its characters, places, factions, items and terms
 * with their relationships. The AI fills it while translating; everything can be edited.
 */
export function WikiScreen({ novelId, title }: { novelId: number; title: string }) {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const user = app.libKey;
  const [rev, setRev] = useState(0);
  const q = useAsync(() => getWiki(user, novelId), [user, novelId, rev, app.aiRev]);
  const [query, setQuery] = useState('');
  const term = useThrottledValue(query.trim(), 350);
  const found = useAsync(() => (term ? searchWiki(user, novelId, term) : Promise.resolve(null)), [user, novelId, term, rev]);
  const [kind, setKind] = useState<WikiKind | 'all'>('all');
  // The entry open in the sheet: an existing one, or `new`.
  const [open, setOpen] = useState<WikiEntry | 'new' | null>(null);
  const reload = () => setRev(n => n + 1);

  const wiki = q.status === 'ready' ? q.data : null;
  const relCount = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of wiki?.relations ?? []) for (const u of [r.fromUid, r.toUid]) m.set(u, (m.get(u) ?? 0) + 1);
    return m;
  }, [wiki]);

  const base = term ? (found.status === 'ready' ? found.data ?? [] : null) : wiki?.entries ?? null;
  const shown = base?.filter(e => kind === 'all' || e.kind === kind) ?? null;
  const kinds = WIKI_KINDS.filter(k => wiki?.entries.some(e => e.kind === k));

  const toggleAuto = async () => {
    if (!wiki) return;
    await setWikiAuto(user, novelId, !wiki.auto).catch(console.error);
    reload();
  };

  return (
    <div className="screen" style={{ paddingBottom: 'calc(var(--nav-h) + 32px)' }}>
      <div style={{ maxWidth: 820, margin: '0 auto', padding: lay.wide ? `${lay.v(0, 44, 32)}px ${lay.px}px 0` : 'calc(var(--safe-top) + 12px) 20px 0', display: 'flex', flexDirection: 'column', gap: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button className="icon-btn icon-btn--bg" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon /></button>
          <div style={{ flex: 1, minWidth: 0 }}>
            <h3 style={{ margin: 0, fontSize: 22 }}>{t('wiki.title')}</h3>
            <div className="muted" style={{ fontSize: 12, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{title}</div>
          </div>
          <button className="btn btn-primary" style={{ height: 40, padding: '0 16px', gap: 6 }} disabled={!wiki} onClick={() => setOpen('new')}>
            <PencilIcon size={14} />{t('wiki.add')}
          </button>
        </div>

        {q.status === 'failed' && <p role="alert">{t('wiki.failed')} {String(q.error)}</p>}
        {!wiki && q.status === 'loading' && <Spinner size={18} />}
        {wiki && (
          <>
            <div style={{ padding: 14, borderRadius: 24, background: 'var(--color-surface)' }}>
              <Checkbox checked={wiki.auto} onChange={toggleAuto} size={22}>
                <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                  <span style={{ fontSize: 14, fontWeight: 700 }}>{t('wiki.auto')}</span>
                  <span className="muted" style={{ fontSize: 12 }}>{t('wiki.autoSub')}</span>
                </span>
              </Checkbox>
              {wiki.lastError && (
                <div role="alert" style={{ fontSize: 12, marginTop: 10, color: 'var(--color-accent-700)', overflowWrap: 'anywhere' }}>
                  {t('wiki.lastError', { error: aiErrorDetail(wiki.lastError) || wiki.lastError })}
                </div>
              )}
            </div>

            <div>
              <SearchField value={query} onChange={setQuery} placeholder={t('wiki.search')} />
              <div className="muted" style={{ fontSize: 11, marginTop: 6 }}>{t(wiki.byMeaning ? 'wiki.byMeaning' : 'wiki.byName')}</div>
            </div>

            {kinds.length > 1 && (
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {(['all', ...kinds] as const).map(k => (
                  <button key={k} className={`chip chip--sm${kind === k ? ' chip--active' : ''}`} aria-pressed={kind === k} onClick={() => setKind(k)}>
                    {k === 'all' ? t('wiki.all') : t(`wiki.kind.${k}`)}
                  </button>
                ))}
              </div>
            )}

            {shown == null ? <Spinner size={16} />
              : shown.length === 0 ? <p className="muted" style={{ fontSize: 14 }}>{t(wiki.entries.length ? 'wiki.noMatch' : 'wiki.empty')}</p>
                : (
                  <div style={{ display: 'grid', gridTemplateColumns: lay.isPhone ? '1fr' : '1fr 1fr', gap: 8 }}>
                    {shown.map(e => {
                      const label = wikiLabel(e, uiLang);
                      const rels = relCount.get(e.uid) ?? 0;
                      return (
                        <button key={e.uid} className="novel-row" style={{ alignItems: 'flex-start', gap: 10, padding: '12px 14px', borderRadius: 22, background: 'var(--color-surface)', minWidth: 0, textAlign: 'left' }} onClick={() => setOpen(e)}>
                          <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
                            <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
                              <span style={{ fontSize: 15, fontWeight: 700 }}>{label}</span>
                              {label !== e.name && <span className="muted" style={{ fontSize: 12 }}>{e.name}</span>}
                            </span>
                            {e.summary && <span style={{ fontSize: 13, color: 'var(--color-neutral-800)', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>{e.summary}</span>}
                            <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 2 }}>
                              <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${e.kind}`)}</span>
                              {rels > 0 && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.relCount', { n: rels })}</span>}
                              {e.firstChapter != null && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.fromChapter', { n: e.firstChapter })}</span>}
                            </span>
                          </span>
                          <ChevronRightIcon size={16} style={{ flex: 'none', marginTop: 4, color: 'var(--color-neutral-500)' }} />
                        </button>
                      );
                    })}
                  </div>
                )}
          </>
        )}
      </div>
      {open && wiki && (
        <EntrySheet key={open === 'new' ? 'new' : open.uid} novelId={novelId} wiki={wiki} entry={open === 'new' ? null : open}
          onClose={() => setOpen(null)} onChanged={reload} />
      )}
    </div>
  );
}

/** One entry: its names, description and relationships, all editable. */
function EntrySheet({ novelId, wiki, entry, onClose, onChanged }: {
  novelId: number; wiki: Wiki; entry: WikiEntry | null; onClose: () => void; onChanged: () => void;
}) {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const [saved, setSaved] = useState(entry);
  const [kind, setKind] = useState<WikiKind>(entry?.kind ?? 'character');
  const [name, setName] = useState(entry?.name ?? '');
  const [names, setNames] = useState<Record<string, string>>(entry?.names ?? {});
  const [aliases, setAliases] = useState(entry?.aliases.join(', ') ?? '');
  const [summary, setSummary] = useState(entry?.summary ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [relTo, setRelTo] = useState('');
  const [relKind, setRelKind] = useState('');
  const [rels, setRels] = useState(() => wiki.relations.filter(r => entry && (r.fromUid === entry.uid || r.toUid === entry.uid)));

  const byUid = useMemo(() => new Map(wiki.entries.map(e => [e.uid, e])), [wiki]);
  const others = wiki.entries.filter(e => e.uid !== saved?.uid);
  const show = (uid: string) => { const e = byUid.get(uid); return e ? wikiLabel(e, uiLang) : '?'; };

  const failed = (e: unknown) => {
    console.error(e);
    const text = e instanceof AiFailure ? e.message : String(e);
    setError(text.includes('wiki-name-taken') ? t('wiki.nameTaken') : `${t('wiki.failed')} ${aiErrorDetail(text)}`);
  };

  const submit = async (ev: React.FormEvent) => {
    ev.preventDefault();
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const e = await saveWikiEntry(user, novelId, {
        uid: saved?.uid, kind, name, names, summary,
        aliases: aliases.split(/[,，、]/).map(a => a.trim()).filter(Boolean)
      });
      setSaved(e);
      onChanged();
      app.showToast(t('wiki.saved'));
      if (saved) onClose();
    } catch (e) {
      failed(e);
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!saved || !(await ask(t('wiki.removeAsk', { name: wikiLabel(saved, uiLang) }), { kind: 'warning' }))) return;
    await removeWikiEntry(user, saved.uid).catch(console.error);
    onChanged();
    app.showToast(t('wiki.removed'));
    onClose();
  };

  const addRel = async () => {
    if (!saved || !relTo || !relKind.trim()) return;
    try {
      const uid = await addWikiRelation(user, novelId, saved.uid, relTo, relKind.trim());
      setRels(r => [...r.filter(x => x.uid !== uid), { uid, fromUid: saved.uid, toUid: relTo, kind: relKind.trim(), note: '', firstChapter: null }]);
      setRelKind('');
      onChanged();
    } catch (e) {
      failed(e);
    }
  };

  const dropRel = async (uid: string) => {
    await removeWikiRelation(user, uid).catch(console.error);
    setRels(r => r.filter(x => x.uid !== uid));
    onChanged();
  };

  return (
    <Modal onClose={busy ? undefined : onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t(saved ? 'wiki.editTitle' : 'wiki.newTitle')}
        onClick={e => e.stopPropagation()} onSubmit={submit}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t(saved ? 'wiki.editTitle' : 'wiki.newTitle')}</div>
        {saved && (
          <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>
            {[saved.firstChapter != null && t('wiki.fromChapter', { n: saved.firstChapter }), saved.source === 'user' && t('wiki.byYou')].filter(Boolean).join(' · ')}
          </div>
        )}
        <div>
          <div style={{ fontSize: 12, fontWeight: 700, marginBottom: 6 }}>{t('wiki.kind')}</div>
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            {WIKI_KINDS.map(k => (
              <button key={k} type="button" className={`chip chip--sm${kind === k ? ' chip--active' : ''}`} aria-pressed={kind === k} onClick={() => setKind(k)}>
                {t(`wiki.kind.${k}`)}
              </button>
            ))}
          </div>
        </div>
        <div className="field">
          <label htmlFor="wiki-name">{t('wiki.name')}</label>
          <input id="wiki-name" className="input" style={{ height: 42 }} value={name} required autoFocus={!saved} onChange={e => setName(e.target.value)} />
        </div>
        {AI_LANGS.map(l => (
          <div key={l.code} className="field">
            <label htmlFor={`wiki-name-${l.code}`}>{t('wiki.nameIn', { lang: l.native })}</label>
            <input id={`wiki-name-${l.code}`} className="input" style={{ height: 42 }} value={names[l.code] ?? ''}
              onChange={e => setNames(n => ({ ...n, [l.code]: e.target.value }))} />
          </div>
        ))}
        <div className="field">
          <label htmlFor="wiki-aliases">{t('wiki.aliases')}</label>
          <input id="wiki-aliases" className="input" style={{ height: 42 }} value={aliases} onChange={e => setAliases(e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="wiki-summary">{t('wiki.summary')}</label>
          <textarea id="wiki-summary" className="input" rows={4} style={{ borderRadius: 20 }} value={summary} onChange={e => setSummary(e.target.value)} />
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.relations')}</div>
          {!saved ? <div className="muted" style={{ fontSize: 12 }}>{t('wiki.relSaveFirst')}</div> : (
            <>
              {rels.length === 0 && <div className="muted" style={{ fontSize: 12 }}>{t('wiki.noRelations')}</div>}
              {rels.map(r => (
                <div key={r.uid} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, padding: '6px 6px 6px 12px', borderRadius: 16, background: 'var(--color-bg)' }}>
                  <span style={{ flex: 1, minWidth: 0 }}>
                    <b>{show(r.fromUid)}</b> · {r.kind} · <b>{show(r.toUid)}</b>
                  </span>
                  <button type="button" className="icon-btn" style={{ width: 30, height: 30 }} aria-label={t('wiki.remove')} onClick={() => dropRel(r.uid)}><XIcon size={14} /></button>
                </div>
              ))}
              {others.length > 0 && (
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                  <input className="input" style={{ height: 38, flex: '1 1 140px' }} placeholder={t('wiki.relKind')} aria-label={t('wiki.relKind')}
                    value={relKind} onChange={e => setRelKind(e.target.value)} />
                  <select className="input" style={{ height: 38, flex: '1 1 140px' }} aria-label={t('wiki.relTo')} value={relTo} onChange={e => setRelTo(e.target.value)}>
                    <option value="">{t('wiki.relTo')}…</option>
                    {others.map(e => <option key={e.uid} value={e.uid}>{wikiLabel(e, uiLang)}</option>)}
                  </select>
                  <button type="button" className="btn btn-secondary" style={{ height: 38, padding: '0 14px' }} disabled={!relTo || !relKind.trim()} onClick={addRel}>{t('wiki.relAdd')}</button>
                </div>
              )}
            </>
          )}
        </div>

        {error && <div role="alert" style={{ fontSize: 13, color: 'var(--color-accent-700)' }}>{error}</div>}
        <div className="dialog-actions">
          {saved && <button type="button" className="btn btn-ghost" style={{ marginRight: 'auto' }} disabled={busy} onClick={remove}>{t('wiki.remove')}</button>}
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
          <button type="submit" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={busy || !name.trim()}>
            {busy ? <Spinner size={14} /> : t('common.save')}
          </button>
        </div>
      </form>
    </Modal>
  );
}
