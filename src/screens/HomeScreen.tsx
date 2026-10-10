import { useEffect, useRef, useState } from 'react';
import { NovelCover } from '../components/NovelCover';
import { OfflineNote } from '../components/OfflineNote';
import { useCatalog } from '../data/useCatalog';
import { NovelRow, SearchField, ShelfItem, Spinner, formatReads } from '../components/ui';
import { getCollections, getPopular, getTopRated, searchNovels } from '../data/repository';
import { useI18n } from '../i18n';
import { useThrottledValue } from '../lib/useThrottledValue';
import { SEARCH_THROTTLE_MS, useApp } from '../store/AppStore';
import { useCatalogNames } from '../data/useCatalogNames';
import { ChevronLeftIcon, ChevronRightIcon } from '../components/Icons';
import { useLayout } from '../lib/useLayout';

const AUTOPLAY_MS = 4200;
/** Autoplay pauses this long after the user swipes or taps a dot. */
const TOUCH_PAUSE_MS = 6000;

export function HomeScreen() {
  const { t } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const [input, setInput] = useState('');
  const query = useThrottledValue(input, SEARCH_THROTTLE_MS);
  const pending = input !== query;
  const results = searchNovels(query);
  const offline = useCatalog().status === 'offline';

  const hr = new Date().getHours();
  const greeting = t(hr < 12 ? 'home.morning' : hr < 18 ? 'home.afternoon' : 'home.evening');
  const firstName = app.user ? app.user.name.split(' ')[0] : t('home.reader');
  const initials = app.user ? app.user.name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G';

  return (
    <div className="screen screen--tabbed" style={{ paddingTop: lay.top }}>
      <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: lay.v('16px 12px', '16px 20px', '16px 24px'), padding: `0 ${lay.px}px` }}>
        <div style={{ flex: lay.portrait ? '1 1 100%' : '1 1 auto', minWidth: 0 }}>
          <div className="muted" style={{ fontSize: 13 }}>{greeting}</div>
          <div className="display" style={{ fontSize: lay.homeH, lineHeight: 1.1 }}>{t('home.whatsNext', { name: firstName })}</div>
        </div>
        <div style={{ width: lay.searchW, order: 2 }}>
          <SearchField value={input} onChange={setInput} placeholder={t('home.searchPlaceholder')} />
        </div>
        {/* Phones already have Profile in the bottom bar. */}
        {!lay.isPhone && (
          <button className="display" aria-label={t('nav.profile')} onClick={() => app.switchTab('profile')}
            style={{ order: 3, width: 44, height: 44, flex: 'none', borderRadius: '50%', border: 'none', background: 'var(--color-accent-2-300)', color: 'var(--color-accent-2-900)', fontSize: 16, cursor: 'pointer' }}>
            {initials}
          </button>
        )}
      </div>

      {offline ? (
        <div style={{ padding: `20px ${lay.px}px 0` }}><OfflineNote showLibrary /></div>
      ) : input ? (
        <div style={{ padding: `12px ${lay.px}px 0` }}>
          <div className="status-line" style={{ padding: '0 4px 6px' }}>
            {pending && <Spinner />}
            <span>{pending ? t('common.searching') : results.length === 1 ? t('home.result', { q: query }) : t('home.results', { n: results.length, q: query })}</span>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: lay.resultCols, gap: '4px 12px' }}>
            {results.map(n => <NovelRow key={n.id} novel={n} onOpen={() => app.openDetail(n.id)} />)}
          </div>
          {!pending && query && results.length === 0 && (
            <div className="empty"><div className="empty__title">{t('home.emptyTitle')}</div><div className="empty__body muted">{t('home.emptyBody')}</div></div>
          )}
        </div>
      ) : (
        <>
          <div style={{ display: 'grid', gridTemplateColumns: lay.showTop ? 'minmax(0,1fr) 320px' : 'minmax(0,1fr)', gap: 28, padding: lay.wide ? `0 ${lay.px}px` : 0, alignItems: 'stretch' }}>
            <HeroSlider />
            {lay.showTop && <TopRated />}
          </div>
          {getCollections().map(c => <CollectionShelf key={c.key} c={c} />)}
        </>
      )}
    </div>
  );
}

function CollectionShelf({ c }: { c: ReturnType<typeof getCollections>[number] }) {
  const { t } = useI18n();
  const names = useCatalogNames();
  const app = useApp();
  const lay = useLayout();
  const rowRef = useRef<HTMLDivElement>(null);
  const page = (dir: number) => rowRef.current?.scrollBy({ left: dir * rowRef.current.clientWidth * 0.8 });

  return (
    <section style={{ paddingTop: 28 }}>
      <div className="section-head" style={{ padding: `0 ${lay.px}px 12px` }}>
        <div>
          <div className="kicker">{names.coll(c.key).kicker}</div>
          <h4 style={{ margin: 0 }}>{names.coll(c.key).title}</h4>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {lay.wide && (
            <>
              <button className="btn btn-icon btn-secondary" style={{ width: 36, height: 36 }} aria-label={t('home.prev')} onClick={() => page(-1)}><ChevronLeftIcon size={16} /></button>
              <button className="btn btn-icon btn-secondary" style={{ width: 36, height: 36 }} aria-label={t('home.next')} onClick={() => page(1)}><ChevronRightIcon size={16} /></button>
            </>
          )}
          <button className="btn btn-ghost" style={{ fontSize: 13 }} onClick={() => app.openCollection(c.key)}>{t('common.seeAll')}</button>
        </div>
      </div>
      <div ref={rowRef} className="shelf nx-scroll" style={{ gap: lay.colGap, padding: `0 ${lay.px}px 4px`, scrollPadding: lay.px, scrollBehavior: 'smooth' }}>
        {c.items.map(n => <ShelfItem key={n.id} novel={n} w={lay.colW} h={Math.round(lay.colW / 0.7)} fs={lay.colFs} onOpen={() => app.openDetail(n.id)} />)}
      </div>
    </section>
  );
}

/** Tablet landscape and desktop: ranked list next to the hero slider. */
function TopRated() {
  const topRated = getTopRated(5);
  const { t } = useI18n();
  const app = useApp();
  return (
    <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column' }}>
      <div className="section-head" style={{ alignItems: 'baseline', padding: '28px 0 12px' }}>
        <h4 style={{ margin: 0 }}>{t('home.topRated')}</h4>
        <button className="btn btn-ghost" style={{ fontSize: 13, padding: '2px 8px' }} onClick={() => app.switchTab('category')}>{t('home.browse')}</button>
      </div>
      <div style={{ flex: 1, background: 'var(--color-surface)', borderRadius: 34, padding: 10, display: 'flex', flexDirection: 'column', gap: 2 }}>
        {topRated.map((n, i) => (
          <button key={n.id} className="top-row" onClick={() => app.openDetail(n.id)}>
            <span className="display" style={{ width: 22, fontSize: 20, fontWeight: 400, color: 'var(--color-accent-700)' }}>{i + 1}</span>
            <NovelCover novel={n} w={36} h={51} fs={6} />
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ display: 'block', fontWeight: 700, fontSize: 13, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{n.title}</span>
              <span className="muted" style={{ fontSize: 11 }}>{n.author}</span>
            </span>
            <span className="rating" style={{ fontSize: 12 }}>★ {n.rating}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function HeroSlider() {
  const { t, uiLang } = useI18n();
  const names = useCatalogNames();
  const app = useApp();
  const lay = useLayout();
  const [idx, setIdx] = useState(0);
  const touchedAt = useRef(0);
  const dragX = useRef<number | null>(null);
  const suppressClick = useRef(false);
  const [popular] = useState(getPopular);
  const count = popular.length;

  useEffect(() => {
    const timer = setInterval(() => {
      if (Date.now() - touchedAt.current < TOUCH_PAUSE_MS) return;
      setIdx(i => (i + 1) % count);
    }, AUTOPLAY_MS);
    return () => clearInterval(timer);
  }, [count]);

  const goTo = (i: number) => { touchedAt.current = Date.now(); setIdx((i + count) % count); };

  const onPointerUp = (e: React.PointerEvent) => {
    const dx = e.clientX - (dragX.current ?? e.clientX);
    dragX.current = null;
    if (Math.abs(dx) <= 40) return;
    suppressClick.current = true;
    setTimeout(() => (suppressClick.current = false), 50);
    goTo(idx + (dx < 0 ? 1 : -1));
  };

  return (
    <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column' }}>
      <div className="section-head" style={{ alignItems: 'baseline', padding: lay.wide ? '28px 0 12px' : '24px 20px 12px' }}>
        <h4 style={{ margin: 0 }}>{t('home.popular')}</h4>
        <span className="muted" style={{ fontSize: 12 }}>{idx + 1} / {count}</span>
      </div>
      <div style={{ overflow: 'hidden', touchAction: 'pan-y', flex: 1, display: 'flex' }} onPointerDown={e => (dragX.current = e.clientX)} onPointerUp={onPointerUp}>
        <div style={{ display: 'flex', width: '100%', transition: 'transform .55s cubic-bezier(.2,.8,.2,1)', transform: `translateX(-${idx * 100}%)` }}>
          {popular.map((n, i) => (
            <div key={n.id} style={{ flex: '0 0 100%', padding: lay.wide ? 0 : '0 20px', boxSizing: 'border-box', display: 'flex' }} aria-hidden={i !== idx}>
              <div role="button" tabIndex={i === idx ? 0 : -1} onClick={() => !suppressClick.current && app.openDetail(n.id)}
                onKeyDown={e => e.key === 'Enter' && app.openDetail(n.id)}
                style={{ position: 'relative', flex: 1, overflow: 'hidden', borderRadius: 34, background: 'var(--color-surface)', padding: lay.heroCardPad, display: 'flex', gap: lay.heroCardGap, cursor: 'pointer', userSelect: 'none' }}>
                <NovelCover novel={n} w={lay.heroCoverW} h={Math.round(lay.heroCoverW / 0.7)} fs={lay.heroCoverFs} style={{ alignSelf: 'flex-start' }} />
                <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span className="tag tag-accent" style={{ alignSelf: 'flex-start' }}>{t('home.rank', { n: i + 1 })}</span>
                  <div className="display" style={{ fontSize: lay.heroTitleFs, lineHeight: 1.1 }}>{n.title}</div>
                  <div className="muted" style={{ fontSize: 12 }}>{lay.wide ? `${n.author} · ${names.cat(n.cat)}` : n.author}</div>
                  <div style={{ fontSize: lay.heroDescFs, lineHeight: 1.45, maxHeight: lay.heroDescMax, overflow: 'hidden', color: 'var(--color-neutral-800)' }}>{n.desc[uiLang]}</div>
                  <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--color-neutral-700)', marginTop: 'auto', alignItems: 'center' }}>
                    <span className="rating">★ {n.rating}</span>
                    <span>{t('common.reads', { n: formatReads(n.reads) })}</span>
                    {lay.wide && <span>{t('common.chapters', { n: n.chapters })}</span>}
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 6, padding: lay.wide ? '14px 0 0' : '14px 20px 0' }}>
        {popular.map((n, i) => (
          <button key={n.id} aria-label={`${i + 1} / ${count}`} onClick={() => goTo(i)}
            style={{ width: i === idx ? 24 : 8, height: 8, borderRadius: 999, border: 'none', padding: 0, cursor: 'pointer', transition: 'width .3s, background .3s', background: i === idx ? 'var(--color-accent)' : 'var(--color-neutral-400)' }} />
        ))}
      </div>
    </div>
  );
}
