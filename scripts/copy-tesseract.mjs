// Copies Tesseract (tesseract.js worker, WASM core, Vietnamese model) into
// public/tesseract so OCR works offline. Runs before `dev` and `build`.
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'public/tesseract');
const nm = join(root, 'node_modules');
mkdirSync(out, { recursive: true });
for (const [from, to] of [
  ['tesseract.js/dist/worker.min.js', 'worker.min.js'],
  // LSTM-only cores: with SIMD, and without for older WebViews.
  ['tesseract.js-core/tesseract-core-simd-lstm.wasm.js', 'tesseract-core-simd-lstm.wasm.js'],
  ['tesseract.js-core/tesseract-core-lstm.wasm.js', 'tesseract-core-lstm.wasm.js'],
  ['@tesseract.js-data/vie/4.0.0_best_int/vie.traineddata.gz', 'vie.traineddata.gz'],
]) copyFileSync(join(nm, from), join(out, to));
