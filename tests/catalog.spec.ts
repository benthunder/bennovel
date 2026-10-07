import { expect, test, type Page } from '@playwright/test';
import { categoryNames, chapterTexts, mostReadNovels, withGlossary } from './supabase';

/** First visible element with exactly this text (sliders and side panels repeat titles). */
const visibleText = (page: Page, text: string) => page.getByText(text, { exact: true }).filter({ visible: true }).first();

/** Skips onboarding and fixes the interface and reading languages. */
async function boot(page: Page, { ui = 'en', content = 'en' }: { ui?: string; content?: string } = {}) {
  await page.addInitScript(([ui, content]) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify(ui));
    localStorage.setItem('bennovel.defaultLang', JSON.stringify(content));
  }, [ui, content]);
}

test('home lists the most-read novels from Supabase', async ({ page }) => {
  const top = (await mostReadNovels()).slice(0, 3);
  await boot(page);
  await page.goto('/');
  for (const n of top) await expect(visibleText(page, n.title)).toBeVisible();
  await expect(page.getByText(top[0].author).filter({ visible: true }).first()).toBeVisible();
});

test('detail lists chapter titles from Supabase', async ({ page }) => {
  const [novel] = await mostReadNovels();
  const chapters = await chapterTexts(novel.id, 'en');
  await boot(page);
  await page.goto('/');
  await visibleText(page, novel.title).click();
  await expect(page.getByRole('heading', { name: novel.title })).toBeVisible();
  for (const c of chapters.slice(0, 3)) await expect(visibleText(page, c.title)).toBeVisible();
});

for (const lang of ['en', 'vi']) {
  test(`reader shows chapter 1 text in ${lang}`, async ({ page }) => {
    const [novel] = await mostReadNovels();
    const [ch1] = await chapterTexts(novel.id, lang);
    const firstPara = await withGlossary(novel.id, lang, ch1.paragraphs[0]);
    await boot(page, { content: lang });
    await page.goto('/');
    await visibleText(page, novel.title).click();
    await visibleText(page, ch1.title).click();
    await expect(page.getByRole('heading', { name: ch1.title })).toBeVisible();
    await expect(page.getByText(firstPara, { exact: true })).toBeVisible();
  });
}

test('categories use their Vietnamese names', async ({ page }) => {
  const names = await categoryNames('vi');
  await boot(page, { ui: 'vi' });
  await page.goto('/');
  await page.getByRole('button', { name: 'Thể loại' }).filter({ visible: true }).first().click();
  for (const name of names) await expect(visibleText(page, name)).toBeVisible();
});

test('shows a retry when Supabase is unreachable', async ({ page }) => {
  const [novel] = await mostReadNovels();
  await boot(page);
  await page.route('**/rest/v1/**', route => route.abort());
  await page.goto('/');
  // supabase-js retries failed reads with backoff (about 7s) before giving up.
  await expect(page.getByRole('alert')).toContainText('Could not reach the library', { timeout: 20_000 });
  await page.unroute('**/rest/v1/**');
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(visibleText(page, novel.title)).toBeVisible();
});
