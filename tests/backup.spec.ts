import { expect, test, type Page } from '@playwright/test';

// Backups to the user's own S3 storage, on Profile. The native app (the library file,
// the S3 requests, src-tauri/src/library/s3.rs) is stood in for, so nothing real is
// stored. These tests never reach Supabase.

/** English interface, no connection to the catalog. */
async function boot(page: Page) {
  await page.addInitScript(() => localStorage.setItem('bennovel.uiLang', JSON.stringify('en')));
  await page.route('**/rest/v1/**', route => route.abort());
}

/**
 * Stands in for the native app. `saved`: S3 settings already set; `stored`: a backup
 * already in the bucket; `refuse`: the error word saving settings fails with. Calls
 * go to `window.__calls`.
 */
async function nativeApp(page: Page, opts: { saved?: boolean; stored?: boolean; refuse?: string } = {}) {
  await page.addInitScript(opts => {
    const w = window as unknown as Record<string, unknown>;
    const calls: { cmd: string; args: unknown }[] = [];
    w.__calls = calls;
    let storage: Record<string, unknown> | null = opts.saved
      ? { endpoint: 'https://s3.example.com', region: 'us-east-1', bucket: 'books', folder: 'bennovel', accessKey: 'AK', hasSecret: true }
      : null;
    let backup: { size: number; modified: string } | null = opts.stored ? { size: 2048, modified: new Date(Date.now() - 2 * 3600_000).toUTCString() } : null;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case 'reading_load': return { favs: [], history: [], novels: [], rev: 0 };
          case 'library_list': return [];
          case 'take_opened_files': return [];
          case 's3_config_get': return storage;
          case 's3_config_set': {
            const config = args.config as Record<string, string> | null;
            if (config && opts.refuse) throw `cannot read book: ${opts.refuse}`;
            storage = config && { ...config, endpoint: `https://${config.endpoint}`, hasSecret: true };
            return null;
          }
          case 's3_backup_info': return backup;
          case 's3_backup':
            backup = { size: 4096, modified: new Date().toUTCString() };
            return backup;
          case 's3_restore':
            if (!backup) throw 'cannot read book: s3-no-backup';
            return null;
          default: return null;
        }
      }
    };
  }, opts);
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: { cmd: string; args: unknown }[] }).__calls);
const cmds = async (page: Page) => (await calls(page)).map(c => c.cmd);

async function openProfile(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
}

test('the app opens on the library, with no Home, Category or sign-in', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Favourites' })).toBeVisible();
  for (const name of [/^Library/, /^Profile$/]) {
    await expect(page.getByRole('button', { name }).filter({ visible: true }).first()).toBeVisible();
  }
  await expect(page.getByRole('button', { name: 'Home' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Categor(y|ies)$/ })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Log in/ })).toHaveCount(0);
});

test('Profile has backup and restore, and no sync', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  await openProfile(page);
  await expect(page.getByText('Backup & restore')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create backup' })).toBeVisible();
  await expect(page.getByText('Back up to S3 storage you run (MinIO, RustFS…).')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sync' })).toHaveCount(0);
  // Backing up to S3 waits for the storage to be set up.
  await expect(page.getByRole('button', { name: 'Back up to S3' })).toHaveCount(0);
});

test('S3 storage is set up, then the library is backed up there and brought back', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  await openProfile(page);
  await page.getByRole('button', { name: 'Set up' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your S3 storage' });
  await dialog.getByLabel('Server address').fill('s3.example.com');
  await dialog.getByLabel('Bucket', { exact: true }).fill('books');
  await dialog.getByLabel('Access key').fill('AK');
  await dialog.getByLabel('Secret key').fill('SK');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Storage saved.')).toBeVisible();
  await expect(page.getByText('S3: books · s3.example.com')).toBeVisible();
  await expect(page.getByText('No backup in this storage yet.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Restore from S3' })).toBeDisabled();

  await page.getByRole('button', { name: 'Back up to S3' }).click();
  await expect(page.getByText('Backed up to S3.')).toBeVisible();
  await expect(page.getByText('Last backup: Just now')).toBeVisible();

  await page.getByRole('button', { name: 'Restore from S3' }).click();
  await expect(page.getByText('Backup restored.')).toBeVisible();
  const done = await calls(page);
  expect(done.find(c => c.cmd === 's3_backup')?.args).toMatchObject({ user: '' });
  expect(done.find(c => c.cmd === 's3_restore')?.args).toMatchObject({ user: '' });
  // A restore lists the library again.
  expect((await cmds(page)).filter(c => c === 'library_list').length).toBeGreaterThan(1);
});

test('a backup already in the storage shows when it was made', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { saved: true, stored: true });
  await openProfile(page);
  await expect(page.getByText('Last backup: 2h ago')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Restore from S3' })).toBeEnabled();
});

test('storage that refuses the keys is not saved', async ({ page }) => {
  await boot(page);
  await nativeApp(page, { refuse: 's3-auth SignatureDoesNotMatch' });
  await openProfile(page);
  await page.getByRole('button', { name: 'Set up' }).click();
  const dialog = page.getByRole('dialog', { name: 'Your S3 storage' });
  await dialog.getByLabel('Server address').fill('s3.example.com');
  await dialog.getByLabel('Bucket', { exact: true }).fill('books');
  await dialog.getByLabel('Access key').fill('AK');
  await dialog.getByLabel('Secret key').fill('wrong');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog.getByRole('alert')).toHaveText('The server refused these keys.');
});

test('a library from an earlier signed-in version keeps being used', async ({ page }) => {
  await boot(page);
  await page.addInitScript(() => localStorage.setItem('bennovel.user', JSON.stringify({ id: 'u1', name: 'Mai', email: 'mai@example.com', provider: 'email' })));
  await nativeApp(page);
  await page.goto('/');
  await expect.poll(async () => (await calls(page)).find(c => c.cmd === 'library_list')?.args).toMatchObject({ user: 'mai@example.com' });
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Mai', { exact: true })).toHaveCount(0);
});
