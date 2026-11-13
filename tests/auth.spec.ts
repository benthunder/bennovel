import { expect, test, type Page, type Route } from '@playwright/test';

// Accounts against a stand-in for the BenNovel API (the staging URL the dev build uses):
// register, log in, forgot password, refresh and log out. The web build runs here; the
// native app differs only in how Google comes back.

const API = 'https://api-staging.benthunder.io.vn';
const USER = { id: 'u1', email: 'lina@example.com', name: 'Lina Hart', hasPassword: true, google: false };
const FREE = { code: 'free', name: 'Free', expiresAt: null, features: ['catalog'] };
const tokens = (n: number) => ({ accessToken: `access-${n}`, expiresIn: 3600, refreshToken: `refresh-${n}` });

interface Calls { path: string; body: Record<string, unknown>; auth?: string }

async function fakeApi(page: Page, handle: (path: string, body: Record<string, unknown>) => { status?: number; json?: unknown } | undefined) {
  const calls: Calls[] = [];
  await page.route(`${API}/**`, async (route: Route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname;
    const body = (req.postDataJSON() ?? {}) as Record<string, unknown>;
    calls.push({ path, body, auth: req.headers().authorization });
    const r = handle(path, body) ?? (path === '/v1/me' ? { json: { user: USER, subscription: FREE } } : { status: 404, json: { error: 'not_found' } });
    await route.fulfill({ status: r.status ?? 200, contentType: 'application/json', body: JSON.stringify(r.json ?? {}) });
  });
  return calls;
}

async function start(page: Page, init: Record<string, unknown> = {}) {
  await page.addInitScript(values => {
    if (localStorage.getItem('bennovel.started')) return;
    localStorage.setItem('bennovel.started', '1');
    localStorage.setItem('bennovel.uiLang', JSON.stringify('en'));
    for (const [k, v] of Object.entries(values)) localStorage.setItem(`bennovel.${k}`, JSON.stringify(v));
  }, init);
  await page.goto('/');
}

const openProfile = (page: Page) => page.getByRole('button', { name: /profile/i }).first().click();

const onlineSwitch = (page: Page) => page.getByRole('switch', { name: /Use online/ });

test('the first launch asks to sign in or use offline; offline makes no API calls', async ({ page }) => {
  const calls = await fakeApi(page, () => undefined);
  await start(page);
  await expect(page.getByRole('button', { name: 'Continue with Google' })).toBeVisible();
  await page.getByRole('button', { name: 'Use offline' }).click();
  await expect(page.getByRole('button', { name: 'Continue with Google' })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Continue with Google' })).toHaveCount(0);
  await openProfile(page);
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('button', { name: 'Log out' })).toHaveCount(0);
  expect(calls).toEqual([]);
});

test('offline ignores a session left from before and calls nothing', async ({ page }) => {
  const calls = await fakeApi(page, () => undefined);
  const user = { id: 'u1', name: 'Lina Hart', email: 'lina@example.com', provider: 'email' };
  await start(page, { appMode: 'offline', session: { user, accessToken: 'old', accessExp: 0, refreshToken: 'refresh-0' } });
  await openProfile(page);
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByText('lina@example.com', { exact: true })).toHaveCount(0);
  expect(calls).toEqual([]);
});

test('Settings: turn online on by signing in, then back to offline', async ({ page }) => {
  const calls = await fakeApi(page, path =>
    path === '/v1/auth/login' ? { json: { user: USER, tokens: tokens(5) } } : path === '/v1/auth/logout' ? { json: {} } : undefined);
  await start(page, { appMode: 'offline' });
  await openProfile(page);
  await onlineSwitch(page).click();
  // Not now: still offline.
  await page.getByRole('button', { name: 'Not now' }).click();
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'false');

  await onlineSwitch(page).click();
  await expect(page.getByRole('button', { name: 'Use offline' })).toHaveCount(0);
  await page.getByLabel('Email').fill('lina@example.com');
  await page.getByLabel('Password').fill('secret123');
  await page.getByRole('button', { name: 'Log in', exact: true }).last().click();
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByText('Plan: Free')).toBeVisible();
  expect(calls.find(c => c.path === '/v1/me')?.auth).toBe('Bearer access-5');

  await onlineSwitch(page).click();
  await page.getByRole('dialog', { name: 'Switch to offline?' }).getByRole('button', { name: 'Switch to offline' }).click();
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByText('lina@example.com', { exact: true })).toHaveCount(0);
  await expect.poll(() => calls.find(c => c.path === '/v1/auth/logout')?.body).toEqual({ refreshToken: 'refresh-5' });
  const before = calls.length;
  await page.reload();
  await openProfile(page);
  await expect(onlineSwitch(page)).toHaveAttribute('aria-checked', 'false');
  expect(calls.length).toBe(before);
});

test('register, see the plan, log out', async ({ page }) => {
  const calls = await fakeApi(page, path =>
    path === '/v1/auth/signup' ? { json: { user: USER, tokens: tokens(1) } } : path === '/v1/auth/logout' ? { json: {} } : undefined);
  await start(page);
  await page.locator('.seg-opt', { hasText: 'Register' }).click();
  await page.getByLabel('Your name').fill('Lina Hart');
  await page.getByLabel('Email').fill('lina@example.com');
  await page.getByLabel('Password').fill('short');
  await page.getByRole('button', { name: 'Create account' }).click();
  await expect(page.getByText('Password needs at least 8 characters')).toBeVisible();
  await page.getByLabel('Password').fill('secret123');
  await page.getByRole('button', { name: 'Create account' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Welcome, Lina' })).toBeVisible();
  expect(calls.find(c => c.path === '/v1/auth/signup')?.body).toEqual({ name: 'Lina Hart', email: 'lina@example.com', password: 'secret123' });

  await openProfile(page);
  await expect(page.getByText('lina@example.com', { exact: true })).toBeVisible();
  await expect(page.getByText('Plan: Free')).toBeVisible();
  expect(calls.find(c => c.path === '/v1/me')?.auth).toBe('Bearer access-1');

  await page.getByRole('button', { name: 'Log out' }).click();
  // Online needs an account: the login screen is back, with the way to go offline.
  await expect(page.getByRole('button', { name: 'Use offline' })).toBeVisible();
  await expect.poll(() => calls.find(c => c.path === '/v1/auth/logout')?.body).toEqual({ refreshToken: 'refresh-1' });
});

test('a wrong password shows the error', async ({ page }) => {
  await fakeApi(page, path => (path === '/v1/auth/login' ? { status: 401, json: { error: 'invalid_credentials' } } : undefined));
  await start(page);
  await page.getByLabel('Email').fill('lina@example.com');
  await page.getByLabel('Password').fill('wrongpass');
  await page.getByRole('button', { name: 'Log in', exact: true }).last().click();
  await expect(page.getByRole('alert')).toHaveText('Wrong email or password');
});

test('forgot password: code from the email, new password, signed in', async ({ page }) => {
  const calls = await fakeApi(page, (path, body) => {
    if (path === '/v1/auth/forgot-password') return { json: {} };
    if (path === '/v1/auth/reset-password') {
      return body.code === '123456' ? { json: { user: USER, tokens: tokens(2) } } : { status: 400, json: { error: 'invalid_code' } };
    }
  });
  await start(page);
  await page.getByRole('button', { name: 'Forgot password?' }).click();
  await page.getByLabel('Email').fill('lina@example.com');
  await page.getByRole('button', { name: 'Send code' }).click();
  await expect(page.getByText('Sent to lina@example.com')).toBeVisible();

  await page.getByLabel('Code from the email').fill('000000');
  await page.getByLabel('New password').fill('newpass123');
  await page.getByRole('button', { name: 'Save password' }).click();
  await expect(page.getByRole('alert')).toHaveText('This code is wrong or has expired');

  await page.getByLabel('Code from the email').fill('123456');
  await page.getByRole('button', { name: 'Save password' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Password changed' })).toBeVisible();
  expect(calls.filter(c => c.path === '/v1/auth/reset-password').at(-1)?.body).toEqual({ email: 'lina@example.com', code: '123456', password: 'newpass123' });
});

test('an expired access token is refreshed before asking for the plan', async ({ page }) => {
  const calls = await fakeApi(page, path => (path === '/v1/auth/refresh' ? { json: { user: USER, tokens: tokens(3) } } : undefined));
  const user = { id: 'u1', name: 'Lina Hart', email: 'lina@example.com', provider: 'email' };
  await start(page, { onboarded: true, session: { user, accessToken: 'old', accessExp: 0, refreshToken: 'refresh-0' } });
  await openProfile(page);
  await expect(page.getByText('Plan: Free')).toBeVisible();
  expect(calls.find(c => c.path === '/v1/auth/refresh')?.body).toEqual({ refreshToken: 'refresh-0' });
  expect(calls.find(c => c.path === '/v1/me')?.auth).toBe('Bearer access-3');
});

test('a revoked session signs out and asks to sign in again', async ({ page }) => {
  await fakeApi(page, path => (path === '/v1/auth/refresh' ? { status: 401, json: { error: 'invalid_refresh' } } : undefined));
  const user = { id: 'u1', name: 'Lina Hart', email: 'lina@example.com', provider: 'email' };
  await start(page, { onboarded: true, session: { user, accessToken: 'old', accessExp: 0, refreshToken: 'gone' } });
  // Asking for the plan at launch finds the session gone.
  await expect(page.getByRole('button', { name: 'Use offline' })).toBeVisible();
});

test('Google: back on the callback page with a code', async ({ page }) => {
  const calls = await fakeApi(page, path => (path === '/v1/auth/exchange' ? { json: { user: { ...USER, google: true }, tokens: tokens(4) } } : undefined));
  await start(page);
  await page.goto('/auth/callback?code=one-time');
  await expect(page.getByRole('status').filter({ hasText: 'Welcome, Lina' })).toBeVisible();
  expect(calls.find(c => c.path === '/v1/auth/exchange')?.body).toEqual({ code: 'one-time' });
  await expect(page).toHaveURL(/\/$/);
});
