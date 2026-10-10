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

/**
 * Stands in for the native app: the system hands it `book.txt` ("Open with"), and the
 * book has two sections.
 */
async function openedWithFile(page: Page) {
  await page.addInitScript(() => {
    let files = ['/home/lina/book.txt'];
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
          default: return null;
        }
      }
    };
  });
}

test('without a connection the app skips online content', async ({ page }) => {
  const calls = await goOffline(page);
  await boot(page);
  await page.goto('/');
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
  await openedWithFile(page);
  await boot(page);
  await page.goto('/');
  await expect(page.getByText('Words of section 1.', { exact: true })).toBeVisible();
  expect(calls).toEqual([]);
});

test('profile asks guests to log in first', async ({ page }) => {
  await goOffline(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  // The sign-in screen opens right away; skipping it leaves only the sign-in prompt.
  await expect(page.getByRole('textbox', { name: 'Email' })).toBeVisible();
  await page.getByRole('button', { name: 'Not now' }).click();
  await expect(page.getByText('Log in to see your profile')).toBeVisible();
  await expect(page.getByText('History', { exact: true })).toHaveCount(0);

  await page.getByRole('button', { name: 'Log in or register' }).click();
  await page.getByRole('textbox', { name: 'Email' }).fill('lina.hart@example.com');
  await page.getByLabel('Password').fill('secret123');
  await page.locator('form').getByRole('button', { name: 'Log in', exact: true }).click();
  await expect(page.getByText('lina.hart@example.com')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Log out' })).toBeVisible();
});

test('signed-in readers see their profile', async ({ page }) => {
  await goOffline(page);
  await boot(page, { signedIn: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await expect(page.getByText('Lina Hart', { exact: true })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Email' })).toHaveCount(0);
});
