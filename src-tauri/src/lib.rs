mod book;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running BenNovel");
}
