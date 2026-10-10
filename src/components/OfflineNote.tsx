import { useCatalog } from '../data/useCatalog';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { Spinner } from './ui';

/** Stands in for online content while there is no connection. */
export function OfflineNote({ showLibrary = false }: { showLibrary?: boolean }) {
  const t = useT();
  const app = useApp();
  const { retry } = useCatalog();
  return (
    <div className="card" role="status" style={{ alignItems: 'flex-start', padding: 22, maxWidth: 420 }}>
      <div className="card-title">{t('load.offline')}</div>
      <p className="card-body">{t('load.offlineBody')}</p>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
        <button className="btn btn-primary" onClick={retry}>{t('load.retry')}</button>
        {showLibrary && <button className="btn btn-secondary" onClick={() => app.switchTab('library')}>{t('nav.library')}</button>}
      </div>
    </div>
  );
}

/** Shown in place of an online screen until the catalog has loaded. */
export function CatalogLoading() {
  const t = useT();
  return (
    <div className="screen" style={{ display: 'grid', placeItems: 'center', padding: 24 }}>
      <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10 }}><Spinner size={18} />{t('load.catalog')}</div>
    </div>
  );
}
