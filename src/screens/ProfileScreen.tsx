import { useEffect, useState } from 'react';
import { GlobeIcon, LanguagesIcon, SparklesIcon } from '../components/Icons';
import { Modal } from '../components/Modal';
import { NovelCover } from '../components/NovelCover';
import { UiLangDialog, uiLangLabel } from '../components/UiLangDialog';
import { CoverGrid, Segmented, Spinner } from '../components/ui';
import { findNovel, getContentLangs, getNovels } from '../data/repository';
import { relativeTime, useI18n } from '../i18n';
import type { MessageKey } from '../i18n/en';
import { AiFailure, LLM_PROVIDERS, aiErrorDetail, getLlmSettings, setLlmConfig, type LlmConfig, type LlmSettings } from '../lib/ai';
import { backupToS3, getS3Backup, restoreFromS3, s3Host, setS3Config, S3Failure, type S3Config } from '../lib/s3';
import { useAsync } from '../lib/useAsync';
import { createBackup, restoreBackup } from '../lib/backup';
import { useApp } from '../store/AppStore';
import { useLayout } from '../lib/useLayout';
import { canOpenLocalBooks, listLibrary, localCover, openLibraryBook, type LibraryBook } from '../lib/localBook';

export function ProfileScreen() {
  const { t, uiLang } = useI18n();
  const app = useApp();
  const lay = useLayout();
  const [tab, setTab] = useState<'history' | 'favs'>('history');
  const [uiLangOpen, setUiLangOpen] = useState(false);
  const [llmOpen, setLlmOpen] = useState(false);
  const [llmRev, setLlmRev] = useState(0);
  const llmQ = useAsync(() => (canOpenLocalBooks() ? getLlmSettings(app.libKey) : Promise.resolve(null)), [app.libKey, llmRev]);
  const llm = llmQ.status === 'ready' ? llmQ.data : null;
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
          <div style={{ borderRadius: 34, background: 'var(--color-surface)', padding: 22, display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div className="display" style={{ fontSize: 22 }}>{t('profile.offlineTitle')}</div>
            <div style={{ fontSize: 13, color: 'var(--color-neutral-800)' }}>{t('profile.offlineBody')}</div>
          </div>
          {canOpenLocalBooks() && <BackupCard />}
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
          {canOpenLocalBooks() && (
            <button className="menu-row" onClick={() => setLlmOpen(true)}>
              <SparklesIcon style={{ color: 'var(--color-accent-2-700)' }} />
              <span style={{ flex: 1, fontSize: 14, fontWeight: 600 }}>{t('ai.settings')}</span>
              <span className="muted" style={{ fontSize: 13, maxWidth: '45%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{llm ? llm.model : t('ai.notSet')} ›</span>
            </button>
          )}
        </div>
      </div>
      {uiLangOpen && <UiLangDialog onClose={() => setUiLangOpen(false)} />}
      {llmOpen && <LlmDialog saved={llm} onClose={() => setLlmOpen(false)} onSaved={() => setLlmRev(n => n + 1)} />}
    </div>
  );
}

type BackupJob = 'file' | 'fileRestore' | 's3' | 's3Restore';

/**
 * Backups of the whole library (books with their AI translations, history, favourites,
 * downloads): to a file the user keeps, or to their own S3 storage, and back.
 */
function BackupCard() {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const [busy, setBusy] = useState<BackupJob | null>(null);
  const [editing, setEditing] = useState(false);
  const [infoRev, setInfoRev] = useState(0);
  const infoQ = useAsync(() => (app.s3 ? getS3Backup(user) : Promise.resolve(null)), [user, app.s3, infoRev]);

  const run = async (job: BackupJob) => {
    setBusy(job);
    try {
      if (job === 'file') {
        if (await createBackup(user)) app.showToast(t('backup.done'));
      } else if (job === 'fileRestore') {
        if (await restoreBackup(user)) {
          app.reloadLibrary();
          app.showToast(t('backup.restored'));
        }
      } else if (job === 's3') {
        await backupToS3(user);
        setInfoRev(n => n + 1);
        app.showToast(t('s3.backedUp'));
      } else {
        await restoreFromS3(user);
        app.reloadLibrary();
        app.showToast(t('backup.restored'));
      }
    } catch (e) {
      if (e instanceof S3Failure) app.showToast(t(e.key));
      else {
        console.error(e);
        app.showToast(`${t(job === 'file' ? 'backup.failed' : 'backup.restoreFailed')} ${String(e)}`);
      }
    } finally {
      setBusy(null);
    }
  };
  const button = (job: BackupJob, label: string, disabled = false) => (
    <button className="btn btn-ghost" style={{ fontSize: 12, padding: '0 6px', gap: 6 }} disabled={!!busy || disabled} onClick={() => void run(job)}>
      {busy === job && <Spinner size={12} />}{label}
    </button>
  );

  const info = infoQ.status === 'ready' ? infoQ.data : null;
  const modified = info?.modified ? Date.parse(info.modified) : NaN;
  const s3Status = !app.s3 ? t('s3.none')
    : infoQ.status === 'loading' ? t('s3.checking')
      : infoQ.status === 'failed' ? t(infoQ.error instanceof S3Failure ? infoQ.error.key : 's3.err.generic')
        : info ? (Number.isNaN(modified) ? t('s3.hasBackup') : t('s3.last', { time: relativeTime(t, modified) })) : t('s3.noBackup');

  return (
    <div style={{ marginTop: 12, borderRadius: 28, background: 'var(--color-surface)', padding: '14px 12px 10px 18px', display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ fontSize: 14, fontWeight: 700 }}>{t('backup.title')}</div>
      <div className="muted" style={{ fontSize: 12 }}>{t('backup.hint')}</div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, borderTop: '1px solid var(--color-neutral-200)', paddingTop: 6 }}>
        <span style={{ flex: 1, minWidth: 0, fontSize: 12, fontWeight: 600 }}>{t('backup.file')}</span>
        {button('file', t('backup.create'))}
        {button('fileRestore', t('backup.restore'))}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2, borderTop: '1px solid var(--color-neutral-200)', paddingTop: 6 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ flex: 1, minWidth: 0, fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {app.s3 ? t('s3.where', { where: `${app.s3.bucket} · ${s3Host(app.s3)}` }) : t('s3.label')}
          </span>
          <button className="btn btn-ghost" style={{ fontSize: 12, padding: '0 6px' }} disabled={!!busy} onClick={() => setEditing(true)}>
            {app.s3 ? t('s3.change') : t('s3.setUp')}
          </button>
        </div>
        <span className="muted" style={{ fontSize: 12 }}>{s3Status}</span>
        {app.s3 && (
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            {button('s3', t('s3.backup'))}
            {button('s3Restore', t('s3.restore'), !info)}
          </div>
        )}
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
  const user = app.libKey;
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
      setError(e instanceof S3Failure ? e.key : 's3.err.generic');
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
    <Modal onClose={busy ? undefined : onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('s3.title')} onClick={e => e.stopPropagation()}
        onSubmit={e => { e.preventDefault(); void apply(form); }}>
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
    </Modal>
  );
}

/**
 * The user's own LLM for AI translation of books on this device: a provider (or any
 * OpenAI-compatible base URL), the model name and the key. Saved only once the API
 * answered; kept on this device, never synced.
 */
function LlmDialog({ saved, onClose, onSaved }: { saved: LlmSettings | null; onClose: () => void; onSaved: () => void }) {
  const { t } = useI18n();
  const app = useApp();
  const user = app.libKey;
  const [form, setForm] = useState<LlmConfig>(() => ({
    provider: saved?.provider ?? 'openai',
    baseUrl: saved?.baseUrl ?? LLM_PROVIDERS[0].baseUrl,
    model: saved?.model ?? LLM_PROVIDERS[0].model,
    apiKey: ''
  }));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const custom = form.provider === 'custom';
  // The key stays if the provider is the same; another provider needs its own.
  const keepsKey = !!saved?.hasKey && saved.provider === form.provider && saved.baseUrl === form.baseUrl;

  const pick = (id: string) => {
    const p = LLM_PROVIDERS.find(x => x.id === id)!;
    setForm(f => ({
      ...f, provider: id,
      baseUrl: id === 'custom' ? (f.provider === 'custom' ? f.baseUrl : '') : p.baseUrl,
      model: id === saved?.provider ? saved.model : p.model
    }));
  };

  const apply = async (config: LlmConfig | null) => {
    setBusy(true);
    setError(null);
    try {
      await setLlmConfig(user, config);
      app.showToast(t(config ? 'ai.saved' : 'ai.removed'));
      onSaved();
      onClose();
    } catch (e) {
      const f = e instanceof AiFailure ? e : null;
      const detail = f && f.key !== 'ai.err.network' ? aiErrorDetail(f.message) : '';
      setError(`${t(f?.key ?? 'ai.err.generic')}${detail ? ` (${detail})` : ''}`);
    } finally {
      setBusy(false);
    }
  };

  const input = (key: keyof LlmConfig, label: string, props: React.InputHTMLAttributes<HTMLInputElement> = {}) => (
    <div className="field">
      <label htmlFor={`llm-${key}`}>{label}</label>
      <input id={`llm-${key}`} className="input" style={{ height: 40 }} value={form[key]} autoCapitalize="off" autoCorrect="off" spellCheck={false}
        onChange={e => setForm(f => ({ ...f, [key]: e.target.value }))} {...props} />
    </div>
  );

  return (
    <Modal onClose={busy ? undefined : onClose}>
      <form className="dialog dialog--sheet" role="dialog" aria-modal="true" aria-label={t('ai.title')} onClick={e => e.stopPropagation()}
        onSubmit={e => { e.preventDefault(); void apply(form); }}>
        <div className="dialog-title" style={{ fontSize: 22 }}>{t('ai.title')}</div>
        <div className="dialog-body" style={{ marginTop: -6 }}>{t('ai.body')}</div>
        <div>
          <div style={{ fontSize: 12, fontWeight: 700, marginBottom: 6 }}>{t('ai.provider')}</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
            {LLM_PROVIDERS.map(p => (
              <button key={p.id} type="button" className={`chip chip--outline${form.provider === p.id ? ' chip--active' : ''}`} style={{ height: 36 }}
                aria-pressed={form.provider === p.id} onClick={() => pick(p.id)}>{p.id === 'custom' ? t('ai.custom') : p.name}</button>
            ))}
          </div>
        </div>
        {custom
          ? input('baseUrl', t('ai.baseUrl'), { placeholder: 'http://localhost:11434/v1', inputMode: 'url', required: true })
          : <div className="muted" style={{ fontSize: 12, overflowWrap: 'anywhere' }}>{t('ai.baseUrl')}: {form.baseUrl}</div>}
        {custom && <div className="muted" style={{ fontSize: 11, marginTop: -8 }}>{t('ai.baseUrlHint')}</div>}
        {input('model', t('ai.model'), { required: true, placeholder: LLM_PROVIDERS.find(p => p.id === form.provider)?.model || 'llama3.1' })}
        {input('apiKey', t('ai.apiKey'), {
          type: 'password', autoComplete: 'new-password', required: !custom && !keepsKey,
          placeholder: keepsKey ? t('ai.keyKept') : custom ? t('ai.keyOptional') : undefined
        })}
        {error && <div role="alert" style={{ fontSize: 13, color: 'var(--color-accent-700)' }}>{error}</div>}
        <div className="dialog-actions">
          {saved && <button type="button" className="btn btn-ghost" style={{ marginRight: 'auto' }} disabled={busy} onClick={() => void apply(null)}>{t('ai.remove')}</button>}
          <button type="button" className="btn btn-secondary" style={{ height: 44, padding: '0 18px' }} disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
          <button type="submit" className="btn btn-primary" style={{ height: 44, padding: '0 22px' }} disabled={busy}>
            {busy ? t('ai.checking') : t('common.save')}
          </button>
        </div>
      </form>
    </Modal>
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
  const user = app.libKey;
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
