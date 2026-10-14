import { useState } from 'react';
import { Modal } from './Modal';
import { Spinner } from './ui';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import type { LibraryBook } from '../lib/localBook';

/** Title, author and description of a book on the device. */
export function EditBookDialog({ book, onSave, onClose }: {
  book: LibraryBook;
  onSave: (details: { title: string; author: string; description: string }) => Promise<void>;
  onClose: () => void;
}) {
  const t = useT();
  const app = useApp();
  const [title, setTitle] = useState(book.title);
  const [author, setAuthor] = useState(book.author ?? '');
  const [description, setDescription] = useState(book.description);
  const [saving, setSaving] = useState(false);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim() || saving) return;
    setSaving(true);
    try {
      await onSave({ title, author, description });
    } catch (err) {
      console.error(err);
      app.showToast(`${t('library.saveFailed')} ${String(err)}`);
      setSaving(false);
    }
  };
  return (
    <Modal onClose={onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('library.editTitle')} onClick={e => e.stopPropagation()} onSubmit={submit}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('library.editTitle')}</div>
        <div className="field">
          <label htmlFor="book-title">{t('library.fieldTitle')}</label>
          <input id="book-title" className="input" style={{ height: 44 }} value={title} onChange={e => setTitle(e.target.value)} required autoFocus />
        </div>
        <div className="field">
          <label htmlFor="book-author">{t('library.fieldAuthor')}</label>
          <input id="book-author" className="input" style={{ height: 44 }} value={author} placeholder={t('library.fieldAuthorEmpty')} onChange={e => setAuthor(e.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="book-description">{t('library.fieldDescription')}</label>
          <textarea id="book-description" className="input" rows={4} style={{ borderRadius: 20 }} value={description} onChange={e => setDescription(e.target.value)} />
        </div>
        <div className="dialog-actions">
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={onClose}>{t('common.cancel')}</button>
          <button type="submit" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={saving || !title.trim()}>
            {saving ? <Spinner size={14} /> : t('common.save')}
          </button>
        </div>
      </form>
    </Modal>
  );
}
