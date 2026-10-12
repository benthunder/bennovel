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

const GOOGLE_CODE = 'google-code-1';

/** The same account after signing in with Google: one user, two ways in. */
function linkedSession() {
  const s = session();
  return {
    ...s,
    user: {
      ...s.user,
      app_metadata: { provider: 'email', providers: ['email', 'google'] },
      user_metadata: { full_name: 'Mai Tran', avatar_url: 'https://example.com/a.png' },
      identities: [{ id: 'i1', provider: 'email' }, { id: 'i2', provider: 'google' }]
    }
  };
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
      case '/authorize':
        // Google says yes and Supabase sends the reader back with a one-time code.
        return route.fulfill({ status: 302, headers: { location: `${url.searchParams.get('redirect_to')}?code=${GOOGLE_CODE}` } });
      case '/token':
        if (url.searchParams.get('grant_type') === 'pkce') {
          if (body?.auth_code !== GOOGLE_CODE || !body?.code_verifier) return json(route, 400, { code: 400, error_code: 'bad_code_verifier', msg: 'bad code' });
          return json(route, 200, linkedSession());
        }
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

test('web: continue with Google comes back signed in', async ({ page }) => {
  const calls = await fakeAuth(page);
  await boot(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Continue with Google' }).click();
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
  expect(calls).toEqual(expect.arrayContaining(['GET /authorize', 'POST /token']));
  await openProfile(page);
  await expect(page.getByText('Signed in with Google')).toBeVisible();
  await expect(page.getByText(EMAIL)).toBeVisible();
});

/** The native app, with the system browser and bennovel:// links stood in for. */
async function nativeWithLinks(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    const calls: unknown[][] = [];
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners = new Map<string, number>();
    let next = 1;
    w.__calls = calls;
    w.__emit = (event: string, payload: unknown) => callbacks.get(listeners.get(event)!)?.({ event, id: 0, payload });
    w.isTauri = true;
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
    w.__TAURI_INTERNALS__ = {
      transformCallback: (cb: (e: unknown) => void) => { const id = next++; callbacks.set(id, cb); return id; },
      unregisterCallback: () => undefined,
      invoke: async (cmd: string, args: Record<string, unknown>) => {
        calls.push([cmd, args?.user ?? args?.url ?? args?.event]);
        switch (cmd) {
          case 'plugin:event|listen': listeners.set(args.event as string, args.handler as number); return next++;
          case 'take_opened_files': case 'library_list': return [];
          default: return null;
        }
      }
    };
  });
  return {
    calls: () => page.evaluate(() => (window as unknown as { __calls: [string, string][] }).__calls),
    openLink: (url: string) => page.evaluate(u => (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit('deep-link://new-url', [u]), url)
  };
}

test('app: Google opens the browser and the link back signs in; the same email also works with a password', async ({ page }) => {
  // Google confirms the email, so the account's password works as well.
  const auth = await fakeAuth(page, { confirmEmail: false });
  await boot(page);
  const app = await nativeWithLinks(page);
  await page.goto('/');

  await page.getByRole('button', { name: 'Continue with Google' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Finish signing in with Google' })).toBeVisible();
  const opened = (await app.calls()).find(([cmd]) => cmd === 'plugin:opener|open_url')?.[1] ?? '';
  expect(opened).toContain('/auth/v1/authorize?provider=google');
  expect(new URL(opened).searchParams.get('redirect_to')).toBe('bennovel://auth/callback');

  await app.openLink(`bennovel://auth/callback?code=${GOOGLE_CODE}`);
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
  await openProfile(page);
  await expect(page.getByText('Signed in with Google')).toBeVisible();

  // Same email with a password: the same account and the same library on this device.
  await page.getByRole('button', { name: 'Log out' }).click();
  await page.getByRole('button', { name: 'Log in or register' }).click();
  await page.getByRole('textbox', { name: 'Email' }).fill(EMAIL);
  await page.getByLabel('Password').fill(PASSWORD);
  await submit(page, 'Log in');
  await expect(page.getByText('Welcome, Mai')).toBeVisible();
  await openProfile(page);
  await expect(page.getByText('Signed in with email')).toBeVisible();
  await expect(page.getByText(EMAIL)).toBeVisible();

  const loads = (await app.calls()).filter(([cmd, user]) => cmd === 'reading_load' && user === EMAIL);
  expect(loads.length).toBeGreaterThanOrEqual(2);
  expect(auth.filter(c => c === 'POST /token')).toHaveLength(2);
});
