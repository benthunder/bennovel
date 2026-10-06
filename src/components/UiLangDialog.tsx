import { useState } from 'react';
import { createPortal } from 'react-dom';
import type { UiLang } from '../data/types';
import { useI18n } from '../i18n';
import { useApp } from '../store/AppStore';

// The interface ships in English and Vietnamese only.
const UI_LANGS: { code: UiLang; native: string; name: string }[] = [
  { code: 'en', native: 'English', name: 'English' },
  { code: 'vi', native: 'Tiếng Việt', name: 'Vietnamese' }
];

export const uiLangLabel = (l: UiLang) => UI_LANGS.find(x => x.code === l)?.native ?? l;

/** App (interface) language picker, opened from Profile. Same sheet style as the reading-language dialog. */
export function UiLangDialog({ onClose }: { onClose: () => void }) {
  const { t, uiLang, setUiLang } = useI18n();
  const app = useApp();
  const [pick, setPick] = useState<UiLang>(uiLang);
  const host = document.querySelector('.app');

  const confirm = () => {
    setUiLang(pick);
    onClose();
    // Toast in the newly picked language.
    if (pick !== uiLang) app.showToast(pick === 'vi' ? 'Đã đổi ngôn ngữ ứng dụng' : 'App language changed');
  };

  const sheet = (
    <div className="dialog-backdrop" style={{ position: 'absolute', zIndex: 60, placeItems: 'end stretch', padding: 10, paddingBottom: 'calc(env(safe-area-inset-bottom) + 10px)', color: 'var(--color-text)' }} onClick={onClose}>
      <div className="dialog" role="dialog" aria-modal="true" aria-label={t('uiLang.title')} style={{ width: '100%', borderRadius: 36, padding: 22, animation: 'nxup .28s ease-out' }} onClick={e => e.stopPropagation()}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('uiLang.title')}</div>
        <div className="dialog-body" style={{ marginTop: -6 }}>{t('uiLang.body')}</div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {UI_LANGS.map(l => (
            <label key={l.code} className="radio" style={{ display: 'flex', justifyContent: 'space-between', padding: '12px 16px', borderRadius: 22, background: pick === l.code ? 'var(--color-accent-100)' : 'var(--color-bg)' }}>
              <span style={{ display: 'flex', flexDirection: 'column', lineHeight: 1.3 }}>
                <span style={{ fontWeight: 700, fontSize: 15 }}>{l.native}</span>
                <span className="muted" style={{ fontSize: 12 }}>{l.name}</span>
              </span>
              <input type="radio" name="ui-lang" checked={pick === l.code} onChange={() => setPick(l.code)} />
              <span className="dot" style={{ width: 20, height: 20 }} />
            </label>
          ))}
        </div>
        <div className="dialog-actions">
          <button className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} onClick={onClose}>{t('common.cancel')}</button>
          <button className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={confirm}>{t('common.save')}</button>
        </div>
      </div>
    </div>
  );

  return host ? createPortal(sheet, host) : sheet;
}
