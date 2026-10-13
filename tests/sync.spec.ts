import { expect, test, type Page, type Route } from '@playwright/test';

// The Sync button on Profile. The native app (its library file and the sync itself,
// src-tauri/src/library/remote.rs) is stood in for, so nothing real is synced.

const UID = '7b0c2a52-1f4e-4c55-9a8e-2b8f9d1e0a11';
const EMAIL = 'mai.tran@example.com';

const b64 = (o: unknown) => Buffer.from(JSON.stringify(o)).toString('base64url');

function session() {
  const exp = Math.floor(Date.now() / 1000) + 3600;
  const user = {
    id: UID, aud: 'authenticated', role: 'authenticated', email: EMAIL, email_confirmed_at: '2026-10-10T00:00:00Z',
    app_metadata: { provider: 'email' }, user_metadata: { full_name: 'Mai Tran' }, identities: [{ id: 'i1', provider: 'email' }],
    created_at: '2026-10-10T00:00:00Z', updated_at: '2026-10-10T00:00:00Z'
  };
  const jwt = `${b64({ alg: 'HS256', typ: 'JWT' })}.${b64({ sub: UID, email: EMAIL, role: 'authenticated', aud: 'authenticated', exp })}.sig`;
  return { access_token: jwt, token_type: 'bearer', expires_in: 3600, expires_at: exp, refresh_token: 'refresh-1', user };
}

const novel = (id: number, title: string) => ({
  id, title, author: 'Mira Vale', cat: 'fantasy', status: 'Ongoing', chapters: 40, rating: 4.6, reads: 12, year: 2024,
  desc: { en: '', vi: '' }, cover: { bg: 'var(--color-accent-300)', fg: 'var(--color-accent-900)', deco: 'var(--color-accent-500)' }
});

/** Signed in as Mai, English interface, offline catalog. */
async function boot(page: Page) {
  await page.addInitScript(s => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    localStorage.setItem('bennovel.user', JSON.stringify({ id: s.user.id, name: 'Mai Tran', email: s.user.email, provider: 'email' }));
    localStorage.setItem('sb-sdryytjmgapekepotmer-auth-token', JSON.stringify(s));
  }, session());
}

/**
 * Stands in for the native app's library: it holds `Here Tale` as a favourite; a sync
 * brings in `There Tale` from another device, or fails with `fail`. Calls go to
 * `window.__calls`, the account each sync was given to `window.__accounts`.
 */
async function nativeApp(page: Page, opts: { saved?: Record<string, unknown>; refuse?: string; fail?: string; hold?: boolean } = {}) {
  await page.addInitScript(({ here, there, opts }) => {
    const w = window as unknown as Record<string, unknown>;
    const calls: string[] = [];
    const accounts: unknown[] = [];
    w.__calls = calls;
    w.__accounts = accounts;
    let next = 1;
    const callbacks = new Map<number, (m: unknown) => void>();
    /** With `hold`, a sync reports half its upload and waits for `window.__release()`. */
    let release = () => undefined as void;
    w.__release = () => release();
    let rev = 0;
    let favs = [here.id];
    let novels = [{ id: here.id, data: here }];
    let syncedAt: number | null = null;
    let storage = opts.saved ?? null;
    w.__s3 = () => storage;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (m: unknown) => void) => {
        callbacks.set(next, cb);
        return next++;
      },
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push(cmd);
        switch (cmd) {
          case 'reading_load': return { favs, history: [], novels, rev };
          case 'reading_save': {
            const s = (args as { state: { favs: number[]; rev: number } }).state;
            if (s.rev !== rev) return false;
            favs = s.favs;
            return true;
          }
          case 'library_list': return [];
          case 'take_opened_files': return [];
          case 'sync_status': return { syncedAt };
          case 's3_config_get': return storage && { ...storage, secretKey: undefined, hasSecret: true };
          case 's3_config_set': {
            const config = (args as { config: Record<string, string> | null }).config;
            if (config && opts.refuse) throw `cannot read book: ${opts.refuse}`;
            storage = config && { ...config, endpoint: `https://${config.endpoint}` };
            return null;
          }
          case 'library_sync': {
            accounts.push((args as { account: unknown }).account);
            if (opts.fail) throw `cannot read book: ${opts.fail}`;
            if (opts.hold) {
              const send = callbacks.get((args as { onProgress: { id: number } }).onProgress.id)!;
              send({ index: 0, message: { step: 'pack' } });
              send({ index: 1, message: { step: 'upload', done: 12 * 1024 * 1024, total: 30 * 1024 * 1024 } });
              await new Promise<void>(r => { release = r; });
            }
            favs = [...favs, there.id];
            novels = [...novels, { id: there.id, data: there }];
            rev++;
            syncedAt = Date.now();
            return syncedAt;
          }
          default: return null;
        }
      }
    };
  }, { here: novel(201, 'Here Tale'), there: novel(202, 'There Tale'), opts });
}

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/** Stands in for Supabase sign-in; returns any other Supabase request the page made. */
async function fakeSupabase(page: Page) {
  const calls: string[] = [];
  await page.route('**/auth/v1/**', route => json(route, 200, session().user));
  await page.route(/\/(rest|storage|functions)\/v1\//, route => {
    calls.push(`${route.request().method()} ${new URL(route.request().url()).pathname}`);
    return route.abort();
  });
  return calls;
}

const appCalls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: string[] }).__calls);
const accounts = (page: Page) => page.evaluate(() => (window as unknown as { __accounts: unknown[] }).__accounts);
const syncCalls = (calls: string[]) => calls.filter(c => /user_sync|commit_sync|storage|functions/.test(c));

async function openProfile(page: Page) {
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
}

test('sync goes through the BenNovel server with the session, and shows the merged lists', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  const server = await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await expect(page.getByText('Not synced yet')).toBeVisible();
  await expect(page.getByText('Sync to: BenNovel server')).toBeVisible();
  await page.locator('label.seg-opt', { hasText: 'Favourites' }).click();
  const cover = (title: string) => page.getByRole('button', { name: `${title} ${title}` });
  await expect(cover('Here Tale')).toBeVisible();
  await expect(cover('There Tale')).toHaveCount(0);

  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Library synced')).toBeVisible();
  await expect(page.getByText('Last synced: Just now')).toBeVisible();
  // The favourite from the other device shows without a restart.
  await expect(cover('There Tale')).toBeVisible();
  await expect(cover('Here Tale')).toBeVisible();

  // The native app gets the function and the session, never storage keys; the page
  // itself calls no storage.
  expect(await accounts(page)).toEqual([{
    url: 'https://sdryytjmgapekepotmer.supabase.co/functions/v1/sync',
    apiKey: expect.any(String),
    token: expect.stringMatching(/^eyJ.+\..+\.sig$/)
  }]);
  expect(syncCalls(server)).toEqual([]);
});

test('a sync shows how far its upload has come', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { hold: true });
  await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Uploading… 40% · 12 / 30 MB')).toBeVisible();
  await page.evaluate(() => (window as unknown as { __release: () => void }).__release());
  await expect(page.getByText('Library synced')).toBeVisible();
  await expect(page.getByText('Last synced: Just now')).toBeVisible();
});

test('a refused session asks to log in again', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { fail: 'sync-sign-in' });
  await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Log in again to sync.')).toBeVisible();
  await expect(page.getByText('Not synced yet')).toBeVisible();
});

test('a library too large for the server says so', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { fail: 'remote 413 too-large' });
  await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Your library is too large to sync.')).toBeVisible();
});

test('guests and the web build have no Sync button', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
  });
  await page.route('**/rest/v1/**', route => route.abort());
  await page.goto('/');
  await openProfile(page);
  await expect(page.getByText('Reading as a guest')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sync', exact: true })).toHaveCount(0);
});

const STORAGE = { endpoint: 'https://minio.example.com:9000', region: 'us-east-1', bucket: 'books', folder: 'bennovel', accessKey: 'mai' };

test('a guest can sync through their own S3 storage', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
  });
  await nativeApp(page);
  const server = await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await expect(page.getByText('No account? Sync through your own S3 storage (MinIO, RustFS…).')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sync', exact: true })).toHaveCount(0);

  await page.getByRole('button', { name: 'Use your own storage' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your S3 storage' });
  await dialog.getByLabel('Server address').fill('minio.example.com:9000');
  await dialog.getByLabel('Bucket', { exact: true }).fill('books');
  await dialog.getByLabel('Access key').fill('mai');
  await dialog.getByLabel('Secret key').fill('s3cret');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Storage saved. Tap Sync to sync with it.')).toBeVisible();
  await expect(dialog).toHaveCount(0);
  const saved = await page.evaluate(() => (window as unknown as { __s3: () => unknown }).__s3());
  expect(saved).toEqual({ endpoint: 'https://minio.example.com:9000', region: '', bucket: 'books', folder: 'bennovel', accessKey: 'mai', secretKey: 's3cret' });
  await expect(page.getByText('Sync to: books · minio.example.com:9000')).toBeVisible();

  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Library synced')).toBeVisible();
  await page.locator('label.seg-opt', { hasText: 'Favourites' }).click();
  await expect(page.getByRole('button', { name: 'There Tale There Tale' })).toBeVisible();
  // No account to send: the native app syncs with the saved storage.
  expect(await accounts(page)).toEqual([null]);
  expect(syncCalls(server)).toEqual([]);
});

test('a signed-in library set to its own storage shows it, and can go back to the server', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { saved: STORAGE });
  const server = await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await expect(page.getByText('Sync to: books · minio.example.com:9000')).toBeVisible();
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Library synced')).toBeVisible();
  expect(await appCalls(page)).toContain('library_sync');
  expect(syncCalls(server)).toEqual([]);

  // Back to the BenNovel server; the saved secret key never reached the page.
  await page.getByRole('button', { name: 'Storage settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your S3 storage' });
  await expect(dialog.getByLabel('Secret key')).toHaveValue('');
  await expect(dialog.getByLabel('Secret key')).toHaveAttribute('placeholder', 'Leave empty to keep the saved key');
  await dialog.getByRole('button', { name: 'Stop using it' }).click();
  await expect(page.getByText('Sync to: BenNovel server')).toBeVisible();
});

test('storage that refuses the keys is not saved', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { refuse: 's3-auth SignatureDoesNotMatch' });
  await fakeSupabase(page);
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Use your own storage' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your S3 storage' });
  await dialog.getByLabel('Server address').fill('minio.example.com');
  await dialog.getByLabel('Bucket', { exact: true }).fill('books');
  await dialog.getByLabel('Access key').fill('mai');
  await dialog.getByLabel('Secret key').fill('wrong');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog.getByRole('alert')).toHaveText('The server refused these keys.');
  await expect(page.getByText('Sync to: BenNovel server')).toBeVisible();
});
