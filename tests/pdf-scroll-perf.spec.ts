import { expect, test } from '@playwright/test';

// Scrolling through a long PDF (10 pages a part, hundreds of paragraphs) must stay
// smooth: before, every scroll event re-rendered each paragraph on screen and each
// frame took ~80 ms on a 4x slower CPU (p95 ~120 ms); now ~17 ms (p95 ~25 ms).

test('scrolling a long PDF stays smooth', async ({ page }, info) => {
  test.skip(info.project.name !== 'phone', 'one device is enough');
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const sections = Array.from({ length: 40 }, (_, s) => ({ label: null, pages: [s * 10 + 1, s * 10 + 10] }));
    const imported = {
      novelId: 1, title: 'Long PDF', author: null, description: '', lang: 'en', format: 'pdf', sourceName: 'long.pdf', chapterCount: sections.length,
      createdAt: '2026-10-01T00:00:00.000Z', lastRead: { index: 0, progress: 0, lastReadAt: new Date().toISOString() }
    };
    const book = { id: 7, format: 'pdf', title: 'Long PDF', sections, pageImages: true };
    const sentence = 'The rain kept falling over the old harbour while the lanterns swung in the wind and nobody spoke. ';
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
          case 'library_open': return { book, novelId: 1, lang: 'en', index: 0, progress: 0 };
          case 'book_section':
            return { index: args.index, paragraphs: Array.from({ length: 300 }, (_, i) => `${i + 1}. ${sentence.repeat(4)}`) };
          case 'replace_rules_get': return [{ from: 'harbour', to: 'harbor' }, { from: 'lanterns', to: 'lamps' }];
          case 'plugin:event|listen': return 1;
          default: return null;
        }
      }
    };
  });
  await page.goto('/');
  await page.getByText('Long PDF').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: /Continue/ }).filter({ visible: true }).first().click();
  await expect(page.locator('[data-part="0"] p').first()).toBeVisible();

  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate: 4 });
  const result = await page.evaluate(async () => {
    const el = document.querySelector<HTMLElement>('.nx-scroll')!;
    const long: number[] = [];
    const po = new PerformanceObserver(l => l.getEntries().forEach(e => long.push(e.duration)));
    po.observe({ type: 'longtask', buffered: false });
    const frames: number[] = [];
    let last = performance.now();
    for (let k = 0; k < 150; k++) {
      el.scrollTop += 40;
      await new Promise(r => requestAnimationFrame(r));
      const now = performance.now();
      frames.push(now - last);
      last = now;
    }
    po.disconnect();
    frames.sort((a, b) => a - b);
    return {
      longTasks: long.length,
      longMs: Math.round(long.reduce((a, b) => a + b, 0)),
      p50: Math.round(frames[Math.floor(frames.length / 2)]),
      p95: Math.round(frames[Math.floor(frames.length * 0.95)]),
      total: Math.round(frames.reduce((a, b) => a + b, 0)),
    };
  });
  console.log('PDF scroll', JSON.stringify(result));
  expect(result.p95).toBeLessThan(60);
});
