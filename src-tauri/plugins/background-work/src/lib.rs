//! Android stops an app's work soon after the screen goes off or the app goes to the
//! background. While the AI translation runs, this plugin keeps it going the way a video
//! player does: a foreground service (type `dataSync`) with an ongoing notification that
//! shows the progress, a partial wake lock for the CPU and a Wi-Fi lock for the network.
//!
//! The app calls `show` as the work moves (the first call starts the service, the next
//! ones only change the notification) and `hide` once nothing is left to do. On other
//! systems the plugin does nothing.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

/// What the notification shows.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub text: String,
    /// Shown small under the text, e.g. "3/120 chương".
    pub sub_text: String,
    /// The bar: `done` of `total`; an indeterminate bar when `total` is 0.
    pub done: u32,
    pub total: u32,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("background-work")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = _api.register_android_plugin(
                    "app.bennovel.backgroundwork",
                    "BackgroundWorkPlugin",
                )?;
                _app.manage(BackgroundWork(handle));
            }
            Ok(())
        })
        .build()
}

/// The foreground service (managed state, Android only).
#[cfg(target_os = "android")]
pub struct BackgroundWork<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> BackgroundWork<R> {
    /// Starts the service with this notification, or changes the notification of the
    /// running one. Blocks until Android took it, so call it off the main thread.
    pub fn show(&self, notice: &Notice) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<serde_json::Value>("show", notice)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Stops the service and takes the notification away.
    pub fn hide(&self) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<serde_json::Value>("hide", ())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
