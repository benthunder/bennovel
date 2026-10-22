import { useCallback, useEffect, useState } from 'react';
import { EditBookDialog } from '../components/EditBookDialog';
import { ask } from '@tauri-apps/plugin-dialog';
import { Spinner } from '../components/ui';
import { NovelCover } from '../components/NovelCover';
import { EyeIcon, FilePlusIcon, FolderOpenIcon, PencilIcon, XIcon } from '../components/Icons';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import {
  canOpenLocalBooks, deleteLibraryBook, importLocalBook, importLocalFolder, canPickFolder, libraryCoverUrl, listLibrary, localCover, pickLocalBook, updateLibraryBook,
  type LibraryBook
} from '../lib/localBook';
import { runOcr, useOcrStatus } from '../lib/ocr';

export function LibraryScreen() {
  const t = useT();
  const app = useApp();
  const lay = useLayout();

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
      {canOpenLocalBooks() && <DeviceBooks onOpenFile={openFile} />}
    </div>
  );
}

/** Books imported into the on-device library (native app only). */
function DeviceBooks({ onOpenFile }: { onOpenFile: () => void }) {
  const t = useT();
  const app = useApp();
  const lay = useLayout();
  const user = app.libKey;
  const [books, setBooks] = useState<LibraryBook[] | null>(null);
  const [importing, setImporting] = useState<{ done: number; total: number } | null>(null);
  const [editing, setEditing] = useState<LibraryBook | null>(null);
  const ocr = useOcrStatus();

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
        runOcr(user);
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
      <div className="muted" style={{ fontSize: 13, marginBottom: 18 }}>
        {books?.length === 1 ? t('library.countOne') : t('library.count', { n: books?.length ?? 0 })}
      </div>
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
      ) : ocr ? (
        <div className="import-progress" role="status" style={{ marginBottom: 14 }}>
          <Spinner size={14} />
          <span>{t('library.ocr', { title: ocr.title, done: ocr.done, total: ocr.total })}</span>
          <div className="import-progress__bar"><div style={{ width: `${Math.round((ocr.done / Math.max(1, ocr.total)) * 100)}%` }} /></div>
        </div>
      ) : (
        <p className="muted import-hint" style={{ marginBottom: 14 }}>{t(canPickFolder() ? 'library.importHint' : 'library.importHintMobile')}</p>
      )}
      {books?.length === 0 && <p className="muted" style={{ fontSize: 13, margin: 0 }}>{t('library.deviceEmpty')}</p>}
      <div className="cover-grid" style={{ gridTemplateColumns: lay.libCols, gap: lay.libGap }}>
        {books?.map(b => (
          <div key={b.novelId} style={{ position: 'relative', minWidth: 0 }}>
            <button className="shelf-item" style={{ minWidth: 0, width: '100%' }} onClick={() => app.push({ s: 'bookDetail', novelId: b.novelId })}>
              <NovelCover novel={{ title: b.title, cover: localCover(b.title), image: libraryCoverUrl(user, b) }} w="100%" h="auto" fs={lay.libFs} style={{ aspectRatio: 0.7 }} />
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
