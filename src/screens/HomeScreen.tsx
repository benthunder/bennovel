import { useEffect, useRef, useState } from 'react';
import { NovelCover } from '../components/NovelCover';
import { NovelRow, SearchField, ShelfItem, Spinner, formatReads } from '../components/ui';
import { getCollections, getPopular, searchNovels } from '../data/repository';
import { useI18n } from '../i18n';
import { useThrottledValue } from '../lib/useThrottledValue';
import { SEARCH_THROTTLE_MS, useApp } from '../store/AppStore';
import { useCatalogNames } from '../data/useCatalogNames';

const AUTOPLAY_MS = 4200;
/** Autoplay pauses this long after the user swipes or taps a dot. */
const TOUCH_PAUSE_MS = 6000;

const popular = getPopular();
const collections = getCollections();

export function HomeScreen() {
  const { t } = useI18n();
  const names = useCatalogNames();
  const app = useApp();
  const [input, setInput] = useState('');
  const query = useThrottledValue(input, SEARCH_THROTTLE_MS);
  const pending = input !== query;
  const results = searchNovels(query);

  const hr = new Date().getHours();
  const greeting = t(hr < 12 ? 'home.morning' : hr < 18 ? 'home.afternoon' : 'home.evening');
  const firstName = app.user ? app.user.name.split(' ')[0] : t('home.reader');
  const initials = app.user ? app.user.name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G';

  return (
    <div className="screen screen--tabbed">
      <div className="pad-x" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div>
          <div className="muted" style={{ fontSize: 13 }}>{greeting}</div>
          <div className="display" style={{ fontSize: 26, lineHeight: 1.1 }}>{t('home.whatsNext', { name: firstName })}</div>
        </div>
        <button className="display" aria-label={t('nav.profile')} onClick={() => app.switchTab('profile')}
          style={{ width: 44, height: 44, flex: 'none', borderRadius: '50%', border: 'none', background: 'var(--color-accent-2-300)', color: 'var(--color-accent-2-900)', fontSize: 16, cursor: 'pointer' }}>
          {initials}
        </button>
      </div>
      <div className="pad-x" style={{ paddingTop: 16 }}>
        <SearchField value={input} onChange={setInput} placeholder={t('home.searchPlaceholder')} />
      </div>

      {input ? (
        <div style={{ padding: '12px 20px 0', display: 'flex', flexDirection: 'column', gap: 4 }}>
          <div className="status-line" style={{ padding: '0 4px 6px' }}>
            {pending && <Spinner />}
            <span>{pending ? t('common.searching') : results.length === 1 ? t('home.result', { q: query }) : t('home.results', { n: results.length, q: query })}</span>
          </div>
          {results.map(n => <NovelRow key={n.id} novel={n} onOpen={() => app.openDetail(n.id)} />)}
          {!pending && query && results.length === 0 && (
            <div className="empty"><div className="empty__title">{t('home.emptyTitle')}</div><div className="empty__body muted">{t('home.emptyBody')}</div></div>
          )}
        </div>
      ) : (
        <>
          <HeroSlider />
          {collections.map(c => (
            <section key={c.key} style={{ paddingTop: 28 }}>
              <div className="section-head pad-x" style={{ paddingBottom: 12 }}>
                <div>
                  <div className="kicker">{names.coll(c.key).kicker}</div>
                  <h4 style={{ margin: 0 }}>{names.coll(c.key).title}</h4>
                </div>
                <button className="btn btn-ghost" style={{ fontSize: 13 }} onClick={() => app.openCollection(c.key)}>{t('common.seeAll')}</button>
              </div>
              <div className="shelf nx-scroll">
                {c.items.map(n => <ShelfItem key={n.id} novel={n} w={112} h={160} fs={15} onOpen={() => app.openDetail(n.id)} />)}
              </div>
            </section>
          ))}
        </>
      )}
    </div>
  );
}

function HeroSlider() {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const [idx, setIdx] = useState(0);
  const touchedAt = useRef(0);
  const dragX = useRef<number | null>(null);
  const suppressClick = useRef(false);
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
    <>
      <div className="section-head pad-x" style={{ alignItems: 'baseline', padding: '24px 20px 12px' }}>
        <h4 style={{ margin: 0 }}>{t('home.popular')}</h4>
        <span className="muted" style={{ fontSize: 12 }}>{idx + 1} / {count}</span>
      </div>
      <div style={{ overflow: 'hidden', touchAction: 'pan-y' }} onPointerDown={e => (dragX.current = e.clientX)} onPointerUp={onPointerUp}>
        <div style={{ display: 'flex', transition: 'transform .55s cubic-bezier(.2,.8,.2,1)', transform: `translateX(-${idx * 100}%)` }}>
          {popular.map((n, i) => (
            <div key={n.id} style={{ flex: '0 0 100%', padding: '0 20px' }} aria-hidden={i !== idx}>
              <div role="button" tabIndex={i === idx ? 0 : -1} onClick={() => !suppressClick.current && app.openDetail(n.id)}
                onKeyDown={e => e.key === 'Enter' && app.openDetail(n.id)}
                style={{ position: 'relative', overflow: 'hidden', borderRadius: 34, background: 'var(--color-surface)', padding: 18, display: 'flex', gap: 16, cursor: 'pointer', userSelect: 'none' }}>
                <NovelCover novel={n} w={118} h={170} fs={17} />
                <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span className="tag tag-accent" style={{ alignSelf: 'flex-start' }}>{t('home.rank', { n: i + 1 })}</span>
                  <div className="display" style={{ fontSize: 20, lineHeight: 1.1 }}>{n.title}</div>
                  <div className="muted" style={{ fontSize: 12 }}>{n.author}</div>
                  <div style={{ fontSize: 12.5, lineHeight: 1.45, maxHeight: 73, overflow: 'hidden', color: 'var(--color-neutral-800)' }}>{n.desc[uiLang]}</div>
                  <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--color-neutral-700)', marginTop: 'auto' }}>
                    <span className="rating">★ {n.rating}</span>
                    <span>{t('common.reads', { n: formatReads(n.reads) })}</span>
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 6, padding: '14px 20px 0' }}>
        {popular.map((n, i) => (
          <button key={n.id} aria-label={`${i + 1} / ${count}`} onClick={() => goTo(i)}
            style={{ width: i === idx ? 24 : 8, height: 8, borderRadius: 999, border: 'none', padding: 0, cursor: 'pointer', transition: 'width .3s, background .3s', background: i === idx ? 'var(--color-accent)' : 'var(--color-neutral-400)' }} />
        ))}
      </div>
    </>
  );
}
