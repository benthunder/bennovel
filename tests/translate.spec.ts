import { expect, test, type Page, type Route } from '@playwright/test';

// Asking for AI translation: from a chapter on in the reader, and of the whole novel on the
// detail page. Supabase is stood in for, so these run where Supabase is blocked too.

const USER = { id: '7b0c2a52-1f4e-4c55-9a8e-2b8f9d1e0a11', name: 'Mai Tran', email: 'mai.tran@example.com', provider: 'email' };

const json = (route: Route, body: unknown) =>
  route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function boot(page: Page, signedIn: boolean) {
  await page.addInitScript(user => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    localStorage.setItem('bennovel.defaultLang', JSON.stringify('en'));
    if (user) localStorage.setItem('bennovel.user', JSON.stringify(user));
  }, signedIn ? USER : null);
}

/** One English novel of three chapters, none in Vietnamese yet. Returns the RPC calls made. */
async function server(page: Page) {
  const calls: { fn: string; args: Record<string, unknown> }[] = [];
  let queued = 0;
  await page.route('**/auth/v1/**', route => route.abort());
  await page.route('**/rest/v1/**', route => {
    const url = new URL(route.request().url());
    const name = url.pathname.split('/').pop();
    if (url.pathname.includes('/rpc/')) {
      const args = route.request().postDataJSON() as Record<string, unknown>;
      calls.push({ fn: name!, args });
      if (name === 'request_novel_translation') {
        queued = 4 - (args.p_from_number as number);
        return json(route, queued);
      }
      if (name === 'novel_translation_progress') return json(route, [{ lang: 'vi', total: 3, translated: 0, queued, running: 0, failed: 0 }]);
      return json(route, null);
    }
    switch (name) {
      case 'novels': return json(route, [{
        id: 7, status: 'ongoing', chapter_count: 3, rating_avg: 4.2, read_count: 900, published_year: 2025, cover_palette: null,
        original_lang: 'en', category_slug: 'fantasy', authors: { name: 'Ana Reyes' },
        novel_translations: [{ lang: 'en', title: 'Quiet Harbor', description: 'Boats and storms.' }]
      }]);
      case 'categories': return json(route, [{ slug: 'fantasy', name: 'Fantasy', sort_order: 1, category_translations: [] }]);
      case 'languages': return json(route, [{ code: 'en', name: 'English', native_name: 'English' }, { code: 'vi', name: 'Vietnamese', native_name: 'Tiếng Việt' }]);
      case 'chapters': return json(route, [1, 2, 3].map(number => ({ number, chapter_translations: [{ lang: 'en', source: 'original', title: `Tide ${number}`, word_count: 500 }] })));
      case 'chapter_translations': return json(route, { title: 'Tide 2', content: 'The boats came in.' });
      default: return json(route, []);
    }
  });
  return calls;
}

async function openNovel(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Home' }).filter({ visible: true }).click();
  await page.getByText('Quiet Harbor').filter({ visible: true }).first().click();
}

test('Reader queues AI translation from the open chapter on', async ({ page }) => {
  const calls = await server(page);
  await boot(page, true);
  await openNovel(page);
  await page.getByRole('button', { name: /Tide 2/ }).filter({ visible: true }).click();
  await expect(page.getByText('The boats came in.')).toBeVisible();

  await page.getByRole('button', { name: 'Reading tools' }).click();
  await page.getByText('AI tools', { exact: true }).click();
  await page.getByRole('button', { name: 'Translate from this chapter on' }).click();
  await expect(page.getByText('Queued 2 chapters from chapter 2 for Tiếng Việt.', { exact: false })).toBeVisible();
  expect(calls.find(c => c.fn === 'request_novel_translation')?.args).toEqual({
    p_novel_id: 7, p_target_lang: 'vi', p_from_number: 2, p_style: 'natural', p_apply_glossary: true
  });
});

test('Detail page queues the whole novel and shows its progress', async ({ page }) => {
  const calls = await server(page);
  await boot(page, true);
  await openNovel(page);

  await page.getByRole('button', { name: 'Translate the whole novel' }).filter({ visible: true }).click();
  await page.getByRole('button', { name: 'Literary' }).filter({ visible: true }).click();
  await page.getByRole('button', { name: 'Start translating' }).filter({ visible: true }).click();
  await expect(page.getByText('Queued 3 chapters for Tiếng Việt.')).toBeVisible();
  await expect(page.getByText('Tiếng Việt · 0/3 chapters translated · 3 waiting').filter({ visible: true })).toBeVisible();
  expect(calls.find(c => c.fn === 'request_novel_translation')?.args).toEqual({
    p_novel_id: 7, p_target_lang: 'vi', p_from_number: 1, p_style: 'literary', p_apply_glossary: true
  });
});

test('Translating needs an account', async ({ page }) => {
  const calls = await server(page);
  await boot(page, false);
  await openNovel(page);
  await page.getByRole('button', { name: 'Translate the whole novel' }).filter({ visible: true }).click();
  await expect(page.getByText('Sign in to ask for an AI translation.')).toBeVisible();
  expect(calls.some(c => c.fn === 'request_novel_translation')).toBe(false);
});
