import type { User } from '../data/types';
import type { MessageKey } from '../i18n/en';
import { canOpenLocalBooks } from './localBook';
import { load, save } from './storage';

/*
 * Accounts on the BenNovel backend (Cloudflare Workers): email + password, and Google.
 * The access token lasts an hour; the refresh token (60 days) gets a new one and is
 * replaced each time, so a reader stays signed in across restarts. Signing in does not
 * change which library is used: books stay on the device.
 */

/** Production builds talk to production; `npm run dev` to staging. VITE_API_URL overrides both. */
export const API_URL: string =
  (import.meta.env.VITE_API_URL as string | undefined) ||
  (import.meta.env.PROD ? 'https://api.benthunder.io.vn' : 'https://api-staging.benthunder.io.vn');

/** The plan the account has now (free, pro or cloud). */
export interface Plan {
  code: string;
  name: string;
  /** Epoch seconds; null when it never expires. */
  expiresAt: number | null;
  features: string[];
}

interface Session {
  user: User;
  accessToken: string;
  /** Epoch ms. */
  accessExp: number;
  refreshToken: string;
}

interface ApiUser { id: string; email: string; name: string; hasPassword: boolean; google: boolean }
interface ApiAuth { user: ApiUser; tokens: { accessToken: string; expiresIn: number; refreshToken: string } }

const SESSION_KEY = 'session';

/** A failed auth call, carrying the message to show. */
export class AuthFailure extends Error {
  constructor(readonly key: MessageKey, cause?: unknown) {
    super(key, { cause });
  }
}

const ERRORS: Record<string, MessageKey> = {
  invalid_email: 'auth.err.email',
  weak_password: 'auth.err.pw',
  email_exists: 'auth.err.exists',
  invalid_credentials: 'auth.err.credentials',
  invalid_code: 'auth.err.code',
  rate_limited: 'auth.err.rateLimit',
  not_configured: 'auth.err.notConfigured',
  google_failed: 'auth.err.google'
};

async function call<T>(path: string, init: RequestInit = {}): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${API_URL}${path}`, { ...init, headers: { 'content-type': 'application/json', ...init.headers } });
  } catch (e) {
    throw new AuthFailure('auth.err.network', e);
  }
  if (res.status === 204) return undefined as T;
  const body = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new AuthFailure(ERRORS[body.error ?? ''] ?? 'auth.err.generic', body.error);
  return body;
}

const post = <T,>(path: string, body: unknown) => call<T>(path, { method: 'POST', body: JSON.stringify(body) });

const toUser = (u: ApiUser, provider: string): User => ({ id: u.id, name: u.name, email: u.email, provider });

let session: Session | null = load<Session | null>(SESSION_KEY, null);
const listeners = new Set<(u: User | null) => void>();

function setSession(s: Session | null) {
  session = s;
  save(SESSION_KEY, s);
  listeners.forEach(f => f(s?.user ?? null));
}

/** Called with the user on sign-in and null on sign-out (also when the session ran out). */
export function onAccountChange(f: (u: User | null) => void) {
  listeners.add(f);
  return () => { listeners.delete(f); };
}

export const currentUser = () => session?.user ?? null;

function keep(a: ApiAuth, provider: string): User {
  const user = toUser(a.user, provider);
  setSession({ user, accessToken: a.tokens.accessToken, accessExp: Date.now() + a.tokens.expiresIn * 1000, refreshToken: a.tokens.refreshToken });
  return user;
}

export async function signIn(email: string, password: string): Promise<User> {
  return keep(await post<ApiAuth>('/v1/auth/login', { email, password }), 'email');
}

export async function signUp(name: string, email: string, password: string): Promise<User> {
  return keep(await post<ApiAuth>('/v1/auth/signup', { name, email, password }), 'email');
}

/** Signs out here even when the server can't be reached. */
export async function signOut() {
  const refreshToken = session?.refreshToken;
  setSession(null);
  if (refreshToken) await post('/v1/auth/logout', { refreshToken }).catch(console.error);
}

/** Emails a 6-digit code for setting a new password. */
export async function sendPasswordReset(email: string) {
  await post('/v1/auth/forgot-password', { email });
}

/** Sets the new password with the emailed code and signs in (other devices are signed out). */
export async function resetPassword(email: string, code: string, password: string): Promise<User> {
  return keep(await post<ApiAuth>('/v1/auth/reset-password', { email, code, password }), 'email');
}

let refreshing: Promise<string | null> | null = null;

/**
 * A valid access token, refreshed when it is about to run out. Null when signed out.
 * Without a connection the session is kept, so the reader stays signed in offline.
 */
export async function accessToken(): Promise<string | null> {
  if (!session) return null;
  if (session.accessExp - 60_000 > Date.now()) return session.accessToken;
  refreshing ??= (async () => {
    const s = session!;
    try {
      const a = await post<ApiAuth>('/v1/auth/refresh', { refreshToken: s.refreshToken });
      keep(a, s.user.provider);
      return session!.accessToken;
    } catch (e) {
      if (e instanceof AuthFailure && e.key === 'auth.err.network') throw e;
      setSession(null); // the refresh token was revoked or expired
      return null;
    } finally {
      refreshing = null;
    }
  })();
  return refreshing;
}

/** The account's plan, or null when signed out. */
export async function getPlan(): Promise<Plan | null> {
  const token = await accessToken();
  if (!token) return null;
  const me = await call<{ user: ApiUser; subscription: Plan }>('/v1/me', { headers: { authorization: `Bearer ${token}` } });
  if (session) setSession({ ...session, user: { ...toUser(me.user, session.user.provider) } });
  return me.subscription;
}

/* ---------- Google ---------- */

/** Where the backend sends the native app back to after Google. */
export const NATIVE_REDIRECT = 'bennovel://auth/callback';
/** The web build comes back to this page. */
const WEB_CALLBACK = '/auth/callback';
const GOOGLE = 'Google';

/**
 * Starts signing in with Google. The native app opens the system browser (Google
 * refuses embedded webviews) and gets a one-time code back through a `bennovel://`
 * link, see `listenForSignInLinks`; the web build leaves for Google and comes back to
 * `/auth/callback`, see `finishWebSignIn`.
 */
export async function signInWithGoogle() {
  const native = canOpenLocalBooks();
  const redirect = native ? NATIVE_REDIRECT : `${window.location.origin}${WEB_CALLBACK}`;
  const url = `${API_URL}/v1/auth/google/start?redirect=${encodeURIComponent(redirect)}`;
  if (!native) {
    window.location.href = url;
    return;
  }
  const { openUrl } = await import('@tauri-apps/plugin-opener');
  await openUrl(url);
}

const DONE_KEY = 'oauthCode';

/** Trades the code from Google's round trip for a session. Null when that link was used already. */
async function finishGoogle(params: URLSearchParams): Promise<User | null> {
  const code = params.get('code');
  if (!code) throw new AuthFailure(params.get('error') === 'google_failed' ? 'auth.err.google' : 'auth.err.generic');
  // A launch link is reported again when the app restarts; each code works once.
  if (load<string | null>(DONE_KEY, null) === code) return null;
  save(DONE_KEY, code);
  return keep(await post<ApiAuth>('/v1/auth/exchange', { code }), GOOGLE);
}

/** Web build: finishes a Google sign-in when the page is the callback. */
export async function finishWebSignIn(): Promise<User | null> {
  if (canOpenLocalBooks() || window.location.pathname !== WEB_CALLBACK) return null;
  const params = new URLSearchParams(window.location.search);
  window.history.replaceState(null, '', '/');
  return finishGoogle(params);
}

/**
 * Native app: finishes a Google sign-in when its `bennovel://auth/callback` link opens
 * the app, at launch or while running. Returns a function that stops listening.
 */
export async function listenForSignInLinks(
  onSignedIn: (u: User) => void,
  onFailed: (key: MessageKey) => void
): Promise<() => void> {
  const { getCurrent, onOpenUrl } = await import('@tauri-apps/plugin-deep-link');
  const handle = async (urls: string[] | null) => {
    const raw = urls?.find(u => u.startsWith(NATIVE_REDIRECT));
    if (!raw) return;
    const u = await finishGoogle(new URL(raw).searchParams);
    if (u) onSignedIn(u);
  };
  const safe = (urls: string[] | null) =>
    handle(urls).catch(e => { console.error(e); onFailed(e instanceof AuthFailure ? e.key : 'auth.err.generic'); });
  const stop = await onOpenUrl(safe);
  await safe(await getCurrent());
  return stop;
}
