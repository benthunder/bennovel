import { expect, test, type Page } from '@playwright/test';

// The wiki of a book on the device (src-tauri/src/library/wiki.rs, with its own Rust
// tests). The native side is stood in for; these never reach a server.

const imported = {
  novelId: 1, title: 'Võ Động Càn Khôn', author: null, description: '', lang: 'zh', format: 'txt', sourceName: 'Vo_Dong_Can_Khon_full_bo_1-1300_chuong_convert_ban_dep_nhat_khong_co_khoang_trang.txt', chapterCount: 2,
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
          case 'wiki_get':
            // A Korean-English book's character whose aliases hold a piece of its name ("Minh").
            if (w.__minh && !wiki.entries.some(e => e.uid === 'x')) {
              wiki.entries.push(entry('x', 'character', 'Goo Jong-myung', 'Cố Tông Minh', 'Nhân vật được Chung Cực Ma giao nhiệm vụ.', 1,
                { aliases: ['Minh', 'Ju Myung', 'Đại Thánh'] }));
            }
            // A fresh copy each time, as the native side sends.
            return w.__minh ? structuredClone(wiki) : wiki;
          case 'wiki_unlink': {
            const e = wiki.entries.find(x => x.uid === args.uid)!;
            const word = String(args.word);
            Object.assign(e, { aliases: e.aliases.filter(a => a.toLowerCase() !== word.toLowerCase()), blocked: [...((e as { blocked?: string[] }).blocked ?? []), word] });
            return e;
          }
          case 'wiki_unblock': return null;
          case 'wiki_remove': wiki.entries = wiki.entries.filter(e => e.uid !== args.uid); return null;
          case 'library_open': return { book: { id: 9, format: 'txt', title: 'Võ Động Càn Khôn', sections: [{ label: 'Chương 1', pages: null }, { label: 'Chương 2', pages: null }], pageImages: false }, novelId: 1, lang: 'zh', index: 0, progress: 0 };
          case 'book_section':
            if (Number(w.__sectionFails) > 0) { w.__sectionFails = Number(w.__sectionFails) - 1; throw 'book 9 is not open'; }
            if (w.__minh) return { index: args.index, paragraphs: ['Nếu nhìn theo cách đó, Goo Jong-myung hẳn phải thuộc hệ Minh.', 'Đại Thánh cúi đầu gật gù.'] };
            return { index: args.index, paragraphs: ['——————————————————————————————————————————————', 'https://www.example.com/truyen/vo-dong-can-khon/chuong-1-lam-dong-di-ra-ngoai-thanh.html', '林动 看着 道宗 的 人。', '动少 笑了。', '萧炎 走了过来。'] };
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
          case 'wiki_merge_entry_allow': {
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            return { entry: by(String(args.intoUid)), applied: { merged: (args.fromUids as string[]).length, rules: 3, renamed: 0, fixed: 0 }, adopted: (args.raws as unknown[]).length };
          }
          case 'wiki_retranslate_ask': {
            const target = args.target as { uid?: string; text?: string };
            if (target.uid) {
              return {
                uid: target.uid, raw: '林动', current: 'Lâm Động', name: 'Lâm Động', kind: 'character', why: 'Hán-Việt của 林动, giữ như cũ.',
                contexts: [
                  { chapter: 1, raw: '林动 看着 道宗 的 人。 动少 笑了。', translated: 'Lâm Động nhìn người của Đạo Tông. Động thiếu cười.' },
                  { chapter: 3, raw: '林动 握紧了 石符。', translated: 'Lâm Đống nắm chặt thạch phù.' }
                ]
              };
            }
            return {
              uid: null, raw: '萧炎', current: String(target.text), name: 'Tiêu Viêm', kind: 'character', why: 'Tên người, đọc theo Hán-Việt.',
              contexts: [{ chapter: 1, raw: '动少 笑了。 萧炎 走了过来。', translated: '' }]
            };
          }
          case 'wiki_retranslate_apply': {
            const keep = args.keep as { uid: string | null; raw: string; name: string; kind: string };
            const by = (u: string) => wiki.entries.find(e => e.uid === u)!;
            if (keep.uid) {
              const e = { ...by(keep.uid), names: { vi: keep.name }, source: 'user' };
              return { entry: e, changed: 4, merge: { raws: [], pairs: [{ from: by('i'), into: e, reason: 'same_name', rules: [], note: '' }] } };
            }
            return { entry: entry('n', keep.kind, keep.raw, keep.name, '', null, { source: 'user' }), changed: 1, merge: { raws: [], pairs: [] } };
          }
          case 'wiki_levels_set': return (args.uids as string[]).length;
          case 'wiki_levels_ai': return [{ system: 'Võ đạo', uids: ['g', 'k', 'f'] }];
          case 'wiki_merge_apply': return { merged: (args.picks as unknown[]).length + ((args.groups as { fromUids: string[] }[]) ?? []).reduce((n, g) => n + g.fromUids.length, 0), rules: 2, renamed: 0 };
          case 'wiki_remove_many': {
            const uids = args.uids as string[];
            wiki.entries = wiki.entries.filter(e => !uids.includes(e.uid));
            return uids.length;
          }
          case 'plugin:dialog|message': return 'Yes';
          case 'wiki_apply_to_text': return 7;
          case 'wiki_merge_skip': return null;
          case 'replace_rules_get':
            return args.lang === 'vi' ? [{ find: 'Hắc Ma Chiến Quân', replace: 'Cố Đại Diễm' }, { find: 'Cổ Đại Viêm', replace: 'Cố Đại Diễm' }, { find: 'Lâm Đống', replace: 'Lâm Động' }] : null;
          case 'ai_marks': case 'ai_progress': case 'ai_versions': return [];
          default: return null;
        }
      }
    };
  }, { imported });
}

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: Calls }).__calls);

/** Whether any panel on screen can be scrolled sideways. */
const sideways = (page: Page) => page.evaluate(() => Array.from(document.querySelectorAll<HTMLElement>('*'))
  .filter(el => {
    const cs = getComputedStyle(el);
    return (cs.overflowX === 'auto' || cs.overflowX === 'scroll') && el.scrollWidth > el.clientWidth + 1 && !el.classList.contains('shelf');
  })
  .map(el => el.className));

test('a long file name, link or row of dashes never makes a phone screen scroll sideways', async ({ page }, info) => {
  test.skip(info.project.name !== 'phone');
  await page.setViewportSize({ width: 360, height: 780 });
  await nativeApp(page);
  await page.goto('/');
  await expect(page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first()).toBeVisible();
  expect(await sideways(page)).toEqual([]);
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await expect(page.getByText(/Vo_Dong_Can_Khon_full/)).toBeVisible();
  expect(await sideways(page)).toEqual([]);
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  await expect(page.getByText(/example\.com/).first()).toBeVisible();
  expect(await sideways(page)).toEqual([]);
});

test('the reader picks a sans, serif or mono face, and side margins leave the text 95%, 90% or 80% of the page', async ({ page }, info) => {
  await nativeApp(page);
  await page.goto('/');
  await page.getByText('Võ Động Càn Khôn').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Bắt đầu đọc|Đọc tiếp/ }).filter({ visible: true }).first().click();
  const para = page.locator('[data-part="0"] p').nth(2);
  await expect(para).toBeVisible();
  await page.getByRole('button', { name: 'Công cụ đọc' }).filter({ visible: true }).first().click();
  const fonts = page.getByRole('radiogroup', { name: 'Phông chữ' });
  await expect(fonts.getByRole('radio')).toHaveCount(4);
  await fonts.locator('[data-reader-font="mono"]').click();
  await expect(fonts.locator('[data-reader-font="mono"]')).toHaveAttribute('aria-checked', 'true');
  expect(await para.evaluate(el => getComputedStyle(el).fontFamily)).toContain('JetBrains Mono');
  await fonts.locator('[data-reader-font="serif"]').click();
  expect(await para.evaluate(el => getComputedStyle(el).fontFamily)).toContain('Noto Serif');

  if (info.project.name !== 'phone') return;
  // On a phone the text column is the page less its two margins.
  const share = () => page.evaluate(() => {
    const p = document.querySelector('[data-part="0"] p')!.getBoundingClientRect();
    return Math.round((p.width / document.documentElement.clientWidth) * 100);
  });
  await page.locator('.seg-opt', { hasText: 'Rộng · 80%' }).click();
  expect(await share()).toBe(80);
  await page.locator('.seg-opt', { hasText: 'Vừa · 90%' }).click();
  expect(await share()).toBe(90);
  await page.locator('.seg-opt', { hasText: 'Hẹp · 95%' }).click();
  expect(await share()).toBe(95);
});
