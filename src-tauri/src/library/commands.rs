use super::folder::{self, FolderBook, Opener};
use super::{db, DbBook, LibraryBook};
use crate::book::commands::{open_picked, pdfium_dirs, Books};
use crate::book::{detect, open_source, BookError, BookFormat, BookInfo, BookSource, OpenCtx};
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
    let book = books.add(format, Box::new(source));
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

/// Favourites and history of online novels, with their snapshots; null before the
/// first save.
#[tauri::command]
pub async fn reading_load<R: Runtime>(
    app: AppHandle<R>,
    user: String,
) -> Result<Option<super::reading::ReadingState>, BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || Ok(super::reading::load(&db::open(&path)?)?)).await
}

#[tauri::command]
pub async fn reading_save<R: Runtime>(
    app: AppHandle<R>,
    user: String,
    state: super::reading::ReadingState,
) -> Result<(), BookError> {
    let path = db_path(&app, &user)?;
    blocking(move || Ok(super::reading::save(&mut db::open(&path)?, &state)?)).await
}
