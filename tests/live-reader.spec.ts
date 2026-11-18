import { expect, test, type Page } from '@playwright/test';

// Reading a chapter while it is translated (LocalBookScreen live mode) and the
// first-part preview of the pipeline panel (AiDock). The native side is stood in for.

const imported = {
  novelId: 1, title: 'Võ Động Càn Khôn', author: null, description: '', lang: 'zh', format: 'txt', sourceName: 'wdqk.txt', chapterCount: 2,
  createdAt: '2026-10-01T00:00:00.000Z', lastRead: null
};

type Calls = { cmd: string; args: Record<string, unknown> }[];

async function nativeApp(page: Page) {
  await page.addInitScript(({ imported }) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('vi'));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const w = window as unknown as Record<string, unknown>;
    const calls: Calls = [];
    w.__calls = calls;
    // The chapter being translated from the reader, and its parts back so far.
    w.__running = true;
    w.__draft = { title: '', paragraphs: ['[vi] Lâm Động nhìn người Đạo Tông.'], positions: [0] };
    w.__done = false;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: { event: string; id: number }[] = [];
    // What the native side sends when a translation moves.
    w.__emit = (event: string) => {
      for (const l of listeners) if (l.event === event) callbacks.get(l.id)?.({ event, id: l.id, payload: { user: '' } });
    };
    let next = 1;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (e: unknown) => void) => { const id = next++; callbacks.set(id, cb); return id; },
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push({ cmd, args });
        switch (cmd) {
          case 'plugin:event|listen': listeners.push({ event: String(args.event), id: Number(args.handler) }); return Number(args.handler);
          case 'take_opened_files': return [];
          case 'library_list': return [imported];
          case 'library_detail': return { book: imported, sections: [{ label: 'Chương 1', pages: null }, { label: 'Chương 2', pages: null }] };
          case 'llm_config_get': return { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', hasKey: true };
          case 'library_open': return { book: { id: 9, format: 'txt', title: 'Võ Động Càn Khôn', sections: [{ label: 'Chương 1', pages: null }, { label: 'Chương 2', pages: null }], pageImages: false }, novelId: 1, lang: 'zh', index: 0, progress: 0 };
          case 'book_section': return { index: args.index, paragraphs: ['林动 看着 道宗 的 人。', '动少 笑了。'] };
          case 'ai_status': return {
            books: [], live: null,
            chapters: w.__running || w.__stopped ? [{ novelId: 1, index: 0, lang: 'vi', style: 'natural', stage: 'translate', partsDone: 1, partsTotal: 2, title: 'Võ Động Càn Khôn', glossary: true, error: null, stopped: !!w.__stopped }] : []
          };
          case 'ai_chapter_draft': return w.__running ? w.__draft : null;
          case 'ai_chapter_stop': w.__running = false; w.__stopped = true; return true;
          case 'ai_chapter_remove': w.__running = false; w.__stopped = false; return true;
          case 'ai_marks': return w.__done ? [{ index: 0, lang: 'vi', style: 'natural', model: 'm' }] : [];
          case 'ai_chapter': return w.__done && args.index === 0
            ? { title: '', paragraphs: ['[vi] Lâm Động nhìn người Đạo Tông.', '[vi] Động thiếu cười.'], mode: 'translate', model: 'm' }
            : null;
          case 'ai_progress': case 'ai_versions': return [];
          default: return null;
        }
      }
    };
  }, { imported });
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: Calls }).__calls);
const emit = (page: Page) => page.evaluate(() => (window as unknown as { __emit: (e: string) => void }).__emit('ai-status'));

async function openReader(page: Page, view = 'vi') {
  await nativeApp(page);
  // The language the book is read in (no one signed in: the library key is empty).
  await page.addInitScript(view => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'vi', style: 'Natural', dict: true, view }));
  }, view);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-part="0"]')).toContainText('动少 笑了。');
}

test('a chapter being translated into the reading language shows each paragraph as it comes, with no button', async ({ page }, info) => {
  await openReader(page);
  await expect(page.getByRole('button', { name: /Xem trực tiếp/ })).toHaveCount(0);
  const part = page.locator('[data-part="0"]');
  // No banner or status line: only the pipeline button says it is translating.
  await expect(part.locator('[data-live-banner]')).toHaveCount(0);
  await expect(page.getByText(/Đang dịch Chương/)).toHaveCount(0);
  await expect(part.locator('p[data-live="done"]')).toHaveText('[vi] Lâm Động nhìn người Đạo Tông.');
  await expect(part.locator('p[data-live="raw"]')).toHaveText('动少 笑了。');
  await page.screenshot({ path: `test-results/live-${info.project.name}-reader.png` });

  // The next part comes back: it takes its place in the page.
  await page.evaluate(() => {
    (window as unknown as Record<string, unknown>).__draft = { title: '', paragraphs: ['[vi] Lâm Động nhìn người Đạo Tông.', '[vi] Động thiếu cười.'], positions: [0, 1] };
  });
  await emit(page);
  await expect(part.locator('p[data-live="done"]')).toHaveCount(2);

  // Saved: the chapter shows its AI version, as any translated chapter does.
  await page.evaluate(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__running = false;
    w.__done = true;
  });
  await emit(page);
  await expect(part.locator('p[data-live]')).toHaveCount(0);
  await expect(part.locator('[data-ai-mark="vi.Natural"]')).toBeVisible();
  await expect(part).toContainText('[vi] Động thiếu cười.');
});

test('the first part is previewed in a popup to carry on or stop', async ({ page }, info) => {
  await openReader(page);
  await page.locator('.ai-dock__pill').click();
  await page.getByRole('button', { name: 'Xem thử đoạn đầu' }).click();
  const sheet = page.getByRole('dialog', { name: 'Xem thử đoạn đầu' });
  await expect(sheet).toContainText('[vi] Lâm Động nhìn người Đạo Tông.');
  await expect(sheet.getByRole('button', { name: 'Dịch tiếp' })).toBeVisible();
  await page.screenshot({ path: `test-results/live-${info.project.name}-preview.png` });
  await sheet.getByRole('button', { name: 'Dừng dịch' }).click();
  await expect(sheet).toHaveCount(0);
  expect((await calls(page)).find(c => c.cmd === 'ai_chapter_stop')?.args).toMatchObject({ novelId: 1, index: 0 });
});

test('read raw, a chapter being translated stays raw', async ({ page }) => {
  await openReader(page, 'raw');
  const part = page.locator('[data-part="0"]');
  await expect(part.locator('p[data-live]')).toHaveCount(0);
  await expect(part).toContainText('林动 看着 道宗 的 人。');
  // The pipeline panel opens it in the language being translated into.
  await page.locator('.ai-dock__pill').click();
  await page.getByRole('button', { name: 'Mở trong reader' }).click();
  await expect(part.locator('p[data-live="done"]')).toHaveText('[vi] Lâm Động nhìn người Đạo Tông.');
});

test('a chapter stops at once and stays on the panel, to carry on or remove', async ({ page }) => {
  await openReader(page);
  await page.locator('.ai-dock__pill').click();
  const row = page.locator('.ai-dock__item');
  await row.getByRole('button', { name: 'Dừng', exact: true }).click();
  await expect(row.locator('[data-stopped]')).toContainText('Đã dừng');
  await expect(row.getByRole('button', { name: 'Chạy tiếp' })).toBeVisible();
  // Stopped, the chapter in the reader is raw again (the native side says it moved).
  await emit(page);
  await expect(page.locator('[data-part="0"] p[data-live]')).toHaveCount(0);
  await row.getByRole('button', { name: 'Xóa' }).click();
  expect((await calls(page)).find(c => c.cmd === 'ai_chapter_remove')?.args).toMatchObject({ novelId: 1, index: 0 });
  await expect(page.locator('.ai-dock')).toHaveCount(0);
});
