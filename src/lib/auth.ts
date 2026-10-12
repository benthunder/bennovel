import { isAuthError, type User as AuthUser } from '@supabase/supabase-js';
import { invoke } from '@tauri-apps/api/core';
import type { User } from '../data/types';
import type { MessageKey } from '../i18n/en';
import { canOpenLocalBooks } from './localBook';
import { supabase } from './supabase';

/*
 * Accounts through Supabase Auth: email + password, and Google. An email used both ways
 * is one account (Supabase links them when the email is confirmed). supabase-js keeps the session in
 * localStorage and refreshes it, so a reader stays signed in across restarts.
 */

// A reset link opened in the web app signs in while supabase-js starts, before the
// app listens, so the event is caught here.
let recovering = false;
supabase.auth.onAuthStateChange(event => {
  if (event === 'PASSWORD_RECOVERY') recovering = true;
});
/** True once after signing in from a password reset email. */
export const takeRecovery = () => {
  const r = recovering;
  recovering = false;
  return r;
};

/** `provider`: how the reader signed in this time (an account can have several). */
export const toUser = (u: AuthUser, provider = 'email'): User => {
  const email = u.email ?? '';
  const meta = u.user_metadata as { full_name?: string; name?: string } | undefined;
  return { id: u.id, name: meta?.full_name || meta?.name || email.split('@')[0], email, provider };
};

/** A failed auth call, carrying the message to show. */
export class AuthFailure extends Error {
  constructor(readonly key: MessageKey, cause?: unknown) {
    super(key, { cause });
  }
}

function fail(error: unknown): never {
  console.error(error);
  if (!isAuthError(error)) throw new AuthFailure('auth.err.generic', error);
  if (error.name === 'AuthRetryableFetchError' || error.status === 0) throw new AuthFailure('auth.err.network', error);
  switch (error.code) {
    case 'invalid_credentials': throw new AuthFailure('auth.err.credentials', error);
    case 'email_not_confirmed': throw new AuthFailure('auth.err.notConfirmed', error);
    case 'user_already_exists':
    case 'email_exists': throw new AuthFailure('auth.err.exists', error);
    case 'weak_password': throw new AuthFailure('auth.err.weak', error);
    case 'same_password': throw new AuthFailure('auth.err.samePassword', error);
    case 'otp_expired': throw new AuthFailure('auth.err.code', error);
    case 'over_email_send_rate_limit':
    case 'over_request_rate_limit': throw new AuthFailure('auth.err.rateLimit', error);
  }
  throw new AuthFailure('auth.err.generic', error);
}

export async function signIn(email: string, password: string): Promise<User> {
  const { data, error } = await supabase.auth.signInWithPassword({ email, password });
  if (error) fail(error);
  return toUser(data.user);
}

/** The new account, or null when it must be confirmed from the email first. */
export async function signUp(name: string, email: string, password: string): Promise<User | null> {
  // The web build gets the confirmation link back; the native app uses the project's site URL.
  const emailRedirectTo = canOpenLocalBooks() ? undefined : window.location.origin;
  const { data, error } = await supabase.auth.signUp({ email, password, options: { data: { full_name: name }, emailRedirectTo } });
  if (error) fail(error);
  // An email that is already registered comes back as a user with no identities.
  if (data.user && data.user.identities?.length === 0) throw new AuthFailure('auth.err.exists');
  return data.session && data.user ? toUser(data.user) : null;
}

/** Signs out here even when the server can't be reached. */
export async function signOut() {
  const { error } = await supabase.auth.signOut();
  if (error) console.error(error);
}

/**
 * Emails a password reset. The email's link opens the web app (which then asks for a
 * new password); the app also takes the code from the email, see `verifyResetCode`.
 */
export async function sendPasswordReset(email: string) {
  const redirectTo = canOpenLocalBooks() ? undefined : window.location.origin;
  const { error } = await supabase.auth.resetPasswordForEmail(email, { redirectTo });
  if (error) fail(error);
}

/** Signs in with the code from the reset email, so a new password can be set. */
export async function verifyResetCode(email: string, token: string) {
  const { error } = await supabase.auth.verifyOtp({ email, token, type: 'recovery' });
  if (error) fail(error);
}

export async function setNewPassword(password: string): Promise<User> {
  const { data, error } = await supabase.auth.updateUser({ password });
  if (error) fail(error);
  return toUser(data.user);
}

/**
 * The first time an account signs in on this device, the guest's library file
 * (imported books, favourites, history) becomes the account's. Native app only.
 */
export async function claimGuestLibrary(user: User) {
  if (!canOpenLocalBooks()) return;
  try {
    await invoke<boolean>('library_claim_guest', { user: user.email });
  } catch (e) {
    console.error(e);
  }
}

/* ---------- Google ---------- */

/** Where Supabase sends the native app back to after Google (see guides/oauth-facebook-google.md). */
export const NATIVE_REDIRECT = 'bennovel://auth/callback';
const GOOGLE = 'Google';

/**
 * Starts signing in with Google. The native app opens the system browser (Google
 * refuses embedded webviews) and gets the result back through a `bennovel://` link,
 * see `listenForSignInLinks`; the web build leaves for Google and comes back signed in.
 */
export async function signInWithGoogle() {
  const native = canOpenLocalBooks();
  const { data, error } = await supabase.auth.signInWithOAuth({
    provider: 'google',
    options: { redirectTo: native ? NATIVE_REDIRECT : window.location.origin, skipBrowserRedirect: native }
  });
  if (error) fail(error);
  // Remembered so the account shows "signed in with Google" when the reader comes back.
  sessionStorage.setItem(PENDING_KEY, GOOGLE);
  if (native) {
    const { openUrl } = await import('@tauri-apps/plugin-opener');
    await openUrl(data.url);
  }
}

const PENDING_KEY = 'bennovel.oauthProvider';
const DONE_KEY = 'bennovel.oauthCode';

/** The provider of a sign-in started on this page, once, or null. */
export function takeOAuthProvider() {
  const p = sessionStorage.getItem(PENDING_KEY);
  sessionStorage.removeItem(PENDING_KEY);
  return p;
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
    const url = new URL(raw);
    const code = url.searchParams.get('code');
    if (!code) {
      console.error(url.searchParams.get('error_description') ?? raw);
      if (url.searchParams.get('error') !== 'access_denied') onFailed('auth.err.generic');
      return;
    }
    // A launch link is reported again when the app restarts; each code works once.
    if (localStorage.getItem(DONE_KEY) === code) return;
    localStorage.setItem(DONE_KEY, code);
    const { data, error } = await supabase.auth.exchangeCodeForSession(code);
    if (error) {
      console.error(error);
      onFailed('auth.err.generic');
      return;
    }
    onSignedIn(toUser(data.user, takeOAuthProvider() ?? GOOGLE));
  };
  const safe = (urls: string[] | null) => handle(urls).catch(e => { console.error(e); onFailed('auth.err.generic'); });
  const stop = await onOpenUrl(safe);
  await safe(await getCurrent());
  return stop;
}
