import { useEffect, useState } from 'react';
import { GlobeIcon, LanguagesIcon, LogOutIcon, RefreshIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { UiLangDialog, uiLangLabel } from '../components/UiLangDialog';
import { CoverGrid, Segmented } from '../components/ui';
import { findNovel, getContentLangs, getNovels } from '../data/repository';
import { relativeTime, useI18n } from '../i18n';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import { canOpenLocalBooks, libraryUser, listLibrary, localCover, openLibraryBook, type LibraryBook } from '../lib/localBook';

export function ProfileScreen() {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const [tab, setTab] = useState<'history' | 'favs'>('history');
  const [uiLangOpen, setUiLangOpen] = useState(false);
  const user = app.user;
  const initials = user ? user.name.split(' ').map(w => w[0]).slice(0, 2).join('').toUpperCase() : 'G';
  const langLabel = getContentLangs().find(l => l.code === app.contentLang)?.native ?? t('lang.askEach');
  const rows = useHistoryRows();

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
          {app.canSync && <SyncCard />}
        </div>

        <Segmented value={tab} onChange={setTab} style={{ gridArea: 'tabs' }} optStyle={{ padding: 10 }}
          options={[{ label: t('profile.history'), value: 'history' }, { label: t('profile.favourites'), value: 'favs' }]} />

        <div style={{ gridArea: 'content', minWidth: 0 }}>
          {tab === 'history' ? (
            <div style={{ display: 'grid', gridTemplateColumns: lay.histCols, gap: '4px 12px', marginTop: -6 }}>
              {rows.map(r => (
                <button key={r.key} className="novel-row" style={{ alignItems: 'center', padding: 8, borderRadius: 22, minWidth: 0 }} onClick={r.open}>
                  <NovelCover novel={r.cover} w={52} h={74} fs={9} />
                  <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 5 }}>
                    <span style={{ fontWeight: 700, fontSize: 14 }}>{r.title}</span>
                    <span className="muted" style={{ fontSize: 12 }}>{r.where} · {r.place} · {relativeTime(t, r.at)}</span>
                    <span style={{ height: 6, borderRadius: 999, background: 'var(--color-neutral-300)', overflow: 'hidden' }}>
                      <span style={{ display: 'block', height: '100%', width: `${Math.max(4, Math.min(100, Math.round(r.pct * 100)))}%`, borderRadius: 999, background: 'var(--color-accent-2)' }} />
                    </span>
                  </span>
                  <span className="tag tag-accent">{t('profile.resume')}</span>
                </button>
              ))}
              {rows.length === 0 && <div className="muted" style={{ fontSize: 13 }}>{t('profile.noHistory')}</div>}
            </div>
          ) : (
            <div style={{ marginTop: -4 }}>
              <CoverGrid novels={getNovels(app.favs)} onOpen={app.openDetail} showAuthor={false} cols={lay.libCols} gap="14px 12px" />
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

/** The Sync button: brings this device's library and the account's copy together. */
function SyncCard() {
  const { t } = useI18n();
  const app = useApp();
  const status = app.syncing ? t('sync.syncing') : app.syncedAt ? t('sync.last', { time: relativeTime(t, app.syncedAt) }) : t('sync.never');
  return (
    <div style={{ marginTop: 12, borderRadius: 28, background: 'var(--color-surface)', padding: '12px 12px 12px 18px', display: 'flex', alignItems: 'center', gap: 12 }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 14, fontWeight: 700 }}>{status}</div>
        <div className="muted" style={{ fontSize: 12 }}>{t('sync.hint')}</div>
      </div>
      <button className="btn btn-primary" style={{ height: 40, padding: '0 16px', gap: 8, flex: 'none' }} disabled={app.syncing} onClick={app.syncNow}>
        <RefreshIcon size={16} style={app.syncing ? { animation: 'nxspin 1s linear infinite' } : undefined} />
        {t('sync.button')}
      </button>
    </div>
  );
}

interface HistoryRow {
  key: string;
  title: string;
  cover: { title: string; cover: { bg: string; fg: string; deco: string } };
  /** Online novel or a book on this device. */
  where: string;
  /** Chapter or part reached. */
  place: string;
  pct: number;
  at: number;
  open: () => void;
}

/**
 * Reading history, newest first: online novels (chapters load from the server when
 * opened) and books imported on this device, which open offline.
 */
function useHistoryRows(): HistoryRow[] {
  const { t } = useI18n();
  const app = useApp();
  const user = libraryUser(app.user);
  const [books, setBooks] = useState<LibraryBook[]>([]);
  useEffect(() => {
    if (!canOpenLocalBooks()) return;
    let gone = false;
    listLibrary(user).then(b => { if (!gone) setBooks(b); }, console.error);
    return () => { gone = true; };
  }, [user, app.libraryRev]);

  const online = app.history.flatMap((h): HistoryRow[] => {
    const n = findNovel(h.id);
    if (!n) return [];
    return [{
      key: `online:${h.id}`, title: n.title, cover: n, where: t('profile.online'), place: t('common.chapter', { n: h.ch }),
      pct: h.ch / Math.max(n.chapters, 1), at: h.at, open: () => app.openChapter(h.id, h.ch)
    }];
  });
  const local = books.flatMap((b): HistoryRow[] => {
    if (!b.lastRead) return [];
    const open = async () => {
      try {
        const { book, place } = await openLibraryBook(user, b.novelId);
        app.push({ s: 'book', book, place });
      } catch (e) {
        console.error(e);
        app.showToast(`${t('book.openFailed')} ${String(e)}`);
      }
    };
    return [{
      key: `local:${b.novelId}`, title: b.title, cover: { title: b.title, cover: localCover(b.title) }, where: t('library.device'),
      place: t('book.part', { n: b.lastRead.index + 1, total: b.chapterCount }),
      pct: (b.lastRead.index + 1) / Math.max(b.chapterCount, 1), at: Date.parse(b.lastRead.lastReadAt) || 0, open
    }];
  });
  return [...online, ...local].sort((a, b) => b.at - a.at);
}
