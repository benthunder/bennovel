import { expect, test, type Page } from '@playwright/test';

// Profile → Advanced settings: how many requests of a chapter go to the translation
// API at the same time. The native side is stood in for; nothing reaches a server.

async function nativeApp(page: Page, uiLang: 'en' | 'vi') {
  await page.addInitScript(({ uiLang }) => {
    localStorage.setItem('bennovel.onboarded', 'true');
    localStorage.setItem('bennovel.uiLang', JSON.stringify(uiLang));
    Object.defineProperty(Navigator.prototype, 'onLine', { get: () => false });
    const w = window as unknown as Record<string, unknown>;
    let next = 1;
    const saved: number[] = [];
    let value = 3;
    w.__saved = saved;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string, proto: string) => `${proto}://localhost/${p}`,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        switch (cmd) {
          case 'take_opened_files': return [];
          case 'library_list': return [];
          case 'llm_config_get': return { provider: 'openai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini', hasKey: true };
          case 'ai_parallel_get': return { value, default: 3, min: 1, max: 10 };
          case 'ai_parallel_set':
            value = Math.max(1, Math.min(10, args.value as number));
            saved.push(value);
            return { value, default: 3, min: 1, max: 10 };
          default: return null;
        }
      }
    };
  }, { uiLang });
}

const shot = async (page: Page, name: string) => {
  await page.waitForTimeout(400); // let the slide-up finish
  await page.screenshot({ path: `test-results/advanced/${test.info().project.name}-${name}.png` });
};

test('translation threads are set in Profile and kept in their range', async ({ page }) => {
  await nativeApp(page, 'en');
  await page.goto('/');
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
  const row = page.getByRole('button', { name: /Advanced settings/ });
  await expect(row).toContainText('3 threads');
  await row.click();
  const dialog = page.getByRole('dialog', { name: 'Advanced settings' });
  const input = dialog.getByRole('spinbutton', { name: 'Translation threads at once' });
  await expect(input).toHaveValue('3');
  await expect(dialog.getByRole('button', { name: 'Reset to default' })).toHaveCount(0);

  await dialog.getByRole('button', { name: 'One thread more' }).click();
  await dialog.getByRole('button', { name: 'One thread more' }).click();
  await expect(input).toHaveValue('5');
  await expect(dialog.getByRole('button', { name: 'Reset to default' })).toBeVisible();

  // Typed values are kept in 1..10.
  await input.fill('25');
  await expect(input).toHaveValue('10');
  await expect(dialog.getByRole('button', { name: 'One thread more' })).toBeDisabled();
  await input.fill('0');
  await expect(input).toHaveValue('1');
  await expect(dialog.getByRole('button', { name: 'One thread less' })).toBeDisabled();

  await input.fill('6');
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __saved: number[] }).__saved)).toEqual([6]);
  await expect(row).toContainText('6 threads');
});

test('advanced settings in Vietnamese (screenshots)', async ({ page }) => {
  await nativeApp(page, 'vi');
  await page.goto('/');
  await page.getByRole('button', { name: 'Cá nhân' }).filter({ visible: true }).first().click();
  const row = page.getByRole('button', { name: /Cài đặt nâng cao/ });
  await expect(row).toContainText('3 luồng');
  await row.scrollIntoViewIfNeeded();
  await shot(page, 'profile-row');
  await row.click();
  const dialog = page.getByRole('dialog', { name: 'Cài đặt nâng cao' });
  await expect(dialog).toBeVisible();
  await shot(page, 'dialog-default');
  await dialog.getByRole('button', { name: 'Thêm một luồng' }).click();
  await dialog.getByRole('button', { name: 'Thêm một luồng' }).click();
  await expect(dialog.getByRole('spinbutton')).toHaveValue('5');
  await shot(page, 'dialog-changed');
});
