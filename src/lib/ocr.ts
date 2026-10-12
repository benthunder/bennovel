import { invoke, isTauri } from '@tauri-apps/api/core';
import { useSyncExternalStore } from 'react';

/*
 * Scanned PDF/DjVu pages in the library are read once with Tesseract (tesseract.js,
 * bundled in public/tesseract by scripts/copy-tesseract.mjs) and their text stored in
 * place of the empty chapter, so books only ever show text. The device picks the next
 * chapter with scanned pages (src-tauri/src/library/ocr.rs); runs resume where they
 * stopped, after an import and each time the app starts.
 */

interface OcrTask {
  novelId: number;
  title: string;
  index: number;
  chapters: number;
  pages: { page: number; text: string; ocr: boolean }[];
}

/** What the OCR run is doing, for the library screen. */
export type OcrStatus = { title: string; done: number; total: number } | null;

let status: OcrStatus = null;
const listeners = new Set<() => void>();
const setStatus = (s: OcrStatus) => {
  status = s;
  listeners.forEach(l => l());
};

export function useOcrStatus() {
  return useSyncExternalStore(cb => { listeners.add(cb); return () => listeners.delete(cb); }, () => status);
}

let running: Promise<void> | null = null;
/** Library to check once the current pass ends (set by calls made during a run). */
let pending: string | null = null;

/** Reads the scanned pages of `user`'s library; one run at a time (a call during a run makes it check again). */
export function runOcr(user: string) {
  if (!isTauri()) return;
  pending = user;
  if (running) return;
  running = (async () => {
    try {
      while (pending !== null) {
        const next: string = pending;
        pending = null;
        await drain(next);
      }
    } catch (e) {
      console.error('OCR stopped', e);
    } finally {
      running = null;
      setStatus(null);
    }
  })();
}

async function drain(user: string) {
  let task = await invoke<OcrTask | null>('library_ocr_next', { user });
  if (!task) return;
  const scheduler = await createScheduler();
  try {
    while (task) {
      const t = task;
      setStatus({ title: t.title, done: t.index, total: t.chapters });
      // Pages are drawn one after another (quick) and read in parallel by the workers.
      const jobs: Promise<string>[] = [];
      for (const p of t.pages) {
        if (!p.ocr) {
          jobs.push(Promise.resolve(p.text));
          continue;
        }
        // A page that can't be drawn or read keeps its own text, so one bad page
        // doesn't stop the book.
        const skip = (e: unknown) => { console.error(`OCR page ${p.page}`, e); return p.text; };
        const bytes = await invoke<ArrayBuffer>('library_ocr_page', { user, novelId: t.novelId, page: p.page }).catch(skip);
        jobs.push(typeof bytes === 'string'
          ? Promise.resolve(bytes)
          : scheduler.addJob('recognize', new Blob([bytes], { type: 'image/jpeg' })).then(r => r.data.text, skip));
      }
      const texts = await Promise.all(jobs);
      await invoke('library_ocr_save', { user, novelId: t.novelId, index: t.index, chapters: t.chapters, pages: texts });
      task = await invoke<OcrTask | null>('library_ocr_next', { user });
    }
  } finally {
    await scheduler.terminate();
  }
}

/** Tesseract workers for Vietnamese, from the files bundled with the app. */
async function createScheduler() {
  const [{ createScheduler, createWorker, OEM }, { simd }] = await Promise.all([import('tesseract.js'), import('wasm-feature-detect')]);
  const base = new URL('/tesseract/', location.href).href;
  const core = (await simd()) ? 'tesseract-core-simd-lstm.wasm.js' : 'tesseract-core-lstm.wasm.js';
  const scheduler = createScheduler();
  // Leave a core or two for the app; phones get fewer workers (memory).
  const cores = navigator.hardwareConcurrency || 2;
  const count = Math.max(1, Math.min(/android/i.test(navigator.userAgent) ? 2 : 4, cores - 1));
  const workers = await Promise.all(Array.from({ length: count }, () =>
    createWorker('vie', OEM.LSTM_ONLY, {
      workerPath: base + 'worker.min.js',
      corePath: base + core,
      langPath: base.replace(/\/$/, ''),
      workerBlobURL: false,
      cacheMethod: 'none',
    })));
  workers.forEach(w => scheduler.addWorker(w));
  return scheduler;
}
