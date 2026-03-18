use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(desktop)]
            if option_env!("TAURI_UPDATER_PUBLIC_KEY").is_some() {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }

            app.manage(tex_tauri_host::tex_sessions::create_store(
                app.path()
                    .app_data_dir()
                    .map_err(|error| format!("Could not resolve app data directory: {error}"))?,
            )?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tex_tauri_host::tex_sessions::tex_open_session_from_file,
            tex_tauri_host::tex_sessions::tex_create_session_at_path,
            tex_tauri_host::tex_sessions::tex_attach_session,
            tex_tauri_host::tex_sessions::tex_update_session,
            tex_tauri_host::tex_sessions::tex_save_session,
            tex_tauri_host::tex_sessions::tex_prepare_popout,
            tex_tauri_host::tex_sessions::tex_release_session,
            tex_tauri_host::tex_sessions::tex_list_recoverable_sessions,
            tex_tauri_host::tex_sessions::tex_discard_recoverable_session,
            tex_tauri_host::tex_sessions::tex_list_detached_windows,
            tex_tauri_host::tex_sessions::tex_list_open_sessions,
            tex_tauri_host::tex_sessions::tex_get_active_speech_target,
            tex_tauri_host::tex_sessions::tex_set_active_speech_target,
            tex_tauri_host::tex_sessions::tex_send_to_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tex");
}
