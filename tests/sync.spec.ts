import { expect, test, type Page, type Route } from '@playwright/test';

// The Sync button on Profile. The native app (its library file) and Supabase (the
// user_sync table, commit_sync and Storage) are stood in for, so nothing real is synced.

const UID = '7b0c2a52-1f4e-4c55-9a8e-2b8f9d1e0a11';
const EMAIL = 'mai.tran@example.com';
const OLD_OBJECT = `${UID}/library-old.sqlite.gz`;

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

/** Signed in as Mai, English interface, offline catalog (only Storage and user_sync answer). */
async function boot(page: Page) {
  await page.addInitScript(s => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    localStorage.setItem('bennovel.user', JSON.stringify({ id: s.user.id, name: 'Mai Tran', email: s.user.email, provider: 'email' }));
    localStorage.setItem('sb-sdryytjmgapekepotmer-auth-token', JSON.stringify(s));
  }, session());
}

/**
 * Stands in for the native app's library: before a merge it holds `Here Tale` as a
 * favourite; merging the downloaded file adds `There Tale`. Calls go to `window.__calls`.
 */
async function nativeApp(page: Page) {
  await page.addInitScript(({ here, there }) => {
    const w = window as unknown as Record<string, unknown>;
    const calls: string[] = [];
    w.__calls = calls;
    let next = 1;
    let rev = 0;
    let favs = [here.id];
    let novels = [{ id: here.id, data: here }];
    let lastVersion = 0;
    let syncedAt: number | null = null;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown> | Uint8Array, opts?: { headers?: Record<string, string> }) => {
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
          case 'sync_status': return { lastVersion, syncedAt };
          case 'sync_merge': {
            calls.push(`merge ${(args as Uint8Array).length} bytes for ${decodeURIComponent(opts?.headers?.['x-user'] ?? '')}`);
            favs = [...favs, there.id];
            novels = [...novels, { id: there.id, data: there }];
            rev++;
            return null;
          }
          case 'sync_mark': {
            lastVersion = (args as { version: number }).version;
            syncedAt = Date.now();
            return null;
          }
          case 'sync_export': return new Uint8Array([31, 139, 8, 0]).buffer;
          default: return null;
        }
      }
    };
  }, { here: novel(201, 'Here Tale'), there: novel(202, 'There Tale') });
}

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * Stands in for Supabase: the account's file is at version `version` (0 = none).
 * `conflicts`: how many commits another device wins first. `uploadStatus`: Storage's
 * answer to uploads. Returns the requests made, as "METHOD path".
 */
async function fakeSupabase(page: Page, opts: { version: number; conflicts?: number; uploadStatus?: number }) {
  const calls: string[] = [];
  let { version } = opts;
  let conflicts = opts.conflicts ?? 0;
  await page.route('**/auth/v1/**', route => json(route, 200, session().user));
  await page.route('**/rest/v1/**', async route => {
    const req = route.request();
    const url = new URL(req.url());
    calls.push(`${req.method()} ${url.pathname}`);
    if (url.pathname.endsWith('/user_sync')) return json(route, 200, version ? [{ version, object: OLD_OBJECT }] : []);
    if (url.pathname.endsWith('/rpc/commit_sync')) {
      const body = req.postDataJSON() as { expected: number; new_object: string; new_size: number };
      calls.push(`commit ${body.expected} ${body.new_object.startsWith(`${UID}/library-`)} ${body.new_size}`);
      if (conflicts > 0) {
        conflicts--;
        version++;
        return json(route, 200, null);
      }
      version = body.expected + 1;
      return json(route, 200, version);
    }
    return route.abort();
  });
  await page.route('**/storage/v1/**', async route => {
    const req = route.request();
    const url = new URL(req.url());
    const path = decodeURIComponent(url.pathname.replace('/storage/v1/object/', ''));
    if (req.method() === 'DELETE') {
      calls.push(`remove ${(req.postDataJSON() as { prefixes: string[] }).prefixes.join(',')}`);
      return json(route, 200, []);
    }
    calls.push(`${req.method()} ${path}`);
    if (req.method() === 'GET') return route.fulfill({ status: 200, contentType: 'application/gzip', body: Buffer.from([31, 139, 8, 0, 1, 2]) });
    const status = opts.uploadStatus ?? 200;
    if (status !== 200) return json(route, status, { statusCode: String(status), error: 'Payload too large', message: 'The object exceeded the maximum allowed size' });
    return json(route, 200, { Key: `user-sync/${path}`, Id: 'obj-1' });
  });
  return calls;
}

async function openProfile(page: Page) {
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
}

test('sync merges the account copy, uploads the result and shows it', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  const server = await fakeSupabase(page, { version: 2 });
  await page.goto('/');
  await openProfile(page);
  await expect(page.getByText('Not synced yet')).toBeVisible();
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

  expect(server).toContain(`GET user-sync/${OLD_OBJECT}`);
  expect(server).toContain('commit 2 true 4');
  expect(server).toContain(`remove ${OLD_OBJECT}`);
  const app = await page.evaluate(() => (window as unknown as { __calls: string[] }).__calls);
  expect(app).toContain(`merge 6 bytes for ${EMAIL}`);
  expect(app.indexOf('sync_export')).toBeGreaterThan(app.indexOf('sync_merge'));
});

test('a first sync uploads without downloading', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  const server = await fakeSupabase(page, { version: 0 });
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Library synced')).toBeVisible();
  expect(server.filter(c => c.startsWith('GET user-sync/'))).toEqual([]);
  expect(server).toContain('commit 0 true 4');
});

test('when another device syncs first, the upload is dropped and the sync retried', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  const server = await fakeSupabase(page, { version: 1, conflicts: 1 });
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Library synced')).toBeVisible();
  // Lost the first commit, merged the newer file, then committed on top of it.
  expect(server.filter(c => c.startsWith('commit'))).toEqual(['commit 1 true 4', 'commit 2 true 4']);
  expect(server.filter(c => c.startsWith('remove')).length).toBe(2);
});

test('a library too large for Storage says so', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  const server = await fakeSupabase(page, { version: 0, uploadStatus: 413 });
  await page.goto('/');
  await openProfile(page);
  await page.getByRole('button', { name: 'Sync', exact: true }).click();
  await expect(page.getByText('Your library is too large to sync.')).toBeVisible();
  await expect(page.getByText('Not synced yet')).toBeVisible();
  expect(server.filter(c => c.startsWith('commit'))).toEqual([]);
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
