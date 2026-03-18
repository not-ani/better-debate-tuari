use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

pub type CommandResult<T> = Result<T, String>;

pub(crate) const DEFAULT_CAPTURE_TARGET: &str = "BlockFile-Captures.docx";

mod runtime {
    use serde::Serialize;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock, RwLock};

    pub type EventCallback = Arc<dyn Fn(String, String) + Send + Sync + 'static>;

    static EVENT_CALLBACK: OnceLock<RwLock<Option<EventCallback>>> = OnceLock::new();

    fn callback_cell() -> &'static RwLock<Option<EventCallback>> {
        EVENT_CALLBACK.get_or_init(|| RwLock::new(None))
    }

    pub fn set_event_callback(callback: Option<EventCallback>) {
        if let Ok(mut writer) = callback_cell().write() {
            *writer = callback;
        }
    }

    #[derive(Clone)]
    pub struct AppHandle {
        state: Arc<AppState>,
    }

    #[derive(Debug)]
    struct AppState {
        app_data_dir: PathBuf,
        resource_dir: Option<PathBuf>,
    }

    impl AppHandle {
        pub fn new(app_data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Self {
            Self {
                state: Arc::new(AppState {
                    app_data_dir,
                    resource_dir,
                }),
            }
        }

        pub fn path(&self) -> PathResolver {
            PathResolver {
                state: Arc::clone(&self.state),
            }
        }

        pub fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
            let callback = callback_cell()
                .read()
                .ok()
                .and_then(|reader| reader.as_ref().cloned());
            let Some(callback) = callback else {
                return Ok(());
            };

            let payload_json = serde_json::to_string(&payload)
                .map_err(|error| format!("Could not serialize event payload: {error}"))?;
            callback(event.to_string(), payload_json);
            Ok(())
        }
    }

    #[derive(Clone)]
    pub struct PathResolver {
        state: Arc<AppState>,
    }

    impl PathResolver {
        pub fn app_data_dir(&self) -> io::Result<PathBuf> {
            Ok(self.state.app_data_dir.clone())
        }

        pub fn resource_dir(&self) -> io::Result<PathBuf> {
            self.state.resource_dir.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Resource directory was not configured",
                )
            })
        }
    }

    pub trait Manager {
        fn path(&self) -> PathResolver;
    }

    impl Manager for AppHandle {
        fn path(&self) -> PathResolver {
            AppHandle::path(self)
        }
    }

    pub trait Emitter {
        fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String>;
    }

    impl Emitter for AppHandle {
        fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
            AppHandle::emit(self, event, payload)
        }
    }

    pub mod async_runtime {
        use std::future::Future;
        use std::sync::OnceLock;
        use tokio::runtime::{Builder, Runtime};
        use tokio::task::JoinHandle;

        static RUNTIME: OnceLock<Runtime> = OnceLock::new();

        fn runtime() -> &'static Runtime {
            RUNTIME.get_or_init(|| {
                Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build async runtime")
            })
        }

        pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
        where
            F: Future + Send + 'static,
            F::Output: Send + 'static,
        {
            runtime().spawn(future)
        }

        pub fn spawn_blocking<F, R>(function: F) -> JoinHandle<R>
        where
            F: FnOnce() -> R + Send + 'static,
            R: Send + 'static,
        {
            runtime().spawn_blocking(function)
        }

        pub fn block_on<F>(future: F) -> F::Output
        where
            F: Future,
        {
            runtime().block_on(future)
        }
    }
}

mod chunking;
mod commands;
mod db;
mod docx_capture;
mod docx_parse;
mod indexer;
mod lexical;
mod preview;
mod query_engine;
mod search;
mod search_engine;
mod semantic;
mod types;
mod util;
mod vector;
pub use runtime::{set_event_callback, AppHandle, Emitter, Manager};

pub mod async_runtime {
    pub use crate::runtime::async_runtime::{block_on, spawn, spawn_blocking};
}

static APP_HANDLE: OnceLock<RwLock<Option<AppHandle>>> = OnceLock::new();

fn app_handle_cell() -> &'static RwLock<Option<AppHandle>> {
    APP_HANDLE.get_or_init(|| RwLock::new(None))
}

fn current_app_handle() -> CommandResult<AppHandle> {
    let reader = app_handle_cell()
        .read()
        .map_err(|_| "Could not read app configuration".to_string())?;
    reader
        .as_ref()
        .cloned()
        .ok_or_else(|| "Core backend is not configured".to_string())
}

fn set_app_handle(app_handle: AppHandle) -> CommandResult<()> {
    let mut writer = app_handle_cell()
        .write()
        .map_err(|_| "Could not update app configuration".to_string())?;
    *writer = Some(app_handle);
    Ok(())
}

pub fn configure(app_data_dir: PathBuf, resource_dir: Option<PathBuf>) -> CommandResult<()> {
    set_app_handle(AppHandle::new(app_data_dir, resource_dir))
}

pub fn invoke(command: String, args: Value) -> CommandResult<Value> {
    invoke_command(InvokeRequest { command, args })
}

#[derive(Deserialize)]
struct InvokeRequest {
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddRootArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveRootArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetIndexSnapshotArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListCaptureTargetsArgs {
    root_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureTargetPreviewArgs {
    root_path: String,
    target_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteCaptureHeadingArgs {
    root_path: String,
    target_path: String,
    heading_order: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveCaptureHeadingArgs {
    root_path: String,
    target_path: String,
    source_heading_order: i64,
    target_heading_order: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddCaptureHeadingArgs {
    root_path: String,
    target_path: String,
    heading_level: i64,
    heading_text: String,
    selected_target_heading_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexRootArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFilePreviewArgs {
    file_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetHeadingPreviewHtmlArgs {
    file_id: i64,
    heading_order: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InsertCaptureArgs {
    root_path: String,
    source_path: String,
    section_title: String,
    content: String,
    paragraph_xml: Option<Vec<String>>,
    target_path: Option<String>,
    heading_level: Option<i64>,
    heading_order: Option<i64>,
    selected_target_heading_order: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchIndexHybridArgs {
    query: String,
    root_path: Option<String>,
    limit: Option<usize>,
    file_name_only: Option<bool>,
    semantic_enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchWarmArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkRootPerformanceArgs {
    path: String,
    queries: Option<Vec<String>>,
    iterations: Option<usize>,
    limit: Option<usize>,
    include_semantic: Option<bool>,
    preview_samples: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkQueryRuntimeArgs {
    path: String,
    queries: Option<Vec<String>>,
    iterations: Option<usize>,
    limit: Option<usize>,
    include_semantic: Option<bool>,
}

fn parse_args<T: DeserializeOwned>(value: Value) -> CommandResult<T> {
    serde_json::from_value(value).map_err(|error| format!("Could not parse command args: {error}"))
}

fn to_json_value<T: Serialize>(value: T) -> CommandResult<Value> {
    serde_json::to_value(value)
        .map_err(|error| format!("Could not serialize command result: {error}"))
}

fn invoke_command(request: InvokeRequest) -> CommandResult<Value> {
    let app = current_app_handle()?;
    let InvokeRequest { command, args } = request;

    match command.as_str() {
        "add_root" => {
            let args: AddRootArgs = parse_args(args)?;
            to_json_value(commands::add_root(app, args.path)?)
        }
        "remove_root" => {
            let args: RemoveRootArgs = parse_args(args)?;
            to_json_value(commands::remove_root(app, args.path)?)
        }
        "list_roots" => {
            let _: EmptyArgs = parse_args(args)?;
            to_json_value(commands::list_roots(app)?)
        }
        "list_root_indexes" => {
            let _: EmptyArgs = parse_args(args)?;
            to_json_value(commands::list_root_indexes(app)?)
        }
        "get_index_snapshot" => {
            let args: GetIndexSnapshotArgs = parse_args(args)?;
            to_json_value(commands::get_index_snapshot(app, args.path)?)
        }
        "list_capture_targets" => {
            let args: ListCaptureTargetsArgs = parse_args(args)?;
            to_json_value(commands::list_capture_targets(app, args.root_path)?)
        }
        "get_capture_target_preview" => {
            let args: CaptureTargetPreviewArgs = parse_args(args)?;
            to_json_value(commands::get_capture_target_preview(
                app,
                args.root_path,
                args.target_path,
            )?)
        }
        "delete_capture_heading" => {
            let args: DeleteCaptureHeadingArgs = parse_args(args)?;
            to_json_value(commands::delete_capture_heading(
                app,
                args.root_path,
                args.target_path,
                args.heading_order,
            )?)
        }
        "move_capture_heading" => {
            let args: MoveCaptureHeadingArgs = parse_args(args)?;
            to_json_value(commands::move_capture_heading(
                app,
                args.root_path,
                args.target_path,
                args.source_heading_order,
                args.target_heading_order,
            )?)
        }
        "add_capture_heading" => {
            let args: AddCaptureHeadingArgs = parse_args(args)?;
            to_json_value(commands::add_capture_heading(
                app,
                args.root_path,
                args.target_path,
                args.heading_level,
                args.heading_text,
                args.selected_target_heading_order,
            )?)
        }
        "index_root" => {
            let args: IndexRootArgs = parse_args(args)?;
            to_json_value(commands::index_root(app, args.path)?)
        }
        "get_file_preview" => {
            let args: GetFilePreviewArgs = parse_args(args)?;
            to_json_value(commands::get_file_preview(app, args.file_id)?)
        }
        "get_heading_preview_html" => {
            let args: GetHeadingPreviewHtmlArgs = parse_args(args)?;
            to_json_value(commands::get_heading_preview_html(
                app,
                args.file_id,
                args.heading_order,
            )?)
        }
        "insert_capture" => {
            let args: InsertCaptureArgs = parse_args(args)?;
            to_json_value(commands::insert_capture(
                app,
                args.root_path,
                args.source_path,
                args.section_title,
                args.content,
                args.paragraph_xml,
                args.target_path,
                args.heading_level,
                args.heading_order,
                args.selected_target_heading_order,
            )?)
        }
        "search_index_hybrid" => {
            let args: SearchIndexHybridArgs = parse_args(args)?;
            to_json_value(async_runtime::block_on(commands::search_index_hybrid(
                app,
                args.query,
                args.root_path,
                args.limit,
                args.file_name_only,
                args.semantic_enabled,
            ))?)
        }
        "search" => {
            let args: crate::types::SearchRequest = parse_args(args)?;
            to_json_value(async_runtime::block_on(commands::search(app, args))?)
        }
        "hydrate_search_results" => {
            let args: crate::types::SearchHydrateRequest = parse_args(args)?;
            to_json_value(async_runtime::block_on(commands::hydrate_search_results(
                app, args,
            ))?)
        }
        "search_warm" => {
            let _: SearchWarmArgs = parse_args(args)?;
            to_json_value(commands::search_warm(app)?)
        }
        "search_explain" => {
            let mut args: crate::types::SearchRequest = parse_args(args)?;
            args.diagnostics = Some(true);
            to_json_value(async_runtime::block_on(commands::search(app, args))?)
        }
        "index_status" => {
            let _: EmptyArgs = parse_args(args)?;
            to_json_value(commands::index_status(app)?)
        }
        "index_optimize" => {
            let _: EmptyArgs = parse_args(args)?;
            to_json_value(commands::index_optimize(app)?)
        }
        "semantic_install_status" => {
            let _: EmptyArgs = parse_args(args)?;
            to_json_value(commands::semantic_install_status(app)?)
        }
        "benchmark_root_performance" => {
            let args: BenchmarkRootPerformanceArgs = parse_args(args)?;
            to_json_value(async_runtime::block_on(
                commands::benchmark_root_performance(
                    app,
                    args.path,
                    args.queries,
                    args.iterations,
                    args.limit,
                    args.include_semantic,
                    args.preview_samples,
                ),
            )?)
        }
        "benchmark_query_runtime" => {
            let args: BenchmarkQueryRuntimeArgs = parse_args(args)?;
            to_json_value(async_runtime::block_on(commands::benchmark_query_runtime(
                app,
                args.path,
                args.queries,
                args.iterations,
                args.limit,
                args.include_semantic,
            ))?)
        }
        _ => Err(format!("Unknown command: {command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{invoke, set_app_handle, AppHandle};
    use serde_json::json;

    fn configure_test_app(label: &str) {
        let root = std::env::temp_dir().join(format!(
            "search-core-test-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        set_app_handle(AppHandle::new(root, None)).unwrap();
    }

    #[test]
    fn invoke_reports_unknown_commands() {
        configure_test_app("unknown");
        let error = invoke("definitely_unknown_command".to_string(), json!({})).unwrap_err();
        assert!(error.contains("Unknown command"));
    }

    #[test]
    fn invoke_reports_argument_parse_failures() {
        configure_test_app("parse");
        let error = invoke("list_roots".to_string(), json!("wrong-shape")).unwrap_err();
        assert!(error.contains("Could not parse command args"));
    }
}
