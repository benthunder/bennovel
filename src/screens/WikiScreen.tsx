import { useMemo, useState, type ReactNode } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { Modal } from '../components/Modal';
import { levelOf, relWords, useWikiT } from '../components/WikiPopup';
import { ChevronLeftIcon, ChevronRightIcon, PencilIcon, SparklesIcon, XIcon } from '../components/Icons';
import { Checkbox, SearchField, Spinner } from '../components/ui';
import { useI18n } from '../i18n';
import { AI_LANGS, AiFailure, aiErrorDetail, getReadingLang } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { useThrottledValue } from '../lib/useThrottledValue';
import {
  RAW, REL_TYPES, WIKI_KINDS, addWikiRelation, getWiki, getWikiMissing, removeWikiEntry, removeWikiRelation, saveWikiEntry,
  searchWiki, setWikiAuto, translateWiki, wikiChapter, wikiLabel, wikiSummary,
  type Wiki, type WikiEntry, type WikiKind
} from '../lib/wiki';
import { useApp } from '../store/AppStore';

/** The wiki's languages: the raw text, then the AI ones. */
const WIKI_LANGS = [RAW, ...AI_LANGS.map(l => l.code as string)];

/**
 * The wiki of a book on this device: its characters, levels, places (as a map), factions,
 * items and terms with their relationships. The AI fills it while translating; everything
 * can be edited. It shows in the language the book is read in (`lang`), falling back to
 * the raw words, and can fill a language in with the AI.
 */
export function WikiScreen({ novelId, title, lang: startLang }: { novelId: number; title: string; lang?: string }) {
  const { t } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const user = app.libKey;
  const [lang, setLang] = useState(() => {
    const l = startLang ?? getReadingLang(user, novelId);
    return WIKI_LANGS.includes(l) ? l : RAW;
  });
  const langName = (l: string) => (l === RAW ? t('reader.raw') : AI_LANGS.find(x => x.code === l)?.native ?? l);
  const [rev, setRev] = useState(0);
  const q = useAsync(() => getWiki(user, novelId), [user, novelId, rev, app.aiRev]);
  const missingQ = useAsync(() => (lang === RAW ? Promise.resolve(0) : getWikiMissing(user, novelId, lang)), [user, novelId, lang, rev, app.aiRev]);
  const [translating, setTranslating] = useState(false);
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
  const missing = missingQ.status === 'ready' ? missingQ.data : 0;

  const toggleAuto = async () => {
    if (!wiki) return;
    const n = await setWikiAuto(user, novelId, !wiki.auto).catch(e => { console.error(e); return 0; });
    if (n > 0) app.showToast(t('wiki.fromKept', { n }));
    reload();
  };

  const translate = async () => {
    if (translating) return;
    setTranslating(true);
    try {
      const n = await translateWiki(user, novelId, lang);
      app.showToast(t('wiki.translated', { lang: langName(lang), n }));
      reload();
    } catch (e) {
      console.error(e);
      const f = e instanceof AiFailure ? e : null;
      app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}` : String(e));
    } finally {
      setTranslating(false);
    }
  };

  const card = (e: WikiEntry, extra?: ReactNode) => {
    const label = wikiLabel(e, lang);
    const sum = wikiSummary(e, lang);
    const rels = relCount.get(e.uid) ?? 0;
    const level = wiki && e.kind === 'character' ? levelOf(wiki, e.uid) : null;
    return (
      <button key={e.uid} data-entry={e.name} className="novel-row" style={{ alignItems: 'flex-start', gap: 10, padding: '12px 14px', borderRadius: 22, background: 'var(--color-surface)', minWidth: 0, textAlign: 'left', width: '100%' }} onClick={() => setOpen(e)}>
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
          <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
            <span style={{ fontSize: 15, fontWeight: 700 }}>{label}</span>
            {label !== e.name && <span className="muted" style={{ fontSize: 12 }}>{e.name}</span>}
          </span>
          {sum.text && (
            <span style={{ fontSize: 13, color: 'var(--color-neutral-800)', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
              {lang !== RAW && !sum.inLang && <span className="tag tag-neutral" style={{ fontSize: 10, marginRight: 6 }}>{sum.from === RAW ? t('wiki.rawTag') : sum.from.toUpperCase()}</span>}
              {sum.text}
            </span>
          )}
          <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 2 }}>
            <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${e.kind}`)}</span>
            {e.kind === 'level' && e.rank != null && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.rankN', { n: e.rank })}</span>}
            {level && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{wikiLabel(level, lang)}</span>}
            {rels > 0 && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.relCount', { n: rels })}</span>}
            {e.firstChapter != null && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.fromChapter', { n: wikiChapter(e)! })}</span>}
          </span>
          {extra}
        </span>
        <ChevronRightIcon size={16} style={{ flex: 'none', marginTop: 4, color: 'var(--color-neutral-500)' }} />
      </button>
    );
  };

  const list = (items: WikiEntry[]) => (
    <div style={{ display: 'grid', gridTemplateColumns: lay.isPhone ? '1fr' : '1fr 1fr', gap: 8 }}>{items.map(e => card(e))}</div>
  );

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

        <div role="radiogroup" aria-label={t('wiki.langAria')} style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          {WIKI_LANGS.map(l => (
            <button key={l} role="radio" aria-checked={lang === l} className={`chip chip--sm${lang === l ? ' chip--active' : ''}`} onClick={() => setLang(l)}>
              {langName(l)}
            </button>
          ))}
        </div>

        {q.status === 'failed' && <p role="alert">{t('wiki.failed')} {String(q.error)}</p>}
        {!wiki && q.status === 'loading' && <Spinner size={18} />}
        {wiki && (
          <>
            {missing > 0 && (
              <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap', padding: '12px 14px', borderRadius: 20, background: 'color-mix(in srgb, var(--color-accent) 10%, var(--color-surface))' }}>
                <span style={{ flex: '1 1 200px', fontSize: 13 }}>{t('wiki.missing', { n: missing, lang: langName(lang) })}</span>
                <button className="btn btn-primary" style={{ height: 38, padding: '0 14px', gap: 6 }} disabled={translating} onClick={translate}>
                  {translating ? <Spinner size={13} /> : <SparklesIcon size={14} />}
                  {translating ? t('wiki.translating') : t('wiki.translate', { lang: langName(lang) })}
                </button>
              </div>
            )}

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
            </div>

            {kinds.length > 0 && (
              <div role="tablist" style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {(['all', ...kinds] as const).map(k => (
                  <button key={k} role="tab" aria-selected={kind === k} className={`chip chip--sm${kind === k ? ' chip--active' : ''}`} onClick={() => setKind(k)}>
                    {k === 'all' ? t('wiki.all') : k === 'place' ? t('wiki.tab.map') : t(`wiki.kind.${k}`)}
                  </button>
                ))}
              </div>
            )}

            {shown == null ? <Spinner size={16} />
              : shown.length === 0 ? <p className="muted" style={{ fontSize: 14 }}>{t(wiki.entries.length ? 'wiki.noMatch' : 'wiki.empty')}</p>
                : !term && kind === 'level' ? <Ladder wiki={wiki} lang={lang} card={card} />
                  : !term && kind === 'place' ? <MapTree wiki={wiki} card={card} />
                    : list(shown)}
          </>
        )}
      </div>
      {open && wiki && (
        <EntrySheet key={open === 'new' ? 'new' : open.uid} novelId={novelId} wiki={wiki} entry={open === 'new' ? null : open} lang={lang}
          onClose={() => setOpen(null)} onChanged={reload} />
      )}
    </div>
  );
}

/** The levels in order, lowest first, with who has reached each. */
function Ladder({ wiki, lang, card }: { wiki: Wiki; lang: string; card: (e: WikiEntry, extra?: ReactNode) => ReactNode }) {
  const levels = wiki.entries.filter(e => e.kind === 'level')
    .sort((a, b) => (a.rank ?? Infinity) - (b.rank ?? Infinity) || a.name.localeCompare(b.name));
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  return (
    <ol data-wiki-ladder style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
      {levels.map(l => {
        const who = wiki.relations
          .filter(r => r.kind === 'has_level' && r.toUid === l.uid)
          .map(r => byUid.get(r.fromUid))
          .filter((e): e is WikiEntry => !!e && levelOf(wiki, e.uid)?.uid === l.uid);
        return (
          <li key={l.uid}>
            {card(l, who.length > 0 && (
              <span style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 4 }}>
                {who.map(e => <span key={e.uid} className="chip chip--sm" style={{ fontSize: 11, pointerEvents: 'none' }}>{wikiLabel(e, lang)}</span>)}
              </span>
            ))}
          </li>
        );
      })}
    </ol>
  );
}

/** Places nested by "located in": continents, then countries, cities, sects… */
function MapTree({ wiki, card }: { wiki: Wiki; card: (e: WikiEntry, extra?: ReactNode) => ReactNode }) {
  const { t } = useI18n();
  const byUid = new Map(wiki.entries.map(e => [e.uid, e]));
  const parent = new Map<string, string>();
  for (const r of wiki.relations) {
    if (r.kind === 'located_in' && byUid.get(r.toUid)?.kind === 'place' && !parent.has(r.fromUid)) parent.set(r.fromUid, r.toUid);
  }
  const children = new Map<string, WikiEntry[]>();
  for (const [child, p] of parent) {
    const e = byUid.get(child);
    if (e) children.set(p, [...(children.get(p) ?? []), e]);
  }
  // A loop ("A in B, B in A") would never end; each place shows once.
  const seen = new Set<string>();
  const node = (e: WikiEntry, depth: number): ReactNode => {
    if (seen.has(e.uid)) return null;
    seen.add(e.uid);
    const kids = children.get(e.uid) ?? [];
    return (
      <li key={e.uid}>
        {card(e)}
        {kids.length > 0 && (
          <ul style={{ listStyle: 'none', margin: '8px 0 0', padding: `0 0 0 ${depth < 4 ? 18 : 8}px`, borderLeft: '2px solid var(--color-neutral-200)', display: 'flex', flexDirection: 'column', gap: 8 }}>
            {kids.map(k => node(k, depth + 1))}
          </ul>
        )}
      </li>
    );
  };
  const roots = wiki.entries.filter(e => e.kind === 'place' && !parent.has(e.uid));
  if (!roots.length && !wiki.entries.some(e => e.kind === 'place')) return <p className="muted" style={{ fontSize: 14 }}>{t('wiki.mapEmpty')}</p>;
  return (
    <ul data-wiki-map style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
      {roots.map(e => node(e, 0))}
      {wiki.entries.filter(e => e.kind === 'place' && !seen.has(e.uid)).map(e => node(e, 0))}
    </ul>
  );
}

/** One entry: its names and description per language, its rank and relationships, all editable. */
function EntrySheet({ novelId, wiki, entry, lang, onClose, onChanged }: {
  novelId: number; wiki: Wiki; entry: WikiEntry | null; lang: string; onClose: () => void; onChanged: () => void;
}) {
  const { t } = useI18n();
  const wt = useWikiT(lang);
  const app = useApp();
  const user = app.libKey;
  const [saved, setSaved] = useState(entry);
  const [kind, setKind] = useState<WikiKind>(entry?.kind ?? 'character');
  const [name, setName] = useState(entry?.name ?? '');
  const [names, setNames] = useState<Record<string, string>>(entry?.names ?? {});
  const [summaries, setSummaries] = useState<Record<string, string>>(entry?.summaries ?? {});
  const [rank, setRank] = useState(entry?.rank != null ? String(entry.rank) : '');
  const [aliases, setAliases] = useState(entry?.aliases.join(', ') ?? '');
  // The language whose words are being edited; starts at the wiki's.
  const [editLang, setEditLang] = useState(lang);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [relTo, setRelTo] = useState('');
  const [relType, setRelType] = useState<string>(REL_TYPES[0]);
  const [relWordsIn, setRelWordsIn] = useState('');
  const [rels, setRels] = useState(() => wiki.relations.filter(r => entry && (r.fromUid === entry.uid || r.toUid === entry.uid)));

  const byUid = useMemo(() => new Map(wiki.entries.map(e => [e.uid, e])), [wiki]);
  const others = wiki.entries.filter(e => e.uid !== saved?.uid);
  const show = (uid: string) => { const e = byUid.get(uid); return e ? wikiLabel(e, lang) : '?'; };
  const langName = (l: string) => (l === RAW ? t('reader.raw') : AI_LANGS.find(x => x.code === l)?.native ?? l);
  // Free words are kept in the language being read; the raw wiki keeps them as written.
  const wordsLang = lang === RAW ? (AI_LANGS.find(l => Object.keys(names).includes(l.code))?.code ?? 'vi') : lang;

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
      const n = Number.parseInt(rank, 10);
      const e = await saveWikiEntry(user, novelId, {
        uid: saved?.uid, kind, name, names, summaries, rank: kind === 'level' && Number.isFinite(n) ? n : null,
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
    if (!saved || !(await ask(t('wiki.removeAsk', { name: wikiLabel(saved, lang) }), { kind: 'warning' }))) return;
    await removeWikiEntry(user, saved.uid).catch(console.error);
    onChanged();
    app.showToast(t('wiki.removed'));
    onClose();
  };

  const custom = relType === 'custom';
  const relKind = custom ? relWordsIn.trim() : relType;
  const addRel = async () => {
    if (!saved || !relTo || !relKind) return;
    try {
      const uid = await addWikiRelation(user, novelId, saved.uid, relTo, relKind, wordsLang);
      setRels(r => [...r.filter(x => x.uid !== uid), {
        uid, fromUid: saved.uid, toUid: relTo, kind: relKind, labels: custom ? { [wordsLang]: relKind } : {}, note: '', firstChapter: null
      }]);
      setRelWordsIn('');
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

  const level = saved && saved.kind === 'character' ? levelOf(wiki, saved.uid) : null;

  return (
    <Modal onClose={busy ? undefined : onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t(saved ? 'wiki.editTitle' : 'wiki.newTitle')}
        onClick={e => e.stopPropagation()} onSubmit={submit}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{saved ? wikiLabel(saved, lang) : t('wiki.newTitle')}</div>
        {saved && (
          <div className="muted" style={{ fontSize: 12, marginTop: -8 }}>
            {[saved.firstChapter != null && t('wiki.fromChapter', { n: wikiChapter(saved)! }), saved.source === 'user' && t('wiki.byYou')].filter(Boolean).join(' · ')}
          </div>
        )}
        {level && (
          <div style={{ fontSize: 13 }}><b>{t('wiki.currentLevel')}:</b> {wikiLabel(level, lang)}</div>
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
        {kind === 'level' && (
          <div className="field">
            <label htmlFor="wiki-rank">{t('wiki.rank')}</label>
            <input id="wiki-rank" className="input" style={{ height: 42 }} inputMode="numeric" value={rank} onChange={e => setRank(e.target.value.replace(/[^\d-]/g, ''))} />
          </div>
        )}
        <div className="field">
          <label htmlFor="wiki-aliases">{t('wiki.aliases')}</label>
          <input id="wiki-aliases" className="input" style={{ height: 42 }} value={aliases} onChange={e => setAliases(e.target.value)} />
        </div>

        <div role="tablist" aria-label={t('wiki.langAria')} style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
          {WIKI_LANGS.map(l => (
            <button key={l} type="button" role="tab" aria-selected={editLang === l} className={`chip chip--sm${editLang === l ? ' chip--active' : ''}`} onClick={() => setEditLang(l)}>
              {langName(l)}{(l === RAW ? summaries[RAW] : names[l] || summaries[l]) ? ' ✓' : ''}
            </button>
          ))}
        </div>
        {editLang !== RAW && (
          <div className="field">
            <label htmlFor="wiki-name-in">{t('wiki.nameIn', { lang: langName(editLang) })}</label>
            <input id="wiki-name-in" className="input" style={{ height: 42 }} value={names[editLang] ?? ''}
              onChange={e => setNames(n => ({ ...n, [editLang]: e.target.value }))} />
          </div>
        )}
        <div className="field">
          <label htmlFor="wiki-summary">{t('wiki.summaryIn', { lang: langName(editLang) })}</label>
          <textarea id="wiki-summary" className="input" rows={4} style={{ borderRadius: 20 }} value={summaries[editLang] ?? ''}
            placeholder={wikiSummary({ ...(saved ?? entryStub), summaries }, editLang).text}
            onChange={e => setSummaries(s => ({ ...s, [editLang]: e.target.value }))} />
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.relations')}</div>
          {!saved ? <div className="muted" style={{ fontSize: 12 }}>{t('wiki.relSaveFirst')}</div> : (
            <>
              {rels.length === 0 && <div className="muted" style={{ fontSize: 12 }}>{t('wiki.noRelations')}</div>}
              {rels.map(r => {
                const other = r.fromUid === saved.uid ? r.toUid : r.fromUid;
                return (
                  <div key={r.uid} data-relation={r.kind} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, padding: '6px 6px 6px 12px', borderRadius: 16, background: 'var(--color-bg)' }}>
                    <span style={{ flex: 1, minWidth: 0 }}>
                      {relWords(r, saved.uid, lang, wt)} <b>{show(other)}</b>
                    </span>
                    <button type="button" className="icon-btn" style={{ width: 30, height: 30 }} aria-label={t('wiki.remove')} onClick={() => dropRel(r.uid)}><XIcon size={14} /></button>
                  </div>
                );
              })}
              {others.length > 0 && (
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                  <select className="input" style={{ height: 38, flex: '1 1 140px' }} aria-label={t('wiki.relType')} value={relType} onChange={e => setRelType(e.target.value)}>
                    {REL_TYPES.map(k => <option key={k} value={k}>{wt(`wiki.rel.${k}`)}</option>)}
                    <option value="custom">{t('wiki.relCustom')}</option>
                  </select>
                  {custom && (
                    <input className="input" style={{ height: 38, flex: '1 1 140px' }} placeholder={t('wiki.relCustomPh')} aria-label={t('wiki.relKind')}
                      value={relWordsIn} onChange={e => setRelWordsIn(e.target.value)} />
                  )}
                  <select className="input" style={{ height: 38, flex: '1 1 140px' }} aria-label={t('wiki.relTo')} value={relTo} onChange={e => setRelTo(e.target.value)}>
                    <option value="">{t('wiki.relTo')}…</option>
                    {others.map(e => <option key={e.uid} value={e.uid}>{wikiLabel(e, lang)}</option>)}
                  </select>
                  <button type="button" className="btn btn-secondary" style={{ height: 38, padding: '0 14px' }} disabled={!relTo || !relKind} onClick={addRel}>{t('wiki.relAdd')}</button>
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

/** A blank entry, for the description placeholder of one not saved yet. */
const entryStub: WikiEntry = { uid: '', kind: 'other', name: '', names: {}, aliases: [], summaries: {}, rank: null, firstChapter: null, source: 'user', updatedAt: 0 };
