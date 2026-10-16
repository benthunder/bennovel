import { invoke } from '@tauri-apps/api/core';
import type { TranslateStyle, UiLang } from '../data/types';
import type { MessageKey } from '../i18n/en';
import { load } from './storage';

/*
 * AI translation of books on this device with the user's own LLM: any chat API in the
 * OpenAI format. The native app does the work (src-tauri/src/library/llm.rs,
 * translate.rs, queue.rs); the settings and key stay in the library file on this
 * device and are left out of every sync.
 */

/** Target languages offered for books on the device. */
export const AI_LANGS = [
  { code: 'en', name: 'English', native: 'English' },
  { code: 'vi', name: 'Vietnamese', native: 'Tiếng Việt' }
] as const;
export type AiLang = (typeof AI_LANGS)[number]['code'];

/** Ready-made providers; `custom` takes any base URL (a local Ollama, LM Studio…). */
export const LLM_PROVIDERS = [
  { id: 'openai', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  { id: 'gemini', name: 'Gemini', baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai', model: 'gemini-2.5-flash' },
  { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  { id: 'openrouter', name: 'OpenRouter', baseUrl: 'https://openrouter.ai/api/v1', model: 'deepseek/deepseek-chat' },
  { id: 'anthropic', name: 'Claude', baseUrl: 'https://api.anthropic.com/v1', model: 'claude-haiku-4-5' },
  { id: 'custom', name: 'Custom', baseUrl: '', model: '' }
] as const;
export type LlmProvider = (typeof LLM_PROVIDERS)[number]['id'];

/** The saved LLM, as shown: no key. */
export interface LlmSettings { provider: string; baseUrl: string; model: string; embedModel: string; hasKey: boolean }
/** LLM settings to save; an empty key keeps the saved one. */
/** `embedModel`: the API's embedding model for the book wiki's search by meaning; empty for none. */
export interface LlmConfig { provider: string; baseUrl: string; model: string; embedModel: string; apiKey: string }

export interface AiChapter { title: string; paragraphs: string[]; mode: 'translate' | 'rewrite'; model: string | null }
export interface AiVersion { lang: AiLang; style: string; chapters: number }
export interface AiProgress { novelId: number; lang: AiLang; style: string; total: number; done: number; failed: number; queued: number; running: number }

/** A request as the app keeps it; the native side takes the style in lower case. */
export interface AiChoice { lang: AiLang; style: TranslateStyle; dict: boolean }
const styleArg = (s: TranslateStyle) => s.toLowerCase();
/** `natural` back to `Natural`. */
export const styleOf = (s: string) => (s.charAt(0).toUpperCase() + s.slice(1)) as TranslateStyle;

/**
 * What a library book is read in, remembered per book: `view` is the reading language,
 * `raw` (the text as imported) or one of `AI_LANGS`; the rest is the last AI request.
 */
export type AiView = AiChoice & { view?: string };
export const aiViewKey = (user: string, novelId: number) => `aiView.${user}:${novelId}`;
/** The reading language of a remembered choice; `raw` when nothing (or something unknown) was picked. */
export function readingLangOf(v: AiView | null): string {
  const l = v?.view ?? v?.lang;
  return l && AI_LANGS.some(x => x.code === l) ? l : 'raw';
}
export const getReadingLang = (user: string, novelId: number) => readingLangOf(load<AiView | null>(aiViewKey(user, novelId), null));

export const getLlmSettings = (user: string) => invoke<LlmSettings | null>('llm_config_get', { user });

/** Saves the LLM once it answered a test request; null forgets it. */
export async function setLlmConfig(user: string, config: LlmConfig | null) {
  try {
    await invoke('llm_config_set', { user, config });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** Translates (or rewrites, in the book's own language) one chapter now. */
export async function aiTranslateChapter(user: string, novelId: number, index: number, c: AiChoice, again = false) {
  try {
    return await invoke<AiChapter>('ai_translate_chapter', { user, novelId, index, lang: c.lang, style: styleArg(c.style), glossary: c.dict, again });
  } catch (e) {
    throw new AiFailure(e);
  }
}

/** How far a chapter translated now is: parts back from the LLM of all its parts. */
export interface AiChapterProgress { novelId: number; index: number; done: number; total: number }

/** Listens to `aiTranslateChapter`'s progress; returns the way to stop listening. */
export async function onAiChapterProgress(cb: (p: AiChapterProgress) => void): Promise<() => void> {
  try {
    const { listen } = await import('@tauri-apps/api/event');
    return await listen<AiChapterProgress>('ai-chapter-progress', e => cb(e.payload));
  } catch (e) {
    console.warn(e);
    return () => {};
  }
}

export const getAiChapter = (user: string, novelId: number, index: number, lang: AiLang, style: TranslateStyle) =>
  invoke<AiChapter | null>('ai_chapter', { user, novelId, index, lang, style: styleArg(style) });

/** One AI version of a chapter: `index` in reading order. A chapter can have several, newest first. */
export interface AiMark { index: number; lang: AiLang; style: string; model: string | null }
export const getAiMarks = (user: string, novelId: number) => invoke<AiMark[]>('ai_marks', { user, novelId });

export const getAiVersions = (user: string, novelId: number) => invoke<AiVersion[]>('ai_versions', { user, novelId });

/** Queues chapters `fromIndex`… for the background translation. */
export async function aiTranslateBook(user: string, novelId: number, fromIndex: number, c: AiChoice, uiLang: UiLang) {
  void askNotifyPermission();
  try {
    return await invoke<{ queued: number; mode: 'translate' | 'rewrite' }>('ai_translate_book', {
      user, novelId, fromIndex, lang: c.lang, style: styleArg(c.style), glossary: c.dict, uiLang
    });
  } catch (e) {
    throw new AiFailure(e);
  }
}

export const getAiProgress = (user: string, novelId: number) => invoke<AiProgress[]>('ai_progress', { user, novelId });
export const cancelAiTranslation = (user: string, novelId: number) => invoke<number>('ai_cancel', { user, novelId });
export const resumeAiQueue = (user: string, uiLang: UiLang) => invoke<void>('ai_resume', { user, uiLang });

/** Lets the "translation done" notification show (Android 13+ and macOS ask once). */
async function askNotifyPermission() {
  try {
    const n = await import('@tauri-apps/plugin-notification');
    if (!(await n.isPermissionGranted())) await n.requestPermission();
  } catch (e) {
    console.warn(e);
  }
}

/** An error from the native side, with the message to show. */
export class AiFailure extends Error {
  readonly key: MessageKey;
  constructor(readonly cause: unknown) {
    const text = String((cause as { message?: string } | null)?.message ?? cause);
    super(text);
    this.key = aiErrorKey(text);
  }
}

export function aiErrorKey(text: string): MessageKey {
  if (text.startsWith('llm-not-set')) return 'ai.err.notSet';
  if (text.startsWith('llm-auth')) return 'ai.err.auth';
  if (text.startsWith('llm-timeout')) return 'ai.err.timeout';
  if (text.startsWith('llm-network')) return 'ai.err.network';
  if (/^llm-http 404/.test(text)) return 'ai.err.model';
  if (/^llm-http 429/.test(text)) return 'ai.err.limit';
  if (text.startsWith('llm-http')) return 'ai.err.http';
  if (text.startsWith('llm-empty')) return 'ai.err.empty';
  return 'ai.err.generic';
}

/** The API's own words after the code ("llm-http 400: model not found" → "model not found"). */
export const aiErrorDetail = (text: string) => text.replace(/^llm-[\w-]+(?: \d+)?:?\s*/, '').slice(0, 200);
