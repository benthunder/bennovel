import { useState } from 'react';
import { GlobeIcon, LanguagesIcon, LogOutIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { UiLangDialog, uiLangLabel } from '../components/UiLangDialog';
import { CoverGrid, Segmented } from '../components/ui';
import { CONTENT_LANGS } from '../data/mock';
import { getNovel } from '../data/repository';
import { relativeTime, useI18n } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';

export function ProfileScreen() {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const [tab, setTab] = useState<'history' | 'favs'>('history');
  const [uiLangOpen, setUiLangOpen] = useState(false);
  const user = app.user;
  const initials = user ? user.name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G';
  const langLabel = CONTENT_LANGS.find(l => l.code === app.contentLang)?.native ?? t('lang.askEach');

  return (
    <div className="screen screen--tabbed" style={{ paddingTop: lay.topPlus, paddingLeft: lay.px, paddingRight: lay.px }}>
      <div style={{
        display: 'grid', gridTemplateColumns: lay.profCols, gap: lay.wide ? '18px 32px' : 18, alignItems: 'start',
        gridTemplateAreas: lay.wide ? '"card tabs" "card content" "settings content" ". content"' : '"card" "tabs" "content" "settings"',
        gridTemplateRows: lay.wide ? 'auto auto auto 1fr' : 'auto'
      }}>
        <div style={{ gridArea: 'card' }}>
          {user ? (
            <div style={{ borderRadius: 34, background: 'var(--color-surface)', padding: 22, display: 'flex', flexDirection: lay.wide ? 'column' : 'row', gap: 16, alignItems: lay.wide ? 'flex-start' : 'center' }}>
              <span className="display" style={{ width: 72, height: 72, flex: 'none', borderRadius: '50%', background: 'var(--color-accent-2-300)', color: 'var(--color-accent-2-900)', display: 'grid', placeItems: 'center', fontSize: 26 }}>{initials}</span>
              <div style={{ minWidth: 0 }}>
                <div className="display" style={{ fontSize: 22, lineHeight: 1.1 }}>{user.name}</div>
                <div className="muted" style={{ fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis' }}>{user.email}</div>
                <span className="tag tag-accent-2" style={{ marginTop: 6 }}>{t('profile.signedInWith', { p: user.provider })}</span>
              </div>
            </div>
          ) : (
            <div style={{ borderRadius: 34, background: 'var(--color-surface)', padding: 22, display: 'flex', flexDirection: 'column', gap: 10, alignItems: 'flex-start' }}>
              <div className="display" style={{ fontSize: 22 }}>{t('profile.guestTitle')}</div>
              <div style={{ fontSize: 13, color: 'var(--color-neutral-800)' }}>{t('profile.guestBody')}</div>
              <button className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} onClick={() => app.push({ s: 'login' })}>{t('profile.loginCta')}</button>
            </div>
          )}
        </div>

        <Segmented value={tab} onChange={setTab} style={{ gridArea: 'tabs' }} optStyle={{ padding: 10 }}
          options={[{ label: t('profile.history'), value: 'history' }, { label: t('profile.favourites'), value: 'favs' }]} />

        <div style={{ gridArea: 'content', minWidth: 0 }}>
          {tab === 'history' ? (
            <div style={{ display: 'grid', gridTemplateColumns: lay.histCols, gap: '4px 12px', marginTop: -6 }}>
              {app.history.map(h => {
                const n = getNovel(h.id);
                const pct = Math.max(4, Math.round((h.ch / n.chapters) * 100));
                return (
                  <button key={h.id} className="novel-row" style={{ alignItems: 'center', padding: 8, borderRadius: 22, minWidth: 0 }} onClick={() => app.openChapter(h.id, h.ch)}>
                    <NovelCover novel={n} w={52} h={74} fs={9} />
                    <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 5 }}>
                      <span style={{ fontWeight: 700, fontSize: 14 }}>{n.title}</span>
                      <span className="muted" style={{ fontSize: 12 }}>{t('common.chapter', { n: h.ch })} · {relativeTime(t, h.at)}</span>
                      <span style={{ height: 6, borderRadius: 999, background: 'var(--color-neutral-300)', overflow: 'hidden' }}>
                        <span style={{ display: 'block', height: '100%', width: `${pct}%`, borderRadius: 999, background: 'var(--color-accent-2)' }} />
                      </span>
                    </span>
                    <span className="tag tag-accent">{t('profile.resume')}</span>
                  </button>
                );
              })}
              {app.history.length === 0 && <div className="muted" style={{ fontSize: 13 }}>{t('profile.noHistory')}</div>}
            </div>
          ) : (
            <div style={{ marginTop: -4 }}>
              <CoverGrid novels={app.favs.map(getNovel)} onOpen={app.openDetail} showAuthor={false} cols={lay.libCols} gap="14px 12px" />
              {app.favs.length === 0 && <div className="muted" style={{ fontSize: 13 }}>{t('profile.noFavs')}</div>}
            </div>
          )}
        </div>

        <div style={{ gridArea: 'settings', borderRadius: 28, background: 'var(--color-surface)', padding: 6 }}>
          <button className="menu-row" onClick={() => app.setLangDialog({ mode: 'settings' })}>
            <LanguagesIcon style={{ color: 'var(--color-accent-2-700)' }} />
            <span style={{ flex: 1, fontSize: 14, fontWeight: 600 }}>{t('profile.defaultLang')}</span>
            <span className="muted" style={{ fontSize: 13 }}>{langLabel} ›</span>
          </button>
          <button className="menu-row" onClick={() => setUiLangOpen(true)}>
            <GlobeIcon style={{ color: 'var(--color-accent-2-700)' }} />
            <span style={{ flex: 1, fontSize: 14, fontWeight: 600 }}>{t('uiLang.label')}</span>
            <span className="muted" style={{ fontSize: 13 }}>{uiLangLabel(uiLang)} ›</span>
          </button>
          {user && (
            <button className="menu-row menu-row--danger" onClick={() => { app.logout(); app.showToast(t('auth.loggedOut')); }}>
              <LogOutIcon />
              <span style={{ flex: 1, fontSize: 14, fontWeight: 700 }}>{t('profile.logout')}</span>
            </button>
          )}
        </div>
      </div>
      {uiLangOpen && <UiLangDialog onClose={() => setUiLangOpen(false)} />}
    </div>
  );
}
