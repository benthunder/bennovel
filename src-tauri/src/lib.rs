mod book;
mod library;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(book::commands::Books::default())
        .invoke_handler(tauri::generate_handler![
            book::commands::book_open,
            book::commands::book_section,
            book::commands::book_close,
            library::commands::library_list,
            library::commands::library_import,
            library::commands::library_open,
            library::commands::library_save_progress,
            library::commands::library_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running BenNovel");
}
