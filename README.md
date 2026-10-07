# BenNovel

A novel-reading app built with **Tauri v2** (desktop + iOS/Android) and **React + Vite + TypeScript**. It implements the Claude Design handoff in `project/BenNovel App.dc.html`. The design brief is in `chats/` and the original handoff notes are in `HANDOFF.md`.

## Running it

```bash
npm install
npm run dev            # frontend only, in the browser at http://localhost:1420
npm run tauri dev      # desktop app window
npm run tauri build    # desktop installer

# mobile (needs Android Studio / Xcode set up for Tauri)
npm run tauri android init && npm run tauri android dev
npm run tauri ios init && npm run tauri ios dev
```

On Linux, Tauri also needs the system WebKit packages: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`.

## What's in it

| Screen | Notes |
| --- | --- |
| Login / Register | Fits on one screen with no scrolling. Google, Facebook and iCloud sit in one row, plus an email login/register form with field checks. You can skip and browse as a guest. Auth is simulated. |
| Home | A "Most popular" slider that you can swipe and that autoplays (it pauses for 6s after you touch it), four collection shelves, and a throttled search (400 ms). |
| Detail | Novel info, favourite button, chapter list, "More in {category}" and "More by {author}". |
| Reading language | The first chapter you open asks which language to read in. The pick is saved on the device (`localStorage`), so later chapters open straight in it. You can change it from Detail, the Reader (EN/VI/ES pill) or Profile. |
| Category / Author / Collection | Throttled search, genre chips (when browsing all), a status filter and sorting (Popular / Newest / Top rated). |
| Reader | No bottom menu. The tools sheet has a **Reading** tab (font size, background, line spacing, paragraph gap, margins, alignment; saved on the device) and an **AI tools** tab (dictionary, novel glossary, AI Translate with a target language, a style and an "Apply dictionary" checkbox). |
| Library / Profile | Favourites grid, reading history with Resume, user card, log in / log out, default reading language and app language. |
| Bottom nav | Home · Category · raised **Continue** (opens your last chapter) · Library · Profile. |

The interface is in English and Vietnamese (`src/i18n/`). It follows the device language at first, and you can switch it in Profile or with the EN/VI button on Login. The font is **Be Vietnam Pro**, bundled through `@fontsource`, so Vietnamese diacritics render correctly offline.

## Structure

```
src/
  data/        mock.ts (sample data), repository.ts (the data seam), types.ts
  store/       AppStore.tsx: navigation stack, auth, favourites, history, saved language, reader prefs
  i18n/        en.ts, vi.ts, the provider and t()
  components/  NovelCover, BottomNav, LangDialog, ReaderTools, shared UI
  screens/     Login, Home, Detail, List, Library, Profile, Reader
  styles/      organic.css (the Organic design system tokens) + app.css
src-tauri/     Tauri v2 shell (Rust)
```

## Database (Supabase)

The schema lives in `supabase/migrations`, sample data in `supabase/seed.sql` (generated from `src/data/mock.ts`).

```bash
npx supabase start       # local Postgres + Auth + API (needs Docker)
npx supabase db reset    # re-run all migrations and the seed
npx supabase db push     # apply migrations to the linked remote project
```

| Table | What it holds | Who can read / write |
| --- | --- | --- |
| `languages`, `categories`, `authors` | Lookups | Everyone reads |
| `novels`, `novel_translations` | Novel info; title and blurb per language | Everyone reads published novels |
| `collections`, `collection_items` | Home shelves (trending, new, completed, picks) | Everyone reads |
| `chapters`, `chapter_translations` | Chapters; text per language (original, human or AI, one AI version per style) | Everyone reads released chapters with a published text |
| `glossary_entries` | Per-novel names and terms per language, fed to the AI translator | Everyone reads |
| `profiles` | Display name, UI language, default reading language, reader settings; created on sign-up | Owner reads and updates |
| `favorites` | The heart button / Library grid | Owner only |
| `reading_history` | One row per novel: last chapter, language, scroll position | Owner only |
| `translation_jobs` | Queue for the AI translation pipeline | Signed-in users read; created via `request_translation()`, moved along by the worker |

Catalog writes, chapter imports and the translation worker use the service role key, which bypasses RLS. The worker claims work with `claim_translation_job()`.

## Offline EPUB / PDF files

In the native app, Library → **Open EPUB / PDF file** opens a book from the device (`src-tauri/src/book`).
The file stays on disk and is read in sections (EPUB: one chapter, PDF: 10 pages). Only the current
section, the 2 after it and the 1 before it are kept in memory; the next ones are read in the background
and sections further away are freed (`KEEP_AHEAD` / `KEEP_BEHIND` in `book/mod.rs`).

PDF text comes from [Pdfium](https://github.com/bblanchon/pdfium-binaries), loaded at runtime from next to
the app, its resources folder (`pdfium/`) or the system. Without it EPUB still works and opening a PDF shows
an error. To run the PDF test: `PDFIUM_DIR=/path/to/pdfium/lib cargo test` in `src-tauri`.

## Simulated for now

- **Data**: everything comes from `src/data/mock.ts` through `src/data/repository.ts`. To connect a backend (for example a NestJS + Postgres API), you only replace that file.
- **Auth**: social and email logins are faked with a 750 ms delay.
- **AI Translate / Dictionary**: faked. Only EN/VI/ES sample text exists, and every novel shows the same sample chapter.
- **Covers**: typographic placeholders until real cover art is available.

## Releases (Linux + Android)

`.github/workflows/release.yml` builds a Linux AppImage/.deb and an Android APK.
Push a version tag (matching `version` in `src-tauri/tauri.conf.json`) to publish a GitHub Release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Or, without git: Actions → **Build & Release** → **Run workflow** on `main`, with `version` set to e.g. `v0.1.0`.

Android signing: set repo secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`
to sign with your own key. Without them the APK is signed with a temporary test key, so a newer build
cannot update an installed one in place (uninstall first).
