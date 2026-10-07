//! Book files the system asks the app to open ("Open with BenNovel", or a double-click
//! on an associated file). They are queued until the page asks for them, because the
//! request can arrive before the page has loaded.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime, State};

#[derive(Default)]
pub struct OpenedFiles(Mutex<Vec<String>>);

impl OpenedFiles {
    /// Files given on the command line (Windows and Linux pass them this way).
    pub fn from_args() -> Self {
        let files = std::env::args_os()
            .skip(1)
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_file())
            .map(|p| p.display().to_string())
            .collect();
        Self(Mutex::new(files))
    }

    /// Queues files and tells the page; on Android and macOS they arrive as URLs.
    #[allow(dead_code)] // only used where the system sends open requests to a running app
    pub fn push<R: Runtime>(&self, app: &AppHandle<R>, files: Vec<String>) {
        if files.is_empty() {
            return;
        }
        self.0.lock().unwrap().extend(files);
        let _ = app.emit("files-opened", ());
    }
}

/// Takes the queued files (paths or `file:`/`content:` URLs), oldest first.
#[tauri::command]
pub fn take_opened_files(files: State<'_, OpenedFiles>) -> Vec<String> {
    std::mem::take(&mut *files.0.lock().unwrap())
}
