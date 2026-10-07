import { useCallback, useEffect, useState } from 'react';
import { ask } from '@tauri-apps/plugin-dialog';
import { CoverGrid, Spinner } from '../components/ui';
import { getNovels } from '../data/repository';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import {
  canOpenLocalBooks, deleteLibraryBook, importLocalBook, libraryUser, listLibrary, openLibraryBook, pickLocalBook,
  type LibraryBook
} from '../lib/localBook';

export function LibraryScreen() {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const favs = getNovels(app.favs);

  const openFile = async () => {
    try {
      const book = await pickLocalBook();
      if (book) app.push({ s: 'book', book });
    } catch (e) {
      console.error(e);
      app.showToast(`${t('book.openFailed')} ${String(e)}`);
    }
  };

  return (
    <div className="screen screen--tabbed" style={{ paddingTop: lay.topPlus, paddingLeft: lay.px, paddingRight: lay.px }}>
      <div className="kicker">{t('library.kicker')}</div>
      <h2 style={{ margin: '0 0 4px', fontSize: lay.libH }}>{t('library.title')}</h2>
      <div className="muted" style={{ fontSize: 13, marginBottom: 18 }}>
        {favs.length === 1 ? t('library.countOne') : t('library.count', { n: favs.length })}
      </div>
      {canOpenLocalBooks() && <DeviceBooks onOpenFile={openFile} />}
      <CoverGrid novels={favs} onOpen={app.openDetail} cols={lay.libCols} gap={lay.libGap} fs={lay.libFs} />
      {favs.length === 0 && (
        <div className="card" style={{ alignItems: 'flex-start', padding: 22, marginTop: 8, maxWidth: 420 }}>
          <div className="card-title">{t('library.emptyTitle')}</div>
          <p className="card-body">{t('library.emptyBody')}</p>
          <button className="btn btn-primary" onClick={() => app.switchTab('category')}>{t('library.browse')}</button>
        </div>
      )}
    </div>
  );
}

/** Books imported into the on-device library (native app only). */
function DeviceBooks({ onOpenFile }: { onOpenFile: () => void }) {
  const t = useT();
  const app = useApp();
  const user = libraryUser(app.user);
  const [books, setBooks] = useState<LibraryBook[] | null>(null);
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);

  const reload = useCallback(() => {
    listLibrary(user).then(setBooks).catch(e => { console.error(e); setBooks([]); });
  }, [user]);
  useEffect(reload, [reload]);

  const fail = (key: 'book.openFailed' | 'library.importFailed', e: unknown) => {
    console.error(e);
    app.showToast(`${t(key)} ${String(e)}`);
  };

  const importFile = async () => {
    setImporting({ done: 0, total: 0 });
    try {
      const book = await importLocalBook(user, (done, total) => setImporting({ done, total }));
      if (book) {
        app.showToast(t('library.imported', { title: book.title }));
        reload();
      }
    } catch (e) {
      fail('library.importFailed', e);
    } finally {
      setImporting(null);
    }
  };

  const read = async (b: LibraryBook) => {
    try {
      const { book, place } = await openLibraryBook(user, b.novelId);
      app.push({ s: 'book', book, place });
    } catch (e) {
      fail('book.openFailed', e);
    }
  };

  const remove = async (b: LibraryBook) => {
    if (!(await ask(t('library.deleteAsk', { title: b.title }), { kind: 'warning' }))) return;
    try {
      await deleteLibraryBook(user, b.novelId);
      reload();
    } catch (e) {
      fail('book.openFailed', e);
    }
  };

  return (
    <section style={{ marginBottom: 26 }}>
      <h3 style={{ margin: '0 0 10px', fontSize: 18 }}>{t('library.device')}</h3>
      <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', marginBottom: 12 }}>
        <button className="btn btn-primary" disabled={!!importing} onClick={importFile}>
          {importing ? <><Spinner size={14} /> {importing.total ? t('library.importing', { done: importing.done, total: importing.total }) : t('book.loading')}</> : t('library.import')}
        </button>
        <button className="btn btn-secondary" disabled={!!importing} onClick={onOpenFile}>{t('library.openOnce')}</button>
      </div>
      {books?.length === 0 && <p className="muted" style={{ fontSize: 13, margin: 0 }}>{t('library.deviceEmpty')}</p>}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {books?.map(b => {
          const at = b.lastRead ? b.lastRead.index + 1 : 0;
          return (
            <div key={b.novelId} className="card" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, padding: 14 }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="card-title" style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{b.title}</div>
                <div className="muted" style={{ fontSize: 12 }}>
                  {[b.format?.toUpperCase(), b.lang.toUpperCase(), b.lastRead ? t('library.at', { n: at, total: b.chapterCount }) : t('library.parts', { n: b.chapterCount })].filter(Boolean).join(' · ')}
                </div>
                <div style={{ height: 4, marginTop: 8, borderRadius: 999, background: 'var(--color-neutral-200)', overflow: 'hidden' }}>
                  <div style={{ height: '100%', width: `${Math.round((at / Math.max(b.chapterCount, 1)) * 100)}%`, background: 'var(--color-accent)' }} />
                </div>
              </div>
              <button className="btn btn-primary" onClick={() => read(b)}>{b.lastRead ? t('library.continue') : t('library.read')}</button>
              <button className="btn btn-secondary" aria-label={t('library.delete')} onClick={() => remove(b)}>✕</button>
            </div>
          );
        })}
      </div>
    </section>
  );
}
