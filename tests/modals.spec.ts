import { expect, test, type Locator, type Page } from '@playwright/test';

// Every dialog and bottom sheet opens over the whole window, even when the page that
// opens it is long and scrolled down, and a tall one scrolls inside itself. The native
// side is stood in for with a long book on the device; nothing reaches a server.

const CHAPTERS = 60;
const imported = {
  novelId: 1, title: 'Long Diary', author: null, description: '', lang: 'en', format: 'txt', sourceName: 'long.txt', chapterCount: CHAPTERS,
  createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 0, progress: 0, lastReadAt: new Date().toISOString() }
};

async function nativeApp(page: Page) {
  await page.addInitScript(({ imported, n }) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const sections = Array.from({ length: n }, (_, i) => ({ label: `Part ${i + 1}`, pages: null }));
    const w = window as unknown as Record<string, unknown>;
    let next = 1;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string) => {
        switch (cmd) {
          case 'take_opened_files': return [];
          case 'library_list': return [imported];
          case 'library_detail': return { book: imported, sections };
          case 'llm_config_get': return { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', hasKey: true };
          case 'ai_marks': return [];
          case 'ai_progress': return [];
          default: return null;
        }
      }
    };
  }, { imported, n: CHAPTERS });
}

/** Scrolls the screen holding `el` to its bottom. */
const scrollDown = (page: Page, el: Locator) =>
  page.locator('.screen').filter({ has: el }).evaluate(s => { s.scrollTop = s.scrollHeight; });

/** The backdrop covers the whole app frame and the sheet sits fully inside the window. */
async function expectOnTop(page: Page, sheet: Locator) {
  await expect(sheet).toBeVisible();
  const vp = page.viewportSize()!;
  const frame = (await page.locator('.app').boundingBox())!;
  const backdrop = (await page.locator('.dialog-backdrop').boundingBox())!;
  expect(backdrop.y).toBe(0);
  expect(backdrop.height).toBeCloseTo(vp.height, 0);
  expect(backdrop.x).toBeCloseTo(frame.x, 0);
  expect(backdrop.width).toBeCloseTo(frame.width, 0);
  await expect(sheet).toBeInViewport({ ratio: 1 });
}

const shot = async (page: Page, name: string) => {
  await page.waitForTimeout(400); // let the slide-up finish
  await page.screenshot({ path: `test-results/modals/${test.info().project.name}-${name}.png` });
};

test.beforeEach(async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
});

test('chapter list and details open on top of a scrolled book page', async ({ page }) => {
  await page.getByText('Long Diary').filter({ visible: true }).first().click();
  const more = page.getByRole('button', { name: `Show more · ${CHAPTERS} chapters` });
  await scrollDown(page, more);
  await more.click();
  const toc = page.getByRole('dialog', { name: 'Chapters' });
  await shot(page, 'book-toc');
  await expectOnTop(page, toc);
  await page.locator('.dialog-backdrop').click({ position: { x: 5, y: 5 } });
  await expect(toc).toHaveCount(0);

  const add = page.getByRole('button', { name: 'Add a description' });
  await scrollDown(page, add);
  await add.evaluate(b => b.scrollIntoView({ block: 'end' }));
  await add.click();
  const edit = page.getByRole('dialog', { name: 'Book details' });
  await shot(page, 'book-edit');
  await expectOnTop(page, edit);
  await expect(edit.getByRole('button', { name: 'Save' })).toBeInViewport();
});

test('Profile sheets open on top when Profile is scrolled down', async ({ page }) => {
  await page.setViewportSize({ width: page.viewportSize()!.width, height: 560 });
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  const llmRow = page.getByRole('button', { name: /AI translation \(your LLM\)/ });
  await scrollDown(page, llmRow);

  await page.getByRole('button', { name: /App language/ }).click();
  const lang = page.getByRole('dialog', { name: 'App language' });
  await shot(page, 'profile-ui-lang');
  await expectOnTop(page, lang);
  await lang.getByRole('button', { name: 'Cancel' }).click();

  // The LLM sheet is taller than this window: it fits and scrolls inside itself.
  await llmRow.click();
  const llm = page.getByRole('dialog', { name: /AI translation/ });
  await shot(page, 'profile-llm');
  await expectOnTop(page, llm);
  const save = llm.getByRole('button', { name: 'Save' });
  await save.scrollIntoViewIfNeeded();
  await expect(save).toBeInViewport();
  await expect(llm).toBeInViewport({ ratio: 1 });
  await shot(page, 'profile-llm-scrolled');
});
