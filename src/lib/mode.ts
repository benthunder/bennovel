import { load, save } from './storage';

/**
 * How the app runs. `offline`: no account and nothing goes to the BenNovel API, the
 * library, translations and settings stay on this device. `online`: an account is
 * required and every online feature goes through the API on the Worker.
 */
export type AppMode = 'offline' | 'online';

const KEY = 'appMode';

/**
 * The saved mode; null on the very first launch, when the login screen asks.
 * Installs from before the mode existed keep working as they did: signed in → online,
 * past the login screen without an account → offline.
 */
function initialMode(): AppMode | null {
  const saved = load<AppMode | null>(KEY, null);
  if (saved === 'offline' || saved === 'online') return saved;
  if (load<unknown>('session', null)) return 'online';
  return load('onboarded', false) ? 'offline' : null;
}

let mode: AppMode | null = initialMode();

export const appMode = () => mode;
export const isOnline = () => mode === 'online';

export function setAppMode(m: AppMode) {
  mode = m;
  save(KEY, m);
  save('onboarded', true);
}
