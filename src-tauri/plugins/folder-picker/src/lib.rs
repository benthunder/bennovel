//! Android has no folder paths for apps: a folder is picked as a document tree and its
//! files are reached through content URIs. This plugin is the thin bridge to that API:
//! `pick` shows the system folder picker and `list` lists one folder. Walking the tree,
//! choosing files and importing them is done by the app in Rust.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

/// A picked folder: the tree URI and the document id of its root.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Folder {
    pub tree: String,
    pub id: String,
    pub name: String,
}

/// One entry of a folder.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Entry {
    /// Document id, for listing it when it is a folder.
    pub id: String,
    pub name: String,
    pub dir: bool,
    /// `content://` URI to open it with.
    pub uri: String,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("folder-picker")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = _api
                    .register_android_plugin("app.bennovel.folderpicker", "FolderPickerPlugin")?;
                _app.manage(FolderPicker(handle));
            }
            Ok(())
        })
        .build()
}

/// Access to the Android folder picker (managed state, Android only).
#[cfg(target_os = "android")]
pub struct FolderPicker<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> FolderPicker<R> {
    /// Shows the folder picker and waits for the choice; None when cancelled.
    /// Blocks, so call it off the main thread.
    pub fn pick(&self) -> Result<Option<Folder>, String> {
        #[derive(serde::Deserialize)]
        struct Picked {
            folder: Option<Folder>,
        }
        let picked: Picked = self
            .0
            .run_mobile_plugin("pickFolder", ())
            .map_err(|e| e.to_string())?;
        Ok(picked.folder)
    }

    /// The entries directly inside folder `id` of `tree`.
    pub fn list(&self, tree: &str, id: &str) -> Result<Vec<Entry>, String> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            tree: &'a str,
            id: &'a str,
        }
        #[derive(serde::Deserialize)]
        struct Listed {
            entries: Vec<Entry>,
        }
        let listed: Listed = self
            .0
            .run_mobile_plugin("listDir", Args { tree, id })
            .map_err(|e| e.to_string())?;
        Ok(listed.entries)
    }
}
