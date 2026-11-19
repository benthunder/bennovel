import { expect, test, type Page } from '@playwright/test';

// The wiki of a book on the device (src-tauri/src/library/wiki.rs, with its own Rust
// tests). The native side is stood in for; these never reach a server.

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
        entry('a', 'character', '林动', 'Lâm Động', 'Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.', 1, {
          aliases: ['动少'],
          meta: { personality: 'Kiên nhẫn, lì đòn, trọng tình nghĩa.', age: '15 tuổi', origin: 'Con trai Lâm Khiếu, chi thứ của Lâm gia ở Thanh Dương trấn.' }
        }),
        entry('h', 'character', '林啸', 'Lâm Khiếu', 'Cha của Lâm Động.', 1),
        entry('i', 'character', '林琅天', 'Lâm Lang Thiên', 'Thiên tài Lâm gia tông tộc.', 1),
        entry('j', 'skill', '通背拳', 'Thông Bối Quyền', 'Quyền pháp của Lâm gia, đánh liên hoàn như sóng cuộn.', 1, {
          meta: { grade: 'Hoàng giai', difficulty: 'Dễ nhập môn, luyện đến tầng cuối rất khó.' }
        }),
        entry('k', 'level', '先天', 'Tiên Thiên', '', 7, { rank: 1, meta: { system: 'Võ đạo' } }),
        entry('l', 'character', '应玄子', 'Ứng Huyền Tử', 'Tông chủ Đạo Tông.', 40),
        entry('m', 'item', '石符', 'Thạch Phù', 'Phù đá bí ẩn.', 1),
        entry('b', 'character', '小貂', 'Tiểu Điêu', 'Yêu thú trong thạch phù, bạn đồng hành của Lâm Động.', 3),
        entry('c', 'faction', '道宗', 'Đạo Tông', 'Một trong những tông môn lớn nhất Đông Huyền vực, chia thành nhiều mạch như Đan Tông, Phù Tông, Thiên Tông và Hoang Điện. Tông chủ là Ứng Huyền Tử. Mỗi lần thi đấu giữa các mạch, đệ tử ưu tú được vào Hoang Điện tu luyện, nơi cất giữ những bí thuật cổ xưa nhất của tông môn.', 40),
        entry('d', 'place', '青阳镇', 'Thanh Dương trấn', 'Thị trấn nhỏ, quê của Lâm gia.', 1),
        entry('e', 'place', '大炎王朝', 'Đại Viêm vương triều', '', 2),
        entry('f', 'level', '地元境', 'Địa Nguyên cảnh', '', 5, { rank: 2 }),
        entry('g', 'level', '淬体', 'Thối Thể', 'Cảnh giới đầu tiên.', 1, { rank: 1 })
      ],
      relations: [
        { uid: 'r1', fromUid: 'b', toUid: 'a', kind: 'bạn đồng hành của', labels: { vi: 'bạn đồng hành của' }, note: 'Gặp nhau trong thạch phù.', firstChapter: 3 },
        { uid: 'r3', fromUid: 'd', toUid: 'e', kind: 'located_in', labels: {}, note: '', firstChapter: 2 },
        { uid: 'r4', fromUid: 'a', toUid: 'g', kind: 'has_level', labels: {}, note: '', firstChapter: 1 },
        { uid: 'r5', fromUid: 'a', toUid: 'f', kind: 'has_level', labels: {}, note: '', firstChapter: 5 },
        { uid: 'r6', fromUid: 'h', toUid: 'a', kind: 'parent_of', labels: {}, note: '', firstChapter: 1 },
        { uid: 'r7', fromUid: 'i', toUid: 'a', kind: 'enemy_of', labels: {}, note: 'Đánh trọng thương Lâm Khiếu năm xưa.', firstChapter: 1 },
        { uid: 'r8', fromUid: 'a', toUid: 'j', kind: 'has_skill', labels: {}, note: '', firstChapter: 1 },
        { uid: 'r9', fromUid: 'k', toUid: 'f', kind: 'equivalent_to', labels: {}, note: '', firstChapter: 7 },
        { uid: 'r10', fromUid: 'l', toUid: 'c', kind: 'leader_of', labels: {}, note: 'Tông chủ đời thứ ba.', firstChapter: 40 },
        { uid: 'r11', fromUid: 'a', toUid: 'c', kind: 'leader_of', labels: {}, note: 'Kế nhiệm Ứng Huyền Tử.', firstChapter: 900 },
        { uid: 'r12', fromUid: 'a', toUid: 'm', kind: 'owns', labels: {}, note: '', firstChapter: 1 }
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
          case 'wiki_search': return wiki.entries.filter(e => e.kind === 'faction');
          case 'wiki_set_auto': wiki.auto = Boolean(args.auto); return null;
          case 'wiki_missing': return Number(w.__missing ?? 0);
          case 'wiki_translate': {
            // An older wiki written in English: the AI writes it in Vietnamese.
            for (const e of wiki.entries) {
              e.names = { vi: `${e.name} (vi)` };
              if (Object.keys(e.summaries).length) e.summaries = { vi: `Về ${e.name}.` };
            }
            w.__missing = 0;
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
          case 'wiki_merge_suggest': {
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            const seol = entry('s1', 'character', 'Seol-Hwi', 'Tuyết Huy', 'Đệ tử Hoa Sơn.', 1);
            const seol2 = entry('s2', 'character', 'Seol Hwi', 'Seol-Hwi', '', 4);
            return [
              { from: seol2, into: seol, reason: 'same_raw', rules: [{ find: 'Seol-Hwi', replace: 'Tuyết Huy' }, { find: 'Seol Hwi', replace: 'Tuyết Huy' }], note: '' },
              { from: by('i'), into: by('a'), reason: 'near_name', rules: [{ find: 'Lâm Lang Thiên', replace: 'Lâm Động' }], note: '' }
            ];
          }
          // The whole wiki's AI run, in the background: 3 blocks, one read, one waiting on a lost connection.
          case 'wiki_merge_ai_start': w.__mergeRun = 'paused'; return { novelId: 1, total: 3, done: 0, failed: 0, queued: 3, running: 0, paused: 0, error: null, groups: [] };
          case 'wiki_merge_ai_control': w.__mergeRun = args.action === 'resume' ? 'going' : w.__mergeRun; return { novelId: 1, total: 3, done: 1, failed: 0, queued: 2, running: 0, paused: 0, error: null, groups: [] };
          case 'wiki_merge_ai_status': {
            if (!w.__mergeRun) return { novelId: 1, total: 0, done: 0, failed: 0, queued: 0, running: 0, paused: 0, error: null, groups: [] };
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            const groups = [{ members: [by('l')], into: by('h'), name: 'Lâm Khiếu', raw: '', rules: [{ find: 'Ứng Huyền Tử', replace: 'Lâm Khiếu' }], note: 'Mô tả cho thấy là một người.' }];
            return w.__mergeRun === 'paused'
              ? { novelId: 1, total: 3, done: 1, failed: 0, queued: 0, running: 0, paused: 2, error: 'llm-network: connection reset', groups }
              : { novelId: 1, total: 3, done: 1, failed: 0, queued: 1, running: 1, paused: 0, error: null, groups };
          }
          case 'wiki_merge_entry': {
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            return {
              raws: [
                { raw: '林动', translated: 'Lâm Động', chapters: [], entryUid: 'a', exact: true },
                { raw: 'Lin Dong', translated: 'Lâm Động', chapters: [3, 7], entryUid: null, exact: true },
                { raw: '林栋', translated: 'Lâm Đống', chapters: [12], entryUid: 'i', exact: false }
              ],
              pairs: [{ from: by('i'), into: by('a'), reason: 'near_name', rules: [], note: '' }]
            };
          }
          case 'wiki_merge_entry_ai': {
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            return [{ members: [by('i')], into: by('a'), name: 'Lâm Động', raw: 'Lin Dong', rules: [{ find: 'Lâm Lang Thiên', replace: 'Lâm Động' }, { find: 'Lâm Đống', replace: 'Lâm Động' }], note: 'Cùng một người.' }];
          }
          case 'wiki_merge_apply': return { merged: (args.picks as unknown[]).length + ((args.groups as { fromUids: string[] }[]) ?? []).reduce((n, g) => n + g.fromUids.length, 0), rules: 2, renamed: 0 };
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
  // The wiki is in Vietnamese only, with the original name beside it; nothing to pick.
  await expect(page.getByRole('radio')).toHaveCount(0);
  await expect(page.getByText('Lâm Động', { exact: true })).toBeVisible();
  await expect(page.locator('[data-entry="林动"]').getByText('林动', { exact: true })).toBeVisible();
  await expect(page.locator('[data-entry="林动"]').getByText('8 quan hệ')).toBeVisible();
  // A character shows the level it reached last.
  await expect(page.locator('[data-entry="林动"]').getByText('Địa Nguyên cảnh')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-list.png` });
  await page.getByRole('tab', { name: 'Cảnh giới' }).click();
  await expect(page.locator('[data-wiki-equals]').first()).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-ladder.png`, fullPage: true });
  await page.getByRole('tab', { name: 'Tất cả' }).click();

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
  // Only Vietnamese words: no tab per language.
  await sheet.getByLabel('Tên khác (cách nhau bằng dấu phẩy)').fill('动少, 林动哥');
  await expect(sheet.getByRole('tab')).toHaveCount(0);
  // The profile is edited in the same order and saved with the entry.
  await expect(sheet.getByLabel('Tính cách')).toHaveValue('Kiên nhẫn, lì đòn, trọng tình nghĩa.');
  await sheet.getByLabel('Kỹ năng').fill('Khắc phù, luyện thể.');
  await expect(sheet.locator('[data-rel-group="family"]')).toContainText(/con của\s*Lâm Khiếu/);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-entry.png` });
  await sheet.getByRole('button', { name: 'Lưu' }).click();
  expect((await calls(page)).find(c => c.cmd === 'wiki_save')?.args.entry).toMatchObject({
    uid: 'a', name: '林动', aliases: ['动少', '林动哥'],
    names: { vi: 'Lâm Động' },
    summaries: { vi: 'Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.' },
    meta: { personality: 'Kiên nhẫn, lì đòn, trọng tình nghĩa.', age: '15 tuổi', abilities: 'Khắc phù, luyện thể.' }
  });
  await expect(page.getByText('Đã lưu vào wiki.')).toBeVisible();
});

test('the wiki stays Vietnamese whatever the reading language, with levels in order and places as a map', async ({ page }, info) => {
  await nativeApp(page);
  // The book is read in English (no one signed in: the library key is empty), and its
  // wiki was written before the wiki was Vietnamese only.
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'en', style: 'Natural', dict: true, view: 'en' }));
    (window as unknown as Record<string, unknown>).__missing = 7;
  });
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Wiki truyện/ }).filter({ visible: true }).first().click();
  // What is missing in Vietnamese is translated on opening, with no button to press.
  await expect(page.getByText('林动 (vi)', { exact: true })).toBeVisible();
  await expect(page.getByText('Về 林动.')).toBeVisible();
  expect((await calls(page)).filter(c => c.cmd === 'wiki_translate').map(c => c.args)).toMatchObject([{ novelId: 1, lang: 'vi' }]);
  await expect(page.getByRole('radio')).toHaveCount(0);

  // Levels in their order, lowest first, with who is at each.
  await page.getByRole('tab', { name: 'Cảnh giới' }).click();
  // The whole tree: each ladder with its levels, and what matches in the other ladders.
  expect(await page.locator('[data-ladder]').evaluateAll(els => els.map(e => e.getAttribute('data-ladder')))).toEqual(['Võ đạo', '']);
  const ladder = page.locator('[data-ladder=""] [data-wiki-ladder] > li');
  await expect(ladder).toHaveCount(2);
  await expect(ladder.nth(0)).toContainText('淬体 (vi)');
  await expect(ladder.nth(1)).toContainText('地元境 (vi)');
  await expect(ladder.nth(1).locator('[data-wiki-equals]')).toContainText('≈ 先天 (vi) · Võ đạo');
  await expect(page.locator('[data-ladder="Võ đạo"] [data-wiki-equals]')).toContainText('≈ 地元境 (vi)');
  // Only the levels: who reached them is on the characters' pages.
  await expect(page.locator('[data-wiki-ladder-tree]')).not.toContainText('林动 (vi)');

  // Places nest by "located in".
  await page.getByRole('tab', { name: 'Bản đồ' }).click();
  const map = page.locator('[data-wiki-map]');
  await expect(map.locator(':scope > li')).toHaveCount(1);
  await expect(map.locator(':scope > li > ul [data-entry="青阳镇"]')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-map.png` });
});

test('names in a chapter open the whole wiki with its links', async ({ page }, info) => {
  await nativeApp(page);
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'vi', style: 'Natural', dict: true, view: 'vi' }));
  });
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  const first = page.locator('[data-part="0"]');
  // Names and aliases are marked, those met in later chapters too (spoilers are fine).
  await expect(first.locator('[data-wiki-term="a"]')).toHaveCount(2);
  await expect(first.locator('[data-wiki-term="c"]')).toHaveCount(1);
  expect((await calls(page)).some(c => c.cmd === 'wiki_at')).toBe(false);
  // Read in Vietnamese, the replace rules are the Vietnamese list (where wiki merges add theirs), not the raw book's.
  const ruleLoads = (await calls(page)).filter(c => c.cmd === 'replace_rules_get').map(c => c.args.lang);
  expect(ruleLoads).toContain('vi');
  expect(ruleLoads).not.toContain('zh');
  await first.locator('[data-wiki-term="a"]').first().click();
  const card = page.getByRole('dialog', { name: 'Lâm Động' });
  await expect(card.getByText('Thiếu niên Lâm gia ở Thanh Dương trấn, tu luyện nhờ thạch phù bí ẩn.')).toBeVisible();
  // The level reached last, and the relationships with their circumstances.
  // A character's profile in order: personality, age, background, skills, level.
  const profile = card.locator('[data-wiki-profile] [data-profile]');
  await expect(profile).toHaveCount(6);
  expect(await profile.evaluateAll(els => els.map(e => e.getAttribute('data-profile')))).toEqual(['personality', 'age', 'origin', 'abilities', 'level', 'items']);
  await expect(profile.nth(5).locator('[data-wiki-skill]')).toHaveText('Thạch Phù');
  await expect(profile.nth(0)).toContainText('Tính cách');
  await expect(profile.nth(0)).toContainText('Kiên nhẫn, lì đòn, trọng tình nghĩa.');
  await expect(profile.nth(1)).toContainText('15 tuổi');
  await expect(profile.nth(2)).toContainText('Con trai Lâm Khiếu');
  await expect(profile.nth(3).locator('[data-wiki-skill]')).toHaveText('Thông Bối Quyền');
  await expect(profile.nth(4)).toContainText('Địa Nguyên cảnh');
  await expect(profile.nth(4)).toContainText('trước đó: Thối Thể');
  // Then the other characters, by kind of relationship; levels and skills are not repeated.
  const groups = card.locator('[data-wiki-rel-people] [data-rel-group]');
  expect(await groups.evaluateAll(els => els.map(e => e.getAttribute('data-rel-group')))).toEqual(['family', 'enemy', 'other']);
  await expect(groups.nth(0)).toContainText('Gia đình');
  await expect(groups.nth(0)).toContainText(/con của\s*Lâm Khiếu/);
  await expect(groups.nth(1)).toContainText(/kẻ thù của\s*Lâm Lang Thiên/);
  await expect(card.locator('[data-relation="has_level"], [data-relation="has_skill"], [data-relation="owns"]')).toHaveCount(0);
  await expect(card.locator('[data-wiki-rel-more] [data-relation="leader_of"]')).toContainText('Đạo Tông');
  await expect(card.getByText('Gặp nhau trong thạch phù.')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-character.png`, fullPage: true });
  await card.getByRole('button', { name: /Tiểu Điêu/ }).click();
  await expect(page.getByRole('dialog', { name: 'Tiểu Điêu' })).toBeVisible();
  await page.getByRole('dialog', { name: 'Tiểu Điêu' }).getByRole('button', { name: /Lâm Động/ }).click();
  await page.waitForTimeout(500);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-reader-card.png` });
  // Smart merge for the entry open while reading: the original names translated as its name.
  await card.getByRole('button', { name: 'Gộp thông minh' }).click();
  const merge = page.locator('[data-wiki-smart-merge-sheet]');
  await expect(merge.locator('[data-merge-raw]')).toHaveCount(3);
  await expect(merge.locator('[data-merge-raw]').nth(1)).toContainText('Lin Dong');
  await expect(merge.locator('[data-merge-raw]').nth(1)).toContainText('chương 3, 7');
  await expect(merge.locator('[data-merge-raw]').nth(1)).toContainText('chưa có trong wiki');
  expect((await calls(page)).find(c => c.cmd === 'wiki_merge_entry')?.args).toMatchObject({ novelId: 1, uid: 'a' });
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-entry-merge.png` });
  await merge.getByRole('button', { name: 'Hủy' }).click();
  await expect(merge).toHaveCount(0);
  await card.getByRole('button', { name: 'Wiki truyện' }).click();
  await expect(page.locator('[data-entry="林动"]').getByText('Lâm Động', { exact: true })).toBeVisible();
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

test('a long wiki description folds to two lines and expands in place', async ({ page }, info) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Wiki truyện/ }).filter({ visible: true }).first().click();

  const card = page.locator('[data-entry="道宗"]');
  const summary = card.locator('[data-wiki-summary]');
  const folded = await summary.evaluate(el => el.clientHeight);
  // A short description has nothing more to show.
  await expect(page.locator('[data-entry="青阳镇"] [data-wiki-more]')).toHaveCount(0);

  await card.getByRole('button', { name: 'Xem thêm' }).click();
  await expect(card.getByRole('button', { name: 'Thu gọn' })).toHaveAttribute('aria-expanded', 'true');
  expect(await summary.evaluate(el => el.clientHeight)).toBeGreaterThan(folded);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-expanded.png` });
  // Expanding does not open the entry.
  await expect(page.getByRole('dialog')).toHaveCount(0);

  await card.getByRole('button', { name: 'Thu gọn' }).click();
  expect(await summary.evaluate(el => el.clientHeight)).toBe(folded);
  // The card itself still opens the entry.
  await card.click({ position: { x: 20, y: 14 } });
  await expect(page.getByRole('dialog', { name: 'Mục wiki' })).toBeVisible();
});

test('a skill tells its grade and difficulty, a faction its leaders in turn', async ({ page }, info) => {
  await nativeApp(page);
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.aiView.:1', JSON.stringify({ lang: 'vi', style: 'Natural', dict: true, view: 'vi' }));
  });
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  await page.locator('[data-part="0"] [data-wiki-term="c"]').first().click();
  const sect = page.getByRole('dialog', { name: 'Đạo Tông' });
  const leaders = sect.locator('[data-wiki-leaders] [data-leader]');
  await expect(leaders).toHaveCount(2);
  await expect(leaders.nth(0)).toContainText('Lần thứ 1');
  await expect(leaders.nth(0)).toContainText('Ứng Huyền Tử');
  await expect(leaders.nth(0)).toContainText('Tông chủ đời thứ ba.');
  await expect(leaders.nth(1)).toContainText('Lần thứ 2');
  await expect(leaders.nth(1)).toContainText('Lâm Động');
  await expect(leaders.nth(1)).toContainText('hiện tại');
  await expect(leaders.nth(1)).toContainText('từ chương 900');
  await expect(sect.locator('[data-relation="leader_of"]')).toHaveCount(0);
  await page.waitForTimeout(600);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-faction.png` });

  // From the leader to his skill: what it is and how high it ranks, not who knows it.
  await leaders.nth(1).click();
  const lam = page.getByRole('dialog', { name: 'Lâm Động' });
  await lam.locator('[data-profile="abilities"] [data-wiki-skill]').click();
  const fist = page.getByRole('dialog', { name: 'Thông Bối Quyền' });
  await expect(fist.getByText('Quyền pháp của Lâm gia, đánh liên hoàn như sóng cuộn.')).toBeVisible();
  await expect(fist.locator('[data-skill="grade"]')).toContainText('Hoàng giai');
  await expect(fist.locator('[data-skill="difficulty"]')).toContainText('Dễ nhập môn');
  await expect(fist.locator('[data-relation]')).toHaveCount(0);
  await page.waitForTimeout(600);
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-skill.png` });
});

test('smart merge lists pairs to review and merges only the ticked ones', async ({ page }, info) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Wiki truyện/ }).filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Gộp thông minh' }).click();
  const sheet = page.locator('[data-wiki-smart-merge-sheet]');
  await expect(sheet.locator('[data-merge-pair]')).toHaveCount(2);
  // Each pair shows the replace line it adds; a near name is not ticked at first.
  await expect(sheet.getByText('Thay thế: Seol-Hwi → Tuyết Huy, Seol Hwi → Tuyết Huy')).toBeVisible();
  const boxes = sheet.locator('input[type=checkbox]');
  await expect(boxes.nth(0)).toBeChecked();
  await expect(boxes.nth(1)).not.toBeChecked();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-smart-merge.png` });
  // The AI finds more from the descriptions, in the background; what it finds is ticked, with its reason.
  await sheet.getByRole('button', { name: 'Hỏi AI tìm thêm' }).click();
  const group = sheet.locator('[data-merge-group]');
  await expect(group).toHaveCount(1);
  // A lost connection pauses the run with the reason; carrying on picks it up.
  await expect(sheet.locator('[data-merge-run]')).toContainText('Tạm dừng ở khối 1/3');
  await sheet.locator('[data-merge-run-resume]').click();
  await expect(sheet.locator('[data-merge-run]')).toContainText('AI đang đọc khối 1/3');
  await expect(sheet.locator('[data-merge-run-pause]')).toBeVisible();
  // Select all ticks every pair and group, and unticks them all again.
  await sheet.locator('[data-merge-pick-all]').click();
  await expect(sheet.getByRole('button', { name: 'Gộp 3 mục chọn' })).toBeVisible();
  await sheet.locator('[data-merge-pick-all]').click();
  await expect(sheet.getByRole('button', { name: 'Gộp 0 mục chọn' })).toBeVisible();
  await sheet.locator('[data-merge-pick-all]').click();
  await sheet.locator('[data-merge-pair]').nth(1).click();
  await expect(boxes.nth(2)).not.toBeChecked();
  await expect(sheet.getByText('Mô tả cho thấy là một người.')).toBeVisible();
  await expect(group.locator('input[type=checkbox]')).toBeChecked();
  // The final name can be changed before merging.
  await group.locator('[data-final-name]').fill('Lâm Khiếu lão gia');
  await expect(group.getByText('Thay thế: Ứng Huyền Tử → Lâm Khiếu lão gia')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-smart-merge-ai.png` });
  await sheet.getByRole('button', { name: 'Gộp 2 mục chọn' }).click();
  await expect(page.getByText('Đã gộp 2 mục, thêm 2 dòng thay thế.')).toBeVisible();
  expect((await calls(page)).find(c => c.cmd === 'wiki_merge_apply')?.args).toMatchObject({
    novelId: 1, picks: [{ fromUid: 's2', intoUid: 's1' }], groups: [{ intoUid: 'h', fromUids: ['l'], name: 'Lâm Khiếu lão gia' }]
  });
  await expect(sheet).toHaveCount(0);

  // One entry's own smart merge, from its sheet: it takes the sheet's place, Cancel brings it back.
  await page.getByText('Lâm Động', { exact: true }).click();
  await page.locator('[data-entry-smart-merge]').click();
  await expect(sheet.locator('[data-merge-raw]')).toHaveCount(3);
  await expect(page.locator('form.dialog')).toHaveCount(0);
  await expect(sheet.locator('[data-merge-pair]')).toHaveCount(1);
  await sheet.getByRole('button', { name: 'Hỏi AI tìm thêm' }).click();
  // The AI's group holds the device's pair, so the pair is shown there only.
  await expect(sheet.locator('[data-merge-group]')).toHaveCount(1);
  await expect(sheet.locator('[data-merge-pair]')).toHaveCount(0);
  await expect(sheet.locator('[data-final-name]')).toHaveValue('Lâm Động');
  // A wrong original name comes back put right, and can be changed too.
  await expect(sheet.locator('[data-final-raw]')).toHaveValue('Lin Dong');
  await expect(sheet.getByText('(đang ghi: 林动)')).toBeVisible();
  await expect(sheet.getByText('AI sửa tên gốc')).toBeVisible();
  await page.screenshot({ path: `test-results/wiki-${info.project.name}-entry-merge-ai.png` });
  await sheet.getByRole('button', { name: 'Hủy' }).click();
  await expect(page.locator('form.dialog')).toBeVisible();
});
