#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            better_debate_tauri_host::configure_backend(app.handle(), env!("CARGO_MANIFEST_DIR"))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            better_debate_tauri_host::commands::invoke_core_rpc,
            better_debate_tauri_host::tex_sessions::tex_open_session_from_file,
            better_debate_tauri_host::tex_sessions::tex_create_session_at_path,
            better_debate_tauri_host::tex_sessions::tex_attach_session,
            better_debate_tauri_host::tex_sessions::tex_update_session,
            better_debate_tauri_host::tex_sessions::tex_save_session,
            better_debate_tauri_host::tex_sessions::tex_prepare_popout,
            better_debate_tauri_host::tex_sessions::tex_release_session,
            better_debate_tauri_host::tex_sessions::tex_list_recoverable_sessions,
            better_debate_tauri_host::tex_sessions::tex_discard_recoverable_session,
            better_debate_tauri_host::tex_sessions::tex_list_detached_windows,
            better_debate_tauri_host::tex_sessions::tex_list_open_sessions,
            better_debate_tauri_host::tex_sessions::tex_get_active_speech_target,
            better_debate_tauri_host::tex_sessions::tex_set_active_speech_target,
            better_debate_tauri_host::tex_sessions::tex_send_to_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tex");
}
