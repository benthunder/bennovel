use super::folder::{self, FolderBook, Opener};
use super::{db, DbBook, LibraryBook};
use crate::book::commands::{open_picked, pdfium_dirs, Books, Reopen};
use crate::book::{
    detect, open_source, BookError, BookFormat, BookInfo, BookSource, OpenCtx, SectionMeta,
};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_fs::FilePath;

/// The user's library file: `<app data>/library/<user>.sqlite3`. `user` is the signed-in
/// email, or empty for a guest.
fn db_path<R: Runtime>(app: &AppHandle<R>, user: &str) -> Result<PathBuf, BookError> {
    let name: String = user
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '-' | '_' | '@' => c,
            _ => '_',
        })
        .collect();
    let name = if name.trim_matches(['.', '_']).is_empty() {
        "guest".to_string()
    } else {
        name
    };
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| BookError::Parse(e.to_string()))?;
    Ok(dir.join("library").join(format!("{name}.sqlite3")))
}

/// Serves a library book's cover through the `bookimg:` scheme, at
/// `cover/{novel_id}/{user}` (the user part URL-encoded, empty for a guest).
pub fn serve_cover<R: Runtime>(app: &AppHandle<R>, path: &str) -> tauri::http::Response<Vec<u8>> {
    let path = crate::book::image::percent_decode(path.trim_start_matches('/'));
    let result = (|| {
        let rest = path.strip_prefix("cover/").unwrap_or_default();
        let (id, user) = rest.split_once('/').unwrap_or((rest, ""));
        let id: i64 = id.parse().map_err(|_| BookError::NoImage(path.clone()))?;
        let conn = rusqlite::Connection::open_with_flags(
            db_path(app, user)?,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        super::cover(&conn, id)
    })();
    let builder = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
    match result {
        Ok(img) => builder
            .header("Content-Type", img.mime)
            .header("Cache-Control", "max-age=3600")
            .body(img.bytes),
        Err(e) => builder
            .status(404)
            .header("Content-Type", "text/plain")
            .body(e.to_string().into_bytes()),
    }
    .unwrap_or_default()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, BookError> + Send + 'static,
) -> Result<T, BookError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| BookError::Parse(e.to_string()))?
}

#[tauri::command]
pub async fn library_list<R: Runtime>(
    app: AppHandle<R>,
    user: String,
) -> Result<Vec<LibraryBook>, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || Ok(super::list(&db::open(&path)?)?)).await
}

#[derive(Clone, Serialize)]
struct ImportProgress<'a> {
    name: &'a str,
    done: usize,
    total: usize,
}

/// Imports a picked file into the library. Sections are read and stored one by one;
/// a `library-import` event reports each stored chapter.
#[tauri::command]
pub async fn library_import<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    path: FilePath,
) -> Result<LibraryBook, BookError> {
    let db_file = db_path(&app, &user)?;
    let (file, ctx) = open_picked(&app, path)?;
    blocking(move || {
        let (file, ctx, kept) = keep_original(file, ctx, &db_file)?;
        let result = (|| {
            let (format, mut source) = open_source(file, &ctx)?;
            let mut conn = db::open(&db_file)?;
            let name = ctx.name.clone();
            let id = super::import(&mut conn, format, source.as_mut(), &name, |done, total| {
                let _ = app.emit(
                    "library-import",
                    ImportProgress {
                        name: &name,
                        done,
                        total,
                    },
                );
            })?;
            Ok((conn, id))
        })();
        // The book (and its file handle) is closed by now, so the copy can be renamed.
        match (result, kept) {
            (Ok((conn, id)), kept) => {
                if let Some((tmp, ext)) = kept {
                    std::fs::rename(
                        &tmp,
                        super::originals_dir(&db_file).join(format!("{id}.{ext}")),
                    )?;
                }
                super::get(&conn, id)?.ok_or(BookError::Parse("import failed".into()))
            }
            (Err(e), kept) => {
                if let Some((tmp, _)) = kept {
                    let _ = std::fs::remove_file(tmp);
                }
                Err(e)
            }
        }
    })
    .await
}

/// Imports a folder of chapter files as one book, a file per chapter, ordered by the
/// chapter numbers in their names (`folder.rs`). `paths` is either the picked folder
/// (desktop; its subfolders are read too) or the picked files (Android, which has
/// no folder picker).
#[tauri::command]
pub async fn library_import_folder<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    paths: Vec<FilePath>,
) -> Result<LibraryBook, BookError> {
    let db_file = db_path(&app, &user)?;
    blocking(move || {
        let (title, files) = folder_files(paths)?;
        let named = files.into_iter().map(|f| (f.to_string(), f)).collect();
        import_files(&app, &db_file, title, named)
    })
    .await
}

/// Android: picks a folder in the system picker, walks it and its subfolders (each
/// folder is listed through the document tree, see `plugins/folder-picker`) and
/// imports its book files as one book. Resolves to None when the picker is cancelled.
#[tauri::command]
pub async fn library_import_android_folder<R: Runtime>(
    app: AppHandle<R>,
    user: String,
) -> Result<Option<LibraryBook>, BookError> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_folder_picker::FolderPicker;
        let db_file = db_path(&app, &user)?;
        blocking(move || {
            let picker = app.state::<FolderPicker<R>>();
            let Some(root) = picker.pick().map_err(BookError::Parse)? else {
                return Ok(None);
            };
            let mut files = Vec::new();
            walk_tree(&picker, &root.tree, &root.id, &root.name, &mut files)?;
            if files.is_empty() {
                return Err(BookError::Parse("no book files in this folder".into()));
            }
            let title = if root.name.is_empty() {
                let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
                folder::common_title(&names)
            } else {
                root.name.clone()
            };
            import_files(&app, &db_file, title, files).map(Some)
        })
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, user);
        Err(BookError::NotYet(
            "Folder picking through the document tree",
        ))
    }
}

/// Collects the book files of a document-tree folder and its subfolders, named by
/// their path inside the picked folder ("Truyện/Quyển 1/Chương 1.txt").
#[cfg(target_os = "android")]
fn walk_tree<R: Runtime>(
    picker: &tauri_plugin_folder_picker::FolderPicker<R>,
    tree: &str,
    id: &str,
    path: &str,
    out: &mut Vec<(String, FilePath)>,
) -> Result<(), BookError> {
    for entry in picker.list(tree, id).map_err(BookError::Parse)? {
        if entry.name.starts_with('.') {
            continue;
        }
        let name = format!("{path}/{}", entry.name);
        if entry.dir {
            walk_tree(picker, tree, &entry.id, &name, out)?;
        } else if folder::is_book_file(&entry.name) {
            let uri = entry
                .uri
                .parse::<FilePath>()
                .map_err(|e| BookError::Parse(e.to_string()))?;
            out.push((name, uri));
        }
    }
    Ok(())
}

/// Imports `files` (display name, file) as one book, a file per chapter.
fn import_files<R: Runtime>(
    app: &AppHandle<R>,
    db_file: &std::path::Path,
    title: String,
    files: Vec<(String, FilePath)>,
) -> Result<LibraryBook, BookError> {
    let (names, files): (Vec<String>, Vec<FilePath>) = files.into_iter().unzip();
    let app2 = app.clone();
    let open: Opener = Box::new(move |i| {
        let (file, ctx) = open_picked(&app2, files[i].clone())?;
        open_source(file, &ctx)
    });
    let (format, mut book) = FolderBook::new(title.clone(), &names, open)?;
    let mut conn = db::open(db_file)?;
    let id = super::import(&mut conn, format, &mut book, &title, |done, total| {
        let _ = app.emit(
            "library-import",
            ImportProgress {
                name: &title,
                done,
                total,
            },
        );
    })?;
    super::get(&conn, id)?.ok_or(BookError::Parse("import failed".into()))
}

/// The book files to import and the book title (the folder's name). A folder is
/// expanded to every book file in it and its subfolders.
fn folder_files(paths: Vec<FilePath>) -> Result<(String, Vec<FilePath>), BookError> {
    let mut files = Vec::new();
    let mut title = None;
    for path in paths {
        match &path {
            FilePath::Path(dir) if dir.is_dir() => {
                title = dir.file_name().map(|n| n.to_string_lossy().into_owned());
                let mut found = Vec::new();
                folder::walk(dir, &mut found)?;
                files.extend(found.into_iter().map(FilePath::Path));
            }
            _ => files.push(path),
        }
    }
    if files.is_empty() {
        return Err(BookError::Parse("no book files in this folder".into()));
    }
    let names: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    let title = title
        .or_else(|| folder::common_parent(&names))
        .unwrap_or_else(|| folder::common_title(&names));
    Ok((title, files))
}

/// PDF and DjVu pages can be shown as pictures, which needs the file itself, so a
/// copy is kept next to the library. Other formats are fully stored as text and
/// pictures. Returns the file to import from and, when copied, (copy, extension).
#[allow(clippy::type_complexity)]
fn keep_original(
    mut file: std::fs::File,
    mut ctx: OpenCtx,
    db_file: &std::path::Path,
) -> Result<(std::fs::File, OpenCtx, Option<(PathBuf, &'static str)>), BookError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut head = Vec::new();
    (&mut file).take(4096).read_to_end(&mut head)?;
    file.seek(SeekFrom::Start(0))?;
    let ext = match detect(&ctx.extension(), &head)? {
        BookFormat::Pdf => "pdf",
        BookFormat::Djvu => "djvu",
        _ => return Ok((file, ctx, None)),
    };
    let dir = super::originals_dir(db_file);
    std::fs::create_dir_all(&dir)?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let tmp = dir.join(format!("importing-{millis}.{ext}"));
    std::io::copy(&mut file, &mut std::fs::File::create(&tmp)?)?;
    ctx.path = Some(tmp.clone());
    Ok((std::fs::File::open(&tmp)?, ctx, Some((tmp, ext))))
}

/// Opens the kept original of a library book (for page pictures), if any.
fn open_original<R: Runtime>(
    app: &AppHandle<R>,
    db_file: &std::path::Path,
    novel_id: i64,
) -> Option<Box<dyn BookSource>> {
    let file = super::original_file(db_file, novel_id)?;
    let ctx = OpenCtx {
        name: file.display().to_string(),
        path: Some(file.clone()),
        cache_dir: app.path().app_cache_dir().ok()?.join("books"),
        pdfium_dirs: pdfium_dirs(app),
    };
    // Without Pdfium the book still opens, as text only.
    open_source(std::fs::File::open(&file).ok()?, &ctx)
        .ok()
        .map(|(_, source)| source)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedLibraryBook {
    pub book: BookInfo,
    pub novel_id: i64,
    /// Language guessed from the text (en, vi, zh or ko).
    pub lang: String,
    /// Section to start at and scroll position in it.
    pub index: usize,
    pub progress: f64,
}

/// Opens a library book in the reader, at the place it was left.
#[tauri::command]
pub async fn library_open<R: Runtime>(
    app: AppHandle<R>,
    books: State<'_, Books>,
    user: String,
    novel_id: i64,
) -> Result<OpenedLibraryBook, BookError> {
    let path = db_path(&app, &user)?;
    let reopen: Reopen = {
        let (app, path) = (app.clone(), path.clone());
        Box::new(move || {
            let original = open_original(&app, &path, novel_id);
            let (format, source) = DbBook::open(&path, novel_id, original)?;
            Ok((format, Box::new(source) as Box<dyn BookSource>))
        })
    };
    let app2 = app.clone();
    let (format, source, lang, last) = blocking(move || {
        let original = open_original(&app2, &path, novel_id);
        let (format, source) = DbBook::open(&path, novel_id, original)?;
        let saved = super::get(&db::open(&path)?, novel_id)?;
        let lang = saved.as_ref().map_or_else(String::new, |b| b.lang.clone());
        let last = saved.and_then(|b| b.last_read);
        Ok((format, source, lang, last))
    })
    .await?;
    let book = books.add(format, Box::new(source), reopen);
    let (index, progress) = last
        .filter(|l| l.index < book.sections.len())
        .map_or((0, 0.0), |l| (l.index, l.progress));
    Ok(OpenedLibraryBook {
        book,
        novel_id,
        lang,
        index,
        progress,
    })
}

/// A library book with its chapter list, for its details page. Nothing stays open.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDetail {
    pub book: LibraryBook,
    pub sections: Vec<SectionMeta>,
}

#[tauri::command]
pub async fn library_detail<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<LibraryDetail, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        // The original gives PDF chapters their page ranges, as in the reader.
        let original = open_original(&app, &path, novel_id);
        let (_, source) = DbBook::open(&path, novel_id, original)?;
        let book = super::get(&db::open(&path)?, novel_id)?
            .ok_or_else(|| BookError::Parse("book not found".into()))?;
        Ok(LibraryDetail {
            book,
            sections: source.sections(),
        })
    })
    .await
}

#[tauri::command]
pub async fn library_save_progress<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    index: usize,
    progress: f64,
) -> Result<(), BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        Ok(super::save_progress(
            &db::open(&path)?,
            novel_id,
            index,
            progress,
        )?)
    })
    .await
}

/// Saves a book's title, author and description; returns the book as listed.
#[tauri::command]
pub async fn library_update<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    title: String,
    author: String,
    description: String,
) -> Result<super::LibraryBook, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        super::update_details(
            &mut db::open(&path)?,
            novel_id,
            &title,
            &author,
            &description,
        )?
        .ok_or_else(|| BookError::Parse("book not found".into()))
    })
    .await
}

#[tauri::command]
pub async fn library_delete<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<(), BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        super::delete(&db::open(&path)?, novel_id)?;
        if let Some(file) = super::original_file(&path, novel_id) {
            let _ = std::fs::remove_file(file);
        }
        Ok(())
    })
    .await
}

impl From<super::sync::SyncError> for BookError {
    fn from(e: super::sync::SyncError) -> Self {
        match e {
            super::sync::SyncError::Db(e) => BookError::Db(e),
            super::sync::SyncError::Io(e) => BookError::Io(e),
            // The app looks for this word to ask for an update.
            super::sync::SyncError::Outdated => BookError::Parse("sync-outdated".into()),
        }
    }
}

/// The next chapter with scanned pages to read with OCR, if any (`ocr.rs`).
#[tauri::command]
pub async fn library_ocr_next<R: Runtime>(
    app: AppHandle<R>,
    user: String,
) -> Result<Option<super::ocr::OcrTask>, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        let conn = db::open(&path)?;
        super::ocr::next(&conn, &path, |id| open_original(&app, &path, id))
    })
    .await
}

/// Page `page` of a library book's kept original, drawn for OCR, as JPEG bytes.
#[tauri::command]
pub async fn library_ocr_page<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    page: u32,
) -> Result<tauri::ipc::Response, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || {
        // Opened for each page, so the file is never held open (deleting the book
        // removes it, which Windows refuses for an open file).
        let mut source = open_original(&app, &path, novel_id)
            .ok_or_else(|| BookError::NoImage(format!("page {page}")))?;
        let img = source.render_page(page, super::ocr::OCR_WIDTH)?;
        Ok(tauri::ipc::Response::new(img.bytes))
    })
    .await
}

/// Stores the text of chapter `index` read with OCR (`pages`: each page's text).
#[tauri::command]
pub async fn library_ocr_save<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    index: usize,
    chapters: usize,
    pages: Vec<String>,
) -> Result<(), BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || super::ocr::save(&mut db::open(&path)?, novel_id, index, chapters, &pages))
        .await
}

/// The reader's replace rules for a book (library id) in one language; null when none
/// were ever saved on this device.
#[tauri::command]
pub async fn replace_rules_get<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    id: i64,
    lang: String,
) -> Result<Option<Vec<super::rules::Rule>>, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || Ok(super::rules::get(&db::open(&path)?, id, &lang)?)).await
}

#[tauri::command]
pub async fn replace_rules_set<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    id: i64,
    lang: String,
    rules: Vec<super::rules::Rule>,
) -> Result<(), BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || Ok(super::rules::set(&db::open(&path)?, id, &lang, &rules)?)).await
}

/* ---------- AI translation with the user's own LLM (llm.rs, translate.rs, queue.rs) ---------- */

use super::llm::{self, AiError};
use super::queue::{self, Queue};
use super::translate;

impl From<BookError> for AiError {
    fn from(e: BookError) -> Self {
        AiError::Other(e.to_string())
    }
}

async fn ai_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, AiError> + Send + 'static,
) -> Result<T, AiError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AiError::Other(e.to_string()))?
}

/// The LLM set for this library, without its key.
#[tauri::command]
pub async fn llm_config_get<R: Runtime>(
    app: AppHandle<R>,
    user: String,
) -> Result<Option<llm::Settings>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(llm::settings(&db::open(&path)?)?)).await
}

/// Saves the LLM after checking it answers (an empty key keeps the saved one); None
/// forgets it. Kept in the library file, never synced.
#[tauri::command]
pub async fn llm_config_set<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    config: Option<llm::Config>,
) -> Result<(), AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        let config = match config {
            None => None,
            Some(c) => {
                let c = llm::with_saved_key(&conn, c)?;
                if c.base_url.is_empty() || c.model.trim().is_empty() {
                    return Err(AiError::Other("base URL and model are needed".into()));
                }
                llm::check(&c)?;
                Some(c)
            }
        };
        Ok(llm::save(&conn, config.as_ref())?)
    })
    .await
}

fn llm_for(conn: &rusqlite::Connection) -> Result<llm::OpenAi, AiError> {
    Ok(llm::OpenAi::new(llm::load(conn)?.ok_or(AiError::NotSet)?))
}

fn chapter_id(conn: &rusqlite::Connection, novel_id: i64, index: usize) -> Result<i64, AiError> {
    translate::chapter_at(conn, novel_id, index)?
        .ok_or_else(|| AiError::Other(format!("chapter {index} does not exist")))
}

/// How far a chapter translated from the reader is: parts done of all its parts.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChapterProgress {
    novel_id: i64,
    index: usize,
    done: usize,
    total: usize,
}

/// Translates (or, in the book's own language, rewrites) one chapter now and returns
/// it; a chapter done before comes back as it is unless `again`. Progress goes out as
/// `ai-chapter-progress`; the wiki learns from it in the background after the save.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_translate_chapter<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    index: usize,
    lang: String,
    style: String,
    glossary: bool,
    again: bool,
) -> Result<translate::Translated, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        let chapter_id = chapter_id(&conn, novel_id, index)?;
        if !again {
            if let Some(t) = translate::get(&conn, chapter_id, &lang, &style)? {
                return Ok(t);
            }
        }
        let model = std::sync::Arc::new(llm_for(&conn)?);
        let progress: translate::OnProgress = std::sync::Arc::new(move |done, total| {
            let _ = app.emit(
                "ai-chapter-progress",
                ChapterProgress {
                    novel_id,
                    index,
                    done,
                    total,
                },
            );
        });
        translate::run_with(
            &conn,
            model,
            translate::Request {
                novel_id,
                chapter_id,
                lang,
                style,
                glossary,
            },
            translate::Hooks {
                progress: Some(progress),
                learn_in_background: true,
                ..Default::default()
            },
        )
    })
    .await
}

/// The AI text of a chapter, if it was made.
#[tauri::command]
pub async fn ai_chapter<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    index: usize,
    lang: String,
    style: String,
) -> Result<Option<translate::Translated>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        let chapter_id = chapter_id(&conn, novel_id, index)?;
        Ok(translate::get(&conn, chapter_id, &lang, &style)?)
    })
    .await
}

/// Which chapters have an AI version, and in which languages and styles.
#[tauri::command]
pub async fn ai_marks<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<Vec<translate::Mark>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(translate::marks(&db::open(&path)?, novel_id)?)).await
}

/// The AI versions of a book (language, style, chapters done).
#[tauri::command]
pub async fn ai_versions<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<Vec<translate::Version>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(translate::versions(&db::open(&path)?, novel_id)?)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    queued: usize,
    mode: translate::Mode,
}

/// Queues the chapters from `from_index` on for the background translation and says
/// whether they will be translated or rewritten (the book's language is asked first).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_translate_book<R: Runtime>(
    app: AppHandle<R>,
    queue: State<'_, Queue>,
    user: String,
    novel_id: i64,
    from_index: usize,
    lang: String,
    style: String,
    glossary: bool,
    ui_lang: Option<String>,
) -> Result<Queued, AiError> {
    let path = db_path(&app, &user)?;
    let db_file = path.clone();
    let target = lang.clone();
    let (queued, source) = ai_blocking(move || {
        let conn = db::open(&db_file)?;
        if !translate::LANGS.contains(&lang.as_str())
            || !translate::STYLES.contains(&style.as_str())
        {
            return Err(AiError::Other(format!("unsupported {lang} / {style}")));
        }
        let model = llm_for(&conn)?;
        let source = translate::detect(&conn, &model, novel_id)?;
        let queued = queue::enqueue(&conn, novel_id, from_index, &lang, &style, glossary)?;
        Ok((queued, source))
    })
    .await?;
    if queued > 0 {
        queue.kick(path, user, ui_lang);
    }
    let mode = if source == target {
        translate::Mode::Rewrite
    } else {
        translate::Mode::Translate
    };
    Ok(Queued { queued, mode })
}

/// A book's background translations: per language and style, chapters done and waiting.
#[tauri::command]
pub async fn ai_progress<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<Vec<queue::Progress>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(queue::progress(&db::open(&path)?, novel_id)?)).await
}

/// The chapter the background queue is on right now, and where in the pipeline it is.
#[tauri::command]
pub fn ai_live<R: Runtime>(
    app: AppHandle<R>,
    queue: State<'_, Queue>,
    user: String,
) -> Result<Option<queue::Live>, AiError> {
    Ok(queue.live(&db_path(&app, &user)?))
}

/// Stops a book's background translation (chapters done so far stay).
#[tauri::command]
pub async fn ai_cancel<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<usize, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(queue::cancel(&db::open(&path)?, novel_id)?)).await
}

/// Carries on a queue left waiting: after the app starts, a new LLM key, a connection back.
#[tauri::command]
pub async fn ai_resume<R: Runtime>(
    app: AppHandle<R>,
    queue: State<'_, Queue>,
    user: String,
    ui_lang: Option<String>,
) -> Result<(), AiError> {
    let path = db_path(&app, &user)?;
    let db_file = path.clone();
    ai_blocking(move || Ok(queue::recover(&db::open(&db_file)?)?)).await?;
    queue.kick(path, user, ui_lang);
    Ok(())
}

/* ---------- Backups (sync.rs): the whole library in one file the user keeps ---------- */

/// Writes a backup of the library to `path` (picked in the save dialog): books with
/// their AI translations, history, favourites, replace rules, glossaries, downloaded
/// novels, the LLM settings (API key included) and the app's `settings` (JSON).
/// Returns its size in bytes.
#[tauri::command]
pub async fn library_backup<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    path: FilePath,
    settings: Option<String>,
) -> Result<u64, BookError> {
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let db_file = db_path(&app, &user)?;
    blocking(move || {
        let gz = db_file.with_extension("backup.gz");
        let result = (|| {
            let size = super::sync::backup_to(
                &db::open(&db_file)?,
                &db_file.with_extension("backup"),
                &gz,
                settings.as_deref(),
            )?;
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            let mut out = app.fs().open(path, opts)?;
            std::io::copy(&mut std::fs::File::open(&gz)?, &mut out)?;
            out.sync_all()?;
            Ok(size)
        })();
        let _ = std::fs::remove_file(&gz);
        result
    })
    .await
}

/// Brings back a backup picked by the user, merged into the library: what it holds
/// returns even if it was deleted here since. Returns the app's settings kept in the
/// backup (JSON), for the app to apply.
#[tauri::command]
pub async fn library_restore<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    path: FilePath,
) -> Result<Option<String>, BookError> {
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let db_file = db_path(&app, &user)?;
    blocking(move || {
        let mut opts = OpenOptions::new();
        opts.read(true);
        let file = app.fs().open(path, opts)?;
        let mut conn = db::open(&db_file)?;
        let restored = super::sync::restore_from(
            &mut conn,
            std::io::BufReader::new(file),
            &db_file.with_extension("restore"),
        )?;
        for id in restored.gone {
            if let Some(f) = super::original_file(&db_file, id) {
                let _ = std::fs::remove_file(f);
            }
        }
        Ok(restored.settings)
    })
    .await
}

/* ---------- The book wiki (wiki.rs) ---------- */

use super::wiki;

/// A book's wiki: entries, relations and its settings.
#[tauri::command]
pub async fn wiki_get<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
) -> Result<wiki::Wiki, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(wiki::load(&db::open(&path)?, novel_id)?)).await
}

/// What the wiki knew by chapter `number` (from 1), for the reader.
#[tauri::command]
pub async fn wiki_at<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    number: i64,
) -> Result<wiki::Wiki, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(wiki::at_chapter(&db::open(&path)?, novel_id, number)?)).await
}

/// Entries whose names or notes match `query`.
#[tauri::command]
pub async fn wiki_search<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    query: String,
) -> Result<Vec<wiki::Entry>, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || wiki::search(&db::open(&path)?, novel_id, &query, 50)).await
}

/// Adds or edits an entry (the user's from now on).
#[tauri::command]
pub async fn wiki_save<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    entry: wiki::EntryInput,
) -> Result<wiki::Entry, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || wiki::save(&db::open(&path)?, novel_id, entry)).await
}

#[tauri::command]
pub async fn wiki_remove<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    uid: String,
) -> Result<(), AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(wiki::remove(&db::open(&path)?, &uid)?)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn wiki_relate<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    from_uid: String,
    to_uid: String,
    kind: String,
    lang: String,
    note: String,
) -> Result<String, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        wiki::add_relation(&conn, novel_id, &from_uid, &to_uid, &kind, &lang, &note)
    })
    .await
}

/// What the wiki is missing in `lang`: entries and free relations without its words.
#[tauri::command]
pub async fn wiki_missing<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    lang: String,
) -> Result<usize, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let (entries, relations) = wiki::missing(&db::open(&path)?, novel_id, &lang)?;
        Ok(entries.len() + relations.len())
    })
    .await
}

/// Translates what the wiki is missing in `lang` with the user's LLM; returns how many
/// entries and relations changed.
#[tauri::command]
pub async fn wiki_translate<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    lang: String,
) -> Result<usize, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        let model = llm::load(&conn)?
            .map(llm::OpenAi::new)
            .ok_or(AiError::NotSet)?;
        wiki::translate_missing(&conn, &model, novel_id, &lang)
    })
    .await
}

#[tauri::command]
pub async fn wiki_unrelate<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    uid: String,
) -> Result<(), AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || Ok(wiki::remove_relation(&db::open(&path)?, &uid)?)).await
}

/// Whether translating this book updates its wiki; turning it on reads in the facts
/// the translations kept so far and returns the changes made.
#[tauri::command]
pub async fn wiki_set_auto<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    novel_id: i64,
    auto: bool,
) -> Result<usize, AiError> {
    let path = db_path(&app, &user)?;
    ai_blocking(move || {
        let conn = db::open(&path)?;
        wiki::set_auto(&conn, novel_id, auto)?;
        if !auto {
            return Ok(0);
        }
        // Turned on: what the translations found so far goes in now.
        Ok(wiki::learn_kept(&conn, novel_id)?)
    })
    .await
}
