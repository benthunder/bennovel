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
    const entry = (uid: string, kind: string, name: string, vi: string, summary: string, firstChapter: number | null, extra: Record<string, unknown> = {}) =>
      ({ uid, kind, name, names: vi ? { vi } : {}, aliases: [], summaries: summary ? { vi: summary } : {}, rank: null, firstChapter, source: 'ai', updatedAt: 1, ...extra });
    const wiki = {
      entries: [
        entry('a', 'character', '林动', 'Lâm Động', 'Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.', 1),
        entry('b', 'character', '小貂', 'Tiểu Điêu', 'Yêu thú trong thạch phù, bạn đồng hành của Lâm Động.', 3),
        entry('c', 'faction', '道宗', 'Đạo Tông', 'Một trong những tông môn lớn nhất Đông Huyền vực.', 40),
        entry('d', 'place', '青阳镇', 'Thanh Dương trấn', 'Thị trấn nhỏ, quê của Lâm gia.', 1),
        entry('e', 'place', '大炎王朝', 'Đại Viêm vương triều', '', 2),
        entry('f', 'level', '地元境', 'Địa Nguyên cảnh', '', 5, { rank: 2 }),
        entry('g', 'level', '淬体', 'Thối Thể', 'Cảnh giới đầu tiên.', 1, { rank: 1 })
      ],
      relations: [
        { uid: 'r1', fromUid: 'b', toUid: 'a', kind: 'bạn đồng hành của', labels: { vi: 'bạn đồng hành của' }, note: '', firstChapter: 3 },
        { uid: 'r3', fromUid: 'd', toUid: 'e', kind: 'located_in', labels: {}, note: '', firstChapter: 2 },
        { uid: 'r4', fromUid: 'a', toUid: 'g', kind: 'has_level', labels: {}, note: '', firstChapter: 1 },
        { uid: 'r5', fromUid: 'a', toUid: 'f', kind: 'has_level', labels: {}, note: '', firstChapter: 5 }
      ],
      auto: true, lastError: null
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
          case 'llm_config_get': return { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', hasKey: true };
          case 'wiki_get': return wiki;
          case 'library_open': return { book: { id: 9, format: 'txt', title: 'Võ Động Càn Khôn', sections: [{ label: 'Chương 1', pages: null }, { label: 'Chương 2', pages: null }], pageImages: false }, novelId: 1, lang: 'zh', index: 0, progress: 0 };
          case 'book_section':
            if (Number(w.__sectionFails) > 0) { w.__sectionFails = Number(w.__sectionFails) - 1; throw 'book 9 is not open'; }
            return { index: args.index, paragraphs: ['林动 看着 道宗 的 人。', '动少 笑了。'] };
          case 'wiki_at': {
            // As of chapter 1: only what was met by then, with the description written then.
            const n = Number(args.number);
            const entries = wiki.entries.filter(e => (e.firstChapter ?? 0) <= n)
              .map(e => (e.uid === 'a' ? { ...e, aliases: ['动少'], summaries: { vi: 'Thiếu niên Lâm gia (chương 1).' } } : e));
            const ids = new Set(entries.map(e => e.uid));
            const relations = wiki.relations.filter(r => (r.firstChapter ?? 0) <= n && ids.has(r.fromUid) && ids.has(r.toUid));
            return { ...wiki, entries, relations };
          }
          case 'wiki_search': return wiki.entries.filter(e => e.kind === 'faction');
          case 'wiki_set_auto': wiki.auto = Boolean(args.auto); return null;
          case 'wiki_missing': return args.lang === 'en' ? 7 : 0;
          case 'wiki_translate': {
            for (const e of wiki.entries) {
              e.names = { ...e.names, en: `${e.name} (en)` };
              if (Object.keys(e.summaries).length) e.summaries = { ...e.summaries, en: `About ${e.name}.` };
            }
            return wiki.entries.length;
          }
          case 'wiki_save': {
            const e = args.entry as { uid?: string; kind: string; name: string; names: Record<string, string>; aliases: string[]; summaries: Record<string, string> };
            const saved = { ...e, uid: e.uid ?? 'new', firstChapter: null, source: 'user', updatedAt: 2 };
            wiki.entries = [...wiki.entries.filter(x => x.uid !== saved.uid), saved as typeof wiki.entries[number]];
            return saved;
          }
          case 'wiki_relate': {
            wiki.relations.push({ uid: 'r2', fromUid: String(args.fromUid), toUid: String(args.toUid), kind: String(args.kind), labels: {}, note: '', firstChapter: null });
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
  // Never read yet: the wiki opens in Raw, the names as in the original.
  await expect(page.getByRole('radio', { name: 'Raw' })).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByText('林动', { exact: true })).toBeVisible();
  await page.getByRole('radio', { name: 'Tiếng Việt' }).click();
  await expect(page.getByText('Lâm Động', { exact: true })).toBeVisible();
  await expect(page.locator('[data-entry="林动"]').getByText('3 quan hệ')).toBeVisible();
  // A character shows the level it reached last.
  await expect(page.locator('[data-entry="林动"]').getByText('Địa Nguyên cảnh')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-list.png` });

  // Search goes to the native side (names, aliases and notes).
  await page.getByPlaceholder('Tìm tên, tên khác, mô tả…').fill('tông môn');
  await expect(page.getByText('Đạo Tông', { exact: true })).toBeVisible();
  await expect(page.getByText('Lâm Động', { exact: true })).toHaveCount(0);
  expect((await calls(page)).some(c => c.cmd === 'wiki_search' && c.args.query === 'tông môn')).toBe(true);
  await page.getByPlaceholder('Tìm tên, tên khác, mô tả…').fill('');

  // Turning off the update while translating.
  await page.getByText('Tự cập nhật khi dịch AI').click();
  expect((await calls(page)).find(c => c.cmd === 'wiki_set_auto')?.args).toMatchObject({ novelId: 1, auto: false });

  // An entry opens with its relationships, typed ones in the wiki's language; a new one is added to it.
  await page.getByText('Lâm Động', { exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Mục wiki' });
  await expect(sheet.getByText('Cảnh giới hiện tại: Địa Nguyên cảnh')).toBeVisible();
  await expect(sheet.locator('[data-relation="bạn đồng hành của"]')).toContainText('bạn đồng hành của Tiểu Điêu');
  await expect(sheet.locator('[data-relation="has_level"]').first()).toContainText('đạt cảnh giới');
  await sheet.getByLabel('Quan hệ', { exact: true }).selectOption({ label: 'thành viên của' });
  await sheet.getByLabel('Với').selectOption({ label: 'Đạo Tông' });
  await sheet.getByRole('button', { name: 'Thêm quan hệ' }).click();
  await expect(sheet.locator('[data-relation="member_of"]')).toContainText('thành viên của Đạo Tông');
  expect((await calls(page)).find(c => c.cmd === 'wiki_relate')?.args).toMatchObject({ fromUid: 'a', toUid: 'c', kind: 'member_of', lang: 'vi' });
  // Words per language: the English description is written on its own tab.
  await sheet.getByLabel('Tên khác (cách nhau bằng dấu phẩy)').fill('动少, 林动哥');
  await sheet.getByRole('tab', { name: /English/ }).click();
  await sheet.getByLabel('Tên trong bản dịch English').fill('Lin Dong');
  await sheet.getByLabel('Mô tả · English').fill('A boy of the Lin clan.');
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-entry.png` });
  await sheet.getByRole('button', { name: 'Lưu' }).click();
  expect((await calls(page)).find(c => c.cmd === 'wiki_save')?.args.entry).toMatchObject({
    uid: 'a', name: '林动', aliases: ['动少', '林动哥'],
    names: { vi: 'Lâm Động', en: 'Lin Dong' },
    summaries: { vi: 'Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.', en: 'A boy of the Lin clan.' }
  });
  await expect(page.getByText('Đã lưu vào wiki.')).toBeVisible();
});

test('the wiki follows the reading language, with levels in order and places as a map', async ({ page }, info) => {
  await nativeApp(page);
  // The book is read in English (no one signed in: the library key is empty).
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'en', style: 'Natural', dict: true, view: 'en' }));
  });
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Wiki truyện/ }).filter({ visible: true }).first().click();
  await expect(page.getByRole('radio', { name: 'English' })).toHaveAttribute('aria-checked', 'true');
  // Nothing in English yet: raw words, and the AI can fill the language in.
  await expect(page.getByText('林动', { exact: true })).toBeVisible();
  await expect(page.getByText('7 mục chưa có chữ English, đang hiện chữ Raw.')).toBeVisible();
  await page.getByRole('button', { name: 'Dịch wiki sang English bằng AI' }).click();
  await expect(page.getByText('Đã dịch wiki sang English: 7 mục.')).toBeVisible();
  expect((await calls(page)).find(c => c.cmd === 'wiki_translate')?.args).toMatchObject({ novelId: 1, lang: 'en' });
  await expect(page.getByText('林动 (en)', { exact: true })).toBeVisible();
  await expect(page.getByText('About 林动.')).toBeVisible();

  // Levels in their order, lowest first, with who is at each.
  await page.getByRole('tab', { name: 'Cảnh giới' }).click();
  const ladder = page.locator('[data-wiki-ladder] > li');
  await expect(ladder).toHaveCount(2);
  await expect(ladder.nth(0)).toContainText('淬体 (en)');
  await expect(ladder.nth(1)).toContainText('地元境 (en)');
  await expect(ladder.nth(1)).toContainText('林动 (en)');
  await expect(ladder.nth(0)).not.toContainText('林动 (en)');

  // Places nest by "located in".
  await page.getByRole('tab', { name: 'Bản đồ' }).click();
  const map = page.locator('[data-wiki-map]');
  await expect(map.locator(':scope > li')).toHaveCount(1);
  await expect(map.locator(':scope > li > ul [data-entry="青阳镇"]')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-map.png` });
});

test('names in a chapter open the wiki as known by that chapter', async ({ page }, info) => {
  await nativeApp(page);
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'vi', style: 'Natural', dict: true, view: 'vi' }));
  });
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  const first = page.locator('[data-part="0"]');
  // Names (and aliases) the wiki knew by chapter 1 are marked; 道宗 comes in chapter 40.
  await expect(first.locator('[data-wiki-term="a"]')).toHaveCount(2);
  await expect(first.locator('[data-wiki-term="c"]')).toHaveCount(0);
  expect((await calls(page)).find(c => c.cmd === 'wiki_at')?.args).toMatchObject({ novelId: 1, number: 1 });
  await first.locator('[data-wiki-term="a"]').first().click();
  const card = page.getByRole('dialog', { name: 'Lâm Động' });
  await expect(card.getByText('Thiếu niên Lâm gia (chương 1).')).toBeVisible();
  await expect(card.getByText('Cảnh giới hiện tại: Thối Thể')).toBeVisible();
  await expect(card.getByText('Thông tin tới Phần 1 / 2, không có gì từ các chương sau.')).toBeVisible();
  // The later realm and the sect are not known yet.
  await expect(card.getByText('Địa Nguyên cảnh')).toHaveCount(0);
  await expect(card.getByText('Đạo Tông')).toHaveCount(0);
  await page.waitForTimeout(500);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-reader-card.png` });
  await card.getByRole('button', { name: 'Wiki truyện' }).click();
  await expect(page.getByRole('radio', { name: 'Tiếng Việt' })).toHaveAttribute('aria-checked', 'true');
});

test('a chapter that fails to load says so in words and loads again', async ({ page }) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-part="0"]')).toContainText('动少 笑了。');
  // The wiki opens over the reader, which closes its book; back in the reader the
  // native side opens it again. Here the first load still fails.
  await page.getByRole('button', { name: 'Wiki', exact: true }).click();
  await expect(page.locator('[data-part="0"]')).toHaveCount(0);
  await page.evaluate(() => { (window as unknown as Record<string, unknown>).__sectionFails = 1; });
  await page.getByRole('button', { name: 'Quay lại' }).filter({ visible: true }).first().click();
  const first = page.locator('[data-part="0"]');
  await expect(first.getByRole('alert')).toContainText('Không tải được chương này.');
  await expect(page.getByText('is not open')).toHaveCount(0);
  await first.getByRole('button', { name: 'Thử lại' }).click();
  await expect(first).toContainText('动少 笑了。');
  expect((await calls(page)).filter(c => c.cmd === 'book_close').length).toBeGreaterThan(0);
});
