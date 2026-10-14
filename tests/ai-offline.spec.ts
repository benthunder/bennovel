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
    const done = new Set<number>();
    let next = 1;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
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
          case 'ai_translate_chapter':
            await new Promise(r => setTimeout(r, 300));
            done.add(Number(args.index));
            return { title: 'Phần một', paragraphs: ['Lời của phần 1.'], mode: 'translate', model: 'gpt-4o-mini' };
          case 'ai_chapter':
            return done.has(Number(args.index)) ? { title: 'Phần một', paragraphs: ['Lời của phần 1.'], mode: 'translate', model: 'gpt-4o-mini' } : null;
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
  expect(set?.args.config).toEqual({ provider: 'custom', baseUrl: 'http://localhost:11434/v1', model: 'qwen2.5:7b', apiKey: '' });
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

test('translating one chapter shows a loader, then the translation', async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Device Diary').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Continue/ }).filter({ visible: true }).first().click();
  await expect(page.getByText('Words of section 1.', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'EN', exact: true }).click();
  await page.getByRole('button', { name: 'Translate this chapter' }).click();
  await expect(page.getByText('Translating chapter…')).toBeVisible();
  await expect(page.getByText('Lời của phần 1.')).toBeVisible();
  await expect(page.getByText(/AI translation into Tiếng Việt · Natural style/)).toBeVisible();
  const tr = (await calls(page)).find(c => c.cmd === 'ai_translate_chapter');
  expect(tr?.args).toMatchObject({ novelId: 1, index: 0, lang: 'vi', style: 'natural', glossary: true });
  // Back to the original.
  await page.getByRole('button', { name: 'Original' }).click();
  await expect(page.getByText('Words of section 1.', { exact: true })).toBeVisible();
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
