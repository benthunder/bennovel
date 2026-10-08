import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { openLibraryBook } from './localBook';

/** The Continue button: reopens whatever was read last, an online novel or an imported book. */
export function useContinue() {
  const t = useT();
  const app = useApp();
  return async () => {
    const local = app.lastLocal;
    if (local) {
      try {
        const { book, place } = await openLibraryBook(local.user, local.novelId);
        app.push({ s: 'book', book, place });
      } catch (e) {
        // The book was removed from the library (or the file is gone).
        console.error(e);
        app.forgetLocal(local.novelId);
        app.showToast(`${t('book.openFailed')} ${String(e)}`);
      }
    } else if (app.last) {
      app.openChapter(app.last.id, app.last.ch);
    } else {
      app.showToast(t('nav.nothing'));
    }
  };
}
