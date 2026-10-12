import { useEffect, useState } from 'react';
import { GlobeIcon, LanguagesIcon, LogOutIcon, RefreshIcon } from '../components/Icons';
import { NovelCover } from '../components/NovelCover';
import { UiLangDialog, uiLangLabel } from '../components/UiLangDialog';
import { CoverGrid, Segmented } from '../components/ui';
import { findNovel, getContentLangs, getNovels } from '../data/repository';
import { relativeTime, useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import { s3Host, setS3Config, type S3Config } from '../lib/s3';
import { SyncFailure } from '../lib/sync';
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
          {canOpenLocalBooks() && <SyncCard />}
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

/**
 * The Sync button: brings this device's library and its synced copy together, in the
 * account's storage or the user's own S3 storage.
 */
function SyncCard() {
  const { t } = useI18n();
  const app = useApp();
  const [editing, setEditing] = useState(false);
  const status = app.syncing ? t('sync.syncing') : app.syncedAt ? t('sync.last', { time: relativeTime(t, app.syncedAt) }) : t('sync.never');
  const where = app.s3 ? `${app.s3.bucket} · ${s3Host(app.s3)}` : app.builtinS3 ? t('s3.server') : t('s3.account');
  return (
    <div style={{ marginTop: 12, borderRadius: 28, background: 'var(--color-surface)', padding: '12px 12px 10px 18px', display: 'flex', flexDirection: 'column', gap: 6 }}>
      {app.canSync ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 14, fontWeight: 700 }}>{status}</div>
            <div className="muted" style={{ fontSize: 12 }}>{t('sync.hint')}</div>
          </div>
          <button className="btn btn-primary" style={{ height: 40, padding: '0 16px', gap: 8, flex: 'none' }} disabled={app.syncing} onClick={app.syncNow}>
            <RefreshIcon size={16} style={app.syncing ? { animation: 'nxspin 1s linear infinite' } : undefined} />
            {t('sync.button')}
          </button>
        </div>
      ) : (
        <div className="muted" style={{ fontSize: 12 }}>{t('s3.guestHint')}</div>
      )}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        {app.canSync && <span className="muted" style={{ flex: 1, minWidth: 0, fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{t('s3.via', { where })}</span>}
        <button className="btn btn-ghost" style={{ fontSize: 12, padding: '0 6px', marginLeft: 'auto' }} disabled={app.syncing} onClick={() => setEditing(true)}>
          {app.s3 ? t('s3.change') : t('s3.setUp')}
        </button>
      </div>
      {editing && <S3Dialog onClose={() => setEditing(false)} />}
    </div>
  );
}

type S3Field = keyof S3Config;

/** Settings of the user's own S3 storage; saved only once the server accepts them. */
function S3Dialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const app = useApp();
  const user = libraryUser(app.user);
  const saved = app.s3;
  const [form, setForm] = useState<S3Config>({
    endpoint: saved?.endpoint ?? '', region: saved?.region ?? '', bucket: saved?.bucket ?? '',
    folder: saved?.folder ?? 'bennovel', accessKey: saved?.accessKey ?? '', secretKey: ''
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<MessageKey | null>(null);

  const apply = async (config: S3Config | null) => {
    setBusy(true);
    setError(null);
    try {
      await setS3Config(user, config);
      await app.refreshS3();
      app.showToast(t(config ? 's3.saved' : 's3.removed'));
      onClose();
    } catch (e) {
      setError(e instanceof SyncFailure ? e.key : 'sync.err.generic');
    } finally {
      setBusy(false);
    }
  };

  const field = (key: S3Field, label: string, props: React.InputHTMLAttributes<HTMLInputElement> = {}) => (
    <div className="field">
      <label htmlFor={`s3-${key}`}>{label}</label>
      <input id={`s3-${key}`} className="input" style={{ height: 40 }} value={form[key]} autoCapitalize="off" autoCorrect="off" spellCheck={false}
        onChange={e => setForm(f => ({ ...f, [key]: e.target.value }))} {...props} />
    </div>
  );

  return (
    <div className="dialog-backdrop dialog-backdrop--sheet" onClick={busy ? undefined : onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('s3.title')} onClick={e => e.stopPropagation()}
        onSubmit={e => { e.preventDefault(); void apply(form); }} style={{ maxHeight: '90dvh', overflowY: 'auto' }}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('s3.title')}</div>
        <div className="dialog-body" style={{ marginTop: -6 }}>{t('s3.body')}</div>
        {field('endpoint', t('s3.endpoint'), { placeholder: 'https://minio.example.com:9000', inputMode: 'url', required: true })}
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
          {field('bucket', t('s3.bucket'), { required: true })}
          {field('region', t('s3.region'), { placeholder: 'us-east-1' })}
        </div>
        {field('folder', t('s3.folder'))}
        {field('accessKey', t('s3.accessKey'), { required: true, autoComplete: 'off' })}
        {field('secretKey', t('s3.secretKey'), {
          type: 'password', autoComplete: 'new-password', required: !saved?.hasSecret,
          placeholder: saved?.hasSecret ? t('s3.secretKept') : undefined
        })}
        {error && <div role="alert" style={{ fontSize: 13, color: 'var(--color-accent-700)' }}>{t(error)}</div>}
        <div className="dialog-actions">
          {saved && <button type="button" className="btn btn-ghost" style={{ marginRight: 'auto' }} disabled={busy} onClick={() => void apply(null)}>{t('s3.remove')}</button>}
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
          <button type="submit" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={busy}>
            {busy ? t('s3.checking') : t('common.save')}
          </button>
        </div>
      </form>
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
