import { expect, test, type Page } from '@playwright/test';

// These tests never reach Supabase, so they also run where it is blocked.

/** English interface. */
async function boot(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
  });
}

/** The device reports no connection; returns the Supabase requests the app still made. */
async function goOffline(page: Page) {
  await page.addInitScript(() => Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false }));
  const calls: string[] = [];
  await page.route('**/rest/v1/**', route => { calls.push(route.request().url()); return route.abort(); });
  await page.route('**/auth/v1/**', route => route.abort());
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
async function nativeApp(page: Page, opts: { file?: boolean; reading?: typeof SAVED | null; books?: unknown[] }) {
  await page.addInitScript(({ file, reading, books }) => {
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
          case 'library_list': return books;
          case 'library_open': return { book, novelId: args.novelId, lang: 'en', index: 1, progress: 0 };
          case 'library_detail': return { book: (books as { novelId: number }[]).find(b => b.novelId === args.novelId), sections: book.sections };
          case 'library_update': w.__updated = args; return null;
          case 'reading_load': return reading;
          case 'reading_save': w.__saved = args.state; return null;
          default: return null;
        }
      }
    };
  }, { file: !!opts.file, reading: opts.reading ?? null, books: opts.books ?? [] });
}

test('without a connection the app skips online content', async ({ page }) => {
  const calls = await goOffline(page);
  await boot(page);
  await page.goto('/');
  // The app opens on the library.
  await expect(page.getByRole('heading', { name: 'Favourites' })).toBeVisible();
  await expect(page.getByText('You are offline').filter({ visible: true })).toBeVisible();
  // This version has no Home or Category page.
  await expect(page.getByRole('button', { name: 'Home' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Categor(y|ies)$/ })).toHaveCount(0);
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

test('Profile works without an account or a connection', async ({ page }) => {
  await goOffline(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Reading on this device')).toBeVisible();
  await expect(page.getByRole('button', { name: /Default reading language/ })).toBeVisible();
  // No sign-in anywhere in this version.
  await expect(page.getByRole('button', { name: /Log in/ })).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Email' })).toHaveCount(0);
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

test('history lists online novels and books on the device, newest first', async ({ page }) => {
  await goOffline(page);
  const imported = {
    novelId: 1, title: 'Device Diary', author: null, description: '', lang: 'en', format: 'txt', sourceName: 'diary.txt', chapterCount: 2,
    createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 1, progress: 0.5, lastReadAt: new Date().toISOString() }
  };
  await nativeApp(page, { reading: SAVED, books: [imported] });
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  const rows = page.locator('.novel-row').filter({ visible: true });
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText('Device Diary');
  await expect(rows.nth(0)).toContainText('On this device · Part 2 of 2');
  await expect(rows.nth(1)).toContainText('Read Tale');
  await expect(rows.nth(1)).toContainText('Online · Chapter 3');
  // The imported book opens offline, where it was left.
  await rows.nth(0).click();
  await expect(page.getByText('Words of section 2.', { exact: true })).toBeVisible();
});

test('a book on the device has a details page like an online novel', async ({ page }) => {
  await goOffline(page);
  const imported = {
    novelId: 1, title: 'Device Diary', author: null, description: '', lang: 'en', format: 'txt', sourceName: 'diary.txt', chapterCount: 2,
    createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 1, progress: 0.5, lastReadAt: new Date().toISOString() }
  };
  await nativeApp(page, { books: [imported] });
  await boot(page);
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  // Details, chapters and Continue, all without a connection.
  await expect(page.getByRole('heading', { name: 'Device Diary' })).toBeVisible();
  await expect(page.getByText('File: diary.txt')).toBeVisible();
  await expect(page.locator('.novel-row').filter({ visible: true })).toHaveCount(2);
  await expect(page.getByRole('button', { name: /Continue · Part 2 of 2/ }).filter({ visible: true })).toBeVisible();
  // Editing the details from here.
  await page.getByRole('button', { name: 'Edit details' }).click();
  await page.getByLabel('Author').fill('Lina Hart');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __updated?: { author: string } }).__updated?.author)).toBe('Lina Hart');
  // A chapter opens in the reader.
  await page.locator('.novel-row').filter({ visible: true }).first().click();
  await expect(page.getByText('Words of section 1.', { exact: true })).toBeVisible();
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
