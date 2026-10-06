import { useState } from 'react';
import { ChevronLeftIcon } from '../components/Icons';
import { NovelRow, SearchField, Segmented, Spinner } from '../components/ui';
import { getCategories, listNovels, type ListFilter, type ListSource, type SortKey } from '../data/repository';
import type { Category, NovelStatus } from '../data/types';
import { useI18n } from '../i18n';
import { useThrottledValue } from '../lib/useThrottledValue';
import { SEARCH_THROTTLE_MS, useApp } from '../store/AppStore';
import { useCatalogNames } from '../data/useCatalogNames';

const SORTS: SortKey[] = ['Popular', 'Newest', 'Rating'];
const STATUSES: ('All' | NovelStatus)[] = ['All', 'Ongoing', 'Completed'];

export function ListScreen({ src }: { src: ListSource }) {
  const { t } = useI18n();
  const names = useCatalogNames();
  const app = useApp();
  const [input, setInput] = useState('');
  const query = useThrottledValue(input, SEARCH_THROTTLE_MS);
  const pending = input !== query;
  const [status, setStatus] = useState<ListFilter['status']>('All');
  const [sort, setSort] = useState<SortKey>('Popular');
  const [cat, setCat] = useState<'All' | Category>('All');

  const items = listNovels(src, { query, status, sort, cat });

  const title = src.type === 'all' ? t('list.categories')
    : src.type === 'category' ? names.cat(src.value)
    : src.type === 'collection' ? names.coll(src.value).title
    : src.value;
  const placeholder = src.type === 'author' ? t('list.searchAuthor', { v: src.value })
    : src.type === 'category' ? t('list.searchIn', { v: names.cat(src.value) })
    : t('list.searchAll');

  return (
    <div className="screen screen--tabbed">
      <div className="pad-x" style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          {app.canBack && <button className="icon-btn" aria-label={t('common.back')} onClick={app.back}><ChevronLeftIcon /></button>}
          {src.type === 'author' && (
            <span className="display" style={{ width: 52, height: 52, flex: 'none', borderRadius: '50%', background: 'var(--color-accent-2-300)', color: 'var(--color-accent-2-900)', display: 'grid', placeItems: 'center', fontSize: 19 }}>
              {src.value.split(' ').map(w => w[0]).join('')}
            </span>
          )}
          <div style={{ minWidth: 0 }}>
            <div className="kicker">{t(`list.kicker.${src.type}`)}</div>
            <h3 style={{ margin: 0, fontSize: 26 }}>{title}</h3>
          </div>
        </div>
        <SearchField value={input} onChange={setInput} placeholder={placeholder} />
      </div>

      {src.type === 'all' && (
        <div className="nx-scroll" style={{ display: 'flex', gap: 8, overflowX: 'auto', padding: '14px 20px 0' }}>
          {(['All', ...getCategories()] as const).map(c => (
            <button key={c} className={`chip${cat === c ? ' chip--active' : ''}`} aria-pressed={cat === c} onClick={() => setCat(c)}>
              {c === 'All' ? t('list.all') : names.cat(c)}
            </button>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8, padding: '12px 20px 0', alignItems: 'center', flexWrap: 'wrap' }}>
        {STATUSES.map(s => (
          <button key={s} className={`chip chip--sm${status === s ? ' chip--active' : ''}`} aria-pressed={status === s} onClick={() => setStatus(s)}>
            {s === 'All' ? t('list.anyStatus') : t(`status.${s}`)}
          </button>
        ))}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, padding: '14px 20px 4px' }}>
        <div className="status-line">
          {pending && <Spinner />}
          <span>{pending ? t('common.searching') : items.length === 1 ? t('list.countOne') : t('list.count', { n: items.length })}</span>
        </div>
        <Segmented value={sort} onChange={setSort} style={{ display: 'inline-flex', fontSize: 12 }} optStyle={{ flex: 'none', padding: '6px 11px', fontSize: 12 }}
          options={SORTS.map(s => ({ label: t(`list.sort.${s}`), value: s }))} />
      </div>

      <div style={{ padding: '4px 10px 0', display: 'flex', flexDirection: 'column', gap: 2 }}>
        {items.map(n => <NovelRow key={n.id} novel={n} statusAsTag coverW={68} coverH={98} onOpen={() => app.openDetail(n.id)} />)}
        {!pending && items.length === 0 && (
          <div className="empty" style={{ padding: '40px 20px' }}>
            <div className="empty__title">{t('list.emptyTitle')}</div>
            <div className="empty__body muted">{t('list.emptyBody')}</div>
          </div>
        )}
      </div>
    </div>
  );
}
