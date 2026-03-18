use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tauri::{Emitter, Manager};

fn resolve_resource_dir(app: &tauri::AppHandle, manifest_dir: &str) -> Result<PathBuf, String> {
    let dev_resources = Path::new(manifest_dir).join("resources");
    if dev_resources.exists() {
        return Ok(dev_resources);
    }

    app.path()
        .resource_dir()
        .map(|path| path.join("resources"))
        .map_err(|error| format!("Could not resolve bundled resources: {error}"))
}

pub fn configure_backend(app: &tauri::AppHandle, manifest_dir: &str) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve app data directory: {error}"))?;
    let resource_dir = resolve_resource_dir(app, manifest_dir)?;

    search_core::set_event_callback(Some(Arc::new({
        let app = app.clone();
        move |event, payload_json| {
            let payload = serde_json::from_str::<Value>(&payload_json)
                .unwrap_or_else(|_| Value::String(payload_json));
            let _ = app.emit(event.as_str(), payload);
        }
    })));

    search_core::configure(app_data_dir, Some(resource_dir))
}

pub mod commands {
    use serde_json::Value;

    #[tauri::command]
    pub async fn invoke_core_rpc(command: String, args: Option<Value>) -> Result<Value, String> {
        tauri::async_runtime::spawn_blocking(move || {
            search_core::invoke(
                command,
                args.unwrap_or(Value::Object(Default::default())),
            )
        })
        .await
        .map_err(|error| format!("Core task failed to join: {error}"))?
    }
}
