import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { loadCatalog } from '../data/repository';
import { useI18n } from '../i18n';
import { Spinner } from './ui';

/** Holds the app back until the catalog has loaded from Supabase. */
export function CatalogGate({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading');

  const load = useCallback(() => {
    setState('loading');
    loadCatalog().then(() => setState('ready'), err => { console.error(err); setState('failed'); });
  }, []);
  useEffect(load, [load]);

  if (state === 'ready') return children;
  return (
    <div className="app" style={{ display: 'grid', placeItems: 'center', padding: 24, textAlign: 'center' }}>
      {state === 'loading' ? (
        <div role="status" style={{ display: 'flex', alignItems: 'center', gap: 10 }}><Spinner size={18} />{t('load.catalog')}</div>
      ) : (
        <div role="alert" style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14, maxWidth: 320 }}>
          <div>{t('load.failed')}</div>
          <button className="btn btn-primary" onClick={load}>{t('load.retry')}</button>
        </div>
      )}
    </div>
  );
}
