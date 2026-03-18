use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::runtime::AppHandle;
use arrow_array::types::Float32Type;
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::database::CreateTableMode;
use lancedb::index::{scalar::BTreeIndexBuilder, Index as LanceIndex};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::{connect as connect_lancedb, Table as LanceTable};
use ort::{session::Session as OrtSession, value::Tensor as OrtTensor};
use rusqlite::{params, OptionalExtension};
use tokenizers::Tokenizer;

use crate::db::{index_vector_dir, open_database};
use crate::types::{SearchHit, SemanticCandidate, SemanticRootIndexState, SemanticRuntime};
use crate::util::{file_name_from_relative, now_ms, path_display};
use crate::CommandResult;

pub(crate) const SEMANTIC_TABLE_NAME: &str = "semantic_hits_v2";
pub(crate) const SEMANTIC_MAX_DOCUMENTS: usize = 2_000_000;
pub(crate) const SEMANTIC_EMBED_BATCH: usize = 24;
pub(crate) const SEMANTIC_MAX_TOKENS: usize = 192;
pub(crate) const SEMANTIC_MIN_QUERY_CHARS: usize = 3;

static SEMANTIC_RUNTIME: OnceLock<Mutex<SemanticRuntime>> = OnceLock::new();
static SEMANTIC_REBUILD_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct SemanticRebuildQueue {
    pending: VecDeque<i64>,
    seen: HashSet<i64>,
    forced: HashSet<i64>,
}

#[derive(Clone)]
struct SemanticResultRow {
    kind: String,
    file_id: i64,
    source_row_id: i64,
    heading_level: Option<i64>,
    heading_order: Option<i64>,
    score: f64,
}

static SEMANTIC_REBUILD_QUEUE: OnceLock<Mutex<SemanticRebuildQueue>> = OnceLock::new();

pub(crate) fn semantic_db_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    index_vector_dir(app)
}

fn resolve_semantic_resource_path(app: &AppHandle, file_name: &str) -> CommandResult<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join(file_name));
        candidates.push(resource_dir.join("resources").join(file_name));
    }
    let manifest_resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    candidates.push(manifest_resources.join(file_name));

    for path in candidates {
        if path.exists() {
            return Ok(path);
        }
    }

    Err(format!(
        "Missing semantic resource '{file_name}'. Expected it under the app resource directory or '{}'",
        path_display(&manifest_resources)
    ))
}

fn build_semantic_runtime(app: &AppHandle) -> CommandResult<SemanticRuntime> {
    let model_path = resolve_semantic_resource_path(app, "model.onnx")?;
    let tokenizer_path = resolve_semantic_resource_path(app, "tokenizer.json")?;
    let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|error| {
        format!(
            "Could not load tokenizer '{}': {error}",
            path_display(&tokenizer_path)
        )
    })?;
    let mut builder = OrtSession::builder().map_err(|error| {
        format!(
            "Could not create ONNX session builder for '{}': {error}",
            path_display(&model_path)
        )
    })?;
    if let Ok(parallelism) = std::thread::available_parallelism() {
        let threads = parallelism.get().clamp(1, 8);
        builder = builder
            .with_intra_threads(threads)
            .map_err(|error| format!("Could not set ONNX thread count: {error}"))?;
    }
    let session = builder.commit_from_file(&model_path).map_err(|error| {
        format!(
            "Could not load ONNX model '{}': {error}",
            path_display(&model_path)
        )
    })?;
    let output_name = session
        .outputs()
        .first()
        .map(|entry| entry.name().to_string())
        .ok_or_else(|| "ONNX model has no outputs".to_string())?;

    Ok(SemanticRuntime {
        tokenizer,
        session,
        output_name,
    })
}

fn load_semantic_runtime(app: &AppHandle) -> CommandResult<&'static Mutex<SemanticRuntime>> {
    if let Some(runtime) = SEMANTIC_RUNTIME.get() {
        return Ok(runtime);
    }

    let runtime = build_semantic_runtime(app)?;
    let _ = SEMANTIC_RUNTIME.set(Mutex::new(runtime));
    SEMANTIC_RUNTIME
        .get()
        .ok_or_else(|| "Could not initialize semantic runtime".to_string())
}

fn semantic_queue() -> &'static Mutex<SemanticRebuildQueue> {
    SEMANTIC_REBUILD_QUEUE.get_or_init(|| Mutex::new(SemanticRebuildQueue::default()))
}

fn spawn_semantic_rebuild_worker(app: AppHandle) {
    crate::async_runtime::spawn_blocking(move || {
        crate::async_runtime::block_on(run_semantic_rebuild_worker(app))
    });
}

fn semantic_root_state(
    connection: &rusqlite::Connection,
    root_id: i64,
) -> CommandResult<Option<SemanticRootIndexState>> {
    connection
        .query_row(
            "
            SELECT root_id, last_indexed_ms, item_count, embedding_dim, updated_at_ms
            FROM semantic_root_state
            WHERE root_id = ?1
            ",
            params![root_id],
            |row| {
                Ok(SemanticRootIndexState {
                    root_id: row.get::<_, i64>(0)?,
                    last_indexed_ms: row.get::<_, i64>(1)?,
                    item_count: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                    embedding_dim: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    updated_at_ms: row.get::<_, i64>(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Could not query semantic root state: {error}"))
}

fn write_semantic_root_state(
    connection: &rusqlite::Connection,
    state: &SemanticRootIndexState,
) -> CommandResult<()> {
    connection
        .execute(
            "
            INSERT INTO semantic_root_state(root_id, last_indexed_ms, item_count, embedding_dim, updated_at_ms)
            VALUES(?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(root_id)
            DO UPDATE SET
              last_indexed_ms = excluded.last_indexed_ms,
              item_count = excluded.item_count,
              embedding_dim = excluded.embedding_dim,
              updated_at_ms = excluded.updated_at_ms
            ",
            params![
                state.root_id,
                state.last_indexed_ms,
                i64::try_from(state.item_count).unwrap_or(0),
                i64::try_from(state.embedding_dim).unwrap_or(0),
                state.updated_at_ms,
            ],
        )
        .map_err(|error| format!("Could not write semantic root state: {error}"))?;
    Ok(())
}

fn stale_semantic_root_ids(
    connection: &rusqlite::Connection,
    requested_root_id: Option<i64>,
    force: bool,
) -> CommandResult<Vec<i64>> {
    let sql = if requested_root_id.is_some() {
        "
        SELECT r.id
        FROM roots r
        LEFT JOIN semantic_root_state s ON s.root_id = r.id
        WHERE r.id = ?1
        "
    } else {
        "
        SELECT r.id
        FROM roots r
        LEFT JOIN semantic_root_state s ON s.root_id = r.id
        "
    };

    let mut output = Vec::new();
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| format!("Could not prepare semantic stale roots query: {error}"))?;
    if let Some(root_id) = requested_root_id {
        let rows = statement
            .query_map(params![root_id], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("Could not query semantic stale roots: {error}"))?;
        for row in rows {
            let root_id =
                row.map_err(|error| format!("Could not parse semantic stale root row: {error}"))?;
            let root_last_indexed_ms = connection
                .query_row(
                    "SELECT last_indexed_ms FROM roots WHERE id = ?1",
                    params![root_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| {
                    format!("Could not read root timestamp for semantic rebuild: {error}")
                })?;
            let current_state = semantic_root_state(connection, root_id)?;
            let is_stale = force
                || current_state
                    .map(|state| state.last_indexed_ms < root_last_indexed_ms)
                    .unwrap_or(true);
            if is_stale {
                output.push(root_id);
            }
        }
    } else {
        let rows = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("Could not query semantic stale roots: {error}"))?;
        for row in rows {
            let root_id =
                row.map_err(|error| format!("Could not parse semantic stale root row: {error}"))?;
            let root_last_indexed_ms = connection
                .query_row(
                    "SELECT last_indexed_ms FROM roots WHERE id = ?1",
                    params![root_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| {
                    format!("Could not read root timestamp for semantic rebuild: {error}")
                })?;
            let current_state = semantic_root_state(connection, root_id)?;
            let is_stale = force
                || current_state
                    .map(|state| state.last_indexed_ms < root_last_indexed_ms)
                    .unwrap_or(true);
            if is_stale {
                output.push(root_id);
            }
        }
    }

    Ok(output)
}

fn semantic_embedding_text(text: &str) -> String {
    let mut value = text.trim().to_string();
    if value.chars().count() > 720 {
        value = value.chars().take(720).collect();
    }
    value
}

fn load_semantic_candidates_for_root(
    connection: &rusqlite::Connection,
    root_id: i64,
    max_documents: usize,
) -> CommandResult<Vec<SemanticCandidate>> {
    if max_documents == 0 {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut semantic_id = 1_i64;
    let max_documents_i64 = i64::try_from(max_documents).unwrap_or(i64::MAX);

    {
        let mut statement = connection
            .prepare(
                "
                SELECT
                  c.id,
                  c.file_id,
                  f.relative_path,
                  f.absolute_path,
                  c.heading_level,
                  c.heading_text,
                  c.heading_order,
                  c.author_text,
                  c.chunk_text
                FROM chunks c
                JOIN files f ON f.id = c.file_id
                WHERE c.root_id = ?1
                ORDER BY c.file_id ASC, c.chunk_order ASC
                LIMIT ?2
                ",
            )
            .map_err(|error| {
                format!("Could not prepare semantic chunk candidates query: {error}")
            })?;

        let rows = statement
            .query_map(params![root_id, max_documents_i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|error| format!("Could not run semantic chunk candidates query: {error}"))?;

        for row in rows {
            if candidates.len() >= max_documents {
                break;
            }
            let (
                chunk_row_id,
                file_id,
                relative_path,
                _absolute_path,
                heading_level,
                heading_text,
                heading_order,
                author_text,
                chunk_text,
            ) =
                row.map_err(|error| format!("Could not parse semantic chunk candidate: {error}"))?;

            let trimmed_chunk = chunk_text.trim();
            if trimmed_chunk.is_empty() {
                continue;
            }

            let file_name = file_name_from_relative(&relative_path);
            let semantic_text = semantic_embedding_text(&format!(
                "heading: {}\nauthor: {}\nchunk: {}\npath: {}\nfile: {}",
                heading_text.clone().unwrap_or_default(),
                author_text.clone().unwrap_or_default(),
                trimmed_chunk,
                relative_path,
                file_name
            ));
            let kind = if author_text.is_some() {
                "author".to_string()
            } else if heading_text.is_some() {
                "heading".to_string()
            } else {
                "file".to_string()
            };
            candidates.push(SemanticCandidate {
                semantic_key: format!("chunk:{chunk_row_id}"),
                semantic_id,
                root_id,
                kind,
                file_id,
                source_row_id: chunk_row_id,
                heading_level,
                heading_order,
                semantic_text,
            });
            semantic_id += 1;
        }
    }

    if !candidates.is_empty() {
        return Ok(candidates);
    }

    // Fallback for roots indexed before chunk rows were written.
    let mut statement = connection
        .prepare(
            "
            SELECT id, relative_path, absolute_path
            FROM files
            WHERE root_id = ?1
            ORDER BY modified_ms DESC, id DESC
            LIMIT ?2
            ",
        )
        .map_err(|error| format!("Could not prepare semantic fallback file query: {error}"))?;
    let rows = statement
        .query_map(params![root_id, max_documents_i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Could not run semantic fallback file query: {error}"))?;
    for row in rows {
        if candidates.len() >= max_documents {
            break;
        }
        let (file_id, relative_path, _absolute_path) =
            row.map_err(|error| format!("Could not parse semantic fallback file row: {error}"))?;
        let file_name = file_name_from_relative(&relative_path);
        let semantic_text =
            semantic_embedding_text(&format!("file: {}\npath: {}", file_name, relative_path));
        candidates.push(SemanticCandidate {
            semantic_key: format!("file:{file_id}"),
            semantic_id,
            root_id,
            kind: "file".to_string(),
            file_id,
            source_row_id: file_id,
            heading_level: None,
            heading_order: None,
            semantic_text,
        });
        semantic_id += 1;
    }

    Ok(candidates)
}

fn normalize_vector_l2(values: &mut [f32]) {
    let norm = values
        .iter()
        .fold(0.0_f32, |acc, value| acc + (value * value))
        .sqrt();
    if norm <= 0.0 {
        return;
    }
    for value in values {
        *value /= norm;
    }
}

fn encode_semantic_batch(
    tokenizer: &Tokenizer,
    texts: &[String],
    max_tokens: usize,
) -> CommandResult<(Vec<i64>, Vec<i64>, Vec<i64>, usize, usize)> {
    let batch_size = texts.len();
    if batch_size == 0 {
        return Ok((Vec::new(), Vec::new(), Vec::new(), 0, 0));
    }
    let seq_len = max_tokens.max(8);
    let mut input_ids = Vec::with_capacity(batch_size.saturating_mul(seq_len));
    let mut attention_mask = Vec::with_capacity(batch_size.saturating_mul(seq_len));
    let mut token_type_ids = Vec::with_capacity(batch_size.saturating_mul(seq_len));

    for text in texts {
        let encoding = tokenizer
            .encode(text.as_str(), true)
            .map_err(|error| format!("Could not tokenize semantic input: {error}"))?;
        let mut ids = encoding
            .get_ids()
            .iter()
            .take(seq_len)
            .map(|value| i64::from(*value))
            .collect::<Vec<i64>>();
        let mut mask = encoding
            .get_attention_mask()
            .iter()
            .take(seq_len)
            .map(|value| i64::from(*value))
            .collect::<Vec<i64>>();
        let mut type_ids = encoding
            .get_type_ids()
            .iter()
            .take(seq_len)
            .map(|value| i64::from(*value))
            .collect::<Vec<i64>>();

        if ids.is_empty() {
            ids.push(101);
            mask.push(1);
            type_ids.push(0);
        }
        while mask.len() < ids.len() {
            mask.push(1);
        }
        while type_ids.len() < ids.len() {
            type_ids.push(0);
        }

        ids.resize(seq_len, 0);
        mask.resize(seq_len, 0);
        type_ids.resize(seq_len, 0);

        input_ids.extend(ids);
        attention_mask.extend(mask);
        token_type_ids.extend(type_ids);
    }

    Ok((
        input_ids,
        attention_mask,
        token_type_ids,
        batch_size,
        seq_len,
    ))
}

pub(crate) fn embed_semantic_texts(
    app: &AppHandle,
    texts: &[String],
) -> CommandResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let runtime = load_semantic_runtime(app)?;
    let mut runtime = runtime
        .lock()
        .map_err(|_| "Could not lock semantic runtime".to_string())?;
    let output_name = runtime.output_name.clone();
    let expects_token_type_ids = runtime
        .session
        .inputs()
        .iter()
        .any(|entry| entry.name() == "token_type_ids");

    let (input_ids, attention_mask, token_type_ids, batch_size, seq_len) =
        encode_semantic_batch(&runtime.tokenizer, texts, SEMANTIC_MAX_TOKENS)?;
    if batch_size == 0 || seq_len == 0 {
        return Ok(Vec::new());
    }

    let shape = vec![
        i64::try_from(batch_size).unwrap_or(0),
        i64::try_from(seq_len).unwrap_or(0),
    ];
    let primary_input_ids = OrtTensor::from_array((shape.clone(), input_ids.clone()))
        .map_err(|error| format!("Could not create semantic input_ids tensor: {error}"))?;
    let primary_attention_mask = OrtTensor::from_array((shape.clone(), attention_mask.clone()))
        .map_err(|error| format!("Could not create semantic attention_mask tensor: {error}"))?;
    let outputs = if expects_token_type_ids {
        let primary_token_type_ids = OrtTensor::from_array((shape.clone(), token_type_ids.clone()))
            .map_err(|error| format!("Could not create semantic token_type_ids tensor: {error}"))?;
        runtime.session.run(ort::inputs! {
            "input_ids" => primary_input_ids,
            "attention_mask" => primary_attention_mask,
            "token_type_ids" => primary_token_type_ids
        })
    } else {
        runtime.session.run(ort::inputs! {
            "input_ids" => primary_input_ids,
            "attention_mask" => primary_attention_mask
        })
    }
    .map_err(|error| format!("Semantic model inference failed: {error}"))?;

    let output = if outputs.contains_key(output_name.as_str()) {
        &outputs[output_name.as_str()]
    } else {
        &outputs[0]
    };
    let output = output
        .try_extract_array::<f32>()
        .map_err(|error| format!("Could not extract semantic output tensor: {error}"))?;

    if output.ndim() != 3 {
        return Err(format!(
            "Semantic model output rank {} is unsupported (expected 3)",
            output.ndim()
        ));
    }
    let output_shape = output.shape();
    let output_batch = output_shape[0];
    let output_seq = output_shape[1];
    let embedding_dim = output_shape[2];
    if output_batch != batch_size || embedding_dim == 0 {
        return Err("Semantic model output shape does not match request".to_string());
    }

    let mut vectors = Vec::with_capacity(batch_size);
    for batch_index in 0..batch_size {
        let mut pooled = vec![0.0_f32; embedding_dim];
        let mut token_count = 0.0_f32;
        let max_steps = output_seq.min(seq_len);

        for token_index in 0..max_steps {
            if attention_mask[batch_index * seq_len + token_index] == 0 {
                continue;
            }
            token_count += 1.0;
            for dim in 0..embedding_dim {
                pooled[dim] += output[[batch_index, token_index, dim]];
            }
        }

        if token_count <= 0.0 {
            for dim in 0..embedding_dim {
                pooled[dim] = output[[batch_index, 0, dim]];
            }
        } else {
            for value in &mut pooled {
                *value /= token_count;
            }
        }
        normalize_vector_l2(&mut pooled);
        vectors.push(pooled);
    }

    Ok(vectors)
}

fn semantic_schema(embedding_dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("semantic_key", DataType::Utf8, false),
        Field::new("semantic_id", DataType::Int64, false),
        Field::new("root_id", DataType::Int64, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("file_id", DataType::Int64, false),
        Field::new("source_row_id", DataType::Int64, false),
        Field::new("heading_level", DataType::Int64, true),
        Field::new("heading_order", DataType::Int64, true),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                i32::try_from(embedding_dim).unwrap_or(0),
            ),
            false,
        ),
    ]))
}

fn semantic_record_batch(
    schema: Arc<Schema>,
    candidates: &[SemanticCandidate],
    embeddings: &[Vec<f32>],
    embedding_dim: usize,
) -> CommandResult<RecordBatch> {
    if candidates.len() != embeddings.len() {
        return Err("Semantic candidate/embedding batch size mismatch".to_string());
    }
    let semantic_keys = StringArray::from_iter_values(
        candidates
            .iter()
            .map(|candidate| candidate.semantic_key.as_str()),
    );
    let semantic_ids =
        Int64Array::from_iter_values(candidates.iter().map(|candidate| candidate.semantic_id));
    let root_ids =
        Int64Array::from_iter_values(candidates.iter().map(|candidate| candidate.root_id));
    let kinds =
        StringArray::from_iter_values(candidates.iter().map(|candidate| candidate.kind.as_str()));
    let file_ids =
        Int64Array::from_iter_values(candidates.iter().map(|candidate| candidate.file_id));
    let source_row_ids =
        Int64Array::from_iter_values(candidates.iter().map(|candidate| candidate.source_row_id));
    let heading_levels = Int64Array::from(
        candidates
            .iter()
            .map(|candidate| candidate.heading_level)
            .collect::<Vec<_>>(),
    );
    let heading_orders = Int64Array::from(
        candidates
            .iter()
            .map(|candidate| candidate.heading_order)
            .collect::<Vec<_>>(),
    );

    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        embeddings.iter().map(|embedding| {
            let mut row = embedding
                .iter()
                .take(embedding_dim)
                .map(|value| Some(*value))
                .collect::<Vec<Option<f32>>>();
            row.resize(embedding_dim, Some(0.0));
            Some(row)
        }),
        i32::try_from(embedding_dim).unwrap_or(0),
    );

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(semantic_keys),
            Arc::new(semantic_ids),
            Arc::new(root_ids),
            Arc::new(kinds),
            Arc::new(file_ids),
            Arc::new(source_row_ids),
            Arc::new(heading_levels),
            Arc::new(heading_orders),
            Arc::new(vectors),
        ],
    )
    .map_err(|error| format!("Could not build semantic record batch: {error}"))
}

fn semantic_in_clause(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<&str>>().join(",")
}

fn semantic_preview_text(chunk_text: &str) -> String {
    let trimmed = chunk_text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.chars().take(240).collect::<String>()
}

async fn open_semantic_table(app: &AppHandle) -> CommandResult<Option<LanceTable>> {
    let semantic_dir = semantic_db_dir(app)?;
    if !semantic_dir.exists() {
        return Ok(None);
    }
    let uri = path_display(&semantic_dir);
    let db = connect_lancedb(&uri)
        .execute()
        .await
        .map_err(|error| format!("Could not open semantic LanceDB at '{}': {error}", uri))?;
    match db.open_table(SEMANTIC_TABLE_NAME).execute().await {
        Ok(table) => Ok(Some(table)),
        Err(_) => Ok(None),
    }
}

async fn rebuild_semantic_root(app: AppHandle, root_id: i64, force: bool) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }

    let connection = open_database(&app)?;
    let root_last_indexed_ms = connection
        .query_row(
            "SELECT last_indexed_ms FROM roots WHERE id = ?1",
            params![root_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("Could not read root timestamp for semantic rebuild: {error}"))?;

    let Some(root_last_indexed_ms) = root_last_indexed_ms else {
        if let Some(table) = open_semantic_table(&app).await? {
            let _ = table.delete(&format!("root_id = {root_id}")).await;
        }
        return Ok(());
    };

    if !force {
        if let Some(state) = semantic_root_state(&connection, root_id)? {
            if state.last_indexed_ms >= root_last_indexed_ms {
                return Ok(());
            }
        }
    }

    let candidates =
        load_semantic_candidates_for_root(&connection, root_id, SEMANTIC_MAX_DOCUMENTS)?;
    let semantic_dir = semantic_db_dir(&app)?;
    fs::create_dir_all(&semantic_dir).map_err(|error| {
        format!(
            "Could not create semantic DB directory '{}': {error}",
            path_display(&semantic_dir)
        )
    })?;
    let uri = path_display(&semantic_dir);
    let db = connect_lancedb(&uri)
        .execute()
        .await
        .map_err(|error| format!("Could not open LanceDB at '{}': {error}", uri))?;

    if candidates.is_empty() {
        if let Ok(table) = db.open_table(SEMANTIC_TABLE_NAME).execute().await {
            let _ = table.delete(&format!("root_id = {root_id}")).await;
        }
        write_semantic_root_state(
            &connection,
            &SemanticRootIndexState {
                root_id,
                last_indexed_ms: root_last_indexed_ms,
                item_count: 0,
                embedding_dim: 0,
                updated_at_ms: now_ms(),
            },
        )?;
        return Ok(());
    }

    let mut schema: Option<Arc<Schema>> = None;
    let mut batches = Vec::new();
    let mut embedding_dim = 0_usize;

    for chunk in candidates.chunks(SEMANTIC_EMBED_BATCH) {
        let texts = chunk
            .iter()
            .map(|candidate| candidate.semantic_text.clone())
            .collect::<Vec<String>>();
        let app_for_embedding = app.clone();
        let embeddings = crate::async_runtime::spawn_blocking(move || {
            embed_semantic_texts(&app_for_embedding, &texts)
        })
        .await
        .map_err(|error| format!("Semantic embedding task failed: {error}"))??;
        if embeddings.is_empty() {
            continue;
        }
        let current_dim = embeddings[0].len();
        if current_dim == 0 {
            continue;
        }
        if embedding_dim == 0 {
            embedding_dim = current_dim;
            schema = Some(semantic_schema(embedding_dim));
        }
        if current_dim != embedding_dim {
            continue;
        }
        let batch = semantic_record_batch(
            schema
                .clone()
                .ok_or_else(|| "Semantic schema was not initialized".to_string())?,
            chunk,
            &embeddings,
            embedding_dim,
        )?;
        batches.push(batch);
    }

    if batches.is_empty() || embedding_dim == 0 {
        return Ok(());
    }

    let schema = schema.ok_or_else(|| "Semantic schema was not created".to_string())?;
    let reader = RecordBatchIterator::new(batches.into_iter().map(Ok), schema.clone());

    let table = match db.open_table(SEMANTIC_TABLE_NAME).execute().await {
        Ok(table) => {
            table
                .delete(&format!("root_id = {root_id}"))
                .await
                .map_err(|error| format!("Could not delete stale semantic rows: {error}"))?;
            table
                .add(Box::new(reader))
                .execute()
                .await
                .map_err(|error| format!("Could not append semantic rows: {error}"))?;
            table
        }
        Err(_) => db
            .create_table(SEMANTIC_TABLE_NAME, Box::new(reader))
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await
            .map_err(|error| format!("Could not create semantic LanceDB table: {error}"))?,
    };

    let _ = table
        .create_index(
            &["root_id"],
            LanceIndex::BTree(BTreeIndexBuilder::default()),
        )
        .execute()
        .await;
    if candidates.len() >= 4_096 {
        let _ = table
            .create_index(&["vector"], LanceIndex::Auto)
            .execute()
            .await;
    }

    write_semantic_root_state(
        &connection,
        &SemanticRootIndexState {
            root_id,
            last_indexed_ms: root_last_indexed_ms,
            item_count: candidates.len(),
            embedding_dim,
            updated_at_ms: now_ms(),
        },
    )?;
    Ok(())
}

async fn run_semantic_rebuild_worker(app: AppHandle) {
    loop {
        let next_root = {
            let mut queue = match semantic_queue().lock() {
                Ok(queue) => queue,
                Err(_) => {
                    SEMANTIC_REBUILD_IN_FLIGHT.store(false, AtomicOrdering::SeqCst);
                    return;
                }
            };
            let next = queue.pending.pop_front();
            if let Some(root_id) = next {
                queue.seen.remove(&root_id);
                let force = queue.forced.remove(&root_id);
                Some((root_id, force))
            } else {
                None
            }
        };

        let Some((root_id, force)) = next_root else {
            break;
        };

        if let Err(error) = rebuild_semantic_root(app.clone(), root_id, force).await {
            eprintln!("Semantic root rebuild failed for root_id={root_id}: {error}");
        }
    }

    SEMANTIC_REBUILD_IN_FLIGHT.store(false, AtomicOrdering::SeqCst);
    let has_pending = semantic_queue()
        .lock()
        .map(|queue| !queue.pending.is_empty())
        .unwrap_or(false);
    if has_pending
        && SEMANTIC_REBUILD_IN_FLIGHT
            .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
            .is_ok()
    {
        spawn_semantic_rebuild_worker(app);
    }
}

pub(crate) fn trigger_semantic_rebuild(
    app: AppHandle,
    requested_root_id: Option<i64>,
    force: bool,
) {
    let connection = match open_database(&app) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("Could not open database for semantic rebuild scheduling: {error}");
            return;
        }
    };

    let stale_roots = match stale_semantic_root_ids(&connection, requested_root_id, force) {
        Ok(root_ids) => root_ids,
        Err(error) => {
            eprintln!("Could not compute stale semantic roots: {error}");
            return;
        }
    };

    if stale_roots.is_empty() {
        return;
    }

    if let Ok(mut queue) = semantic_queue().lock() {
        for root_id in stale_roots {
            if force {
                queue.forced.insert(root_id);
            }
            if queue.seen.insert(root_id) {
                queue.pending.push_back(root_id);
            }
        }
    }

    if SEMANTIC_REBUILD_IN_FLIGHT
        .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
        .is_err()
    {
        return;
    }
    spawn_semantic_rebuild_worker(app);
}

pub(crate) async fn purge_semantic_root(app: &AppHandle, root_id: i64) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }
    if let Some(table) = open_semantic_table(app).await? {
        table
            .delete(&format!("root_id = {root_id}"))
            .await
            .map_err(|error| format!("Could not delete semantic rows for root: {error}"))?;
    }
    let connection = open_database(app)?;
    connection
        .execute(
            "DELETE FROM semantic_root_state WHERE root_id = ?1",
            params![root_id],
        )
        .map_err(|error| format!("Could not delete semantic root state: {error}"))?;
    Ok(())
}

fn semantic_result_rows_from_batches(
    batches: &[RecordBatch],
    limit: usize,
) -> CommandResult<Vec<SemanticResultRow>> {
    let mut rows_out = Vec::new();
    let mut seen = HashSet::new();

    for batch in batches {
        let file_id_col = batch
            .column_by_name("file_id")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .ok_or_else(|| "Semantic result batch missing file_id column".to_string())?;
        let kind_col = batch
            .column_by_name("kind")
            .and_then(|column| column.as_any().downcast_ref::<StringArray>())
            .ok_or_else(|| "Semantic result batch missing kind column".to_string())?;
        let source_row_id_col = batch
            .column_by_name("source_row_id")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
            .ok_or_else(|| "Semantic result batch missing source_row_id column".to_string())?;
        let heading_level_col = batch
            .column_by_name("heading_level")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>());
        let heading_order_col = batch
            .column_by_name("heading_order")
            .and_then(|column| column.as_any().downcast_ref::<Int64Array>());
        let distance_f32 = batch
            .column_by_name("_distance")
            .and_then(|column| column.as_any().downcast_ref::<Float32Array>());

        for row_index in 0..batch.num_rows() {
            if rows_out.len() >= limit {
                return Ok(rows_out);
            }
            let file_id = file_id_col.value(row_index);
            let kind = kind_col.value(row_index).to_string();
            let source_row_id = source_row_id_col.value(row_index);
            let dedupe_key = format!("{kind}:{source_row_id}");
            if !seen.insert(dedupe_key) {
                continue;
            }
            let distance = distance_f32
                .and_then(|column| {
                    (!column.is_null(row_index)).then_some(f64::from(column.value(row_index)))
                })
                .unwrap_or(1.0);
            rows_out.push(SemanticResultRow {
                kind,
                file_id,
                source_row_id,
                heading_level: heading_level_col.and_then(|column| {
                    (!column.is_null(row_index)).then_some(column.value(row_index))
                }),
                heading_order: heading_order_col.and_then(|column| {
                    (!column.is_null(row_index)).then_some(column.value(row_index))
                }),
                score: 7000.0 + (distance * 1000.0),
            });
        }
    }

    Ok(rows_out)
}

fn hydrate_semantic_hits(
    connection: &rusqlite::Connection,
    rows: &[SemanticResultRow],
    limit: usize,
) -> CommandResult<Vec<SearchHit>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut file_ids = Vec::new();
    let mut seen_file_ids = HashSet::new();
    let mut chunk_row_ids = Vec::new();
    let mut seen_chunk_row_ids = HashSet::new();
    for row in rows {
        if seen_file_ids.insert(row.file_id) {
            file_ids.push(row.file_id);
        }
        if row.kind != "file" && seen_chunk_row_ids.insert(row.source_row_id) {
            chunk_row_ids.push(row.source_row_id);
        }
    }

    let file_sql = format!(
        "SELECT id, relative_path, absolute_path FROM files WHERE id IN ({})",
        semantic_in_clause(file_ids.len())
    );
    let mut file_statement = connection
        .prepare(&file_sql)
        .map_err(|error| format!("Could not prepare semantic file hydration query: {error}"))?;
    let file_rows = file_statement
        .query_map(
            rusqlite::params_from_iter(file_ids.iter().copied()),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| format!("Could not execute semantic file hydration query: {error}"))?;
    let mut files = std::collections::HashMap::new();
    for row in file_rows {
        let (file_id, relative_path, absolute_path) =
            row.map_err(|error| format!("Could not parse semantic file hydration row: {error}"))?;
        files.insert(
            file_id,
            (
                file_name_from_relative(&relative_path),
                relative_path,
                absolute_path,
            ),
        );
    }

    let mut chunks = std::collections::HashMap::new();
    if !chunk_row_ids.is_empty() {
        let chunk_sql = format!(
            "
            SELECT id, heading_level, heading_order, heading_text, author_text, chunk_text
            FROM chunks
            WHERE id IN ({})
            ",
            semantic_in_clause(chunk_row_ids.len())
        );
        let mut chunk_statement = connection.prepare(&chunk_sql).map_err(|error| {
            format!("Could not prepare semantic chunk hydration query: {error}")
        })?;
        let chunk_rows = chunk_statement
            .query_map(
                rusqlite::params_from_iter(chunk_row_ids.iter().copied()),
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .map_err(|error| {
                format!("Could not execute semantic chunk hydration query: {error}")
            })?;
        for row in chunk_rows {
            let (chunk_id, heading_level, heading_order, heading_text, author_text, chunk_text) =
                row.map_err(|error| {
                    format!("Could not parse semantic chunk hydration row: {error}")
                })?;
            chunks.insert(
                chunk_id,
                (
                    heading_level,
                    heading_order,
                    heading_text,
                    author_text,
                    chunk_text,
                ),
            );
        }
    }

    let mut hits = Vec::new();
    for row in rows {
        if hits.len() >= limit {
            break;
        }
        let Some((file_name, relative_path, absolute_path)) = files.get(&row.file_id) else {
            continue;
        };

        let (heading_level, heading_text, heading_order) = if row.kind == "file" {
            (None, None, None)
        } else if let Some((
            chunk_heading_level,
            chunk_heading_order,
            chunk_heading_text,
            chunk_author_text,
            chunk_text,
        )) = chunks.get(&row.source_row_id)
        {
            match row.kind.as_str() {
                "author" => (
                    None,
                    chunk_author_text.clone(),
                    row.heading_order.or(*chunk_heading_order),
                ),
                "heading" => {
                    let preview = semantic_preview_text(chunk_text);
                    (
                        row.heading_level.or(*chunk_heading_level),
                        chunk_heading_text
                            .clone()
                            .or_else(|| chunk_author_text.clone())
                            .or_else(|| (!preview.is_empty()).then_some(preview)),
                        row.heading_order.or(*chunk_heading_order),
                    )
                }
                _ => (None, None, None),
            }
        } else {
            continue;
        };

        hits.push(SearchHit {
            source: "semantic".to_string(),
            kind: row.kind.clone(),
            file_id: row.file_id,
            file_name: file_name.clone(),
            relative_path: relative_path.clone(),
            absolute_path: absolute_path.clone(),
            heading_level,
            heading_text,
            heading_order,
            score: row.score,
        });
    }

    Ok(hits)
}

pub(crate) async fn semantic_search(
    app: &AppHandle,
    query: &str,
    requested_root_id: Option<i64>,
    limit: usize,
) -> CommandResult<Vec<SearchHit>> {
    let Some(table) = open_semantic_table(app).await? else {
        return Ok(Vec::new());
    };

    let app_for_embedding = app.clone();
    let query_text = query.to_string();
    let query_embedding = crate::async_runtime::spawn_blocking(move || {
        embed_semantic_texts(&app_for_embedding, &[query_text])
    })
    .await
    .map_err(|error| format!("Semantic query embedding task failed: {error}"))??;
    if query_embedding.is_empty() || query_embedding[0].is_empty() {
        return Ok(Vec::new());
    }

    let mut vector_query = table
        .query()
        .nearest_to(query_embedding[0].as_slice())
        .map_err(|error| format!("Could not build semantic vector query: {error}"))?
        .limit(limit.saturating_mul(2))
        .select(Select::columns(&[
            "file_id",
            "kind",
            "source_row_id",
            "heading_level",
            "heading_order",
        ]))
        .nprobes(18)
        .refine_factor(2);

    if let Some(root_id) = requested_root_id {
        vector_query = vector_query.only_if(format!("root_id = {root_id}"));
    }

    let batches = vector_query
        .execute()
        .await
        .map_err(|error| format!("Semantic search execution failed: {error}"))?
        .try_collect::<Vec<RecordBatch>>()
        .await
        .map_err(|error| format!("Semantic search result stream failed: {error}"))?;

    let rows = semantic_result_rows_from_batches(&batches, limit)?;
    let connection = open_database(app)?;
    hydrate_semantic_hits(&connection, &rows, limit)
}
