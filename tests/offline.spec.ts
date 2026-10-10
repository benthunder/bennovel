import { expect, test, type Page } from '@playwright/test';

// These tests never reach Supabase, so they also run where it is blocked.

/** Skips onboarding with an English interface; `signedIn` stores a signed-in reader. */
async function boot(page: Page, { signedIn = false } = {}) {
  await page.addInitScript(signedIn => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    if (signedIn) localStorage.setItem('bennovel.user', JSON.stringify({ name: 'Lina Hart', email: 'lina@example.com', provider: 'email' }));
  }, signedIn);
}

/** The device reports no connection; returns the Supabase requests the app still made. */
async function goOffline(page: Page) {
  await page.addInitScript(() => Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false }));
  const calls: string[] = [];
  await page.route('**/rest/v1/**', route => { calls.push(route.request().url()); return route.abort(); });
  return calls;
}

const novel = (id: number, title: string) => ({
  id, title, author: 'Mira Vale', cat: 'fantasy', status: 'Ongoing', chapters: 40, rating: 4.6, reads: 12, year: 2024,
  desc: { en: '', vi: '' }, cover: { bg: 'var(--color-accent-300)', fg: 'var(--color-accent-900)', deco: 'var(--color-accent-500)' }
});

/** Favourites and history saved earlier, with their novel snapshots. */
const SAVED = {
  favs: [101],
  history: [{ id: 102, ch: 3, at: Date.now() - 60_000 }],
  novels: [{ id: 101, data: novel(101, 'Saved Tale') }, { id: 102, data: novel(102, 'Read Tale') }]
};

/**
 * Stands in for the native app. `file`: the system hands it `book.txt` ("Open with"),
 * a book with two sections. `reading`: what the SQLite reading store holds.
 */
async function nativeApp(page: Page, opts: { file?: boolean; reading?: typeof SAVED | null }) {
  await page.addInitScript(({ file, reading }) => {
    let files = file ? ['/home/lina/book.txt'] : [];
    const book = { id: 7, format: 'txt', title: 'Offline Tales', sections: [{ label: 'Part one', pages: null }, { label: 'Part two', pages: null }], pageImages: false };
    const w = window as unknown as Record<string, unknown>;
    let next = 1;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        switch (cmd) {
          case 'take_opened_files': { const f = files; files = []; return f; }
          case 'book_open': return book;
          case 'book_section': return { index: args.index, paragraphs: [`Words of section ${Number(args.index) + 1}.`] };
          case 'library_list': return [];
          case 'reading_load': return reading;
          case 'reading_save': w.__saved = args.state; return null;
          default: return null;
        }
      }
    };
  }, { file: !!opts.file, reading: opts.reading ?? null });
}

test('without a connection the app skips online content', async ({ page }) => {
  const calls = await goOffline(page);
  await boot(page);
  await page.goto('/');
  // The app opens on the library.
  await expect(page.getByRole('heading', { name: 'Favourites' })).toBeVisible();
  await expect(page.getByText('You are offline').filter({ visible: true })).toBeVisible();
  await page.getByRole('button', { name: 'Home' }).filter({ visible: true }).first().click();
  await expect(page.getByText('You are offline').filter({ visible: true })).toBeVisible();
  await page.getByRole('button', { name: /^Categor(y|ies)$/ }).filter({ visible: true }).first().click();
  await expect(page.getByText('You are offline').filter({ visible: true })).toBeVisible();
  expect(calls).toEqual([]);
});

test('an unreachable server shows the offline note instead of blocking the app', async ({ page }) => {
  await page.route('**/rest/v1/**', route => route.abort());
  await boot(page);
  await page.goto('/');
  await expect(page.getByText('You are offline').filter({ visible: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try again' }).filter({ visible: true })).toBeVisible();
});

test('a book opened from the system goes straight to the reader, even offline', async ({ page }) => {
  const calls = await goOffline(page);
  await nativeApp(page, { file: true });
  await boot(page);
  await page.goto('/');
  await expect(page.getByText('Words of section 1.', { exact: true })).toBeVisible();
  expect(calls).toEqual([]);
});

test('guests use Profile offline and can log in from it', async ({ page }) => {
  await goOffline(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  // No sign-in wall: settings work without an account or a connection.
  await expect(page.getByText('Reading as a guest')).toBeVisible();
  await expect(page.getByRole('button', { name: /Default reading language/ })).toBeVisible();

  await page.getByRole('button', { name: 'Log in or register' }).click();
  await page.getByRole('textbox', { name: 'Email' }).fill('lina.hart@example.com');
  await page.getByLabel('Password').fill('secret123');
  await page.locator('form').getByRole('button', { name: 'Log in', exact: true }).click();
  await expect(page.getByText('lina.hart@example.com')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Log out' })).toBeVisible();
});

test('favourites and history from the SQLite store show offline', async ({ page }) => {
  await goOffline(page);
  await nativeApp(page, { reading: SAVED });
  await boot(page);
  await page.goto('/');
  // The app opens on the library: the saved novel is there without a connection.
  await expect(page.getByText('Saved Tale').filter({ visible: true }).first()).toBeVisible();
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Read Tale').filter({ visible: true }).first()).toBeVisible();
  await expect(page.getByText(/Chapter 3/).filter({ visible: true }).first()).toBeVisible();
  // Reading needs the chapter from the server.
  await page.getByText('Read Tale').filter({ visible: true }).first().click();
  await expect(page.getByRole('status').filter({ hasText: 'This needs an internet connection.' })).toBeVisible();
});

test('favourites and history are written back to the store with snapshots', async ({ page }) => {
  await goOffline(page);
  await nativeApp(page, { reading: SAVED });
  await boot(page);
  await page.goto('/');
  await expect(page.getByText('Saved Tale').filter({ visible: true }).first()).toBeVisible();
  const saved = await page.waitForFunction(() => (window as unknown as { __saved?: unknown }).__saved).then(h => h.jsonValue()) as typeof SAVED;
  expect(saved.favs).toEqual([101]);
  expect(saved.history.map(h => h.id)).toEqual([102]);
  expect(saved.novels.map(n => n.data?.title).sort()).toEqual(['Read Tale', 'Saved Tale']);
});

test('the web build keeps snapshots for offline too', async ({ page }) => {
  await goOffline(page);
  await boot(page);
  await page.addInitScript(saved => {
    localStorage.setItem('bennovel.favs', JSON.stringify(saved.favs));
    localStorage.setItem('bennovel.history', JSON.stringify(saved.history));
    localStorage.setItem('bennovel.novelCache', JSON.stringify(saved.novels));
  }, SAVED);
  await page.goto('/');
  await expect(page.getByText('Saved Tale').filter({ visible: true }).first()).toBeVisible();
});

test('signed-in readers see their profile', async ({ page }) => {
  await goOffline(page);
  await boot(page, { signedIn: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Lina Hart', { exact: true })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Email' })).toHaveCount(0);
});
