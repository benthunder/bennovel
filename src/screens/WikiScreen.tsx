import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { Modal } from '../components/Modal';
import { SmartMergeSheet } from '../components/SmartMergeSheet';
import { EntryMergeCheck, RetranslateSheet } from '../components/WikiNameTools';
import { GroupedRelations, equivalentsOf, levelOf, relWords, useWikiT } from '../components/WikiPopup';
import { CheckIcon, ChevronLeftIcon, ChevronRightIcon, LanguagesIcon, PencilIcon, SparklesIcon, XIcon } from '../components/Icons';
import { Checkbox, SearchField, Spinner } from '../components/ui';
import { useI18n } from '../i18n';
import { AI_LANGS, AiFailure, aiErrorDetail } from '../lib/ai';
import { useAsync } from '../lib/useAsync';
import { useLayout } from '../lib/useLayout';
import { useThrottledValue } from '../lib/useThrottledValue';
import {
  PROFILE_FIELDS, RAW, REL_TYPES, SKILL_FIELDS, WIKI_KINDS, WIKI_LANG, addWikiRelation, applyWikiToText, getWiki, getWikiMissing, mergeWikiEntry, removeWikiEntries, removeWikiEntry, removeWikiRelation, saveWikiEntry,
  searchWiki, setWikiAuto,  translateWiki, wikiChapter, wikiLabel, wikiSummary,
  type ProfileField, type SkillField, type Wiki, type WikiEntry, type WikiKind, metaStage } from '../lib/wiki';
import { useApp } from '../store/AppStore';

/**
 * The wiki of a book on this device: its characters, levels, places (as a map), factions,
 * items and terms with their relationships. The AI fills it while translating; everything
 * can be edited. It is written in Vietnamese only (the original names are kept as the
 * text writes them); what is still missing in Vietnamese is translated when it opens.
 */
export function WikiScreen({ novelId, title }: { novelId: number; title: string; lang?: string }) {
  const { t } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const user = app.libKey;
  // The wiki is written in Vietnamese only; what is still missing is translated on open.
  const lang = WIKI_LANG;
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
  const [merging, setMerging] = useState(false);
  // Picking many entries to delete them at once.
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [removing, setRemoving] = useState(false);
  const [applying, setApplying] = useState(false);
  // A merge or an edited name may have changed the translated chapters too.
  const reload = () => { setRev(n => n + 1); app.reloadText(); };

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

  const togglePick = (uid: string) => setPicked(p => {
    const n = new Set(p);
    if (n.has(uid)) n.delete(uid); else n.add(uid);
    return n;
  });
  const allPicked = !!shown?.length && shown.every(e => picked.has(e.uid));
  const pickAll = () => setPicked(p => {
    const n = new Set(p);
    for (const e of shown ?? []) { if (allPicked) n.delete(e.uid); else n.add(e.uid); }
    return n;
  });
  const stopPicking = () => { setPicking(false); setPicked(new Set()); };
  const removePicked = async () => {
    const uids = [...picked];
    if (!uids.length || removing || !(await ask(t('wiki.removeManyAsk', { n: uids.length }), { kind: 'warning' }))) return;
    setRemoving(true);
    try {
      const n = await removeWikiEntries(user, uids);
      app.showToast(t('wiki.removedMany', { n }));
      stopPicking();
      reload();
    } catch (e) {
      console.error(e);
      app.showToast(String(e));
    } finally {
      setRemoving(false);
    }
  };

  const applyText = async () => {
    if (applying || !(await ask(t('wiki.applyTextAsk'), { kind: 'info' }))) return;
    setApplying(true);
    try {
      const n = await applyWikiToText(user, novelId);
      app.showToast(t('wiki.applyTextDone', { n }));
      app.reloadText();
    } catch (e) {
      console.error(e);
      app.showToast(String(e));
    } finally {
      setApplying(false);
    }
  };

  const toggleAuto = async () => {
    if (!wiki) return;
    const n = await setWikiAuto(user, novelId, !wiki.auto).catch(e => { console.error(e); return 0; });
    if (n > 0) app.showToast(t('wiki.fromKept', { n }));
    reload();
  };

  const translate = async (quiet = false) => {
    if (translating) return;
    setTranslating(true);
    try {
      const n = await translateWiki(user, novelId, lang);
      if (!quiet) app.showToast(t('wiki.translated', { lang: langName(lang), n }));
      reload();
    } catch (e) {
      console.error(e);
      const f = e instanceof AiFailure ? e : null;
      // Opening the wiki tries once on its own; with no AI set up the button stays.
      if (quiet && f?.key === 'ai.err.notSet') return;
      app.showToast(f ? `${t(f.key)} ${f.key === 'ai.err.generic' || f.key === 'ai.err.http' ? aiErrorDetail(f.message) : ''}` : String(e));
    } finally {
      setTranslating(false);
    }
  };
  // Words written before the wiki was Vietnamese only are translated once on opening.
  const tried = useRef(false);
  useEffect(() => {
    if (missing > 0 && !tried.current) {
      tried.current = true;
      void translate(true);
    }
  }, [missing]); // eslint-disable-line react-hooks/exhaustive-deps

  const card = (e: WikiEntry, extra?: ReactNode) => {
    const label = wikiLabel(e, lang);
    const sum = wikiSummary(e, lang);
    const rels = relCount.get(e.uid) ?? 0;
    const level = wiki && e.kind === 'character' ? levelOf(wiki, e.uid) : null;
    return (
      <div key={e.uid} data-entry={e.name} role={picking ? 'checkbox' : 'button'} aria-checked={picking ? picked.has(e.uid) : undefined} tabIndex={0} className="novel-row"
        style={{ alignItems: 'flex-start', gap: 10, padding: '12px 14px', borderRadius: 22, background: picking && picked.has(e.uid) ? 'color-mix(in srgb, var(--color-accent) 14%, var(--color-surface))' : 'var(--frost)', minWidth: 0, textAlign: 'left', width: '100%' }}
        onClick={() => (picking ? togglePick(e.uid) : setOpen(e))}
        onKeyDown={ev => { if (ev.target === ev.currentTarget && (ev.key === 'Enter' || ev.key === ' ')) { ev.preventDefault(); if (picking) togglePick(e.uid); else setOpen(e); } }}>
        {picking && (
          <span className="check" aria-hidden style={{ flex: 'none', marginTop: 1, pointerEvents: 'none' }}>
            <input type="checkbox" checked={picked.has(e.uid)} readOnly tabIndex={-1} />
            <span className="check__box" style={{ width: 22, height: 22 }}><CheckIcon size={12} /></span>
          </span>
        )}
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
          <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
            <span style={{ fontSize: 15, fontWeight: 700 }}>{label}</span>
            {label !== e.name && <span className="muted" style={{ fontSize: 12 }}>{e.name}</span>}
          </span>
          {sum.text && (
            <Summary text={sum.text}>
              {lang !== RAW && !sum.inLang && <span className="tag tag-neutral" style={{ fontSize: 10, marginRight: 6 }}>{sum.from === RAW ? t('wiki.rawTag') : sum.from.toUpperCase()}</span>}
            </Summary>
          )}
          <span style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginTop: 2 }}>
            <span className="tag tag-neutral" style={{ fontSize: 11 }}>{t(`wiki.kind.${e.kind}`)}</span>
            {e.kind === 'level' && e.meta?.system && <span className="tag tag-neutral" style={{ fontSize: 11 }}>{e.meta.system}</span>}
            {e.kind === 'skill' && e.meta?.grade && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{e.meta.grade}</span>}
            {e.kind === 'level' && e.rank != null && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.rankN', { n: e.rank })}</span>}
            {(level || e.meta?.level) && (
              <span className="tag tag-accent-2" style={{ fontSize: 11 }}>
                {[level ? wikiLabel(level, lang) : e.meta?.level, metaStage(e.meta, n => t('wiki.stageN', { n }))].filter(Boolean).join(', ')}
              </span>
            )}
            {rels > 0 && <span className="tag tag-accent-2" style={{ fontSize: 11 }}>{t('wiki.relCount', { n: rels })}</span>}
            {e.firstChapter != null && <span className="muted" style={{ fontSize: 11 }}>{t('wiki.fromChapter', { n: wikiChapter(e)! })}</span>}
          </span>
          {extra}
        </span>
        {!picking && <ChevronRightIcon size={16} style={{ flex: 'none', marginTop: 4, color: 'var(--color-neutral-500)' }} />}
      </div>
    );
  };

  // One line per entry, for the ladder and the map: its name and, short, what it is.
  const row = (e: WikiEntry, extra?: ReactNode) => {
    const label = wikiLabel(e, lang);
    const sum = wikiSummary(e, lang);
    return (
      <div key={e.uid} data-entry={e.name} role="button" tabIndex={0} className="novel-row" style={{ alignItems: 'center', gap: 10, padding: '9px 12px', borderRadius: 16, background: 'var(--frost)', minWidth: 0, textAlign: 'left', width: '100%' }}
        onClick={() => setOpen(e)} onKeyDown={ev => { if (ev.target === ev.currentTarget && (ev.key === 'Enter' || ev.key === ' ')) { ev.preventDefault(); setOpen(e); } }}>
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ display: 'flex', gap: 6, alignItems: 'baseline', flexWrap: 'wrap' }}>
            <span style={{ fontSize: 14, fontWeight: 700 }}>{label}</span>
            {label !== e.name && <span className="muted" style={{ fontSize: 12 }}>{e.name}</span>}
          </span>
          {sum.text && <span className="muted" style={{ fontSize: 12, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{sum.text}</span>}
          {extra}
        </span>
        <ChevronRightIcon size={14} style={{ flex: 'none', color: 'var(--color-neutral-500)' }} />
      </div>
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
          <button className="btn btn-secondary" data-wiki-smart-merge aria-label={t('wiki.smartMerge')} title={t('wiki.smartMerge')}
            style={{ height: 40, padding: lay.isPhone ? 0 : '0 14px', width: lay.isPhone ? 40 : undefined, gap: 6 }} disabled={!wiki || !wiki.entries.length} onClick={() => setMerging(true)}>
            <SparklesIcon size={14} />{!lay.isPhone && t('wiki.smartMerge')}
          </button>
          <button className="btn btn-primary" style={{ height: 40, padding: '0 16px', gap: 6 }} disabled={!wiki} onClick={() => setOpen('new')}>
            <PencilIcon size={14} />{t('wiki.add')}
          </button>
        </div>

        {q.status === 'failed' && <p role="alert">{t('wiki.failed')} {String(q.error)}</p>}
        {!wiki && q.status === 'loading' && <Spinner size={18} />}
        {wiki && (
          <>
            {missing > 0 && (
              <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap', padding: '12px 14px', borderRadius: 20, background: 'color-mix(in srgb, var(--color-accent) 10%, var(--color-surface))' }}>
                <span style={{ flex: '1 1 200px', fontSize: 13 }}>{t('wiki.missing', { n: missing, lang: langName(lang) })}</span>
                <button className="btn btn-primary" style={{ height: 38, padding: '0 14px', gap: 6 }} disabled={translating} onClick={() => translate()}>
                  {translating ? <Spinner size={13} /> : <SparklesIcon size={14} />}
                  {translating ? t('wiki.translating') : t('wiki.translate', { lang: langName(lang) })}
                </button>
              </div>
            )}

            <div style={{ padding: 14, borderRadius: 24, background: 'var(--frost)' }}>
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

            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <div style={{ flex: 1, minWidth: 0 }}><SearchField value={query} onChange={setQuery} placeholder={t('wiki.search')} /></div>
              {!picking && wiki.entries.length > 0 && (
                <button type="button" className="btn btn-secondary" data-wiki-apply title={t('wiki.applyText')} aria-label={t('wiki.applyText')}
                  style={{ height: 40, padding: lay.isPhone ? 0 : '0 14px', width: lay.isPhone ? 40 : undefined, flex: 'none', gap: 6 }} disabled={applying} onClick={applyText}>
                  {applying ? <Spinner size={13} /> : <SparklesIcon size={14} />}{!lay.isPhone && t('wiki.applyText')}
                </button>
              )}
              {!picking && wiki.entries.length > 0 && (
                <button type="button" className="btn btn-secondary" data-wiki-pick style={{ height: 40, padding: '0 14px', flex: 'none' }} onClick={() => setPicking(true)}>
                  {t('common.pickMany')}
                </button>
              )}
            </div>
            {picking && (
              <div data-wiki-pick-bar role="toolbar" style={{ position: 'sticky', top: 8, zIndex: 2, display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', padding: '10px 12px', borderRadius: 20, background: 'var(--color-surface)', boxShadow: '0 4px 16px rgba(0,0,0,.08)' }}>
                <span style={{ flex: '1 1 auto', fontSize: 13, fontWeight: 700 }}>{t('common.picked', { n: picked.size })}</span>
                <button type="button" className="btn btn-secondary" data-wiki-pick-all style={{ height: 36, padding: '0 12px' }} disabled={!shown?.length} onClick={pickAll}>
                  {t(allPicked ? 'common.unselectAll' : 'common.selectAll')}
                </button>
                <button type="button" className="btn btn-primary" data-wiki-pick-remove style={{ height: 36, padding: '0 12px', gap: 6 }} disabled={!picked.size || removing} onClick={removePicked}>
                  {removing && <Spinner size={12} />}{t('common.deleteN', { n: picked.size })}
                </button>
                <button type="button" className="btn btn-ghost" data-wiki-pick-done style={{ height: 36, padding: '0 12px' }} onClick={stopPicking}>{t('common.done')}</button>
              </div>
            )}

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
                : picking ? list(shown)
                  : !term && kind === 'level' ? <Ladder wiki={wiki} lang={lang} row={row} />
                  : !term && kind === 'place' ? <MapTree wiki={wiki} card={row} />
                    : list(shown)}
          </>
        )}
      </div>
      {merging && (
        <SmartMergeSheet novelId={novelId} lang={lang} onClose={() => setMerging(false)} onMerged={reload} />
      )}
      {open && wiki && (
        <EntrySheet key={open === 'new' ? 'new' : open.uid} novelId={novelId} wiki={wiki} entry={open === 'new' ? null : open} lang={lang}
          onClose={() => setOpen(null)} onChanged={reload} />
      )}
    </div>
  );
}

/**
 * An entry's description, two lines at first. When it runs longer, a "Show more" link
 * opens the whole text in place (and "Show less" folds it back) without opening the entry.
 */
function Summary({ text, children }: { text: string; children?: ReactNode }) {
  const { t } = useI18n();
  const ref = useRef<HTMLSpanElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [long, setLong] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => { if (!expanded) setLong(el.scrollHeight > el.clientHeight + 1); };
    measure();
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    ro?.observe(el);
    return () => ro?.disconnect();
  }, [text, expanded]);
  const clamp = expanded ? {} : { display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' as const, overflow: 'hidden' };
  return (
    <>
      <span ref={ref} data-wiki-summary style={{ fontSize: 13, lineHeight: 1.45, color: 'var(--color-neutral-800)', whiteSpace: 'pre-line', overflowWrap: 'anywhere', ...clamp }}>
        {children}{text}
      </span>
      {(long || expanded) && (
        <button type="button" aria-expanded={expanded} data-wiki-more
          onClick={ev => { ev.stopPropagation(); setExpanded(x => !x); }} onKeyDown={ev => ev.stopPropagation()}
          style={{ alignSelf: 'flex-start', padding: '2px 0', border: 'none', background: 'none', color: 'var(--color-accent-700)', font: 'inherit', fontSize: 12, fontWeight: 700, cursor: 'pointer' }}>
          {t(expanded ? 'wiki.showLess' : 'wiki.showMore')}
        </button>
      )}
    </>
  );
}

/**
 * The story's whole tree of levels: each ladder (cultivation, alchemy…) with its levels
 * in order, lowest first, and what each matches in the other ladders.
 */
function Ladder({ wiki, lang, row }: { wiki: Wiki; lang: string; row: (e: WikiEntry, extra?: ReactNode) => ReactNode }) {
  const { t } = useI18n();
  const levels = wiki.entries.filter(e => e.kind === 'level');
  const systems = [...new Set(levels.map(l => l.meta?.system?.trim() ?? ''))]
    .sort((a, b) => (a ? 0 : 1) - (b ? 0 : 1) || a.localeCompare(b));
  return (
    <div data-wiki-ladder-tree style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {systems.map(sys => {
        const own = levels.filter(l => (l.meta?.system?.trim() ?? '') === sys)
          .sort((a, b) => (a.rank ?? Infinity) - (b.rank ?? Infinity) || a.name.localeCompare(b.name));
        return (
          <section key={sys || '-'} data-ladder={sys}>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 6 }}>
              <span style={{ fontSize: 15, fontWeight: 700 }}>{sys || t('wiki.ladderNoSystem')}</span>
              <span className="muted" style={{ fontSize: 12 }}>{t('wiki.ladderCount', { n: own.length })}</span>
            </div>
            <ol data-wiki-ladder style={{ listStyle: 'none', margin: 0, padding: '0 0 0 14px', borderLeft: '2px solid var(--color-neutral-200)', display: 'flex', flexDirection: 'column', gap: 6 }}>
              {own.map((l, i) => {
                const eq = equivalentsOf(wiki, l.uid);
                return (
                  <li key={l.uid} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span className="tag tag-neutral" style={{ fontSize: 11, flex: 'none', minWidth: 26, justifyContent: 'center' }}>{l.rank ?? i + 1}</span>
                    {row(l, eq.length > 0 && (
                      <span data-wiki-equals style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginTop: 2 }}>
                        {eq.map(e => (
                          <span key={e.uid} className="chip chip--sm" style={{ fontSize: 11, pointerEvents: 'none' }}>
                            ≈ {wikiLabel(e, lang)}{e.meta?.system ? ` · ${e.meta.system}` : ''}
                          </span>
                        ))}
                      </span>
                    ))}
                  </li>
                );
              })}
            </ol>
          </section>
        );
      })}
    </div>
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
  const [system, setSystem] = useState(entry?.meta?.system ?? '');
  const [curLevel, setCurLevel] = useState(entry?.meta?.level ?? '');
  const [stageName, setStageName] = useState(entry?.meta?.stageName ?? '');
  const [skill, setSkill] = useState<Record<SkillField, string>>(() =>
    Object.fromEntries(SKILL_FIELDS.map(f => [f, entry?.meta?.[f] ?? ''])) as Record<SkillField, string>);
  const [profile, setProfile] = useState<Record<ProfileField, string>>(() =>
    Object.fromEntries(PROFILE_FIELDS.map(f => [f, entry?.meta?.[f] ?? ''])) as Record<ProfileField, string>);
  const [aliases, setAliases] = useState(entry?.aliases.join(', ') ?? '');
  // The wiki's words are Vietnamese; the original name stays in `name` and the aliases.
  const editLang = WIKI_LANG;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [relTo, setRelTo] = useState('');
  const [relType, setRelType] = useState<string>(REL_TYPES[0]);
  const [relWordsIn, setRelWordsIn] = useState('');
  const [relNote, setRelNote] = useState('');
  const [mergeInto, setMergeInto] = useState('');
  const [smart, setSmart] = useState(false);
  const [retr, setRetr] = useState(false);
  // A name kept while translating again: the wiki loads again when the sheet closes, not under it.
  const retrDirty = useRef(false);
  const [rels, setRels] = useState(() => wiki.relations.filter(r => entry && (r.fromUid === entry.uid || r.toUid === entry.uid)));

  const byUid = useMemo(() => new Map(wiki.entries.map(e => [e.uid, e])), [wiki]);
  const others = wiki.entries.filter(e => e.uid !== saved?.uid);
  const show = (uid: string) => { const e = byUid.get(uid); return e ? wikiLabel(e, lang) : '?'; };
  const langName = (l: string) => (l === RAW ? t('reader.raw') : AI_LANGS.find(x => x.code === l)?.native ?? l);
  const wordsLang = WIKI_LANG;

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
        meta: {
          ...saved?.meta, system: system.trim(), level: curLevel.trim(), stageName: stageName.trim(),
          ...Object.fromEntries(PROFILE_FIELDS.map(f => [f, profile[f].trim()])),
          ...Object.fromEntries(SKILL_FIELDS.map(f => [f, skill[f].trim()]))
        },
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
      const note = relNote.trim();
      const uid = await addWikiRelation(user, novelId, saved.uid, relTo, relKind, wordsLang, note);
      setRels(r => [...r.filter(x => x.uid !== uid), {
        uid, fromUid: saved.uid, toUid: relTo, kind: relKind, labels: custom ? { [wordsLang]: relKind } : {}, note, firstChapter: null
      }]);
      setRelWordsIn('');
      setRelNote('');
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

  // This entry is another one written differently ("bé áo xanh" and her name): it goes into that one.
  const merge = async () => {
    const into = byUid.get(mergeInto);
    if (!saved || !into || !(await ask(t('wiki.mergeAsk', { from: wikiLabel(saved, lang), into: wikiLabel(into, lang) }), { kind: 'warning' }))) return;
    try {
      await mergeWikiEntry(user, novelId, saved.uid, into.uid);
      onChanged();
      app.showToast(t('wiki.merged', { into: wikiLabel(into, lang) }));
      onClose();
    } catch (e) {
      failed(e);
    }
  };

  const level = saved && saved.kind === 'character' ? levelOf(wiki, saved.uid) : null;

  return (
    smart && saved ? (
      <SmartMergeSheet novelId={novelId} lang={lang} entry={saved} onClose={() => setSmart(false)}
        onMerged={() => { onChanged(); onClose(); }} />
    ) : retr && saved ? (
      <RetranslateSheet novelId={novelId} lang={lang} target={{ uid: saved.uid, label: wikiLabel(saved, lang) }}
        onClose={() => { setRetr(false); if (retrDirty.current) onChanged(); onClose(); }} onDone={() => { retrDirty.current = true; }} />
    ) : <Modal onClose={busy ? undefined : onClose}>
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
        {saved && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {/* Step 1: this name looked for in the book; a merge waits for the user's tap. */}
            <EntryMergeCheck novelId={novelId} entry={saved} lang={lang} onDetails={() => setSmart(true)}
              onMerged={() => { onChanged(); onClose(); }} />
            <button type="button" className="btn btn-secondary" data-entry-retranslate style={{ height: 34, padding: '0 12px', gap: 6, fontSize: 13, alignSelf: 'flex-start' }}
              disabled={busy} onClick={() => setRetr(true)}>
              <LanguagesIcon size={13} />{t('wiki.retranslate')}
            </button>
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
        {kind === 'level' && (
          <div className="field">
            <label htmlFor="wiki-rank">{t('wiki.rank')}</label>
            <input id="wiki-rank" className="input" style={{ height: 42 }} inputMode="numeric" value={rank} onChange={e => setRank(e.target.value.replace(/[^\d-]/g, ''))} />
          </div>
        )}
        {kind === 'level' && (
          <div className="field">
            <label htmlFor="wiki-system">{t('wiki.system')}</label>
            <input id="wiki-system" className="input" style={{ height: 42 }} value={system} placeholder={t('wiki.systemPh')} onChange={e => setSystem(e.target.value)} />
          </div>
        )}
        <div className="field">
          <label htmlFor="wiki-aliases">{t('wiki.aliases')}</label>
          <input id="wiki-aliases" className="input" style={{ height: 42 }} value={aliases} onChange={e => setAliases(e.target.value)} />
        </div>

        <div className="field">
          <label htmlFor="wiki-name-in">{t('wiki.nameIn', { lang: langName(editLang) })}</label>
          <input id="wiki-name-in" className="input" style={{ height: 42 }} value={names[editLang] ?? ''}
            onChange={e => setNames(n => ({ ...n, [editLang]: e.target.value }))} />
        </div>
        <div className="field">
          <label htmlFor="wiki-summary">{t('wiki.summaryIn', { lang: langName(editLang) })}</label>
          <textarea id="wiki-summary" className="input" rows={8} style={{ borderRadius: 20, resize: 'vertical', minHeight: 140 }} value={summaries[editLang] ?? ''}
            placeholder={wikiSummary({ ...(saved ?? entryStub), summaries }, editLang).text}
            onChange={e => setSummaries(s => ({ ...s, [editLang]: e.target.value }))} />
        </div>

        {kind === 'skill' && (
          <section data-wiki-skill-edit style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.skill')}</div>
            {SKILL_FIELDS.map(f => (
              <div key={f} className="field">
                <label htmlFor={`wiki-${f}`}>{t(`wiki.skill.${f}`)}</label>
                <input id={`wiki-${f}`} className="input" style={{ height: 42 }} value={skill[f]} placeholder={t(`wiki.skill.${f}Ph`)}
                  onChange={e => setSkill(p => ({ ...p, [f]: e.target.value }))} />
              </div>
            ))}
          </section>
        )}
        {kind === 'character' && (
          <section data-wiki-profile-edit style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.profile')}</div>
            {PROFILE_FIELDS.map(f => (
              <div key={f} className="field">
                <label htmlFor={`wiki-${f}`}>{t(`wiki.profile.${f}`)}</label>
                {f === 'age'
                  ? <input id={`wiki-${f}`} className="input" style={{ height: 42 }} value={profile[f]} placeholder={t(`wiki.profile.${f}Ph`)}
                      onChange={e => setProfile(p => ({ ...p, [f]: e.target.value }))} />
                  : <textarea id={`wiki-${f}`} className="input" rows={2} style={{ borderRadius: 20, resize: 'vertical', minHeight: 64 }} value={profile[f]} placeholder={t(`wiki.profile.${f}Ph`)}
                      onChange={e => setProfile(p => ({ ...p, [f]: e.target.value }))} />}
              </div>
            ))}
            <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
              <div className="field" style={{ flex: '2 1 180px' }}>
                <label htmlFor="wiki-cur-level">{t('wiki.profile.level')}</label>
                <input id="wiki-cur-level" className="input" style={{ height: 42 }} value={curLevel} onChange={e => setCurLevel(e.target.value)} />
              </div>
              <div className="field" style={{ flex: '1 1 120px' }}>
                <label htmlFor="wiki-stage">{t('wiki.stage')}</label>
                <input id="wiki-stage" className="input" style={{ height: 42 }} value={stageName} placeholder={t('wiki.stagePh')} onChange={e => setStageName(e.target.value)} />
              </div>
            </div>
          </section>
        )}

        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 12, fontWeight: 700 }}>{t('wiki.relations')}</div>
          {!saved ? <div className="muted" style={{ fontSize: 12 }}>{t('wiki.relSaveFirst')}</div> : (
            <>
              {rels.length === 0 && <div className="muted" style={{ fontSize: 12 }}>{t('wiki.noRelations')}</div>}
              <GroupedRelations wiki={wiki} uid={saved.uid} rels={rels} profiled={false} titled={false} row={r => {
                const other = r.fromUid === saved.uid ? r.toUid : r.fromUid;
                return (
                  <div key={r.uid} data-relation={r.kind} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 13, padding: '6px 6px 6px 12px', borderRadius: 16, background: 'var(--color-bg)' }}>
                    <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
                      <span>{relWords(r, saved.uid, lang, wt)} <b>{show(other)}</b></span>
                      {r.note && <span className="muted" data-relation-note style={{ fontSize: 12 }}>{r.note}</span>}
                    </span>
                    <button type="button" className="icon-btn" style={{ width: 30, height: 30 }} aria-label={t('wiki.remove')} onClick={() => dropRel(r.uid)}><XIcon size={14} /></button>
                  </div>
                );
              }} />
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
                  <input className="input" style={{ height: 38, flex: '1 1 100%' }} placeholder={t('wiki.relNotePh')} aria-label={t('wiki.relNote')}
                    value={relNote} onChange={e => setRelNote(e.target.value)} />
                  <button type="button" className="btn btn-secondary" style={{ height: 38, padding: '0 14px' }} disabled={!relTo || !relKind} onClick={addRel}>{t('wiki.relAdd')}</button>
                </div>
              )}
            </>
          )}
        </div>

        {saved && others.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <div style={{ fontSize: 12, fontWeight: 700, flex: 1 }}>{t('wiki.mergeTitle')}</div>
              <button type="button" className="btn btn-secondary" data-entry-smart-merge style={{ height: 34, padding: '0 12px', gap: 6, fontSize: 13 }} disabled={busy} onClick={() => setSmart(true)}>
                <SparklesIcon size={13} />{t('wiki.smartMerge')}
              </button>
            </div>
            <div className="muted" style={{ fontSize: 12 }}>{t('wiki.mergeSub')}</div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              <select className="input" style={{ height: 38, flex: '1 1 180px' }} aria-label={t('wiki.mergeInto')} value={mergeInto} onChange={e => setMergeInto(e.target.value)}>
                <option value="">{t('wiki.mergeInto')}…</option>
                {others.filter(e => e.kind === saved.kind).concat(others.filter(e => e.kind !== saved.kind))
                  .map(e => <option key={e.uid} value={e.uid}>{wikiLabel(e, lang)}{e.kind !== saved.kind ? ` · ${t(`wiki.kind.${e.kind}`)}` : ''}</option>)}
              </select>
              <button type="button" className="btn btn-secondary" style={{ height: 38, padding: '0 14px' }} disabled={!mergeInto || busy} onClick={merge}>{t('wiki.merge')}</button>
            </div>
          </div>
        )}

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
