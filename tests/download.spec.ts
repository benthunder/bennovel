import { expect, test, type Page, type Route } from '@playwright/test';

// Downloading an online novel to read offline. The native app (its library file) and
// Supabase are stood in for, so these run where Supabase is blocked too.

const NOVEL = {
  id: 101, title: 'Saved Tale', author: 'Mira Vale', cat: 'fantasy', status: 'Ongoing', chapters: 2, rating: 4.6, reads: 12, year: 2024,
  desc: { en: 'A tale kept for the road.', vi: '' }, cover: { bg: 'var(--color-accent-300)', fg: 'var(--color-accent-900)', deco: 'var(--color-accent-500)' }
};

/** Chapters as Supabase returns them: one original (English) and one AI Vietnamese text each. */
const CHAPTERS = [1, 2].map(number => ({
  number,
  chapter_translations: [
    { lang: 'en', source: 'original', style: null, title: `The road ${number}`, content: `Words of chapter ${number}.\n\nWith [[hero]] on it.`, word_count: 900 },
    { lang: 'vi', source: 'ai', style: 'natural', title: `Con đường ${number}`, content: `Chữ của chương ${number}.`, word_count: 800 }
  ]
}));

async function boot(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
  });
}

const json = (route: Route, body: unknown, headers: Record<string, string> = {}) =>
  route.fulfill({ status: 200, contentType: 'application/json', headers, body: JSON.stringify(body) });

/** Supabase with one novel, NOVEL, of two chapters. */
async function server(page: Page) {
  await page.route('**/auth/v1/**', route => route.abort());
  await page.route('**/rest/v1/**', route => {
    const url = new URL(route.request().url());
    switch (url.pathname.split('/').pop()) {
      case 'novels': return json(route, [{
        id: NOVEL.id, status: 'ongoing', chapter_count: 2, rating_avg: 4.6, read_count: 12000, published_year: 2024, cover_palette: null,
        original_lang: 'en', category_slug: 'fantasy', authors: { name: NOVEL.author },
        novel_translations: [{ lang: 'en', title: NOVEL.title, description: NOVEL.desc.en }]
      }]);
      case 'categories': return json(route, [{ slug: 'fantasy', name: 'Fantasy', sort_order: 1, category_translations: [] }]);
      case 'collections': return json(route, []);
      case 'languages': return json(route, [{ code: 'en', name: 'English', native_name: 'English' }, { code: 'vi', name: 'Vietnamese', native_name: 'Tiếng Việt' }]);
      case 'glossary_entries': return json(route, [{ term_key: 'hero', lang: 'en', value: 'Lin' }]);
      case 'chapters': return route.request().method() === 'HEAD'
        ? route.fulfill({ status: 200, headers: { 'content-range': '0-1/2' }, body: '' })
        : json(route, CHAPTERS.filter(c => !url.searchParams.get('number') || url.searchParams.get('number') === `eq.${c.number}`));
      default: return json(route, []);
    }
  });
}

/** The device reports no connection and Supabase can't be reached. */
async function goOffline(page: Page) {
  await page.addInitScript(() => Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false }));
  const calls: string[] = [];
  await page.route('**/rest/v1/**', route => { calls.push(route.request().url()); return route.abort(); });
  await page.route('**/auth/v1/**', route => route.abort());
  return calls;
}

/**
 * Stands in for the native app: NOVEL is a favourite, and the offline_* commands keep
 * downloads in memory (`downloaded`: what is on the device at start).
 */
async function nativeApp(page: Page, downloaded: { novel: typeof NOVEL; chapters: typeof CHAPTERS } | null) {
  await page.addInitScript(({ novel, downloaded }) => {
    type Row = { number: number; lang: string; source: string; title: string; content: string; words: number };
    const w = window as unknown as Record<string, unknown>;
    const dl = new Map<number, { data: unknown; total: number; complete: boolean; rows: Row[]; glossary: { key: string; lang: string; value: string }[] }>();
    if (downloaded) {
      dl.set(downloaded.novel.id, {
        data: downloaded.novel, total: 2, complete: true, glossary: [{ key: 'hero', lang: 'en', value: 'Lin' }],
        rows: downloaded.chapters.flatMap(c => c.chapter_translations.map(t => ({ number: c.number, lang: t.lang, source: t.source, title: t.title, content: t.content, words: t.word_count })))
      });
    }
    w.__downloads = dl;
    let next = 1;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, a: Record<string, never>) => {
        switch (cmd) {
          case 'reading_load': return { favs: [novel.id], history: [], novels: [{ id: novel.id, data: novel }] };
          case 'library_list': return [];
          case 'take_opened_files': return [];
          case 'offline_start': dl.set(a.novelId, { data: a.data, total: a.total, complete: false, rows: dl.get(a.novelId)?.rows ?? [], glossary: [] }); return 1;
          case 'offline_ensure': if (!dl.has(a.novelId)) dl.set(a.novelId, { data: a.data, total: a.total, complete: false, rows: [], glossary: [] }); return null;
          case 'offline_numbers': return [...new Set(dl.get(a.novelId)?.rows.map(r => r.number) ?? [])].sort();
          case 'offline_add': dl.get(a.novelId)!.rows.push(...(a.chapters as Row[])); return null;
          case 'offline_set_glossary': dl.get(a.novelId)!.glossary = a.entries; return null;
          case 'offline_finish': dl.get(a.novelId)!.complete = true; return null;
          case 'offline_delete': dl.delete(a.novelId); return null;
          case 'offline_list': return [...dl].map(([id, d]) => ({
            id, data: d.data, total: d.total, complete: d.complete, downloadedAt: 1,
            chapters: new Set(d.rows.map(r => r.number)).size,
            langs: [...new Set(d.rows.map(r => r.lang))].sort(),
            origLang: d.rows.find(r => r.source === 'original')?.lang ?? null
          }));
          case 'offline_chapters': return dl.get(a.novelId)!.rows.map(({ content: _, ...r }) => r);
          case 'offline_chapter': {
            const r = dl.get(a.novelId)?.rows.find(x => x.number === a.number && x.lang === a.lang);
            return r ? { title: r.title, content: r.content } : null;
          }
          case 'offline_glossary': return dl.get(a.novelId)?.glossary ?? [];
          default: return null;
        }
      }
    };
  }, { novel: NOVEL, downloaded });
}

test('Download saves every chapter and the glossary on the device', async ({ page }) => {
  await server(page);
  await nativeApp(page, null);
  await boot(page);
  await page.goto('/');
  await page.getByText('Saved Tale').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Download for offline reading' }).filter({ visible: true }).click();
  await expect(page.getByText('Downloaded “Saved Tale”. It opens without a connection now.')).toBeVisible();
  await expect(page.getByText('Downloaded · 2 chapters').filter({ visible: true })).toBeVisible();

  const stored = await page.evaluate(() => {
    const d = (window as unknown as { __downloads: Map<number, { rows: { number: number; lang: string }[]; glossary: unknown[]; complete: boolean }> }).__downloads.get(101)!;
    return { rows: d.rows.map(r => `${r.number}-${r.lang}`).sort(), glossary: d.glossary.length, complete: d.complete };
  });
  expect(stored).toEqual({ rows: ['1-en', '1-vi', '2-en', '2-vi'], glossary: 1, complete: true });

  // The library lists it, and it can be removed again.
  await page.getByRole('button', { name: 'Remove download' }).filter({ visible: true }).click();
  await expect(page.getByRole('button', { name: 'Download for offline reading' }).filter({ visible: true })).toBeVisible();
});

test('a single chapter can be downloaded and read offline', async ({ page }) => {
  await server(page);
  await nativeApp(page, null);
  await boot(page);
  await page.goto('/');
  await page.getByText('Saved Tale').filter({ visible: true }).first().click();
  await page.getByRole('button', { name: 'Download chapter 2' }).filter({ visible: true }).click();
  await expect(page.getByText('Downloaded chapter 2.')).toBeVisible();
  await expect(page.getByRole('img', { name: 'Chapter 2 is downloaded' }).filter({ visible: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Download chapter 1' }).filter({ visible: true })).toBeVisible();
  await expect(page.getByText('Partly downloaded · 1/2 chapters').filter({ visible: true })).toBeVisible();
  const rows = await page.evaluate(() =>
    (window as unknown as { __downloads: Map<number, { rows: { number: number; lang: string }[] }> }).__downloads.get(101)!.rows.map(r => `${r.number}-${r.lang}`).sort());
  expect(rows).toEqual(['2-en', '2-vi']);
});

test('a downloaded novel opens and reads without a connection', async ({ page }) => {
  const calls = await goOffline(page);
  await nativeApp(page, { novel: NOVEL, chapters: CHAPTERS });
  await boot(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Downloaded' })).toBeVisible();
  await page.getByText('Saved Tale').filter({ visible: true }).first().click();
  // The page and its chapters come from the device.
  await expect(page.getByText('The road 2').filter({ visible: true })).toBeVisible();
  await page.getByText('The road 2').filter({ visible: true }).click();
  // No language saved: it opens in the original's.
  await expect(page.getByText('Words of chapter 2.')).toBeVisible();
  await expect(page.getByText('With Lin on it.')).toBeVisible();
  expect(calls.filter(u => !u.includes('/replace_rules'))).toEqual([]);
});
