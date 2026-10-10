import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from 'react';
import { loadCatalog } from './repository';

/**
 * `offline`: no connection at startup, or the catalog could not be reached. Online
 * screens then show nothing from the server, while books on the device still open.
 */
export type CatalogStatus = 'loading' | 'ready' | 'offline';

interface CatalogState {
  status: CatalogStatus;
  retry: () => void;
}

const Ctx = createContext<CatalogState>({ status: 'loading', retry: () => undefined });

const isOnline = () => typeof navigator === 'undefined' || navigator.onLine !== false;

/** Loads the catalog from Supabase once at startup, without holding the app back. */
export function CatalogProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<CatalogStatus>(() => (isOnline() ? 'loading' : 'offline'));

  const retry = useCallback(() => {
    if (!isOnline()) return setStatus('offline');
    setStatus('loading');
    loadCatalog().then(() => setStatus('ready'), err => { console.error(err); setStatus('offline'); });
  }, []);

  useEffect(() => { if (isOnline()) retry(); }, [retry]);

  // Coming back online fetches what was skipped.
  useEffect(() => {
    if (status !== 'offline') return;
    window.addEventListener('online', retry);
    return () => window.removeEventListener('online', retry);
  }, [status, retry]);

  return <Ctx.Provider value={{ status, retry }}>{children}</Ctx.Provider>;
}

export const useCatalog = () => useContext(Ctx);
