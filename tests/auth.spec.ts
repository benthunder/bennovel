import { expect, test, type Page, type Route } from '@playwright/test';

// Email + password accounts. Supabase Auth is stood in for, so these tests never
// create real accounts or send email, and also run where Supabase is blocked.

const EMAIL = 'mai.tran@example.com';
const PASSWORD = 'secret123';

const b64 = (o: unknown) => Buffer.from(JSON.stringify(o)).toString('base64url');

function authUser(name = 'Mai Tran') {
  return {
    id: '7b0c2a52-1f4e-4c55-9a8e-2b8f9d1e0a11', aud: 'authenticated', role: 'authenticated', email: EMAIL,
    email_confirmed_at: '2026-10-10T00:00:00Z', app_metadata: { provider: 'email', providers: ['email'] },
    user_metadata: { full_name: name }, identities: [{ id: 'i1', provider: 'email' }],
    created_at: '2026-10-10T00:00:00Z', updated_at: '2026-10-10T00:00:00Z'
  };
}

function session(name?: string) {
  const user = authUser(name);
  const exp = Math.floor(Date.now() / 1000) + 3600;
  const jwt = `${b64({ alg: 'HS256', typ: 'JWT' })}.${b64({ sub: user.id, email: EMAIL, role: 'authenticated', aud: 'authenticated', exp })}.sig`;
  return { access_token: jwt, token_type: 'bearer', expires_in: 3600, expires_at: exp, refresh_token: 'refresh-1', user };
}

const json = (route: Route, status: number, body: unknown) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * Stands in for Supabase Auth. `confirmEmail`: new accounts wait for their email to be
 * confirmed. Returns the auth requests made, as "METHOD path".
 */
async function fakeAuth(page: Page, { confirmEmail = true } = {}) {
  const calls: string[] = [];
  let confirmed = !confirmEmail;
  await page.route('**/rest/v1/**', route => route.abort());
  await page.route('**/auth/v1/**', async route => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace('/auth/v1', '');
    const body = req.postDataJSON() as Record<string, string> | null;
    calls.push(`${req.method()} ${path}`);
    switch (path) {
      case '/signup':
        if (!confirmEmail) return json(route, 200, session(body?.data && (body.data as unknown as { full_name: string }).full_name));
        confirmed = true; // as if the link in the email was opened
        return json(route, 200, { ...authUser(), email_confirmed_at: null });
      case '/token':
        if (body?.password !== PASSWORD) return json(route, 400, { code: 400, error_code: 'invalid_credentials', msg: 'Invalid login credentials' });
        if (!confirmed) return json(route, 400, { code: 400, error_code: 'email_not_confirmed', msg: 'Email not confirmed' });
        return json(route, 200, session());
      case '/recover': return json(route, 200, {});
      case '/verify':
        if (body?.token !== '123456') return json(route, 403, { code: 403, error_code: 'otp_expired', msg: 'Token has expired or is invalid' });
        return json(route, 200, session());
      case '/user': return json(route, 200, authUser());
      case '/logout': return route.fulfill({ status: 204 });
      default: return json(route, 404, { msg: 'not found' });
    }
  });
  return calls;
}

/** English interface; the app opens on the sign-in screen until onboarded. */
async function boot(page: Page) {
  await page.addInitScript(() => localStorage.setItem('bennovel.uiLang', JSON.stringify('en')));
}

const form = (page: Page) => page.locator('form');
const submit = (page: Page, name: string) => form(page).getByRole('button', { name, exact: true }).click();

async function openProfile(page: Page) {
  await page.getByRole('button', { name: 'Profile' }).filter({ visible: true }).first().click();
}

test('register, confirm the email, log in, stay signed in, log out', async ({ page }) => {
  const calls = await fakeAuth(page);
  await boot(page);
  await page.goto('/');

  await page.locator('label.seg-opt', { hasText: 'Register' }).click();
  await page.getByRole('textbox', { name: 'Your name' }).fill('Mai Tran');
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Create account');
  await expect(page.getByRole('status').filter({ hasText: 'Almost done' })).toContainText(EMAIL);

  // Back on the log in tab with the email kept.
  await expect(page.getByRole('textbox', { name: 'Email' })).toHaveValue(EMAIL);
  await page.getByLabel('Password').fill('wrong-pass');
  await submit(page, 'Log in');
  await expect(page.getByRole('alert')).toHaveText('Wrong email or password');

  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Log in');
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
  await openProfile(page);
  await expect(page.getByText('Mai Tran', { exact: true })).toBeVisible();
  await expect(page.getByText(EMAIL)).toBeVisible();

  // The session survives a restart.
  await page.reload();
  await openProfile(page);
  await expect(page.getByText('Mai Tran', { exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Log out' }).click();
  await expect(page.getByText('Reading as a guest')).toBeVisible();
  expect(calls).toContain('POST /logout');
  await page.reload();
  await openProfile(page);
  await expect(page.getByText('Reading as a guest')).toBeVisible();
});

test('logging in before confirming the email says so', async ({ page }) => {
  await fakeAuth(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Log in');
  await expect(page.getByRole('alert')).toHaveText('Confirm your email first: open the link we sent you');
});

test('without email confirmation, registering signs in at once', async ({ page }) => {
  await fakeAuth(page, { confirmEmail: false });
  await boot(page);
  await page.goto('/');
  await page.locator('label.seg-opt', { hasText: 'Register' }).click();
  await page.getByRole('textbox', { name: 'Your name' }).fill('Mai Tran');
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Create account');
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
});

test('forgotten password: code from the email, then a new password', async ({ page }) => {
  const calls = await fakeAuth(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Forgot password?' }).click();
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await submit(page, 'Send reset email');
  await expect(page.getByRole('status').filter({ hasText: 'Sent to' })).toContainText(EMAIL);

  await page.getByRole('textbox', { name: 'Code from the email' }).fill('000000');
  await submit(page, 'Continue');
  await expect(page.getByRole('alert')).toHaveText('This code is wrong or has expired');
  await page.getByRole('textbox', { name: 'Code from the email' }).fill('123456');
  await submit(page, 'Continue');

  await expect(page.getByText('Choose a new password')).toBeVisible();
  await page.getByLabel('New password').fill('newsecret1');
  await submit(page, 'Save password');
  await expect(page.getByText('Password changed')).toBeVisible();
  expect(calls).toEqual(expect.arrayContaining(['POST /recover', 'POST /verify', 'PUT /user']));
  await openProfile(page);
  await expect(page.getByText(EMAIL)).toBeVisible();
});

test('the first sign-in on a device takes over the guest library', async ({ page }) => {
  await fakeAuth(page, { confirmEmail: false });
  await boot(page);
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    const calls: unknown[] = [];
    w.__calls = calls;
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      unregisterCallback: () => undefined,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push([cmd, args?.user]);
        if (cmd === 'take_opened_files' || cmd === 'library_list') return [];
        return null;
      }
    };
  });
  await page.goto('/');
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Log in');
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
  const calls = await page.evaluate(() => (window as unknown as { __calls: [string, string][] }).__calls);
  const claim = calls.findIndex(([cmd, user]) => cmd === 'library_claim_guest' && user === EMAIL);
  const firstLoad = calls.findIndex(([cmd, user]) => cmd === 'reading_load' && user === EMAIL);
  expect(claim).toBeGreaterThanOrEqual(0);
  // The account's file is only read after the guest's became it.
  expect(firstLoad).toBeGreaterThan(claim);
});
