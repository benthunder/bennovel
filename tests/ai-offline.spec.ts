import { expect, test, type Page } from '@playwright/test';

// AI translation of books on the device with the user's own LLM. The native side
// (src-tauri/src/library/llm.rs, translate.rs, queue.rs) is stood in for here; it
// has its own Rust tests. These never reach Supabase.

const imported = {
  novelId: 1, title: 'Device Diary', author: null, description: '', lang: 'en', format: 'txt', sourceName: 'diary.txt', chapterCount: 2,
  createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 0, progress: 0, lastReadAt: new Date().toISOString() }
};

type Calls = { cmd: string; args: Record<string, unknown> }[];

async function nativeApp(page: Page, { llm = true } = {}) {
  await page.addInitScript(({ imported, llm }) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const book = { id: 7, format: 'txt', title: 'Device Diary', sections: [{ label: 'Part one', pages: null }, { label: 'Part two', pages: null }], pageImages: false };
    const w = window as unknown as Record<string, unknown>;
    const calls: { cmd: string; args: Record<string, unknown> }[] = [];
    w.__calls = calls;
    let settings: unknown = llm ? { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', hasKey: true } : null;
    // Chapter index → how many times it was translated.
    const done = new Map<number, number>();
    let next = 1;
    // Callbacks by id, and the event each listener is for, so progress can be sent.
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: { event: string; handler: number }[] = [];
    const emit = (event: string, payload: unknown) =>
      listeners.filter(l => l.event === event).forEach(l => callbacks.get(l.handler)?.({ event, id: l.handler, payload }));
    // Set `__hold` to keep a chapter translating until `__release()`.
    let release: () => void = () => undefined;
    w.__release = () => release();
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (e: unknown) => void) => { const id = next++; callbacks.set(id, cb); return id; },
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case 'take_opened_files': return [];
          case 'library_list': return [imported];
          case 'library_detail': return { book: imported, sections: book.sections };
          case 'library_open': return { book, novelId: 1, lang: 'en', index: 0, progress: 0 };
          case 'book_section': return { index: args.index, paragraphs: [`Words of section ${Number(args.index) + 1}.`] };
          case 'llm_config_get': return settings;
          case 'llm_config_set': {
            const c = args.config as { provider: string; baseUrl: string; model: string } | null;
            settings = c && { provider: c.provider, baseUrl: c.baseUrl, model: c.model, hasKey: false };
            return null;
          }
          case 'plugin:event|listen': {
            listeners.push({ event: String(args.event), handler: Number(args.handler) });
            return listeners.length;
          }
          case 'ai_translate_chapter': {
            emit('ai-chapter-progress', { novelId: args.novelId, index: args.index, done: 0, total: 3 });
            await new Promise(r => setTimeout(r, 150));
            emit('ai-chapter-progress', { novelId: args.novelId, index: args.index, done: 1, total: 3 });
            if (w.__hold) await new Promise<void>(r => { release = r; });
            await new Promise(r => setTimeout(r, 150));
            const i = Number(args.index);
            done.set(i, (done.get(i) ?? 0) + 1);
            return { title: 'Phần một', paragraphs: [`Lời của phần ${i + 1} (${done.get(i)}).`], mode: 'translate', model: 'gpt-4o-mini' };
          }
          case 'ai_chapter': {
            const i = Number(args.index);
            return done.has(i) && args.lang === 'vi' && args.style === 'natural'
              ? { title: 'Phần một', paragraphs: [`Lời của phần ${i + 1} (${done.get(i)}).`], mode: 'translate', model: 'gpt-4o-mini' } : null;
          }
          case 'ai_marks':
            return [...done.keys()].map(index => ({ index, lang: 'vi', style: 'natural', model: 'gpt-4o-mini' }));
          case 'ai_translate_book': return { queued: 2, mode: 'translate' };
          case 'ai_progress': return [];
          case 'ai_versions': return [];
          case 'plugin:dialog|save': return '/home/lina/bennovel-backup.bnbackup';
          case 'plugin:dialog|open': return '/home/lina/bennovel-backup.bnbackup';
          case 'library_backup': return 1024;
          case 'library_restore': return null;
          default: return null;
        }
      }
    };
  }, { imported, llm });
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: Calls }).__calls);

test('the LLM is set up in Profile and kept on the device', async ({ page }) => {
  await nativeApp(page, { llm: false });
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /AI translation \(your LLM\)/ }).click();
  await page.getByRole('button', { name: 'Custom' }).click();
  await page.getByLabel('Base URL').fill('http://localhost:11434/v1');
  await page.getByLabel('Model name').fill('qwen2.5:7b');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('LLM saved.')).toBeVisible();
  const set = (await calls(page)).find(c => c.cmd === 'llm_config_set');
  expect(set?.args.config).toEqual({ provider: 'custom', baseUrl: 'http://localhost:11434/v1', model: 'qwen2.5:7b', embedModel: '', apiKey: '' });
  await expect(page.getByRole('button', { name: /AI translation \(your LLM\)/ })).toContainText('qwen2.5:7b');
});

test('the LLM sheet opens fully in view when Profile is scrolled down', async ({ page }) => {
  await nativeApp(page);
  await page.setViewportSize({ width: 412, height: 560 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  const row = page.getByRole('button', { name: /AI translation \(your LLM\)/ });
  await row.scrollIntoViewIfNeeded();
  await page.locator('.screen').filter({ has: row }).evaluate(el => { el.scrollTop = el.scrollHeight; });
  await row.click();
  const sheet = page.getByRole('dialog', { name: /AI translation/ });
  await expect(sheet).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole('button', { name: 'Save' })).toBeInViewport();
});

test('each chapter is marked, translated again and switched to its original', async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Continue/ }).filter({ visible: true }).first().click();
  const first = page.locator('[data-part="0"]');
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  // Read in the book's own language: the original, nothing to translate.
  await expect(first.locator('[data-ai-mark]')).toHaveCount(0);
  await expect(first.getByRole('button', { name: /Translate/ })).toHaveCount(0);
  // The language button switches the reading language: English (original) or Vietnamese.
  await page.getByRole('button', { name: 'Reading language' }).click();
  const menu = page.getByRole('menu', { name: 'Reading language' });
  await expect(menu.getByRole('menuitemradio')).toHaveText([/EN · English/, /VI · Tiếng Việt/]);
  await menu.getByRole('menuitemradio', { name: /VI · Tiếng Việt/ }).click();
  await expect(page.getByRole('button', { name: 'Reading language' })).toHaveText('VI');
  // No Vietnamese version yet: the chapter keeps its original and offers the AI.
  await expect(first.getByText('No Tiếng Việt version yet')).toBeVisible();
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await first.getByRole('button', { name: 'Translate into Tiếng Việt with AI' }).click();
  await expect(page.getByTestId('reader-status')).toContainText('Translating Part 1 of 2…');
  await expect(first.getByText('Lời của phần 1 (1).')).toBeVisible();
  await expect(first.getByText('AI · Tiếng Việt · Natural')).toBeVisible();
  const tr = (await calls(page)).find(c => c.cmd === 'ai_translate_chapter');
  expect(tr?.args).toMatchObject({ novelId: 1, index: 0, lang: 'vi', style: 'natural', glossary: true, again: true });
  // The second chapter has no AI version: it says so on its own.
  await expect(page.locator('[data-part="1"]').getByText('No Tiếng Việt version yet')).toBeVisible();
  // Translating the same chapter again makes a new version.
  await first.getByRole('button', { name: 'Translate again' }).click();
  await expect(first.getByText('Lời của phần 1 (2).')).toBeVisible();
  // The original of this chapter, then back to its AI version.
  await first.getByRole('button', { name: 'Show original' }).click();
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await expect(first.getByText('Original', { exact: true })).toBeVisible();
  await first.getByRole('button', { name: 'Show AI version' }).click();
  await expect(first.getByText('Lời của phần 1 (2).')).toBeVisible();
  // Back to English: the originals again.
  await page.getByRole('button', { name: 'Reading language' }).click();
  await page.getByRole('menuitemradio', { name: /EN · English/ }).click();
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await expect(first.locator('[data-ai-mark]')).toHaveCount(0);
  // The chapter list marks the translated chapter only.
  await page.getByRole('button', { name: 'Choose a chapter' }).click();
  const sheet = page.getByRole('dialog');
  await expect(sheet.locator('[data-ai-tag]')).toHaveCount(1);
  await expect(sheet.getByRole('button').filter({ has: page.locator('[data-ai-tag]') })).toContainText('Part one');
});

test('a chapter translates in the status bar while the reader goes on', async ({ page }) => {
  await nativeApp(page);
  await page.addInitScript(() => { (window as unknown as { __hold: boolean }).__hold = true; });
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Continue/ }).filter({ visible: true }).first().click();
  const first = page.locator('[data-part="0"]');
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Reading language' }).click();
  await page.getByRole('menuitemradio', { name: /VI · Tiếng Việt/ }).click();
  await first.getByRole('button', { name: 'Translate into Tiếng Việt with AI' }).click();
  const status = page.getByTestId('reader-status');
  await expect(status).toContainText('Translating Part 1 of 2… · 1/3 parts');
  // Nothing covers the text: the chapter stays readable and its buttons work.
  await expect(first.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await page.locator('[data-part="1"]').getByRole('button', { name: 'Translate into Tiếng Việt with AI' }).click();
  await expect(page.getByText('Another chapter is being translated. Wait for it to finish.')).toBeVisible();
  await page.evaluate(() => (window as unknown as { __release: () => void }).__release());
  await expect(page.getByText('The AI version of Part 1 of 2 is ready.')).toBeVisible();
  await expect(first.getByText('Lời của phần 1 (1).')).toBeVisible();
  await expect(status.getByRole('status')).toHaveCount(0);
});

test('the whole book goes to the background queue from its page', async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Translate the whole novel' }).filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Start translating' }).click();
  await expect(page.getByText(/Queued 2 chapters into Tiếng Việt/)).toBeVisible();
  const req = (await calls(page)).find(c => c.cmd === 'ai_translate_book');
  expect(req?.args).toMatchObject({ novelId: 1, fromIndex: 0, lang: 'vi', style: 'natural', uiLang: 'en' });
});

test('without an LLM the book page points to the settings', async ({ page }) => {
  await nativeApp(page, { llm: false });
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  await expect(page.getByText(/No LLM set up yet/).filter({ visible: true }).first()).toBeVisible();
  await page.getByRole('button', { name: 'Set up LLM' }).filter({ visible: true }).first().click();
  await expect(page.getByRole('button', { name: /AI translation \(your LLM\)/ })).toBeVisible();
});

test('the whole library is backed up to a file and brought back', async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Create backup' }).click();
  await expect(page.getByText('Backup saved.')).toBeVisible();
  await page.getByRole('button', { name: 'Restore' }).click();
  await expect(page.getByText('Backup restored.')).toBeVisible();
  const all = await calls(page);
  expect(all.find(c => c.cmd === 'library_backup')?.args).toMatchObject({ user: '', path: '/home/lina/bennovel-backup.bnbackup' });
  expect(all.find(c => c.cmd === 'library_restore')?.args).toMatchObject({ user: '', path: '/home/lina/bennovel-backup.bnbackup' });
});
