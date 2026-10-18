import { useState } from 'react';
import { CONTENT_LANG_INFO, type ContentLang } from '../data/types';
import { useT } from '../i18n';
import { useApp } from '../store/AppStore';
import { Modal } from './Modal';

type Pick = ContentLang | 'ask';

/** Default reading-language setting, for books whose language isn't known. */
export function LangDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const app = useApp();
  const [pick, setPick] = useState<Pick>(app.contentLang ?? 'ask');

  const confirm = () => {
    app.setContentLang(pick === 'ask' ? null : pick);
    app.showToast(t(pick === 'ask' ? 'lang.askToast' : 'lang.savedToast'));
    onClose();
  };

  const row = (value: Pick, content: React.ReactNode) => (
    <label key={value} className="radio" style={{ display: 'flex', justifyContent: 'space-between', padding: '12px 16px', borderRadius: 22, background: pick === value ? 'var(--color-accent-100)' : 'var(--color-bg)' }}>
      {content}
      <input type="radio" name="content-lang" checked={pick === value} onChange={() => setPick(value)} />
      <span className="dot" style={{ width: 20, height: 20 }} />
    </label>
  );

  return (
    <Modal onClose={onClose}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('lang.titleSettings')} onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('lang.titleSettings')}</div>
        <div className="dialog-body" style={{ marginTop: -6 }}>{t('lang.bodySettings')}</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {CONTENT_LANG_INFO.map(l => row(l.code, (
            <span style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.3 }}>
              <span style={{ fontWeight: 700, fontSize: 15 }}>{l.native}</span>
              <span className="muted" style={{ fontSize: 12 }}>{l.name}</span>
            </span>
          )))}
          {row('ask', <span style={{ fontWeight: 700, fontSize: 15 }}>{t('lang.askEveryTime')}</span>)}
        </div>
        <div className="dialog-actions">
          <button className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={onClose}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={confirm}>{t('common.save')}</button>
        </div>
      </div>
    </Modal>
  );
}
