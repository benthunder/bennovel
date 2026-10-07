use super::{open_source, BookError, BookInfo, OpenBook, OpenCtx, Section};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};

/// Books currently open in the reader, by id.
#[derive(Default)]
pub struct Books {
    open: Mutex<HashMap<u32, Arc<OpenBook>>>,
    next_id: AtomicU32,
}

impl Books {
    fn get(&self, id: u32) -> Result<Arc<OpenBook>, BookError> {
        self.open
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or(BookError::NotOpen(id))
    }
}

/// Folders searched for the Pdfium library before the system one.
fn pdfium_dirs<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        dirs.push(res.join("pdfium"));
        dirs.push(res);
    }
    dirs
}

/// Opens a book picked by the user (a path, or a `content://` URI on Android).
/// Only the table of contents is read here (or, for compressed formats, the text is
/// extracted to the cache folder); text is read per section by `book_section`.
#[tauri::command]
pub async fn book_open<R: Runtime>(
    app: AppHandle<R>,
    books: State<'_, Books>,
    path: FilePath,
) -> Result<BookInfo, BookError> {
    let name = path.to_string();
    let local_path = match &path {
        FilePath::Path(p) => Some(p.clone()),
        FilePath::Url(u) => u.to_file_path().ok(),
    };
    let mut opts = OpenOptions::new();
    opts.read(true);
    let file = app.fs().open(path, opts)?;
    let ctx = OpenCtx {
        name,
        path: local_path,
        cache_dir: app
            .path()
            .app_cache_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("books"),
        pdfium_dirs: pdfium_dirs(&app),
    };
    let id = books.next_id.fetch_add(1, Ordering::SeqCst) + 1;

    let book = tauri::async_runtime::spawn_blocking(move || {
        let (format, source) = open_source(file, &ctx)?;
        Ok::<_, BookError>(OpenBook::new(id, format, source))
    })
    .await
    .map_err(|e| BookError::Parse(e.to_string()))??;

    let info = book.info.clone();
    books.open.lock().unwrap().insert(id, Arc::new(book));
    Ok(info)
}

/// Text of one section. Moves the in-memory window there: neighbours are read
/// ahead in the background and sections far from it are freed.
#[tauri::command]
pub async fn book_section(
    books: State<'_, Books>,
    id: u32,
    index: usize,
) -> Result<Section, BookError> {
    let book = books.get(id)?;
    let section = tauri::async_runtime::spawn_blocking(move || book.section(index))
        .await
        .map_err(|e| BookError::Parse(e.to_string()))??;
    // The cache keeps its own copy; the frontend gets a serialized clone.
    Ok(Section {
        index: section.index,
        paragraphs: section.paragraphs.clone(),
    })
}

/// Closes the file and frees every cached section.
#[tauri::command]
pub fn book_close(books: State<'_, Books>, id: u32) {
    books.open.lock().unwrap().remove(&id);
}
