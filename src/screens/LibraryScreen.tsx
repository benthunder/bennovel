import { useCallback, useEffect, useState } from 'react';
import { EditBookDialog } from '../components/EditBookDialog';
import { ask } from '@tauri-apps/plugin-dialog';
import { CoverGrid, Spinner } from '../components/ui';
import { NovelCover } from '../components/NovelCover';
import { OfflineNote } from '../components/OfflineNote';
import { useCatalog } from '../data/useCatalog';
import { EyeIcon, FilePlusIcon, FolderOpenIcon, PencilIcon, XIcon } from '../components/Icons';
import { getNovels } from '../data/repository';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import {
  canOpenLocalBooks, deleteLibraryBook, importLocalBook, importLocalFolder, canPickFolder, libraryUser, listLibrary, localCover, pickLocalBook, updateLibraryBook,
  type LibraryBook
} from '../lib/localBook';

export function LibraryScreen() {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const favs = getNovels(app.favs);
  /** Online novels downloaded to this device, newest first. */
  const downloaded = app.downloads.list.flatMap(d => d.data ?? []);
  // Saved before this device kept snapshots: those show once the catalog loads.
  const missing = useCatalog().status === 'offline' && favs.length < app.favs.length;

  const openFile = async () => {
    try {
      const picked = await pickLocalBook();
      if (picked) app.push({ s: 'book', book: picked.book, path: picked.path });
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
      {downloaded.length > 0 && (
        <section style={{ marginBottom: 26 }}>
          <h3 style={{ margin: '0 0 10px', fontSize: 18 }}>{t('library.downloaded')}</h3>
          <CoverGrid novels={downloaded} onOpen={app.openDetail} cols={lay.libCols} gap={lay.libGap} fs={lay.libFs} />
        </section>
      )}
      <CoverGrid novels={favs} onOpen={app.openDetail} cols={lay.libCols} gap={lay.libGap} fs={lay.libFs} />
      {missing && <div style={{ marginTop: 8 }}><OfflineNote /></div>}
      {favs.length === 0 && !missing && (
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
  const lay = useLayout();
  const user = libraryUser(app.user);
  const [books, setBooks] = useState<LibraryBook[] | null>(null);
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);
  const [editing, setEditing] = useState<LibraryBook | null>(null);

  // Listed again after a sync, which can add or remove books.
  const reload = useCallback(() => {
    listLibrary(user).then(setBooks).catch(e => { console.error(e); setBooks([]); });
  }, [user, app.libraryRev]);
  useEffect(reload, [reload]);

  const fail = (key: 'book.openFailed' | 'library.importFailed', e: unknown) => {
    console.error(e);
    app.showToast(`${t(key)} ${String(e)}`);
  };

  const runImport = async (pick: typeof importLocalBook) => {
    setImporting({ done: 0, total: 0 });
    try {
      const book = await pick(user, (done, total) => setImporting({ done, total }));
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

  const remove = async (b: LibraryBook) => {
    if (!(await ask(t('library.deleteAsk', { title: b.title }), { kind: 'warning' }))) return;
    try {
      await deleteLibraryBook(user, b.novelId);
      app.forgetLocal(b.novelId);
      reload();
    } catch (e) {
      fail('book.openFailed', e);
    }
  };

  return (
    <section style={{ marginBottom: 26 }}>
      <h3 style={{ margin: '0 0 10px', fontSize: 18 }}>{t('library.device')}</h3>
      <div className="import-actions">
        <button className="import-action import-action--primary" disabled={!!importing} onClick={() => runImport(importLocalBook)}>
          <span className="import-action__icon"><FilePlusIcon size={19} /></span>{t('library.import')}
        </button>
        <button className="import-action" disabled={!!importing} onClick={() => runImport(importLocalFolder)}>
          <span className="import-action__icon"><FolderOpenIcon size={19} /></span>{t('library.importFolder')}
        </button>
        <button className="import-action" disabled={!!importing} onClick={onOpenFile}>
          <span className="import-action__icon"><EyeIcon size={19} /></span>{t('library.openOnce')}
        </button>
      </div>
      {importing ? (
        <div className="import-progress" role="status" style={{ marginBottom: 14 }}>
          <Spinner size={14} />
          <span>{importing.total ? t('library.importing', { done: importing.done, total: importing.total }) : t('book.loading')}</span>
          <div className="import-progress__bar"><div style={{ width: `${importing.total ? Math.round((importing.done / importing.total) * 100) : 0}%` }} /></div>
        </div>
      ) : (
        <p className="muted import-hint" style={{ marginBottom: 14 }}>{t(canPickFolder() ? 'library.importHint' : 'library.importHintMobile')}</p>
      )}
      {books?.length === 0 && <p className="muted" style={{ fontSize: 13, margin: 0 }}>{t('library.deviceEmpty')}</p>}
      {/* Shown like the online novels, opening a details page; Continue in the navigation resumes the one read last. */}
      <div className="cover-grid" style={{ gridTemplateColumns: lay.libCols, gap: lay.libGap }}>
        {books?.map(b => (
          <div key={b.novelId} style={{ position: 'relative', minWidth: 0 }}>
            <button className="shelf-item" style={{ minWidth: 0, width: '100%' }} onClick={() => app.push({ s: 'bookDetail', novelId: b.novelId })}>
              <NovelCover novel={{ title: b.title, cover: localCover(b.title) }} w="100%" h="auto" fs={lay.libFs} style={{ aspectRatio: 0.7 }} />
              <div className="shelf-item__title" style={{ fontSize: 12 }}>{b.title}</div>
              <div className="shelf-item__author muted">{b.author ?? [b.format?.toUpperCase(), b.lang.toUpperCase()].filter(Boolean).join(' · ')}</div>
            </button>
            <div style={{ position: 'absolute', top: 6, right: 6, display: 'flex', flexDirection: 'column', gap: 6 }}>
              <button className="btn btn-icon btn-secondary" aria-label={t('library.delete')} title={t('library.delete')} onClick={() => remove(b)}
                style={{ width: 30, height: 30 }}><XIcon size={14} /></button>
              <button className="btn btn-icon btn-secondary" aria-label={t('library.edit')} title={t('library.edit')} onClick={() => setEditing(b)}
                style={{ width: 30, height: 30 }}><PencilIcon size={13} /></button>
            </div>
          </div>
        ))}
      </div>
      {editing && (
        <EditBookDialog book={editing} onClose={() => setEditing(null)} onSave={async details => {
          await updateLibraryBook(user, editing.novelId, details);
          setEditing(null);
          app.showToast(t('library.saved'));
          reload();
        }} />
      )}
    </section>
  );
}
