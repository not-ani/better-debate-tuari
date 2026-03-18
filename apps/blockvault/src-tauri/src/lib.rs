#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            search_tauri_host::configure_backend(app.handle(), env!("CARGO_MANIFEST_DIR"))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_tauri_host::commands::invoke_core_rpc
        ])
        .run(tauri::generate_context!())
        .expect("error while running BlockVault");
}
