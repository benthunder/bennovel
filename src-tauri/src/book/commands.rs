use super::{open_source, BookError, BookFormat, BookInfo, BookSource, OpenBook, OpenCtx, Section};
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
    /// Registers an opened book and returns what the reader needs to show it.
    pub(crate) fn add(&self, format: BookFormat, source: Box<dyn BookSource>) -> BookInfo {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let book = OpenBook::new(id, format, source);
        let info = book.info.clone();
        self.open.lock().unwrap().insert(id, Arc::new(book));
        info
    }

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

/// Opens the file the user picked (a path, or a `content://` URI on Android) and
/// describes where the book reader may put cache files.
pub(crate) fn open_picked<R: Runtime>(
    app: &AppHandle<R>,
    path: FilePath,
) -> Result<(std::fs::File, OpenCtx), BookError> {
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
        pdfium_dirs: pdfium_dirs(app),
    };
    Ok((file, ctx))
}

/// Opens a book picked by the user without importing it.
/// Only the table of contents is read here (or, for compressed formats, the text is
/// extracted to the cache folder); text is read per section by `book_section`.
#[tauri::command]
pub async fn book_open<R: Runtime>(
    app: AppHandle<R>,
    books: State<'_, Books>,
    path: FilePath,
) -> Result<BookInfo, BookError> {
    let (file, ctx) = open_picked(&app, path)?;
    let (format, source) = tauri::async_runtime::spawn_blocking(move || open_source(file, &ctx))
        .await
        .map_err(|e| BookError::Parse(e.to_string()))??;
    Ok(books.add(format, source))
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
