import { useState } from 'react';
import { ChevronLeftIcon, ChevronRightIcon, HeartIcon, LanguagesIcon, PlayIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { ShelfItem, Spinner, formatReads } from '../components/ui';
import { getChapterList, getContentLangs, getNovel, sameAuthor, sameCategory } from '../data/repository';
import { useAsync } from '../lib/useAsync';
import type { Novel } from '../data/types';
import { useI18n } from '../i18n';
import { useApp } from '../store/AppStore';
import { useCatalogNames } from '../data/useCatalogNames';
import { useLayout } from '../lib/useLayout';

const MAX_LISTED_CHAPTERS = 20;
const WORDS_PER_MIN = 220;

export function DetailScreen({ id }: { id: number }) {
  const { t, uiLang } = useI18n();
  const names = useCatalogNames();
  const app = useApp();
  const lay = useLayout();
  const d = getNovel(id);
  const [allCh, setAllCh] = useState(false);
  const chList = useAsync(() => getChapterList(id, app.contentLang), [id, app.contentLang]);
  const chapters = chList.status === 'ready' ? chList.data : [];

  const readUpTo = app.history.find(h => h.id === id)?.ch ?? 0;
  const isFav = app.favs.includes(id);
  const related = sameCategory(d);
  const byAuthor = sameAuthor(d);
  const shown = chapters.slice(0, allCh ? MAX_LISTED_CHAPTERS : lay.chPreview);
  const langLabel = getContentLangs().find(l => l.code === app.contentLang)?.native ?? t('lang.askEach');

  const toggleFav = () => app.showToast(t(app.toggleFav(id) ? 'detail.saved' : 'detail.removed'));

  const startBtn = (
    <button className={`btn btn-primary${lay.wide ? ' btn-block' : ''}`} style={{ height: 52, fontSize: 16, gap: 10, marginTop: lay.wide ? 16 : 0 }} onClick={() => app.openChapter(id, readUpTo || 1)}>
      <PlayIcon size={16} />{readUpTo ? t('detail.continue', { n: readUpTo }) : t('detail.start')}
    </button>
  );

  return (
    <div className="screen" style={{ paddingBottom: 'calc(var(--nav-h) + 32px)' }}>
      <div style={{ display: 'grid', gridTemplateColumns: lay.detailCols, gap: lay.detailGap, padding: lay.wide ? `${lay.v(0, 44, 32)}px ${lay.px}px 0` : 0, alignItems: 'start' }}>
        <div style={{
          position: lay.wide ? 'sticky' : 'relative', top: lay.wide ? lay.v(0, 44, 32) : 'auto', background: 'var(--color-surface)',
          borderRadius: lay.wide ? 34 : '0 0 44px 44px', padding: lay.wide ? 20 : 'calc(var(--safe-top) + 12px) 20px 24px'
        }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 18 }}>
            <button className="icon-btn icon-btn--bg" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon /></button>
            <button className="icon-btn icon-btn--bg" aria-label={t('detail.favourite')} aria-pressed={isFav} onClick={toggleFav}
              style={{ color: isFav ? 'var(--color-accent)' : 'var(--color-text)' }}>
              <HeartIcon fill={isFav ? 'currentColor' : 'none'} />
            </button>
          </div>
          <div style={{ display: 'flex', flexDirection: lay.wide ? 'column' : 'row', gap: 18, alignItems: lay.wide ? 'stretch' : 'flex-end' }}>
            {lay.wide
              ? <NovelCover novel={d} w="100%" h="auto" fs={lay.dCoverFs} style={{ maxWidth: lay.dCoverMax, aspectRatio: 0.7, boxShadow: 'var(--shadow-md)' }} />
              : <NovelCover novel={d} w={124} h={178} fs={18} style={{ boxShadow: 'var(--shadow-md)' }} />}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                <button className="tag tag-accent-2" style={{ border: 'none', cursor: 'pointer', font: 'inherit', fontSize: 11 }} onClick={() => app.openCategory(d.cat)}>{names.cat(d.cat)}</button>
                <span className="tag tag-neutral">{t(`status.${d.status}`)}</span>
              </div>
              <h3 style={{ margin: 0, fontSize: lay.dTitleFs }}>{d.title}</h3>
              <button className="text-link" style={{ fontSize: 14 }} onClick={() => app.openAuthor(d.author)}>{d.author} ›</button>
            </div>
          </div>
          <div style={{ display: 'flex', gap: 10, marginTop: 20 }}>
            <Stat value={`★ ${d.rating}`} label={t('detail.rating')} />
            <Stat value={d.chapters} label={t('detail.chapters')} />
            <Stat value={formatReads(d.reads)} label={t('detail.reads')} />
          </div>
          {lay.wide && startBtn}
        </div>

        <div style={{ padding: lay.wide ? 0 : '20px 20px 0', display: 'flex', flexDirection: 'column', gap: lay.wide ? 22 : 20, minWidth: 0 }}>
          {lay.isPhone && startBtn}

          <div>
            <h5 className="section-title" style={{ marginBottom: 6 }}>{t('detail.synopsis')}</h5>
            <p style={{ margin: 0, fontSize: lay.synFs, lineHeight: 1.6, color: 'var(--color-neutral-800)', textWrap: 'pretty', maxWidth: 680 }}>{d.desc[uiLang]}</p>
          </div>

          <div>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
              <h5 className="section-title">{t('detail.chapters')}</h5>
              <button className="tag tag-outline" style={{ background: 'none', cursor: 'pointer', font: 'inherit', fontSize: 12, gap: 6, padding: '5px 12px' }}
                onClick={() => app.setLangDialog({ mode: 'settings' })}>
                <LanguagesIcon size={13} />{langLabel}
              </button>
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: lay.chCols, gap: '2px 12px' }}>
              {chList.status === 'loading' && <div style={{ padding: 12 }}><Spinner size={18} /></div>}
              {shown.map(c => {
                const n = c.number, read = n <= readUpTo;
                return (
                  <button key={n} className="novel-row" style={{ alignItems: 'center', gap: 12, padding: '10px 8px', borderRadius: 20, minWidth: 0 }} onClick={() => app.openChapter(id, n)}>
                    <span style={{ width: 36, height: 36, flex: 'none', borderRadius: '50%', display: 'grid', placeItems: 'center', fontSize: 12, fontWeight: 700, background: read ? 'var(--color-accent-2-200)' : 'var(--color-surface)', color: read ? 'var(--color-accent-2-800)' : 'var(--color-text)' }}>{n}</span>
                    <span style={{ flex: 1, minWidth: 0 }}>
                      <span style={{ display: 'block', fontSize: 14, fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.title}</span>
                      <span className="muted" style={{ fontSize: 11 }}>
                        {read ? t(n === readUpTo ? 'detail.lastRead' : 'detail.read') : t('detail.minRead', { n: Math.max(1, Math.round(c.words / WORDS_PER_MIN)) })}
                      </span>
                    </span>
                    <ChevronRightIcon size={16} style={{ flex: 'none', color: 'var(--color-neutral-500)' }} />
                  </button>
                );
              })}
            </div>
            {chapters.length > lay.chPreview && (
              <button className="btn btn-secondary btn-block" style={{ marginTop: 6 }} onClick={() => setAllCh(a => !a)}>
                {allCh ? t('detail.showFewer') : t('detail.showMore', { n: chapters.length })}
              </button>
            )}
          </div>

          <RelatedShelf title={t('detail.moreIn', { cat: names.cat(d.cat) })} novels={related} onSeeAll={() => app.openCategory(d.cat)} />
          <RelatedShelf title={t('detail.moreBy', { author: d.author })} novels={byAuthor} onSeeAll={() => app.openAuthor(d.author)} />
        </div>
      </div>
    </div>
  );
}

function Stat({ value, label }: { value: React.ReactNode; label: string }) {
  return (
    <div style={{ flex: 1, background: 'var(--color-bg)', borderRadius: 20, padding: '10px 12px' }}>
      <div className="display" style={{ fontSize: 18 }}>{value}</div>
      <div className="muted" style={{ fontSize: 11 }}>{label}</div>
    </div>
  );
}

function RelatedShelf({ title, novels, onSeeAll }: { title: string; novels: Novel[]; onSeeAll: () => void }) {
  const { t } = useI18n();
  const app = useApp();
  const lay = useLayout();
  if (!novels.length) return null;
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
        <h5 className="section-title">{title}</h5>
        <button className="btn btn-ghost" style={{ fontSize: 13 }} onClick={onSeeAll}>{t('common.seeAll')}</button>
      </div>
      <div className="shelf nx-scroll" style={{ gap: lay.wide ? lay.colGap : 12, margin: lay.wide ? 0 : '0 -20px', padding: lay.wide ? '0 0 4px' : undefined, scrollSnapType: 'none' }}>
        {novels.map(n => <ShelfItem key={n.id} novel={n} w={lay.relW} h={Math.round(lay.relW / 0.7)} fs={lay.relFs} titleSize={12} showAuthor={false} onOpen={() => app.openDetail(n.id)} />)}
      </div>
    </div>
  );
}
