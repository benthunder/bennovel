import { expect, test, type Page } from '@playwright/test';

// The wiki of a book on the device (src-tauri/src/library/wiki.rs, with its own Rust
// tests). The native side is stood in for; these never reach Supabase.

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
    const calls: { cmd: string; args: Record<string, unknown> }[] = [];
    w.__calls = calls;
    const entry = (uid: string, kind: string, name: string, vi: string, summary: string, firstChapter: number | null, source = 'ai') =>
      ({ uid, kind, name, names: { vi }, aliases: [], summary, firstChapter, source, updatedAt: 1 });
    const wiki = {
      entries: [
        entry('a', 'character', '林动', 'Lâm Động', 'Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.', 1),
        entry('b', 'character', '小貂', 'Tiểu Điêu', 'Yêu thú trong thạch phù, bạn đồng hành của Lâm Động.', 3),
        entry('c', 'faction', '道宗', 'Đạo Tông', 'Một trong những tông môn lớn nhất Đông Huyền vực.', 40),
        entry('d', 'place', '青阳镇', 'Thanh Dương trấn', 'Thị trấn nhỏ, quê của Lâm gia.', 1)
      ],
      relations: [{ uid: 'r1', fromUid: 'b', toUid: 'a', kind: 'bạn đồng hành của', note: '', firstChapter: 3 }],
      auto: true, lastError: null, byMeaning: true
    };
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
          case 'library_detail': return { book: imported, sections: [{ label: 'Chương 1', pages: null }, { label: 'Chương 2', pages: null }] };
          case 'llm_config_get': return { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', embedModel: 'text-embedding-3-small', hasKey: true };
          case 'wiki_get': return wiki;
          case 'wiki_search': return wiki.entries.filter(e => e.kind === 'faction');
          case 'wiki_set_auto': wiki.auto = Boolean(args.auto); return null;
          case 'wiki_save': {
            const e = args.entry as { uid?: string; kind: string; name: string; names: Record<string, string>; aliases: string[]; summary: string };
            const saved = { ...e, uid: e.uid ?? 'new', firstChapter: null, source: 'user', updatedAt: 2 };
            wiki.entries = [...wiki.entries.filter(x => x.uid !== saved.uid), saved as typeof wiki.entries[number]];
            return saved;
          }
          case 'wiki_relate': {
            wiki.relations.push({ uid: 'r2', fromUid: String(args.fromUid), toUid: String(args.toUid), kind: String(args.kind), note: '', firstChapter: null });
            return 'r2';
          }
          case 'ai_marks': case 'ai_progress': case 'ai_versions': return [];
          default: return null;
        }
      }
    };
  }, { imported });
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: Calls }).__calls);

test('the book wiki lists, searches and edits entries', async ({ page }, info) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Wiki truyện/ }).filter({ visible: true }).first().click();
  await expect(page.getByText('Lâm Động', { exact: true })).toBeVisible();
  await expect(page.getByText('1 quan hệ')).toHaveCount(2);
  await expect(page.getByText('Đang tìm theo tên và theo ý nghĩa.')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-list.png` });

  // Search goes to the native side (by name, then by meaning).
  await page.getByPlaceholder('Tìm tên, tên khác, mô tả…').fill('tông môn nào');
  await expect(page.getByText('Đạo Tông', { exact: true })).toBeVisible();
  await expect(page.getByText('Lâm Động', { exact: true })).toHaveCount(0);
  expect((await calls(page)).some(c => c.cmd === 'wiki_search' && c.args.query === 'tông môn nào')).toBe(true);
  await page.getByPlaceholder('Tìm tên, tên khác, mô tả…').fill('');

  // Turning off the update while translating.
  await page.getByText('Tự cập nhật khi dịch AI').click();
  expect((await calls(page)).find(c => c.cmd === 'wiki_set_auto')?.args).toMatchObject({ novelId: 1, auto: false });

  // An entry opens with its relationships; a new one is added to it.
  await page.getByText('Lâm Động', { exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Mục wiki' });
  await expect(sheet.getByText('bạn đồng hành của')).toBeVisible();
  await sheet.getByLabel('Quan hệ (vd: sư phụ của)').fill('đệ tử của');
  await sheet.getByLabel('Với').selectOption({ label: 'Đạo Tông' });
  await sheet.getByRole('button', { name: 'Thêm quan hệ' }).click();
  await expect(sheet.getByText('đệ tử của')).toBeVisible();
  expect((await calls(page)).find(c => c.cmd === 'wiki_relate')?.args).toMatchObject({ fromUid: 'a', toUid: 'c', kind: 'đệ tử của' });
  await sheet.getByLabel('Tên khác (cách nhau bằng dấu phẩy)').fill('动少, 林动哥');
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-entry.png` });
  await sheet.getByRole('button', { name: 'Lưu' }).click();
  expect((await calls(page)).find(c => c.cmd === 'wiki_save')?.args.entry).toMatchObject({ uid: 'a', name: '林动', aliases: ['动少', '林动哥'] });
  await expect(page.getByText('Đã lưu vào wiki.')).toBeVisible();
});
