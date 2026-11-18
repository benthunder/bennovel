//! Android stops an app's work soon after the screen goes off or the app goes to the
//! background. While the AI translation runs, this plugin keeps it going the way a video
//! player does: a foreground service (type `dataSync`) with an ongoing notification that
//! shows the progress, a partial wake lock for the CPU and a Wi-Fi lock for the network.
//!
//! The app calls `show` as the work moves (the first call starts the service, the next
//! ones only change the notification) and `hide` once nothing is left to do. The calls go
//! to the service through JNI, so they work after the app was swiped away too: the work
//! then goes on with no window, like a download (`set_headless`). On other systems the
//! plugin does nothing.

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
pub struct BackgroundWork<R: Runtime>(#[allow(dead_code)] tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> BackgroundWork<R> {
    /// Starts the service with this notification, or changes the notification of the
    /// running one. Call it off the main thread.
    pub fn show(&self, notice: &Notice) -> Result<(), String> {
        native::call(|env, class, context| {
            let title = env.new_string(&notice.title)?;
            let text = env.new_string(&notice.text)?;
            let sub = env.new_string(&notice.sub_text)?;
            env.call_static_method(
                class,
                "showNative",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;II)V",
                &[
                    context.into(),
                    (&title).into(),
                    (&text).into(),
                    (&sub).into(),
                    (notice.done.min(i32::MAX as u32) as i32).into(),
                    (notice.total.min(i32::MAX as u32) as i32).into(),
                ],
            )?;
            Ok(())
        })
    }

    /// Stops the service and takes the notification away.
    pub fn hide(&self) -> Result<(), String> {
        native::call(|env, class, context| {
            env.call_static_method(class, "hideNative", "(Landroid/content/Context;)V", &[context.into()])?;
            Ok(())
        })
    }

    /// The app's window is gone and the work goes on without it (`on`), or not any more.
    /// While it is so, opening the app again starts it afresh (Relaunch.kt).
    pub fn set_headless(&self, on: bool) -> Result<(), String> {
        native::call(|env, class, context| {
            env.call_static_method(
                class,
                "setHeadless",
                "(Landroid/content/Context;Z)V",
                &[context.into(), on.into()],
            )?;
            Ok(())
        })
    }
}

#[cfg(target_os = "android")]
mod native {
    use jni::objects::{JClass, JObject, JValue};
    use jni::{JNIEnv, JavaVM};

    /// Runs `f` with TranslateService's class and the app's context, on this thread
    /// (attached to the VM for good: it is the notification's own thread).
    pub fn call(
        f: impl FnOnce(&mut JNIEnv, &JClass, &JObject) -> jni::errors::Result<()>,
    ) -> Result<(), String> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|e| e.to_string())?;
        // Owned by the app (a global reference); never deleted here.
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let result = env
            .with_local_frame(16, |env| -> jni::errors::Result<()> {
                // Classes of the app load through its own class loader on threads that are
                // not Java's.
                let loader = env
                    .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                    .l()?;
                let name = env.new_string("app.bennovel.backgroundwork.TranslateService")?;
                let class = env
                    .call_method(
                        &loader,
                        "loadClass",
                        "(Ljava/lang/String;)Ljava/lang/Class;",
                        &[JValue::Object(&name)],
                    )?
                    .l()?;
                f(env, &JClass::from(class), &context)
            })
            .map_err(|e| e.to_string());
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return Err("Java exception in the translation service".into());
        }
        result
    }
}
