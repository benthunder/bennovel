import { useState } from 'react';
import { ChevronLeftIcon, ChevronRightIcon, HeartIcon, LanguagesIcon, PlayIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { ShelfItem, formatReads } from '../components/ui';
import { CHAPTER_TITLES, CONTENT_LANGS } from '../data/mock';
import { getNovel, sameAuthor, sameCategory } from '../data/repository';
import type { Novel } from '../data/types';
import { useI18n } from '../i18n';
import { useApp } from '../store/AppStore';

const PREVIEW_CHAPTERS = 6;
const MAX_LISTED_CHAPTERS = 20;

export function DetailScreen({ id }: { id: number }) {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const d = getNovel(id);
  const [allCh, setAllCh] = useState(false);

  const readUpTo = app.history.find(h => h.id === id)?.ch ?? 0;
  const isFav = app.favs.includes(id);
  const related = sameCategory(d);
  const byAuthor = sameAuthor(d);
  const shown = allCh ? Math.min(MAX_LISTED_CHAPTERS, d.chapters) : Math.min(PREVIEW_CHAPTERS, d.chapters);
  const langLabel = CONTENT_LANGS.find(l => l.code === app.contentLang)?.native ?? t('lang.askEach');

  const toggleFav = () => app.showToast(t(app.toggleFav(id) ? 'detail.saved' : 'detail.removed'));

  return (
    <div className="screen" style={{ paddingBottom: 'calc(var(--nav-h) + 32px)' }}>
      <div style={{ background: 'var(--color-surface)', borderRadius: '0 0 44px 44px', padding: 'calc(var(--safe-top) + 12px) 20px 24px' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 18 }}>
          <button className="icon-btn icon-btn--bg" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon /></button>
          <button className="icon-btn icon-btn--bg" aria-label={t('detail.favourite')} aria-pressed={isFav} onClick={toggleFav}
            style={{ color: isFav ? 'var(--color-accent)' : 'var(--color-text)' }}>
            <HeartIcon fill={isFav ? 'currentColor' : 'none'} />
          </button>
        </div>
        <div style={{ display: 'flex', gap: 18, alignItems: 'flex-end' }}>
          <NovelCover novel={d} w={124} h={178} fs={18} style={{ boxShadow: 'var(--shadow-md)' }} />
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              <button className="tag tag-accent-2" style={{ border: 'none', cursor: 'pointer', font: 'inherit', fontSize: 11 }} onClick={() => app.openCategory(d.cat)}>{t(`cat.${d.cat}`)}</button>
              <span className="tag tag-neutral">{t(`status.${d.status}`)}</span>
            </div>
            <h3 style={{ margin: 0, fontSize: 24 }}>{d.title}</h3>
            <button className="text-link" style={{ fontSize: 14 }} onClick={() => app.openAuthor(d.author)}>{d.author} ›</button>
          </div>
        </div>
        <div style={{ display: 'flex', gap: 10, marginTop: 20 }}>
          <Stat value={`★ ${d.rating}`} label={t('detail.rating')} />
          <Stat value={d.chapters} label={t('detail.chapters')} />
          <Stat value={formatReads(d.reads)} label={t('detail.reads')} />
        </div>
      </div>

      <div style={{ padding: '20px 20px 0', display: 'flex', flexDirection: 'column', gap: 20 }}>
        <button className="btn btn-primary" style={{ height: 52, fontSize: 16, gap: 10 }} onClick={() => app.openChapter(id, readUpTo || 1)}>
          <PlayIcon size={16} />{readUpTo ? t('detail.continue', { n: readUpTo }) : t('detail.start')}
        </button>

        <div>
          <h5 className="section-title" style={{ marginBottom: 6 }}>{t('detail.synopsis')}</h5>
          <p style={{ margin: 0, fontSize: 14, lineHeight: 1.6, color: 'var(--color-neutral-800)', textWrap: 'pretty' }}>{d.desc[uiLang]}</p>
        </div>

        <div>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
            <h5 className="section-title">{t('detail.chapters')}</h5>
            <button className="tag tag-outline" style={{ background: 'none', cursor: 'pointer', font: 'inherit', fontSize: 12, gap: 6, padding: '5px 12px' }}
              onClick={() => app.setLangDialog({ mode: 'settings' })}>
              <LanguagesIcon size={13} />{langLabel}
            </button>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {Array.from({ length: shown }, (_, i) => {
              const n = i + 1, read = n <= readUpTo;
              return (
                <button key={n} className="novel-row" style={{ alignItems: 'center', gap: 12, padding: '10px 8px', borderRadius: 20 }} onClick={() => app.openChapter(id, n)}>
                  <span style={{ width: 36, height: 36, flex: 'none', borderRadius: '50%', display: 'grid', placeItems: 'center', fontSize: 12, fontWeight: 700, background: read ? 'var(--color-accent-2-200)' : 'var(--color-surface)', color: read ? 'var(--color-accent-2-800)' : 'var(--color-text)' }}>{n}</span>
                  <span style={{ flex: 1, minWidth: 0 }}>
                    <span style={{ display: 'block', fontSize: 14, fontWeight: 600 }}>{CHAPTER_TITLES[i % CHAPTER_TITLES.length]}</span>
                    <span className="muted" style={{ fontSize: 11 }}>
                      {read ? t(n === readUpTo ? 'detail.lastRead' : 'detail.read') : t('detail.minRead', { n: 8 + ((n * 7) % 9) })}
                    </span>
                  </span>
                  <ChevronRightIcon size={16} style={{ color: 'var(--color-neutral-500)' }} />
                </button>
              );
            })}
          </div>
          {d.chapters > PREVIEW_CHAPTERS && (
            <button className="btn btn-secondary btn-block" onClick={() => setAllCh(a => !a)}>
              {allCh ? t('detail.showFewer') : t('detail.showMore', { n: d.chapters })}
            </button>
          )}
        </div>

        <RelatedShelf title={t('detail.moreIn', { cat: t(`cat.${d.cat}`) })} novels={related} onSeeAll={() => app.openCategory(d.cat)} />
        <RelatedShelf title={t('detail.moreBy', { author: d.author })} novels={byAuthor} onSeeAll={() => app.openAuthor(d.author)} />
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
  if (!novels.length) return null;
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
        <h5 className="section-title">{title}</h5>
        <button className="btn btn-ghost" style={{ fontSize: 13 }} onClick={onSeeAll}>{t('common.seeAll')}</button>
      </div>
      <div className="shelf nx-scroll" style={{ gap: 12, margin: '0 -20px', scrollSnapType: 'none' }}>
        {novels.map(n => <ShelfItem key={n.id} novel={n} w={96} h={138} fs={13} titleSize={12} showAuthor={false} onOpen={() => app.openDetail(n.id)} />)}
      </div>
    </div>
  );
}
