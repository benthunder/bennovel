import { useEffect, useRef } from 'react';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { canOpenLocalBooks, openBookFile, takeOpenedFiles } from './localBook';

/**
 * Opens book files the system hands to the app ("Open with BenNovel", or tapping a
 * book file), both at launch and while the app is running.
 */
export function useOpenedFiles() {
  const t = useT();
  const app = useApp();
  // The latest store, so the listener registered once never calls stale callbacks.
  const ref = useRef({ app, t });
  ref.current = { app, t };

  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    let stop: (() => void) | undefined;
    let gone = false;
    const openQueued = async () => {
      const files = await takeOpenedFiles();
      // Several at once: the last one is what the user picked most recently.
      const path = files[files.length - 1];
      if (!path) return;
      const { app, t } = ref.current;
      try {
        app.push({ s: 'book', book: await openBookFile(path) });
      } catch (e) {
        console.error(e);
        app.showToast(`${t('book.openFailed')} ${String(e)}`);
      }
    };
    import('@tauri-apps/api/event').then(({ listen }) => listen('files-opened', openQueued)).then(un => {
      if (gone) un();
      else stop = un;
    });
    openQueued().catch(console.error);
    return () => { gone = true; stop?.(); };
  }, []);
}
