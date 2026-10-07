import { useState } from 'react';
import { getContentLangs } from '../data/repository';
import type { ContentLang } from '../data/types';
import { useT } from '../i18n';
import { useApp, type LangDialog as Dialog } from '../store/AppStore';
import { Checkbox } from './ui';

type Pick = ContentLang | 'ask';

/**
 * Reading-language picker. Three modes:
 * - chapter: first chapter opened with no saved language — pick, optionally save, then open it
 * - reader: switch the language of the open chapter
 * - settings: change (or clear) the language saved on this device
 */
export function LangDialog({ dialog }: { dialog: Dialog }) {
  const t = useT();
  const app = useApp();
  const readerLang = app.top.s === 'reader' ? app.top.lang : null;
  const [pick, setPick] = useState<Pick>(
    dialog.mode === 'settings' ? app.contentLang ?? 'ask' : dialog.mode === 'reader' ? readerLang ?? 'en' : 'en'
  );
  const [remember, setRemember] = useState(true);
  const close = () => app.setLangDialog(null);

  const confirm = () => {
    if (dialog.mode === 'settings') {
      app.setContentLang(pick === 'ask' ? null : pick);
      app.showToast(t(pick === 'ask' ? 'lang.askToast' : 'lang.savedToast'));
    } else if (pick !== 'ask') {
      if (remember) app.setContentLang(pick);
      if (dialog.mode === 'reader') app.setReaderLang(pick);
      else app.goReader(dialog.id, dialog.ch, pick);
    }
    close();
  };

  const row = (value: Pick, content: React.ReactNode) => (
    <label key={value} className="radio" style={{ display: 'flex', justifyContent: 'space-between', padding: '12px 16px', borderRadius: 22, background: pick === value ? 'var(--color-accent-100)' : 'var(--color-bg)' }}>
      {content}
      <input type="radio" name="content-lang" checked={pick === value} onChange={() => setPick(value)} />
      <span className="dot" style={{ width: 20, height: 20 }} />
    </label>
  );

  const title = dialog.mode === 'settings' ? t('lang.titleSettings') : dialog.mode === 'reader' ? t('lang.titleReader') : t('lang.titleChapter');

  return (
    <div className="dialog-backdrop dialog-backdrop--sheet" onClick={close}>
      <div className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={title} onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{title}</div>
        <div className="dialog-body" style={{ marginTop: -6 }}>{dialog.mode === 'settings' ? t('lang.bodySettings') : t('lang.bodyChapter')}</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {getContentLangs().map(l => row(l.code, (
            <span style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.3 }}>
              <span style={{ fontWeight: 700, fontSize: 15 }}>{l.native}</span>
              <span className="muted" style={{ fontSize: 12 }}>{l.name}</span>
            </span>
          )))}
          {dialog.mode === 'settings' && row('ask', <span style={{ fontWeight: 700, fontSize: 15 }}>{t('lang.askEveryTime')}</span>)}
        </div>
        {dialog.mode !== 'settings' && (
          <Checkbox checked={remember} onChange={() => setRemember(r => !r)} size={22} style={{ fontSize: 13 }}>{t('lang.remember')}</Checkbox>
        )}
        <div className="dialog-actions">
          <button className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={close}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={confirm}>
            {dialog.mode === 'chapter' ? t('lang.startReading') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
