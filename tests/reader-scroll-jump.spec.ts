import { expect, test } from '@playwright/test';

// Reading down a book part after part, the text must move with the finger: the reader
// drops the parts far above and adds the next ones, and none of that may move the page.

test('scrolling to the end of a book never jumps', async ({ page }, info) => {
  test.skip(info.project.name !== 'phone', 'one device is enough');
  test.setTimeout(120_000);
  const N = 30;
  await page.addInitScript((N: number) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const sections = Array.from({ length: N }, (_, s) => ({ label: `Title ${s + 1}`, pages: null, number: s + 1 }));
    const imported = {
      novelId: 1, title: 'Long Novel', author: null, description: '', lang: 'en', format: 'txt', sourceName: 'n.txt', chapterCount: N,
      createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 20, progress: 0, lastReadAt: new Date().toISOString() }
    };
    const book = { id: 7, format: 'txt', title: 'Long Novel', sections, pageImages: false };
    const w = window as unknown as Record<string, unknown>;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    let next = 1;
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        switch (cmd) {
          case 'take_opened_files': return [];
          case 'library_list': return [imported];
          case 'library_detail': return { book: imported, sections };
          case 'library_open': return { book, novelId: 1, lang: 'en', index: 20, progress: 0 };
          case 'book_section': {
            const s = args.index as number;
            // Parts of very different lengths, loading at different speeds.
            await new Promise(r => setTimeout(r, 30 + (s * 37) % 120));
            const n = 20 + (s * 53) % 60;
            return { index: s, paragraphs: [`Chapter ${s + 1}`, ...Array.from({ length: n }, (_, i) => `§${s}.${i} ` + 'Rain fell over the harbour and nobody spoke a word. '.repeat(1 + (i % 4)))] };
          }
          case 'plugin:event|listen': return 1;
          default: return null;
        }
      }
    };
  }, N);
  await page.goto('/');
  await page.getByText('Long Novel').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Continue/ }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-part="20"] p').first()).toBeVisible();

  const result = await page.evaluate(async () => {
    const el = document.querySelector<HTMLElement>('.nx-scroll')!;
    /** The paragraph at the middle of the view, as a global position (part * 1000 + paragraph). */
    const at = () => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      const p = hit?.closest('p');
      const m = p && /^§(\d+)\.(\d+)/.exec(p.textContent ?? '');
      return m ? { s: Number(m[1]), i: Number(m[2]), y: p!.getBoundingClientRect().top } : null;
    };
    const STEP = 400;
    const jumps: string[] = [];
    const tick = () => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    type At = NonNullable<ReturnType<typeof at>>;
    const show = (a: At) => `${a.s}.${a.i}@${Math.round(a.y)}`;
    /** Checks a move of `dy` px (down when positive) went to the paragraphs it should. */
    const check = (prev: At | null, cur: At | null, dy: number) => {
      if (!prev || !cur) return;
      const before = cur.s < prev.s || (cur.s === prev.s && cur.i < prev.i);
      const after = cur.s > prev.s || (cur.s === prev.s && cur.i > prev.i);
      const far = Math.abs(cur.s - prev.s) > 1 || (cur.s === prev.s && Math.abs(cur.i - prev.i) > 12);
      const wrongWay = dy > 0 ? before : dy < 0 ? after : before || after;
      // Same paragraph: it must have moved by exactly what was scrolled.
      const moved = cur.s === prev.s && cur.i === prev.i && Math.abs(prev.y - cur.y - dy) > 4;
      if (wrongWay || far || moved) jumps.push(`${dy}: ${show(prev)} -> ${show(cur)}`);
    };
    let prev = at();
    const last = () => document.querySelector('[data-part="29"][data-ready]');
    // Down to the end of the book.
    for (let k = 0; k < 2500 && !(last() && el.scrollTop + el.clientHeight >= el.scrollHeight - 2); k++) {
      const room = el.scrollHeight - el.clientHeight - el.scrollTop;
      // At the bottom while the next part loads: wait for it, as a reader would; nothing may move.
      const dy = room >= STEP ? STEP : last() ? Math.floor(room) : 0;
      if (dy) el.scrollTop += dy; else await new Promise(r => setTimeout(r, 50));
      await tick();
      const cur = at();
      check(prev, cur, dy);
      if (cur) prev = cur;
    }
    const end = prev;
    // Back up a few parts: the ones dropped come back in place.
    for (let k = 0; k < 2500 && prev && prev.s > 22; k++) {
      const dy = el.scrollTop < STEP ? 0 : -STEP;
      if (dy) el.scrollTop += dy; else await new Promise(r => setTimeout(r, 50));
      await tick();
      const cur = at();
      check(prev, cur, dy);
      if (cur) prev = cur;
    }
    return { jumps, end, back: prev };
  });
  console.log('jumps', JSON.stringify(result));
  expect(result.end?.s).toBe(N - 1);
  expect(result.back?.s).toBe(22);
  expect(result.jumps).toEqual([]);
});
