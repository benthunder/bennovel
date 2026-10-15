import { expect, test, type Page } from '@playwright/test';

// The offline version: no accounts, no Home or Category, and backups to a file on
// Profile. The native app (the library file, src-tauri/src/library) is stood in for,
// so nothing real is saved. These tests never reach Supabase.

/** English interface, no connection to the catalog. */
async function boot(page: Page) {
  // Set once, so settings a restore brings back survive the restart.
  await page.addInitScript(() => {
    if (!localStorage.getItem('bennovel.uiLang')) localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
  });
  await page.route('**/rest/v1/**', route => route.abort());
}

/**
 * Stands in for the native app; the save and open dialogs pick `/home/mai/b.bnbackup`,
 * and the backup restored holds `settings`. Calls go to `window.__calls`.
 */
async function nativeApp(page: Page, settings: string | null = null) {
  await page.addInitScript(settings => {
    const w = window as unknown as Record<string, unknown>;
    const calls: { cmd: string; args: unknown }[] = [];
    w.__calls = calls;
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
          case 'plugin:dialog|save':
          case 'plugin:dialog|open': return '/home/mai/b.bnbackup';
          case 'library_restore': return settings;
          default: return null;
        }
      }
    };
  }, settings);
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: { cmd: string; args: unknown }[] }).__calls);

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

test('Profile backs the library up to a file and brings it back, with no sync or S3', async ({ page }) => {
  await boot(page);
  await nativeApp(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Backup & restore')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sync' })).toHaveCount(0);
  await expect(page.getByText(/S3/)).toHaveCount(0);

  await page.getByRole('button', { name: 'Create backup' }).click();
  await expect(page.getByText('Backup saved.')).toBeVisible();
  await page.getByRole('button', { name: 'Restore' }).click();
  await expect(page.getByText('Backup restored.')).toBeVisible();
  const done = await calls(page);
  const backup = done.find(c => c.cmd === 'library_backup')?.args as { user: string; path: string; settings: string };
  expect(backup).toMatchObject({ user: '', path: '/home/mai/b.bnbackup' });
  // The app's settings go into the backup with the library.
  expect(JSON.parse(backup.settings)).toMatchObject({ uiLang: 'en' });
  expect(done.find(c => c.cmd === 'library_restore')?.args).toMatchObject({ user: '', path: '/home/mai/b.bnbackup' });
});

test('a library from an earlier signed-in version keeps being used', async ({ page }) => {
  await boot(page);
  await page.addInitScript(() => localStorage.setItem('bennovel.user', JSON.stringify({ id: 'u1', name: 'Mai', email: 'mai@example.com', provider: 'email' })));
  await nativeApp(page);
  await page.goto('/');
  await expect.poll(async () => (await calls(page)).find(c => c.cmd === 'library_list')?.args).toMatchObject({ user: 'mai@example.com' });
});

test('restoring a backup brings back the app and reading settings', async ({ page }) => {
  await boot(page);
  await nativeApp(page, JSON.stringify({ uiLang: 'vi', defaultLang: 'vi', readerPrefs: { fontSize: 22, theme: 'night', lineH: 1.75, gap: 'normal', margin: 'normal', align: 'left' } }));
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Restore' }).click();
  // The app starts again in the backup's language and says so.
  await expect(page.getByText('Đã khôi phục bản sao lưu.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Cá nhân' }).filter({ visible: true }).first()).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('bennovel.readerPrefs')!).theme)).toBe('night');
  expect(await page.evaluate(() => localStorage.getItem('bennovel.defaultLang'))).toBe('"vi"');
});
