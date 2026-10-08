mod book;
mod library;
mod opened;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_folder_picker::init())
        .manage(book::commands::Books::default())
        .manage(opened::OpenedFiles::from_args())
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
            library::commands::library_save_progress,
            library::commands::library_update,
            library::commands::library_delete,
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
