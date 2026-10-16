mod book;
mod library;
mod opened;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // Registered first: a second launch hands its sign-in link (to the deep-link plugin)
    // or its book files to the running app, and ends.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            use tauri::Manager;
            let files = argv
                .into_iter()
                .skip(1)
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_file())
                .map(|p| p.display().to_string())
                .collect();
            app.state::<opened::OpenedFiles>().push(app, files);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_folder_picker::init())
        .plugin(tauri_plugin_notification::init())
        .manage(book::commands::Books::default())
        .manage(opened::OpenedFiles::from_args())
        .setup(|_app| {
            use tauri::Manager;
            // AI translation runs in the background, one chapter at a time.
            _app.manage(library::queue::Queue::start(_app.handle().clone()));
            // Android: the WebView starts without view focus, so the first tap on a text
            // field only focuses the WebView itself and the keyboard stays hidden. Give it
            // focus up front so that first tap opens the keyboard.
            #[cfg(target_os = "android")]
            if let Some(window) = _app.get_webview_window("main") {
                let _ = window.with_webview(|webview| {
                    use jni::objects::JValue;
                    webview.jni_handle().exec(|env, _, webview| {
                        let _ = env.call_method(webview, "setFocusableInTouchMode", "(Z)V", &[JValue::Bool(1)]);
                        let _ = env.call_method(webview, "requestFocus", "()Z", &[]);
                    });
                });
            }
            // Linux (and Windows dev builds) register the bennovel:// sign-in links at run time.
            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                _app.deep_link().register_all()?;
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("bookimg", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            // Decoding and drawing pages is slow; keep it off the webview's thread.
            std::thread::spawn(move || {
                responder.respond(book::commands::serve_image(&app, &path));
            });
        })
        .invoke_handler(tauri::generate_handler![
            book::commands::book_open,
            book::commands::book_section,
            book::commands::book_close,
            library::commands::library_list,
            library::commands::library_import,
            library::commands::library_import_folder,
            library::commands::library_import_android_folder,
            library::commands::library_open,
            library::commands::library_ocr_next,
            library::commands::library_ocr_page,
            library::commands::library_ocr_save,
            library::commands::library_detail,
            library::commands::library_save_progress,
            library::commands::library_update,
            library::commands::library_delete,
            library::commands::reading_load,
            library::commands::reading_save,
            library::commands::replace_rules_get,
            library::commands::replace_rules_set,
            library::commands::library_backup,
            library::commands::library_restore,
            library::commands::offline_start,
            library::commands::offline_add,
            library::commands::offline_set_glossary,
            library::commands::offline_finish,
            library::commands::offline_list,
            library::commands::offline_chapters,
            library::commands::offline_chapter,
            library::commands::offline_glossary,
            library::commands::offline_delete,
            library::commands::offline_ensure,
            library::commands::offline_numbers,
            library::commands::llm_config_get,
            library::commands::llm_config_set,
            library::commands::ai_translate_chapter,
            library::commands::ai_marks,
            library::commands::ai_chapter,
            library::commands::ai_versions,
            library::commands::ai_translate_book,
            library::commands::ai_progress,
            library::commands::ai_cancel,
            library::commands::ai_resume,
            library::commands::wiki_get,
            library::commands::wiki_search,
            library::commands::wiki_save,
            library::commands::wiki_remove,
            library::commands::wiki_relate,
            library::commands::wiki_unrelate,
            library::commands::wiki_set_auto,
            library::commands::wiki_missing,
            library::commands::wiki_translate,
            opened::take_opened_files,
        ])
        .build(tauri::generate_context!())
        .expect("error while building BenNovel")
        .run(|_app, _event| {
            // Android and macOS hand files to open to the running app.
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
            if let tauri::RunEvent::Opened { urls } = _event {
                use tauri::Manager;
                let files = urls
                    .into_iter()
                    .filter(|u| matches!(u.scheme(), "file" | "content"))
                    .map(String::from)
                    .collect();
                _app.state::<opened::OpenedFiles>().push(_app, files);
            }
        });
}
