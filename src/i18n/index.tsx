import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { load, save } from '../lib/storage';
import type { UiLang } from '../data/types';
import { en, type MessageKey } from './en';
import { vi } from './vi';

const MESSAGES: Record<UiLang, Record<MessageKey, string>> = { en, vi };

export type TFn = (key: MessageKey, vars?: Record<string, string | number>) => string;

interface I18nCtx {
  uiLang: UiLang;
  setUiLang: (l: UiLang) => void;
  t: TFn;
}

const Ctx = createContext<I18nCtx | null>(null);

const initialLang = (): UiLang => {
  const saved = load<UiLang | null>('uiLang', null);
  if (saved === 'en' || saved === 'vi') return saved;
  return navigator.language.toLowerCase().startsWith('vi') ? 'vi' : 'en';
};

export function I18nProvider({ children }: { children: ReactNode }) {
  const [uiLang, setLang] = useState<UiLang>(initialLang);

  useEffect(() => { document.documentElement.lang = uiLang; }, [uiLang]);

  const setUiLang = useCallback((l: UiLang) => {
    save('uiLang', l);
    setLang(l);
  }, []);

  const t = useCallback<TFn>((key, vars) => {
    const msg = MESSAGES[uiLang][key] ?? en[key] ?? key;
    return vars ? msg.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? `{${k}}`)) : msg;
  }, [uiLang]);

  const value = useMemo(() => ({ uiLang, setUiLang, t }), [uiLang, setUiLang, t]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useI18n() {
  const c = useContext(Ctx);
  if (!c) throw new Error('useI18n must be used inside I18nProvider');
  return c;
}

export const useT = () => useI18n().t;

export function relativeTime(t: TFn, at: number, now = Date.now()) {
  const min = Math.floor((now - at) / 60000);
  if (min < 1) return t('time.justNow');
  if (min < 60) return t('time.minutes', { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t('time.hours', { n: h });
  const d = Math.floor(h / 24);
  return d === 1 ? t('time.yesterday') : t('time.days', { n: d });
}
