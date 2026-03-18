use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::runtime::AppHandle;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, Schema, TextFieldIndexing, TextOptions, Value,
    STORED, STRING,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

use crate::db::{index_lexical_dir, open_database};
use crate::search::normalize_for_search;
use crate::types::{LexicalSearchTelemetry, SearchHit};
use crate::CommandResult;

const PREFIX_TOKENIZER: &str = "bf_prefix";
const NGRAM_TOKENIZER: &str = "bf_ngram";
const ROOT_SHARD_CACHE_CAPACITY: usize = 8;
const ROOT_SHORTLIST_LIMIT: usize = 8;
const ROOT_SHORTLIST_LIMIT_LOW_SPECIFICITY: usize = 16;
const ROOT_EXHAUSTIVE_THRESHOLD: usize = 8;
const ROOTS_DIR_NAME: &str = "roots";
const ROOT_CATALOG_DIR_NAME: &str = "root-catalog";
const META_DIR_NAME: &str = "meta";
const CHUNK_DIR_NAME: &str = "chunk";

const MIN_FETCH_MULTIPLIER: usize = 5;
const MIN_FETCH_FLOOR: usize = 80;
const MAX_FETCH_LIMIT: usize = 1_800;
const CHUNK_PREVIEW_CHARS: usize = 240;
const FULL_REBUILD_WRITER_HEAP_BYTES: usize = 256 * 1024 * 1024;
const FULL_REBUILD_WRITER_FALLBACK_HEAP_BYTES: usize = 64 * 1024 * 1024;
const INCREMENTAL_WRITER_HEAP_BYTES: usize = 64 * 1024 * 1024;
const INCREMENTAL_WRITER_FALLBACK_HEAP_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct LexicalDocument {
    pub root_id: i64,
    pub file_id: i64,
    pub source_row_id: i64,
    pub kind: String,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub heading_level: Option<i64>,
    pub heading_text: Option<String>,
    pub heading_order: Option<i64>,
    pub author_text: Option<String>,
    pub chunk_order: Option<i64>,
    pub chunk_text: Option<String>,
}

#[derive(Clone)]
struct MetaFields {
    kind: Field,
    root_id: Field,
    file_id: Field,
    source_row_id: Field,
    heading_level: Field,
    heading_order: Field,
    query_text: Field,
    prefix_text: Field,
}

#[derive(Clone)]
struct ChunkFields {
    root_id: Field,
    file_id: Field,
    source_row_id: Field,
    chunk_order: Field,
    heading_level: Field,
    heading_order: Field,
    query_text: Field,
    ngram_text: Field,
}

struct RootShardRuntime {
    meta_index: Index,
    meta_reader: IndexReader,
    meta_fields: MetaFields,
    chunk_index: Index,
    chunk_reader: IndexReader,
    chunk_fields: ChunkFields,
    rebuild_lock: Mutex<()>,
}

#[derive(Clone)]
struct RootCatalogFields {
    root_id: Field,
    root_path: Field,
    folder_name: Field,
    query_text: Field,
    prefix_text: Field,
}

struct RootCatalogRuntime {
    index: Index,
    reader: IndexReader,
    fields: RootCatalogFields,
    rebuild_lock: Mutex<()>,
}

struct LexicalRuntime {
    roots: HashMap<i64, Arc<RootShardRuntime>>,
    lru: VecDeque<i64>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum CandidateKind {
    File,
    Heading,
    Author,
    Chunk,
}

#[derive(Clone, Copy)]
struct CandidateHit {
    kind: CandidateKind,
    file_id: i64,
    source_row_id: i64,
    heading_level: Option<i64>,
    heading_order: Option<i64>,
    chunk_order: Option<i64>,
    score: f64,
}

#[derive(Clone)]
struct FileHydration {
    file_name: String,
    relative_path: String,
    absolute_path: String,
    file_name_normalized: String,
    relative_path_normalized: String,
}

#[derive(Clone)]
struct HeadingHydration {
    heading_level: i64,
    heading_text: String,
}

#[derive(Clone)]
struct AuthorHydration {
    author_text: String,
}

#[derive(Clone)]
struct ChunkHydration {
    heading_level: Option<i64>,
    heading_order: Option<i64>,
    heading_text: Option<String>,
    author_text: Option<String>,
    chunk_text: String,
}

#[derive(Clone, Copy)]
enum WriterMode {
    FullRebuild,
    Incremental,
}

static LEXICAL_RUNTIME: OnceLock<Mutex<LexicalRuntime>> = OnceLock::new();
static ROOT_CATALOG_RUNTIME: OnceLock<Mutex<Option<Arc<RootCatalogRuntime>>>> = OnceLock::new();

fn indexed_text_options(tokenizer: &str) -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

fn build_meta_schema() -> Schema {
    let mut builder = Schema::builder();
    let numeric = NumericOptions::default()
        .set_fast()
        .set_stored()
        .set_indexed();

    builder.add_text_field("kind", STRING | STORED);
    builder.add_u64_field("root_id", numeric.clone());
    builder.add_u64_field("file_id", numeric.clone());
    builder.add_u64_field("source_row_id", numeric.clone());
    builder.add_i64_field("heading_level", numeric.clone());
    builder.add_i64_field("heading_order", numeric.clone());
    builder.add_text_field("query_text", indexed_text_options("default"));
    builder.add_text_field("prefix_text", indexed_text_options(PREFIX_TOKENIZER));

    builder.build()
}

fn build_chunk_schema() -> Schema {
    let mut builder = Schema::builder();
    let numeric = NumericOptions::default()
        .set_fast()
        .set_stored()
        .set_indexed();

    builder.add_u64_field("root_id", numeric.clone());
    builder.add_u64_field("file_id", numeric.clone());
    builder.add_u64_field("source_row_id", numeric.clone());
    builder.add_i64_field("chunk_order", numeric.clone());
    builder.add_i64_field("heading_level", numeric.clone());
    builder.add_i64_field("heading_order", numeric);
    builder.add_text_field("query_text", indexed_text_options("default"));
    builder.add_text_field("ngram_text", indexed_text_options(NGRAM_TOKENIZER));

    builder.build()
}

fn build_root_catalog_schema() -> Schema {
    let mut builder = Schema::builder();
    let numeric = NumericOptions::default()
        .set_fast()
        .set_stored()
        .set_indexed();

    builder.add_u64_field("root_id", numeric);
    builder.add_text_field("root_path", STORED);
    builder.add_text_field("folder_name", STORED);
    builder.add_text_field("query_text", indexed_text_options("default"));
    builder.add_text_field("prefix_text", indexed_text_options(PREFIX_TOKENIZER));

    builder.build()
}

fn has_required_meta_fields(schema: &Schema) -> bool {
    schema.get_field("kind").is_ok()
        && schema.get_field("root_id").is_ok()
        && schema.get_field("file_id").is_ok()
        && schema.get_field("source_row_id").is_ok()
        && schema.get_field("heading_level").is_ok()
        && schema.get_field("heading_order").is_ok()
        && schema.get_field("query_text").is_ok()
        && schema.get_field("prefix_text").is_ok()
}

fn has_required_chunk_fields(schema: &Schema) -> bool {
    schema.get_field("root_id").is_ok()
        && schema.get_field("file_id").is_ok()
        && schema.get_field("source_row_id").is_ok()
        && schema.get_field("chunk_order").is_ok()
        && schema.get_field("heading_level").is_ok()
        && schema.get_field("heading_order").is_ok()
        && schema.get_field("query_text").is_ok()
        && schema.get_field("ngram_text").is_ok()
}

fn has_required_root_catalog_fields(schema: &Schema) -> bool {
    schema.get_field("root_id").is_ok()
        && schema.get_field("root_path").is_ok()
        && schema.get_field("folder_name").is_ok()
        && schema.get_field("query_text").is_ok()
        && schema.get_field("prefix_text").is_ok()
}

fn register_tokenizers(index: &Index) -> CommandResult<()> {
    let prefix_tokenizer = NgramTokenizer::new(2, 18, true)
        .map_err(|error| format!("Could not build lexical prefix tokenizer: {error}"))?;
    let ngram_tokenizer = NgramTokenizer::new(3, 4, false)
        .map_err(|error| format!("Could not build lexical ngram tokenizer: {error}"))?;

    index.tokenizers().register(
        PREFIX_TOKENIZER,
        TextAnalyzer::builder(prefix_tokenizer)
            .filter(LowerCaser)
            .build(),
    );
    index.tokenizers().register(
        NGRAM_TOKENIZER,
        TextAnalyzer::builder(ngram_tokenizer)
            .filter(LowerCaser)
            .build(),
    );
    Ok(())
}

fn field(schema: &Schema, name: &str) -> CommandResult<Field> {
    schema
        .get_field(name)
        .map_err(|error| format!("Missing lexical schema field '{name}': {error}"))
}

fn meta_fields(schema: &Schema) -> CommandResult<MetaFields> {
    Ok(MetaFields {
        kind: field(schema, "kind")?,
        root_id: field(schema, "root_id")?,
        file_id: field(schema, "file_id")?,
        source_row_id: field(schema, "source_row_id")?,
        heading_level: field(schema, "heading_level")?,
        heading_order: field(schema, "heading_order")?,
        query_text: field(schema, "query_text")?,
        prefix_text: field(schema, "prefix_text")?,
    })
}

fn chunk_fields(schema: &Schema) -> CommandResult<ChunkFields> {
    Ok(ChunkFields {
        root_id: field(schema, "root_id")?,
        file_id: field(schema, "file_id")?,
        source_row_id: field(schema, "source_row_id")?,
        chunk_order: field(schema, "chunk_order")?,
        heading_level: field(schema, "heading_level")?,
        heading_order: field(schema, "heading_order")?,
        query_text: field(schema, "query_text")?,
        ngram_text: field(schema, "ngram_text")?,
    })
}

fn root_catalog_fields(schema: &Schema) -> CommandResult<RootCatalogFields> {
    Ok(RootCatalogFields {
        root_id: field(schema, "root_id")?,
        root_path: field(schema, "root_path")?,
        folder_name: field(schema, "folder_name")?,
        query_text: field(schema, "query_text")?,
        prefix_text: field(schema, "prefix_text")?,
    })
}

fn open_or_create_index(
    path: &PathBuf,
    schema: &Schema,
    is_compatible: fn(&Schema) -> bool,
    label: &str,
) -> CommandResult<Index> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "Could not create lexical {label} index directory '{}': {error}",
            path.display()
        )
    })?;

    let recreate = match Index::open_in_dir(path) {
        Ok(index) => !is_compatible(&index.schema()),
        Err(_) => true,
    };

    if recreate {
        let _ = fs::remove_dir_all(path);
        fs::create_dir_all(path).map_err(|error| {
            format!(
                "Could not reset lexical {label} index directory '{}': {error}",
                path.display()
            )
        })?;
        return Index::create_in_dir(path, schema.clone())
            .map_err(|error| format!("Could not create lexical {label} index: {error}"));
    }

    Index::open_in_dir(path)
        .map_err(|error| format!("Could not open lexical {label} index: {error}"))
}

fn roots_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_lexical_dir(app)?.join(ROOTS_DIR_NAME))
}

fn root_catalog_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_lexical_dir(app)?.join(ROOT_CATALOG_DIR_NAME))
}

fn root_dir(app: &AppHandle, root_id: i64) -> CommandResult<PathBuf> {
    Ok(roots_dir(app)?.join(root_id.to_string()))
}

fn meta_dir(app: &AppHandle, root_id: i64) -> CommandResult<PathBuf> {
    Ok(root_dir(app, root_id)?.join(META_DIR_NAME))
}

fn chunk_dir(app: &AppHandle, root_id: i64) -> CommandResult<PathBuf> {
    Ok(root_dir(app, root_id)?.join(CHUNK_DIR_NAME))
}

fn init_root_runtime(app: &AppHandle, root_id: i64) -> CommandResult<RootShardRuntime> {
    let meta_schema = build_meta_schema();
    let chunk_schema = build_chunk_schema();

    let meta_index = open_or_create_index(
        &meta_dir(app, root_id)?,
        &meta_schema,
        has_required_meta_fields,
        "meta",
    )?;
    register_tokenizers(&meta_index)?;
    let meta_fields = meta_fields(&meta_index.schema())?;
    let meta_reader = meta_index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|error| format!("Could not build lexical meta reader: {error}"))?;

    let chunk_index = open_or_create_index(
        &chunk_dir(app, root_id)?,
        &chunk_schema,
        has_required_chunk_fields,
        "chunk",
    )?;
    register_tokenizers(&chunk_index)?;
    let chunk_fields = chunk_fields(&chunk_index.schema())?;
    let chunk_reader = chunk_index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|error| format!("Could not build lexical chunk reader: {error}"))?;

    Ok(RootShardRuntime {
        meta_index,
        meta_reader,
        meta_fields,
        chunk_index,
        chunk_reader,
        chunk_fields,
        rebuild_lock: Mutex::new(()),
    })
}

fn init_root_catalog_runtime(app: &AppHandle) -> CommandResult<RootCatalogRuntime> {
    let schema = build_root_catalog_schema();
    let index = open_or_create_index(
        &root_catalog_dir(app)?,
        &schema,
        has_required_root_catalog_fields,
        "root catalog",
    )?;
    register_tokenizers(&index)?;
    let fields = root_catalog_fields(&index.schema())?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|error| format!("Could not build lexical root catalog reader: {error}"))?;

    Ok(RootCatalogRuntime {
        index,
        reader,
        fields,
        rebuild_lock: Mutex::new(()),
    })
}

fn lexical_runtime() -> &'static Mutex<LexicalRuntime> {
    LEXICAL_RUNTIME.get_or_init(|| {
        Mutex::new(LexicalRuntime {
            roots: HashMap::new(),
            lru: VecDeque::new(),
        })
    })
}

fn root_catalog_runtime(app: &AppHandle) -> CommandResult<Arc<RootCatalogRuntime>> {
    let runtime_mutex = ROOT_CATALOG_RUNTIME.get_or_init(|| Mutex::new(None));
    let mut runtime = runtime_mutex
        .lock()
        .map_err(|_| "Could not lock lexical root catalog runtime".to_string())?;

    if runtime.is_none() {
        *runtime = Some(Arc::new(init_root_catalog_runtime(app)?));
    }

    runtime
        .as_ref()
        .cloned()
        .ok_or_else(|| "Could not load lexical root catalog runtime".to_string())
}

pub(crate) fn drop_root_runtime(root_id: i64) {
    if root_id <= 0 {
        return;
    }

    if let Ok(mut runtime) = lexical_runtime().lock() {
        runtime.roots.remove(&root_id);
        runtime.lru.retain(|entry| *entry != root_id);
    }
}

fn touch_lru(runtime: &mut LexicalRuntime, root_id: i64) {
    runtime.lru.retain(|entry| *entry != root_id);
    runtime.lru.push_back(root_id);
}

fn enforce_lru_capacity(runtime: &mut LexicalRuntime, current_root_id: i64) {
    while runtime.roots.len() > ROOT_SHARD_CACHE_CAPACITY {
        let Some(evict_id) = runtime.lru.pop_front() else {
            break;
        };
        if evict_id == current_root_id {
            runtime.lru.push_back(evict_id);
            break;
        }
        runtime.roots.remove(&evict_id);
    }
}

fn root_runtime(app: &AppHandle, root_id: i64) -> CommandResult<Arc<RootShardRuntime>> {
    let runtime_mutex = lexical_runtime();
    let mut runtime = runtime_mutex
        .lock()
        .map_err(|_| "Could not lock lexical runtime".to_string())?;

    if !runtime.roots.contains_key(&root_id) {
        runtime
            .roots
            .insert(root_id, Arc::new(init_root_runtime(app, root_id)?));
    }

    touch_lru(&mut runtime, root_id);
    enforce_lru_capacity(&mut runtime, root_id);

    runtime
        .roots
        .get(&root_id)
        .cloned()
        .ok_or_else(|| format!("Could not load lexical runtime for root_id={root_id}"))
}

fn indexed_root_doc_count_for_runtime(root_runtime: &RootShardRuntime) -> u64 {
    let meta_docs = u64::try_from(root_runtime.meta_reader.searcher().num_docs()).unwrap_or(0);
    let chunk_docs = u64::try_from(root_runtime.chunk_reader.searcher().num_docs()).unwrap_or(0);
    meta_docs.saturating_add(chunk_docs)
}

fn folder_name_from_root_path(root_path: &str) -> String {
    std::path::Path::new(root_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| root_path.to_string())
}

fn root_catalog_doc_count_for_runtime(root_catalog: &RootCatalogRuntime) -> u64 {
    u64::try_from(root_catalog.reader.searcher().num_docs()).unwrap_or(0)
}

fn root_catalog_terms(
    connection: &Connection,
    root_id: i64,
) -> CommandResult<Option<(String, String)>> {
    let root_path = connection
        .query_row(
            "SELECT path FROM roots WHERE id = ?1",
            params![root_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Could not read root path for lexical root catalog: {error}"))?;
    let Some(root_path) = root_path else {
        return Ok(None);
    };

    let folder_name = folder_name_from_root_path(&root_path);
    let mut parts = vec![root_path.clone(), folder_name.clone()];

    {
        let mut statement = connection
            .prepare(
                "
                SELECT relative_path
                FROM files
                WHERE root_id = ?1
                ORDER BY modified_ms DESC, id DESC
                LIMIT 256
                ",
            )
            .map_err(|error| {
                format!("Could not prepare lexical root catalog file query: {error}")
            })?;
        let rows = statement
            .query_map(params![root_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Could not query lexical root catalog files: {error}"))?;
        for row in rows {
            let relative_path = row.map_err(|error| {
                format!("Could not parse lexical root catalog file row: {error}")
            })?;
            parts.push(crate::util::file_name_from_relative(&relative_path));
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT h.text
                FROM headings h
                JOIN files f ON f.id = h.file_id
                WHERE f.root_id = ?1
                ORDER BY f.modified_ms DESC, h.heading_order ASC
                LIMIT 256
                ",
            )
            .map_err(|error| {
                format!("Could not prepare lexical root catalog heading query: {error}")
            })?;
        let rows = statement
            .query_map(params![root_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Could not query lexical root catalog headings: {error}"))?;
        for row in rows {
            parts.push(row.map_err(|error| {
                format!("Could not parse lexical root catalog heading row: {error}")
            })?);
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT a.text
                FROM authors a
                JOIN files f ON f.id = a.file_id
                WHERE f.root_id = ?1
                ORDER BY f.modified_ms DESC, a.author_order ASC
                LIMIT 128
                ",
            )
            .map_err(|error| {
                format!("Could not prepare lexical root catalog author query: {error}")
            })?;
        let rows = statement
            .query_map(params![root_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Could not query lexical root catalog authors: {error}"))?;
        for row in rows {
            parts.push(row.map_err(|error| {
                format!("Could not parse lexical root catalog author row: {error}")
            })?);
        }
    }

    let query_text = parts.join("\n");
    let prefix_text = parts.join(" ");
    Ok(Some((query_text, prefix_text)))
}

fn replace_root_catalog_documents_from_connection(
    app: &AppHandle,
    connection: &Connection,
) -> CommandResult<()> {
    let root_catalog = root_catalog_runtime(app)?;
    let _guard = root_catalog
        .rebuild_lock
        .lock()
        .map_err(|_| "Could not lock lexical root catalog rebuild".to_string())?;
    let mut writer =
        new_index_writer(&root_catalog.index, "root catalog", WriterMode::FullRebuild)?;
    writer
        .delete_all_documents()
        .map_err(|error| format!("Could not clear lexical root catalog index: {error}"))?;

    let mut statement = connection
        .prepare("SELECT id, path FROM roots ORDER BY id ASC")
        .map_err(|error| format!("Could not prepare lexical root catalog roots query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Could not query lexical root catalog roots: {error}"))?;

    for row in rows {
        let (root_id, root_path) =
            row.map_err(|error| format!("Could not parse lexical root catalog row: {error}"))?;
        let Some((query_text, prefix_text)) = root_catalog_terms(connection, root_id)? else {
            continue;
        };
        let folder_name = folder_name_from_root_path(&root_path);
        let document = doc!(
            root_catalog.fields.root_id => u64::try_from(root_id).unwrap_or(0),
            root_catalog.fields.root_path => root_path,
            root_catalog.fields.folder_name => folder_name,
            root_catalog.fields.query_text => query_text,
            root_catalog.fields.prefix_text => prefix_text,
        );
        writer
            .add_document(document)
            .map_err(|error| format!("Could not add lexical root catalog document: {error}"))?;
    }

    writer
        .commit()
        .map_err(|error| format!("Could not commit lexical root catalog index: {error}"))?;
    root_catalog
        .reader
        .reload()
        .map_err(|error| format!("Could not reload lexical root catalog reader: {error}"))?;
    Ok(())
}

fn upsert_root_catalog_document(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }
    let root_catalog = root_catalog_runtime(app)?;
    let _guard = root_catalog
        .rebuild_lock
        .lock()
        .map_err(|_| "Could not lock lexical root catalog rebuild".to_string())?;
    let mut writer =
        new_index_writer(&root_catalog.index, "root catalog", WriterMode::Incremental)?;
    writer.delete_term(Term::from_field_u64(
        root_catalog.fields.root_id,
        u64::try_from(root_id).unwrap_or(0),
    ));
    if let Some((query_text, prefix_text)) = root_catalog_terms(connection, root_id)? {
        let root_path = connection
            .query_row(
                "SELECT path FROM roots WHERE id = ?1",
                params![root_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| {
                format!("Could not read root path for lexical root catalog: {error}")
            })?;
        if let Some(root_path) = root_path {
            let folder_name = folder_name_from_root_path(&root_path);
            let document = doc!(
                root_catalog.fields.root_id => u64::try_from(root_id).unwrap_or(0),
                root_catalog.fields.root_path => root_path,
                root_catalog.fields.folder_name => folder_name,
                root_catalog.fields.query_text => query_text,
                root_catalog.fields.prefix_text => prefix_text,
            );
            writer.add_document(document).map_err(|error| {
                format!("Could not add lexical root catalog document for root {root_id}: {error}")
            })?;
        }
    }
    writer
        .commit()
        .map_err(|error| format!("Could not commit lexical root catalog update: {error}"))?;
    root_catalog
        .reader
        .reload()
        .map_err(|error| format!("Could not reload lexical root catalog reader: {error}"))?;
    Ok(())
}

pub(crate) fn remove_root_catalog_document(app: &AppHandle, root_id: i64) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }
    let root_catalog = root_catalog_runtime(app)?;
    let _guard = root_catalog
        .rebuild_lock
        .lock()
        .map_err(|_| "Could not lock lexical root catalog rebuild".to_string())?;
    let mut writer =
        new_index_writer(&root_catalog.index, "root catalog", WriterMode::Incremental)?;
    writer.delete_term(Term::from_field_u64(
        root_catalog.fields.root_id,
        u64::try_from(root_id).unwrap_or(0),
    ));
    writer
        .commit()
        .map_err(|error| format!("Could not commit lexical root catalog delete: {error}"))?;
    root_catalog
        .reader
        .reload()
        .map_err(|error| format!("Could not reload lexical root catalog reader: {error}"))?;
    Ok(())
}

fn ensure_root_catalog_populated(app: &AppHandle, connection: &Connection) -> CommandResult<()> {
    let root_count = connection
        .query_row("SELECT COUNT(*) FROM roots", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not count roots for lexical root catalog: {error}"))?;
    if root_count <= 0 {
        return Ok(());
    }
    let root_catalog = root_catalog_runtime(app)?;
    if root_catalog_doc_count_for_runtime(&root_catalog) == 0 {
        replace_root_catalog_documents_from_connection(app, connection)?;
    }
    Ok(())
}

fn shortlist_root_ids(
    app: &AppHandle,
    connection: &Connection,
    normalized: &str,
) -> CommandResult<Vec<i64>> {
    ensure_root_catalog_populated(app, connection)?;
    let root_catalog = root_catalog_runtime(app)?;
    let query_tokens = sorted_unique_tokens(normalized);
    if query_tokens.is_empty() {
        return Ok(Vec::new());
    }

    let low_specificity = query_tokens.len() <= 1 && normalized.chars().count() <= 3;
    let shortlist_limit = if low_specificity {
        ROOT_SHORTLIST_LIMIT_LOW_SPECIFICITY
    } else {
        ROOT_SHORTLIST_LIMIT
    };

    let searcher = root_catalog.reader.searcher();
    let mut root_ids = Vec::new();
    let mut seen = HashSet::new();

    if let Some(query) = build_token_query(&[root_catalog.fields.query_text], &query_tokens, true) {
        let docs = searcher
            .search(&query, &TopDocs::with_limit(shortlist_limit))
            .map_err(|error| format!("Could not search lexical root catalog: {error}"))?;
        for (_, address) in docs {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(|error| format!("Could not read lexical root catalog doc: {error}"))?;
            let Some(root_id_u64) = field_u64(&document, root_catalog.fields.root_id) else {
                continue;
            };
            let Ok(root_id) = i64::try_from(root_id_u64) else {
                continue;
            };
            if seen.insert(root_id) {
                root_ids.push(root_id);
            }
        }
    }

    if normalized.chars().count() >= 4 && root_ids.len() < shortlist_limit {
        if let Some(query) =
            build_token_query(&[root_catalog.fields.prefix_text], &query_tokens, true)
        {
            let docs = searcher
                .search(&query, &TopDocs::with_limit(shortlist_limit))
                .map_err(|error| {
                    format!("Could not search lexical root catalog prefix tier: {error}")
                })?;
            for (_, address) in docs {
                let document = searcher.doc::<TantivyDocument>(address).map_err(|error| {
                    format!("Could not read lexical root catalog prefix doc: {error}")
                })?;
                let Some(root_id_u64) = field_u64(&document, root_catalog.fields.root_id) else {
                    continue;
                };
                let Ok(root_id) = i64::try_from(root_id_u64) else {
                    continue;
                };
                if seen.insert(root_id) {
                    root_ids.push(root_id);
                    if root_ids.len() >= shortlist_limit {
                        break;
                    }
                }
            }
        }
    }

    Ok(root_ids)
}

fn delete_file_documents(
    root_runtime: &RootShardRuntime,
    meta_writer: &mut IndexWriter,
    chunk_writer: &mut IndexWriter,
    file_ids: &[i64],
) -> CommandResult<()> {
    for file_id in file_ids {
        let file_id_u64 = u64::try_from(*file_id)
            .map_err(|_| format!("Could not convert lexical file id '{file_id}' to u64"))?;
        let meta_term = Term::from_field_u64(root_runtime.meta_fields.file_id, file_id_u64);
        let chunk_term = Term::from_field_u64(root_runtime.chunk_fields.file_id, file_id_u64);
        meta_writer.delete_term(meta_term);
        chunk_writer.delete_term(chunk_term);
    }

    Ok(())
}

fn append_file_documents(
    connection: &Connection,
    root_id: i64,
    file_id: i64,
    root_runtime: &RootShardRuntime,
    meta_writer: &mut IndexWriter,
    chunk_writer: &mut IndexWriter,
) -> CommandResult<()> {
    let file_row = connection
        .query_row(
            "
            SELECT relative_path, absolute_path
            FROM files
            WHERE root_id = ?1 AND id = ?2
            ",
            params![root_id, file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| {
            format!("Could not load lexical file row for file_id={file_id}: {error}")
        })?;

    let Some((relative_path, absolute_path)) = file_row else {
        return Ok(());
    };

    let file_name = crate::util::file_name_from_relative(&relative_path);
    let file_entry = LexicalDocument {
        root_id,
        file_id,
        source_row_id: file_id,
        kind: "file".to_string(),
        file_name: file_name.clone(),
        relative_path: relative_path.clone(),
        absolute_path: absolute_path.clone(),
        heading_level: None,
        heading_text: None,
        heading_order: None,
        author_text: None,
        chunk_order: None,
        chunk_text: None,
    };
    add_meta_document_to_writer(meta_writer, &root_runtime.meta_fields, &file_entry)?;

    {
        let mut statement = connection
            .prepare(
                "
                SELECT id, level, text, heading_order
                FROM headings
                WHERE file_id = ?1
                ORDER BY heading_order ASC
                ",
            )
            .map_err(|error| {
                format!(
                    "Could not prepare lexical heading rows query for file_id={file_id}: {error}"
                )
            })?;

        let rows = statement
            .query_map(params![file_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| {
                format!("Could not read lexical heading rows for file_id={file_id}: {error}")
            })?;

        for row in rows {
            let (heading_id, heading_level, heading_text, heading_order) =
                row.map_err(|error| {
                    format!("Could not parse lexical heading row for file_id={file_id}: {error}")
                })?;
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: heading_id,
                kind: "heading".to_string(),
                file_name: file_name.clone(),
                relative_path: relative_path.clone(),
                absolute_path: absolute_path.clone(),
                heading_level: Some(heading_level),
                heading_text: Some(heading_text),
                heading_order: Some(heading_order),
                author_text: None,
                chunk_order: None,
                chunk_text: None,
            };
            add_meta_document_to_writer(meta_writer, &root_runtime.meta_fields, &entry)?;
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT id, text, author_order
                FROM authors
                WHERE file_id = ?1
                ORDER BY author_order ASC
                ",
            )
            .map_err(|error| {
                format!(
                    "Could not prepare lexical author rows query for file_id={file_id}: {error}"
                )
            })?;

        let rows = statement
            .query_map(params![file_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| {
                format!("Could not read lexical author rows for file_id={file_id}: {error}")
            })?;

        for row in rows {
            let (author_id, author_text, author_order) = row.map_err(|error| {
                format!("Could not parse lexical author row for file_id={file_id}: {error}")
            })?;
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: author_id,
                kind: "author".to_string(),
                file_name: file_name.clone(),
                relative_path: relative_path.clone(),
                absolute_path: absolute_path.clone(),
                heading_level: None,
                heading_text: Some(author_text.clone()),
                heading_order: Some(author_order),
                author_text: Some(author_text),
                chunk_order: None,
                chunk_text: None,
            };
            add_meta_document_to_writer(meta_writer, &root_runtime.meta_fields, &entry)?;
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT id, chunk_order, heading_level, heading_text, heading_order, author_text, chunk_text
                FROM chunks
                WHERE file_id = ?1
                ORDER BY chunk_order ASC
                ",
            )
            .map_err(|error| {
                format!("Could not prepare lexical chunk rows query for file_id={file_id}: {error}")
            })?;

        let rows = statement
            .query_map(params![file_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| {
                format!("Could not read lexical chunk rows for file_id={file_id}: {error}")
            })?;

        for row in rows {
            let (
                chunk_row_id,
                chunk_order,
                heading_level,
                heading_text,
                heading_order,
                author_text,
                chunk_text,
            ) = row.map_err(|error| {
                format!("Could not parse lexical chunk row for file_id={file_id}: {error}")
            })?;

            if chunk_text.trim().is_empty() {
                continue;
            }

            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: chunk_row_id,
                kind: "chunk".to_string(),
                file_name: file_name.clone(),
                relative_path: relative_path.clone(),
                absolute_path: absolute_path.clone(),
                heading_level,
                heading_text,
                heading_order,
                author_text,
                chunk_order: Some(chunk_order),
                chunk_text: Some(chunk_text),
            };
            add_chunk_document_to_writer(chunk_writer, &root_runtime.chunk_fields, &entry)?;
        }
    }

    Ok(())
}

fn apply_file_changes_for_runtime(
    connection: &Connection,
    root_id: i64,
    root_runtime: &RootShardRuntime,
    updated_file_ids: &[i64],
    removed_file_ids: &[i64],
    wait_for_merges: bool,
) -> CommandResult<()> {
    let mut delete_file_ids = updated_file_ids
        .iter()
        .chain(removed_file_ids.iter())
        .copied()
        .collect::<Vec<i64>>();
    delete_file_ids.sort_unstable();
    delete_file_ids.dedup();

    let mut reindex_file_ids = updated_file_ids.to_vec();
    reindex_file_ids.sort_unstable();
    reindex_file_ids.dedup();

    if delete_file_ids.is_empty() && reindex_file_ids.is_empty() {
        return Ok(());
    }

    let mut meta_writer =
        new_index_writer(&root_runtime.meta_index, "meta", WriterMode::Incremental)?;
    let mut chunk_writer =
        new_index_writer(&root_runtime.chunk_index, "chunk", WriterMode::Incremental)?;

    delete_file_documents(
        root_runtime,
        &mut meta_writer,
        &mut chunk_writer,
        &delete_file_ids,
    )?;

    for file_id in reindex_file_ids {
        append_file_documents(
            connection,
            root_id,
            file_id,
            root_runtime,
            &mut meta_writer,
            &mut chunk_writer,
        )?;
    }

    meta_writer
        .commit()
        .map_err(|error| format!("Could not commit lexical meta index: {error}"))?;

    chunk_writer
        .commit()
        .map_err(|error| format!("Could not commit lexical chunk index: {error}"))?;

    if wait_for_merges {
        meta_writer
            .wait_merging_threads()
            .map_err(|error| format!("Could not finish lexical meta merges: {error}"))?;
        chunk_writer
            .wait_merging_threads()
            .map_err(|error| format!("Could not finish lexical chunk merges: {error}"))?;
    }

    root_runtime
        .meta_reader
        .reload()
        .map_err(|error| format!("Could not reload lexical meta reader: {error}"))?;
    root_runtime
        .chunk_reader
        .reload()
        .map_err(|error| format!("Could not reload lexical chunk reader: {error}"))?;

    Ok(())
}

fn replace_root_documents_for_runtime(
    connection: &Connection,
    root_id: i64,
    root_runtime: &RootShardRuntime,
    wait_for_merges: bool,
) -> CommandResult<()> {
    let mut meta_writer =
        new_index_writer(&root_runtime.meta_index, "meta", WriterMode::FullRebuild)?;
    let mut chunk_writer =
        new_index_writer(&root_runtime.chunk_index, "chunk", WriterMode::FullRebuild)?;

    meta_writer
        .delete_all_documents()
        .map_err(|error| format!("Could not clear lexical meta index: {error}"))?;
    chunk_writer
        .delete_all_documents()
        .map_err(|error| format!("Could not clear lexical chunk index: {error}"))?;

    {
        let mut statement = connection
            .prepare(
                "
                SELECT id, relative_path, absolute_path
                FROM files
                WHERE root_id = ?1
                ORDER BY relative_path ASC
                ",
            )
            .map_err(|error| format!("Could not prepare lexical file rows query: {error}"))?;

        let rows = statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("Could not read lexical file rows: {error}"))?;

        for row in rows {
            let (file_id, relative_path, absolute_path) =
                row.map_err(|error| format!("Could not parse lexical file row: {error}"))?;
            let file_name = crate::util::file_name_from_relative(&relative_path);
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: file_id,
                kind: "file".to_string(),
                file_name,
                relative_path,
                absolute_path,
                heading_level: None,
                heading_text: None,
                heading_order: None,
                author_text: None,
                chunk_order: None,
                chunk_text: None,
            };
            add_meta_document_to_writer(&mut meta_writer, &root_runtime.meta_fields, &entry)?;
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT
                  f.id,
                  f.relative_path,
                  f.absolute_path,
                  h.id,
                  h.level,
                  h.text,
                  h.heading_order
                FROM headings h
                JOIN files f ON f.id = h.file_id
                WHERE f.root_id = ?1
                ORDER BY f.id ASC, h.heading_order ASC
                ",
            )
            .map_err(|error| format!("Could not prepare lexical heading rows query: {error}"))?;

        let rows = statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(|error| format!("Could not read lexical heading rows: {error}"))?;

        for row in rows {
            let (
                file_id,
                relative_path,
                absolute_path,
                heading_id,
                level,
                heading_text,
                heading_order,
            ) = row.map_err(|error| format!("Could not parse lexical heading row: {error}"))?;
            let file_name = crate::util::file_name_from_relative(&relative_path);
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: heading_id,
                kind: "heading".to_string(),
                file_name,
                relative_path,
                absolute_path,
                heading_level: Some(level),
                heading_text: Some(heading_text),
                heading_order: Some(heading_order),
                author_text: None,
                chunk_order: None,
                chunk_text: None,
            };
            add_meta_document_to_writer(&mut meta_writer, &root_runtime.meta_fields, &entry)?;
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT
                  f.id,
                  f.relative_path,
                  f.absolute_path,
                  a.id,
                  a.text,
                  a.author_order
                FROM authors a
                JOIN files f ON f.id = a.file_id
                WHERE f.root_id = ?1
                ORDER BY f.id ASC, a.author_order ASC
                ",
            )
            .map_err(|error| format!("Could not prepare lexical author rows query: {error}"))?;

        let rows = statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| format!("Could not read lexical author rows: {error}"))?;

        for row in rows {
            let (file_id, relative_path, absolute_path, author_id, author_text, author_order) =
                row.map_err(|error| format!("Could not parse lexical author row: {error}"))?;
            let file_name = crate::util::file_name_from_relative(&relative_path);
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: author_id,
                kind: "author".to_string(),
                file_name,
                relative_path,
                absolute_path,
                heading_level: None,
                heading_text: Some(author_text.clone()),
                heading_order: Some(author_order),
                author_text: Some(author_text),
                chunk_order: None,
                chunk_text: None,
            };
            add_meta_document_to_writer(&mut meta_writer, &root_runtime.meta_fields, &entry)?;
        }
    }

    {
        let mut statement = connection
            .prepare(
                "
                SELECT
                  c.id,
                  c.file_id,
                  c.chunk_order,
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
                ",
            )
            .map_err(|error| format!("Could not prepare lexical chunk rows query: {error}"))?;

        let rows = statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            })
            .map_err(|error| format!("Could not read lexical chunk rows: {error}"))?;

        for row in rows {
            let (
                chunk_row_id,
                file_id,
                chunk_order,
                relative_path,
                absolute_path,
                heading_level,
                heading_text,
                heading_order,
                author_text,
                chunk_text,
            ) = row.map_err(|error| format!("Could not parse lexical chunk row: {error}"))?;

            if chunk_text.trim().is_empty() {
                continue;
            }

            let file_name = crate::util::file_name_from_relative(&relative_path);
            let entry = LexicalDocument {
                root_id,
                file_id,
                source_row_id: chunk_row_id,
                kind: "chunk".to_string(),
                file_name,
                relative_path,
                absolute_path,
                heading_level,
                heading_text,
                heading_order,
                author_text,
                chunk_order: Some(chunk_order),
                chunk_text: Some(chunk_text),
            };
            add_chunk_document_to_writer(&mut chunk_writer, &root_runtime.chunk_fields, &entry)?;
        }
    }

    meta_writer
        .commit()
        .map_err(|error| format!("Could not commit lexical meta index: {error}"))?;

    chunk_writer
        .commit()
        .map_err(|error| format!("Could not commit lexical chunk index: {error}"))?;

    if wait_for_merges {
        meta_writer
            .wait_merging_threads()
            .map_err(|error| format!("Could not finish lexical meta merges: {error}"))?;
        chunk_writer
            .wait_merging_threads()
            .map_err(|error| format!("Could not finish lexical chunk merges: {error}"))?;
    }

    root_runtime
        .meta_reader
        .reload()
        .map_err(|error| format!("Could not reload lexical meta reader: {error}"))?;
    root_runtime
        .chunk_reader
        .reload()
        .map_err(|error| format!("Could not reload lexical chunk reader: {error}"))?;

    Ok(())
}

fn field_text(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn field_i64(document: &TantivyDocument, field: Field) -> Option<i64> {
    document.get_first(field).and_then(|value| value.as_i64())
}

fn field_u64(document: &TantivyDocument, field: Field) -> Option<u64> {
    document.get_first(field).and_then(|value| value.as_u64())
}

fn dedupe_key(hit: &SearchHit) -> String {
    match hit.kind.as_str() {
        "file" => format!("file:{}", hit.file_id),
        "author" => {
            if let Some(order) = hit.heading_order {
                format!("author:{}:{order}", hit.file_id)
            } else {
                format!(
                    "author:{}:{}",
                    hit.file_id,
                    hit.heading_text.as_deref().unwrap_or_default()
                )
            }
        }
        _ => {
            if let Some(order) = hit.heading_order {
                format!("heading:{}:{order}", hit.file_id)
            } else {
                format!(
                    "heading:{}:{}",
                    hit.file_id,
                    hit.heading_text.as_deref().unwrap_or_default()
                )
            }
        }
    }
}

fn candidate_key(candidate: &CandidateHit) -> String {
    match candidate.kind {
        CandidateKind::File => format!("file:{}", candidate.source_row_id),
        CandidateKind::Author => format!("author:{}", candidate.source_row_id),
        CandidateKind::Chunk => format!("chunk:{}", candidate.source_row_id),
        CandidateKind::Heading => format!("heading:{}", candidate.source_row_id),
    }
}

fn preview_text_for_chunk(chunk_text: &str) -> String {
    let trimmed = chunk_text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= CHUNK_PREVIEW_CHARS {
        return trimmed.to_string();
    }
    trimmed
        .chars()
        .take(CHUNK_PREVIEW_CHARS)
        .collect::<String>()
}

fn add_meta_document_to_writer(
    writer: &mut tantivy::IndexWriter,
    fields: &MetaFields,
    entry: &LexicalDocument,
) -> CommandResult<()> {
    let heading_text = entry.heading_text.clone().unwrap_or_default();
    let author_text = entry.author_text.clone().unwrap_or_default();
    let query_text = format!(
        "{}\n{}\n{}\n{}",
        heading_text, author_text, entry.file_name, entry.relative_path
    );
    let prefix_text = format!(
        "{} {} {} {}",
        heading_text, author_text, entry.file_name, entry.relative_path
    );

    let mut document = doc!(
        fields.kind => entry.kind.as_str(),
        fields.root_id => u64::try_from(entry.root_id).unwrap_or(0),
        fields.file_id => u64::try_from(entry.file_id).unwrap_or(0),
        fields.source_row_id => u64::try_from(entry.source_row_id).unwrap_or(0),
        fields.query_text => query_text,
        fields.prefix_text => prefix_text,
    );

    if let Some(level) = entry.heading_level {
        document.add_i64(fields.heading_level, level);
    }
    if let Some(order) = entry.heading_order {
        document.add_i64(fields.heading_order, order);
    }

    writer.add_document(document).map_err(|error| {
        format!(
            "Could not add lexical meta document for '{}': {error}",
            entry.relative_path
        )
    })?;
    Ok(())
}

fn add_chunk_document_to_writer(
    writer: &mut tantivy::IndexWriter,
    fields: &ChunkFields,
    entry: &LexicalDocument,
) -> CommandResult<()> {
    let chunk_text = entry.chunk_text.clone().unwrap_or_default();
    if chunk_text.trim().is_empty() {
        return Ok(());
    }

    let heading_text = entry.heading_text.clone().unwrap_or_default();
    let author_text = entry.author_text.clone().unwrap_or_default();
    let chunk_preview = preview_text_for_chunk(&chunk_text);
    let query_text = chunk_text.clone();
    let ngram_text = format!(
        "{} {} {} {} {}",
        heading_text, author_text, chunk_preview, entry.file_name, entry.relative_path
    );

    let mut document = doc!(
        fields.root_id => u64::try_from(entry.root_id).unwrap_or(0),
        fields.file_id => u64::try_from(entry.file_id).unwrap_or(0),
        fields.source_row_id => u64::try_from(entry.source_row_id).unwrap_or(0),
        fields.chunk_order => entry.chunk_order.unwrap_or(0),
        fields.query_text => query_text,
        fields.ngram_text => ngram_text,
    );

    if let Some(level) = entry.heading_level {
        document.add_i64(fields.heading_level, level);
    }
    if let Some(order) = entry.heading_order {
        document.add_i64(fields.heading_order, order);
    }

    writer.add_document(document).map_err(|error| {
        format!(
            "Could not add lexical chunk document for '{}': {error}",
            entry.relative_path
        )
    })?;
    Ok(())
}

fn new_index_writer(
    index: &Index,
    label: &str,
    mode: WriterMode,
) -> CommandResult<tantivy::IndexWriter> {
    let (primary_bytes, fallback_bytes, mode_label) = match mode {
        WriterMode::FullRebuild => (
            FULL_REBUILD_WRITER_HEAP_BYTES,
            FULL_REBUILD_WRITER_FALLBACK_HEAP_BYTES,
            "full",
        ),
        WriterMode::Incremental => (
            INCREMENTAL_WRITER_HEAP_BYTES,
            INCREMENTAL_WRITER_FALLBACK_HEAP_BYTES,
            "incremental",
        ),
    };
    eprintln!(
        "[lexical] opening {label} writer mode={mode_label} primary_bytes={primary_bytes} fallback_bytes={fallback_bytes}"
    );
    index
        .writer(primary_bytes)
        .or_else(|_| index.writer(fallback_bytes))
        .map_err(|error| format!("Could not create lexical {label} index writer: {error}"))
}

fn remove_stale_root_dirs(app: &AppHandle, active_roots: &HashSet<i64>) -> CommandResult<()> {
    let roots_dir = roots_dir(app)?;
    if !roots_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(&roots_dir).map_err(|error| {
        format!(
            "Could not read lexical roots directory '{}': {error}",
            roots_dir.display()
        )
    })?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Could not read lexical root shard entry: {error}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(root_id) = name.parse::<i64>() else {
            continue;
        };
        if active_roots.contains(&root_id) {
            continue;
        }
        fs::remove_dir_all(&path).map_err(|error| {
            format!(
                "Could not remove stale lexical shard '{}': {error}",
                path.display()
            )
        })?;
    }

    if let Ok(mut runtime) = lexical_runtime().lock() {
        runtime
            .roots
            .retain(|root_id, _| active_roots.contains(root_id));
        runtime.lru.retain(|root_id| active_roots.contains(root_id));
    }

    Ok(())
}

pub(crate) fn replace_root_documents_from_connection(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
) -> CommandResult<()> {
    replace_root_documents_from_connection_with_options(app, connection, root_id, true)
}

pub(crate) fn replace_root_documents_from_connection_with_options(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
    wait_for_merges: bool,
) -> CommandResult<()> {
    let root_runtime = root_runtime(app, root_id)?;
    let _rebuild_guard = root_runtime
        .rebuild_lock
        .lock()
        .map_err(|_| format!("Could not lock lexical rebuild for root_id={root_id}"))?;
    replace_root_documents_for_runtime(connection, root_id, &root_runtime, wait_for_merges)?;
    upsert_root_catalog_document(app, connection, root_id)?;
    Ok(())
}

pub(crate) fn apply_file_changes_from_connection_with_options(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
    updated_file_ids: &[i64],
    removed_file_ids: &[i64],
    wait_for_merges: bool,
) -> CommandResult<()> {
    let root_runtime = root_runtime(app, root_id)?;
    let _rebuild_guard = root_runtime
        .rebuild_lock
        .lock()
        .map_err(|_| format!("Could not lock lexical rebuild for root_id={root_id}"))?;
    apply_file_changes_for_runtime(
        connection,
        root_id,
        &root_runtime,
        updated_file_ids,
        removed_file_ids,
        wait_for_merges,
    )?;
    upsert_root_catalog_document(app, connection, root_id)?;
    Ok(())
}

pub(crate) fn replace_all_documents_from_connection(
    app: &AppHandle,
    connection: &Connection,
) -> CommandResult<()> {
    let mut statement = connection
        .prepare("SELECT id FROM roots ORDER BY id ASC")
        .map_err(|error| format!("Could not prepare roots query for lexical rebuild: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not read roots for lexical rebuild: {error}"))?;

    let mut root_ids = Vec::new();
    for row in rows {
        root_ids.push(row.map_err(|error| format!("Could not parse lexical root id: {error}"))?);
    }

    let active_roots = root_ids.iter().copied().collect::<HashSet<i64>>();
    for root_id in &root_ids {
        replace_root_documents_from_connection(app, connection, *root_id)?;
    }

    replace_root_catalog_documents_from_connection(app, connection)?;
    remove_stale_root_dirs(app, &active_roots)?;
    Ok(())
}

pub(crate) fn indexed_root_doc_count(app: &AppHandle, root_id: i64) -> CommandResult<u64> {
    if root_id <= 0 {
        return Ok(0);
    }

    let root_runtime = root_runtime(app, root_id)?;
    Ok(indexed_root_doc_count_for_runtime(&root_runtime))
}

fn sorted_unique_tokens(normalized: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tokens = Vec::new();
    for token in normalized.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        let token = token.to_string();
        if seen.insert(token.clone()) {
            tokens.push(token);
        }
    }
    tokens
}

fn ngram_tokens(normalized: &str) -> Vec<String> {
    let compact = normalized.replace(' ', "");
    let chars = compact.chars().collect::<Vec<char>>();
    if chars.len() < 3 {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut output = Vec::new();

    for start in 0..chars.len() {
        for size in [3usize, 4usize] {
            let end = start.saturating_add(size);
            if end > chars.len() {
                continue;
            }
            let gram = chars[start..end].iter().collect::<String>();
            if seen.insert(gram.clone()) {
                output.push(gram);
            }
        }
    }

    output
}

fn prefix_wildcard_tokens(tokens: &[String]) -> Vec<String> {
    let mut output = Vec::new();
    for token in tokens {
        if token.chars().count() < 4 {
            continue;
        }
        let shortened = token
            .chars()
            .take(token.chars().count().saturating_sub(1))
            .collect::<String>();
        if shortened.chars().count() >= 3 {
            output.push(shortened);
        }
    }
    output
}

fn build_token_query(
    fields: &[Field],
    tokens: &[String],
    conjunction: bool,
) -> Option<Box<dyn Query>> {
    if fields.is_empty() || tokens.is_empty() {
        return None;
    }

    if conjunction {
        let mut clauses = Vec::new();
        for token in tokens {
            let mut token_clauses = Vec::new();
            for field in fields {
                let term = Term::from_field_text(*field, token);
                token_clauses.push((
                    Occur::Should,
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)) as Box<dyn Query>,
                ));
            }

            if token_clauses.is_empty() {
                continue;
            }

            let token_query: Box<dyn Query> = if token_clauses.len() == 1 {
                token_clauses
                    .into_iter()
                    .next()
                    .map(|(_, query)| query)
                    .unwrap_or_else(|| Box::new(BooleanQuery::new(Vec::new())) as Box<dyn Query>)
            } else {
                Box::new(BooleanQuery::new(token_clauses))
            };
            clauses.push((Occur::Must, token_query));
        }

        if clauses.is_empty() {
            None
        } else {
            Some(Box::new(BooleanQuery::new(clauses)))
        }
    } else {
        let mut clauses = Vec::new();
        for token in tokens {
            for field in fields {
                let term = Term::from_field_text(*field, token);
                clauses.push((
                    Occur::Should,
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)) as Box<dyn Query>,
                ));
            }
        }

        if clauses.is_empty() {
            None
        } else {
            Some(Box::new(BooleanQuery::new(clauses)))
        }
    }
}

fn build_file_filter_query(field: Field, file_ids: &[i64]) -> Option<Box<dyn Query>> {
    if file_ids.is_empty() {
        return None;
    }

    let mut clauses = Vec::new();
    for file_id in file_ids {
        let Ok(file_id_u64) = u64::try_from(*file_id) else {
            continue;
        };
        clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(
                Term::from_field_u64(field, file_id_u64),
                IndexRecordOption::Basic,
            )) as Box<dyn Query>,
        ));
    }

    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn run_meta_tier(
    searcher: &tantivy::Searcher,
    fields: &MetaFields,
    tokens: &[String],
    conjunction: bool,
    tier_fetch_limit: usize,
    score_base: f64,
    file_name_only: bool,
    tier_name: &str,
    target_limit: usize,
    telemetry: &mut LexicalSearchTelemetry,
    candidates: &mut Vec<CandidateHit>,
    seen_candidates: &mut HashSet<String>,
) -> CommandResult<()> {
    let started = Instant::now();
    let Some(parsed_query) = build_token_query(&[fields.query_text], tokens, conjunction) else {
        return Ok(());
    };

    let mut clauses = vec![(Occur::Must, parsed_query)];
    if file_name_only {
        clauses.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(fields.kind, "file"),
                IndexRecordOption::Basic,
            )) as Box<dyn Query>,
        ));
    }

    let query: Box<dyn Query> = Box::new(BooleanQuery::new(clauses));
    let docs = searcher
        .search(&query, &TopDocs::with_limit(tier_fetch_limit))
        .map_err(|error| format!("Lexical meta tier '{tier_name}' failed: {error}"))?;

    let mut accepted = 0_usize;
    let fetch_started = Instant::now();
    for (rank, (tier_score, address)) in docs.into_iter().enumerate() {
        if candidates.len() >= target_limit {
            break;
        }

        let document = searcher
            .doc::<TantivyDocument>(address)
            .map_err(|error| format!("Could not read lexical meta doc: {error}"))?;

        let kind = field_text(&document, fields.kind).unwrap_or_else(|| "file".to_string());
        if file_name_only && kind != "file" {
            continue;
        }

        let Some(file_id_u64) = field_u64(&document, fields.file_id) else {
            continue;
        };
        let Ok(file_id) = i64::try_from(file_id_u64) else {
            continue;
        };
        let Some(source_row_id_u64) = field_u64(&document, fields.source_row_id) else {
            continue;
        };
        let Ok(source_row_id) = i64::try_from(source_row_id_u64) else {
            continue;
        };

        let candidate_kind = match kind.as_str() {
            "file" => CandidateKind::File,
            "author" => CandidateKind::Author,
            _ => CandidateKind::Heading,
        };

        let bm25 = if tier_score.is_finite() {
            f64::from(tier_score)
        } else {
            0.0
        };
        let candidate = CandidateHit {
            kind: candidate_kind,
            file_id,
            source_row_id,
            heading_level: field_i64(&document, fields.heading_level),
            heading_order: field_i64(&document, fields.heading_order),
            chunk_order: None,
            score: score_base + f64::from(rank as u32) - bm25,
        };

        let key = candidate_key(&candidate);
        if !seen_candidates.insert(key) {
            continue;
        }

        candidates.push(candidate);
        accepted += 1;
    }
    telemetry.doc_fetch_ms += fetch_started.elapsed().as_secs_f64() * 1000.0;

    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    *telemetry
        .tier_timings_ms
        .entry(tier_name.to_string())
        .or_insert(0.0) += elapsed;
    *telemetry
        .tier_hit_counts
        .entry(tier_name.to_string())
        .or_insert(0) += accepted;

    Ok(())
}

fn run_meta_prefix_tier(
    searcher: &tantivy::Searcher,
    fields: &MetaFields,
    tokens: &[String],
    conjunction: bool,
    tier_fetch_limit: usize,
    score_base: f64,
    file_name_only: bool,
    tier_name: &str,
    target_limit: usize,
    telemetry: &mut LexicalSearchTelemetry,
    candidates: &mut Vec<CandidateHit>,
    seen_candidates: &mut HashSet<String>,
) -> CommandResult<()> {
    let started = Instant::now();
    let Some(parsed_query) = build_token_query(&[fields.prefix_text], tokens, conjunction) else {
        return Ok(());
    };

    let mut clauses = vec![(Occur::Must, parsed_query)];
    if file_name_only {
        clauses.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(fields.kind, "file"),
                IndexRecordOption::Basic,
            )) as Box<dyn Query>,
        ));
    }

    let query: Box<dyn Query> = Box::new(BooleanQuery::new(clauses));
    let docs = searcher
        .search(&query, &TopDocs::with_limit(tier_fetch_limit))
        .map_err(|error| format!("Lexical meta prefix tier '{tier_name}' failed: {error}"))?;

    let mut accepted = 0_usize;
    let fetch_started = Instant::now();
    for (rank, (tier_score, address)) in docs.into_iter().enumerate() {
        if candidates.len() >= target_limit {
            break;
        }

        let document = searcher
            .doc::<TantivyDocument>(address)
            .map_err(|error| format!("Could not read lexical meta prefix doc: {error}"))?;

        let kind = field_text(&document, fields.kind).unwrap_or_else(|| "file".to_string());
        if file_name_only && kind != "file" {
            continue;
        }

        let Some(file_id_u64) = field_u64(&document, fields.file_id) else {
            continue;
        };
        let Ok(file_id) = i64::try_from(file_id_u64) else {
            continue;
        };
        let Some(source_row_id_u64) = field_u64(&document, fields.source_row_id) else {
            continue;
        };
        let Ok(source_row_id) = i64::try_from(source_row_id_u64) else {
            continue;
        };

        let candidate_kind = match kind.as_str() {
            "file" => CandidateKind::File,
            "author" => CandidateKind::Author,
            _ => CandidateKind::Heading,
        };

        let bm25 = if tier_score.is_finite() {
            f64::from(tier_score)
        } else {
            0.0
        };
        let candidate = CandidateHit {
            kind: candidate_kind,
            file_id,
            source_row_id,
            heading_level: field_i64(&document, fields.heading_level),
            heading_order: field_i64(&document, fields.heading_order),
            chunk_order: None,
            score: score_base + f64::from(rank as u32) - bm25,
        };

        let key = candidate_key(&candidate);
        if !seen_candidates.insert(key) {
            continue;
        }

        candidates.push(candidate);
        accepted += 1;
    }
    telemetry.doc_fetch_ms += fetch_started.elapsed().as_secs_f64() * 1000.0;

    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    *telemetry
        .tier_timings_ms
        .entry(tier_name.to_string())
        .or_insert(0.0) += elapsed;
    *telemetry
        .tier_hit_counts
        .entry(tier_name.to_string())
        .or_insert(0) += accepted;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_chunk_tier(
    searcher: &tantivy::Searcher,
    fields: &ChunkFields,
    tokens: &[String],
    query_field: Field,
    conjunction: bool,
    tier_fetch_limit: usize,
    score_base: f64,
    tier_name: &str,
    target_limit: usize,
    candidate_file_ids: &[i64],
    telemetry: &mut LexicalSearchTelemetry,
    candidates: &mut Vec<CandidateHit>,
    seen_candidates: &mut HashSet<String>,
) -> CommandResult<()> {
    let started = Instant::now();
    let Some(parsed_query) = build_token_query(&[query_field], tokens, conjunction) else {
        return Ok(());
    };

    let mut clauses = vec![(Occur::Must, parsed_query)];
    if !candidate_file_ids.is_empty() {
        if let Some(file_filter) = build_file_filter_query(fields.file_id, candidate_file_ids) {
            clauses.push((Occur::Must, file_filter));
        }
    }

    let query: Box<dyn Query> = Box::new(BooleanQuery::new(clauses));
    let docs = searcher
        .search(&query, &TopDocs::with_limit(tier_fetch_limit))
        .map_err(|error| format!("Lexical chunk tier '{tier_name}' failed: {error}"))?;

    let mut accepted = 0_usize;
    let fetch_started = Instant::now();
    for (rank, (tier_score, address)) in docs.into_iter().enumerate() {
        if candidates.len() >= target_limit {
            break;
        }

        let document = searcher
            .doc::<TantivyDocument>(address)
            .map_err(|error| format!("Could not read lexical chunk doc: {error}"))?;

        let Some(file_id_u64) = field_u64(&document, fields.file_id) else {
            continue;
        };
        let Ok(file_id) = i64::try_from(file_id_u64) else {
            continue;
        };
        let Some(source_row_id_u64) = field_u64(&document, fields.source_row_id) else {
            continue;
        };
        let Ok(source_row_id) = i64::try_from(source_row_id_u64) else {
            continue;
        };

        let bm25 = if tier_score.is_finite() {
            f64::from(tier_score)
        } else {
            0.0
        };
        let candidate = CandidateHit {
            kind: CandidateKind::Chunk,
            file_id,
            source_row_id,
            heading_level: field_i64(&document, fields.heading_level),
            heading_order: field_i64(&document, fields.heading_order),
            chunk_order: field_i64(&document, fields.chunk_order),
            score: score_base + f64::from(rank as u32) - bm25,
        };

        let key = candidate_key(&candidate);
        if !seen_candidates.insert(key) {
            continue;
        }

        candidates.push(candidate);
        accepted += 1;
    }
    telemetry.doc_fetch_ms += fetch_started.elapsed().as_secs_f64() * 1000.0;

    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    *telemetry
        .tier_timings_ms
        .entry(tier_name.to_string())
        .or_insert(0.0) += elapsed;
    *telemetry
        .tier_hit_counts
        .entry(tier_name.to_string())
        .or_insert(0) += accepted;

    Ok(())
}

fn collect_candidate_file_ids(candidates: &[CandidateHit], cap: usize) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();

    for candidate in candidates {
        if !seen.insert(candidate.file_id) {
            continue;
        }
        output.push(candidate.file_id);
        if output.len() >= cap {
            break;
        }
    }

    output
}

fn root_file_count(connection: &Connection, root_id: i64) -> CommandResult<i64> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM files WHERE root_id = ?1",
            params![root_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not count indexed files for root {root_id}: {error}"))
}

fn ensure_root_shard_populated(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
) -> CommandResult<()> {
    let file_count = root_file_count(connection, root_id)?;
    if file_count <= 0 {
        return Ok(());
    }

    let indexed_docs = indexed_root_doc_count(app, root_id).unwrap_or(0);
    if indexed_docs == 0 {
        let root_runtime = root_runtime(app, root_id)?;
        let _rebuild_guard = root_runtime
            .rebuild_lock
            .lock()
            .map_err(|_| format!("Could not lock lexical rebuild for root_id={root_id}"))?;
        if indexed_root_doc_count_for_runtime(&root_runtime) == 0 {
            replace_root_documents_for_runtime(connection, root_id, &root_runtime, true)?;
            upsert_root_catalog_document(app, connection, root_id)?;
        }
    }

    Ok(())
}

fn build_in_clause(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<&str>>().join(",")
}

fn resolve_local_file_ids_for_root(
    connection: &Connection,
    root_id: i64,
    shared_file_ids: &[i64],
) -> CommandResult<HashMap<i64, i64>> {
    if root_id <= 0 || shared_file_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let has_shared_source = connection
        .query_row(
            "SELECT COUNT(*) FROM shared_root_sources WHERE root_id = ?1",
            params![root_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not read shared root source metadata: {error}"))?;
    if has_shared_source <= 0 {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "
        SELECT shared_file_id, local_file_id
        FROM shared_file_maps
        WHERE root_id = ?1 AND shared_file_id IN ({})
        ",
        build_in_clause(shared_file_ids.len())
    );

    let mut values = Vec::with_capacity(shared_file_ids.len() + 1);
    values.push(root_id);
    values.extend(shared_file_ids.iter().copied());

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Could not prepare shared file-id map query: {error}"))?;

    let rows = statement
        .query_map(params_from_iter(values.into_iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| format!("Could not execute shared file-id map query: {error}"))?;

    let mut mapping = HashMap::new();
    for row in rows {
        let (shared_file_id, local_file_id) =
            row.map_err(|error| format!("Could not parse shared file-id map row: {error}"))?;
        mapping.insert(shared_file_id, local_file_id);
    }

    Ok(mapping)
}

fn fetch_file_hydration(
    connection: &Connection,
    file_ids: &[i64],
) -> CommandResult<HashMap<i64, FileHydration>> {
    if file_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT id, relative_path, absolute_path FROM files WHERE id IN ({})",
        build_in_clause(file_ids.len())
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Could not prepare file hydration query: {error}"))?;

    let rows = statement
        .query_map(params_from_iter(file_ids.iter().copied()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Could not execute file hydration query: {error}"))?;

    let mut output = HashMap::new();
    for row in rows {
        let (file_id, relative_path, absolute_path) =
            row.map_err(|error| format!("Could not parse file hydration row: {error}"))?;
        let file_name = crate::util::file_name_from_relative(&relative_path);
        output.insert(
            file_id,
            FileHydration {
                file_name_normalized: normalize_for_search(&file_name),
                relative_path_normalized: normalize_for_search(&relative_path),
                file_name,
                relative_path,
                absolute_path,
            },
        );
    }

    Ok(output)
}

fn fetch_heading_hydration(
    connection: &Connection,
    row_ids: &[i64],
) -> CommandResult<HashMap<i64, HeadingHydration>> {
    if row_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT id, level, text FROM headings WHERE id IN ({})",
        build_in_clause(row_ids.len())
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Could not prepare heading hydration query: {error}"))?;

    let rows = statement
        .query_map(params_from_iter(row_ids.iter().copied()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Could not execute heading hydration query: {error}"))?;

    let mut output = HashMap::new();
    for row in rows {
        let (heading_id, heading_level, heading_text) =
            row.map_err(|error| format!("Could not parse heading hydration row: {error}"))?;
        output.insert(
            heading_id,
            HeadingHydration {
                heading_level,
                heading_text,
            },
        );
    }

    Ok(output)
}

fn fetch_author_hydration(
    connection: &Connection,
    row_ids: &[i64],
) -> CommandResult<HashMap<i64, AuthorHydration>> {
    if row_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT id, text FROM authors WHERE id IN ({})",
        build_in_clause(row_ids.len())
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Could not prepare author hydration query: {error}"))?;

    let rows = statement
        .query_map(params_from_iter(row_ids.iter().copied()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Could not execute author hydration query: {error}"))?;

    let mut output = HashMap::new();
    for row in rows {
        let (author_id, author_text) =
            row.map_err(|error| format!("Could not parse author hydration row: {error}"))?;
        output.insert(author_id, AuthorHydration { author_text });
    }

    Ok(output)
}

fn fetch_chunk_hydration(
    connection: &Connection,
    row_ids: &[i64],
) -> CommandResult<HashMap<i64, ChunkHydration>> {
    if row_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "
        SELECT id, heading_level, heading_order, heading_text, author_text, chunk_text
        FROM chunks
        WHERE id IN ({})
        ",
        build_in_clause(row_ids.len())
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("Could not prepare chunk hydration query: {error}"))?;

    let rows = statement
        .query_map(params_from_iter(row_ids.iter().copied()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| format!("Could not execute chunk hydration query: {error}"))?;

    let mut output = HashMap::new();
    for row in rows {
        let (chunk_row_id, heading_level, heading_order, heading_text, author_text, chunk_text) =
            row.map_err(|error| format!("Could not parse chunk hydration row: {error}"))?;
        output.insert(
            chunk_row_id,
            ChunkHydration {
                heading_level,
                heading_order,
                heading_text,
                author_text,
                chunk_text,
            },
        );
    }

    Ok(output)
}

fn rerank_bonus(
    normalized_query: &str,
    query_tokens: &[String],
    file_hydration: &FileHydration,
    heading_text: Option<&str>,
    heading_order: Option<i64>,
) -> f64 {
    let mut bonus = 0.0_f64;

    if file_hydration
        .relative_path_normalized
        .contains(normalized_query)
        || file_hydration
            .file_name_normalized
            .contains(normalized_query)
        || heading_text
            .map(normalize_for_search)
            .unwrap_or_default()
            .contains(normalized_query)
    {
        bonus += 180.0;
    }

    let heading_norm = heading_text.map(normalize_for_search).unwrap_or_default();
    let mut matched = 0_usize;
    for token in query_tokens {
        if file_hydration
            .file_name_normalized
            .split_whitespace()
            .any(|word| word == token)
        {
            bonus += 24.0;
            matched += 1;
            continue;
        }
        if file_hydration.relative_path_normalized.contains(token) {
            bonus += 16.0;
            matched += 1;
            continue;
        }
        if heading_norm.contains(token) {
            bonus += 12.0;
            matched += 1;
        }
    }

    if !query_tokens.is_empty() {
        let coverage = (matched as f64) / (query_tokens.len() as f64);
        bonus += coverage * 80.0;
    }

    if let Some(order) = heading_order {
        let bounded = order.max(0) as f64;
        bonus += 28.0 / (1.0 + bounded.sqrt());
    }

    bonus
}

#[allow(clippy::too_many_arguments)]
fn hydrate_hits(
    connection: &Connection,
    root_id: i64,
    normalized_query: &str,
    query_tokens: &[String],
    candidates: &[CandidateHit],
    limit: usize,
) -> CommandResult<Vec<SearchHit>> {
    let candidate_file_ids = collect_candidate_file_ids(candidates, candidates.len());
    let shared_to_local =
        resolve_local_file_ids_for_root(connection, root_id, &candidate_file_ids)?;

    let remap_candidate = |candidate: CandidateHit| {
        let remapped_file_id = shared_to_local
            .get(&candidate.file_id)
            .copied()
            .unwrap_or(candidate.file_id);
        let remapped_source_row_id = if matches!(candidate.kind, CandidateKind::File) {
            remapped_file_id
        } else {
            candidate.source_row_id
        };
        CandidateHit {
            file_id: remapped_file_id,
            source_row_id: remapped_source_row_id,
            ..candidate
        }
    };

    let mapped_candidates = candidates
        .iter()
        .copied()
        .map(remap_candidate)
        .collect::<Vec<CandidateHit>>();

    let file_ids = collect_candidate_file_ids(&mapped_candidates, mapped_candidates.len());
    let file_rows = fetch_file_hydration(connection, &file_ids)?;

    let mut heading_row_ids = Vec::new();
    let mut author_row_ids = Vec::new();
    let mut chunk_row_ids = Vec::new();
    let mut seen_heading_ids = HashSet::new();
    let mut seen_author_ids = HashSet::new();
    let mut seen_chunk_ids = HashSet::new();

    for candidate in &mapped_candidates {
        match candidate.kind {
            CandidateKind::Heading => {
                if seen_heading_ids.insert(candidate.source_row_id) {
                    heading_row_ids.push(candidate.source_row_id);
                }
            }
            CandidateKind::Author => {
                if seen_author_ids.insert(candidate.source_row_id) {
                    author_row_ids.push(candidate.source_row_id);
                }
            }
            CandidateKind::Chunk => {
                if seen_chunk_ids.insert(candidate.source_row_id) {
                    chunk_row_ids.push(candidate.source_row_id);
                }
            }
            CandidateKind::File => {}
        }
    }

    let heading_rows = fetch_heading_hydration(connection, &heading_row_ids)?;
    let author_rows = fetch_author_hydration(connection, &author_row_ids)?;
    let chunk_rows = fetch_chunk_hydration(connection, &chunk_row_ids)?;

    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for candidate in &mapped_candidates {
        if results.len() >= limit {
            break;
        }

        let Some(file_row) = file_rows.get(&candidate.file_id) else {
            continue;
        };

        let (kind, heading_level, heading_text, heading_order) = match candidate.kind {
            CandidateKind::File => ("file".to_string(), None, None, None),
            CandidateKind::Heading => {
                let Some(order) = candidate.heading_order else {
                    continue;
                };
                let Some(heading_row) = heading_rows.get(&candidate.source_row_id) else {
                    continue;
                };
                (
                    "heading".to_string(),
                    Some(heading_row.heading_level),
                    Some(heading_row.heading_text.clone()),
                    Some(order),
                )
            }
            CandidateKind::Author => {
                let Some(order) = candidate.heading_order else {
                    continue;
                };
                let Some(author_row) = author_rows.get(&candidate.source_row_id) else {
                    continue;
                };
                (
                    "author".to_string(),
                    None,
                    Some(author_row.author_text.clone()),
                    Some(order),
                )
            }
            CandidateKind::Chunk => {
                let Some(_chunk_order) = candidate.chunk_order else {
                    continue;
                };
                let Some(chunk_row) = chunk_rows.get(&candidate.source_row_id) else {
                    continue;
                };
                let chunk_text = preview_text_for_chunk(&chunk_row.chunk_text);
                (
                    "heading".to_string(),
                    chunk_row.heading_level,
                    chunk_row
                        .heading_text
                        .clone()
                        .or_else(|| chunk_row.author_text.clone())
                        .or_else(|| {
                            if chunk_text.is_empty() {
                                None
                            } else {
                                Some(chunk_text)
                            }
                        }),
                    chunk_row.heading_order.or(candidate.heading_order),
                )
            }
        };

        let bonus = rerank_bonus(
            normalized_query,
            query_tokens,
            file_row,
            heading_text.as_deref(),
            heading_order,
        );

        let hit = SearchHit {
            source: "lexical".to_string(),
            kind,
            file_id: candidate.file_id,
            file_name: file_row.file_name.clone(),
            relative_path: file_row.relative_path.clone(),
            absolute_path: file_row.absolute_path.clone(),
            heading_level,
            heading_text,
            heading_order,
            score: candidate.score - bonus,
        };

        let key = dedupe_key(&hit);
        if !seen.insert(key) {
            continue;
        }

        results.push(hit);
    }

    results.sort_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(Ordering::Equal)
            .then(left.relative_path.cmp(&right.relative_path))
            .then(
                left.heading_order
                    .unwrap_or(0)
                    .cmp(&right.heading_order.unwrap_or(0)),
            )
            .then(left.kind.cmp(&right.kind))
    });
    results.truncate(limit);

    Ok(results)
}

fn search_for_root(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
    normalized: &str,
    limit: usize,
    file_name_only: bool,
) -> CommandResult<(Vec<SearchHit>, LexicalSearchTelemetry)> {
    ensure_root_shard_populated(app, connection, root_id)?;

    let root_runtime = root_runtime(app, root_id)?;

    let query_tokens = sorted_unique_tokens(normalized);
    if query_tokens.is_empty() {
        return Ok((Vec::new(), LexicalSearchTelemetry::default()));
    }

    let target_limit = limit.clamp(10, 400);
    let fetch_limit = target_limit
        .saturating_mul(MIN_FETCH_MULTIPLIER)
        .clamp(MIN_FETCH_FLOOR, MAX_FETCH_LIMIT);

    let token_count = query_tokens.len();
    let has_short_token = query_tokens.iter().any(|token| token.len() <= 2);
    let low_specificity = token_count <= 1 && normalized.chars().count() <= 3;

    let metadata_fetch_limit = fetch_limit.min(500);
    let prefix_fetch_limit = fetch_limit
        .saturating_mul(3)
        .saturating_div(4)
        .clamp(MIN_FETCH_FLOOR, 380);
    let chunk_fetch_limit = target_limit.saturating_mul(4).clamp(80, 260);
    let ngram_fetch_limit = target_limit.saturating_mul(2).clamp(40, 120);

    let mut candidates = Vec::new();
    let mut seen_candidates = HashSet::new();
    let mut telemetry = LexicalSearchTelemetry::default();

    let meta_searcher = root_runtime.meta_reader.searcher();
    run_meta_tier(
        &meta_searcher,
        &root_runtime.meta_fields,
        &query_tokens,
        true,
        metadata_fetch_limit,
        1_000.0_f64,
        file_name_only,
        "meta_exact",
        target_limit.saturating_mul(3),
        &mut telemetry,
        &mut candidates,
        &mut seen_candidates,
    )?;

    if !file_name_only && normalized.chars().count() >= 4 {
        run_meta_prefix_tier(
            &meta_searcher,
            &root_runtime.meta_fields,
            &query_tokens,
            true,
            prefix_fetch_limit,
            2_000.0_f64,
            file_name_only,
            "meta_prefix",
            target_limit.saturating_mul(3),
            &mut telemetry,
            &mut candidates,
            &mut seen_candidates,
        )?;
    }

    let chunk_searcher = root_runtime.chunk_reader.searcher();
    if !file_name_only && !low_specificity && candidates.len() < target_limit.saturating_mul(2) {
        telemetry
            .fallbacks_triggered
            .push("chunk_exact".to_string());
        let candidate_file_ids = collect_candidate_file_ids(&candidates, 120);
        run_chunk_tier(
            &chunk_searcher,
            &root_runtime.chunk_fields,
            &query_tokens,
            root_runtime.chunk_fields.query_text,
            true,
            chunk_fetch_limit,
            2_500.0_f64,
            "chunk_exact",
            target_limit.saturating_mul(3),
            &candidate_file_ids,
            &mut telemetry,
            &mut candidates,
            &mut seen_candidates,
        )?;
    }

    let wildcard_tokens = prefix_wildcard_tokens(&query_tokens);
    if !file_name_only
        && !wildcard_tokens.is_empty()
        && token_count <= 3
        && !has_short_token
        && candidates.len() < target_limit
    {
        telemetry
            .fallbacks_triggered
            .push("meta_prefix_wildcard".to_string());
        run_meta_prefix_tier(
            &meta_searcher,
            &root_runtime.meta_fields,
            &wildcard_tokens,
            false,
            target_limit.saturating_mul(2).clamp(40, 140),
            2_250.0_f64,
            file_name_only,
            "meta_prefix_wildcard",
            target_limit.saturating_mul(3),
            &mut telemetry,
            &mut candidates,
            &mut seen_candidates,
        )?;
    }

    if !file_name_only
        && !low_specificity
        && normalized.chars().count() >= 8
        && token_count <= 4
        && !has_short_token
        && candidates.len() < target_limit
    {
        let grams = ngram_tokens(normalized);
        if !grams.is_empty() {
            telemetry
                .fallbacks_triggered
                .push("chunk_ngram".to_string());
            let candidate_file_ids = collect_candidate_file_ids(&candidates, 140);
            run_chunk_tier(
                &chunk_searcher,
                &root_runtime.chunk_fields,
                &grams,
                root_runtime.chunk_fields.ngram_text,
                false,
                ngram_fetch_limit,
                3_000.0_f64,
                "chunk_ngram",
                target_limit.saturating_mul(3),
                &candidate_file_ids,
                &mut telemetry,
                &mut candidates,
                &mut seen_candidates,
            )?;
        }
    }

    let hydrate_started = Instant::now();
    let mut results = hydrate_hits(
        connection,
        root_id,
        normalized,
        &query_tokens,
        &candidates,
        target_limit,
    )?;
    telemetry.doc_fetch_ms += hydrate_started.elapsed().as_secs_f64() * 1000.0;

    // Single-root query quality guard: if chunk-only matches dominate with no metadata signal,
    // keep top slice deterministic and cheap.
    if file_name_only {
        results.retain(|hit| hit.kind == "file");
        results.truncate(target_limit);
    }

    Ok((results, telemetry))
}

fn all_root_ids(connection: &Connection) -> CommandResult<Vec<i64>> {
    let mut statement = connection
        .prepare("SELECT id FROM roots ORDER BY id ASC")
        .map_err(|error| format!("Could not prepare root id query: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not query root ids: {error}"))?;

    let mut output = Vec::new();
    for row in rows {
        output.push(row.map_err(|error| format!("Could not parse root id row: {error}"))?);
    }

    Ok(output)
}

fn merge_telemetry(target: &mut LexicalSearchTelemetry, incoming: &LexicalSearchTelemetry) {
    for (tier, value) in &incoming.tier_timings_ms {
        *target.tier_timings_ms.entry(tier.clone()).or_insert(0.0) += value;
    }
    for (tier, value) in &incoming.tier_hit_counts {
        *target.tier_hit_counts.entry(tier.clone()).or_insert(0) += value;
    }
    target.doc_fetch_ms += incoming.doc_fetch_ms;
    for fallback in &incoming.fallbacks_triggered {
        if !target.fallbacks_triggered.contains(fallback) {
            target.fallbacks_triggered.push(fallback.clone());
        }
    }
}

pub(crate) fn search_with_telemetry(
    app: &AppHandle,
    query: &str,
    requested_root_id: Option<i64>,
    limit: usize,
    file_name_only: bool,
) -> CommandResult<(Vec<SearchHit>, LexicalSearchTelemetry)> {
    let started = Instant::now();
    let normalized = normalize_for_search(query);
    if normalized.is_empty() {
        return Ok((Vec::new(), LexicalSearchTelemetry::default()));
    }

    let connection = open_database(app)?;

    if let Some(root_id) = requested_root_id {
        let (results, telemetry) = search_for_root(
            app,
            &connection,
            root_id,
            &normalized,
            limit,
            file_name_only,
        )?;
        if started.elapsed().as_millis() > 80 {
            eprintln!(
                "Lexical search exceeded 80ms budget: {}ms query='{}'",
                started.elapsed().as_millis(),
                normalized
            );
        }
        return Ok((results, telemetry));
    }

    let all_roots = all_root_ids(&connection)?;
    let root_ids = if all_roots.len() <= ROOT_EXHAUSTIVE_THRESHOLD {
        all_roots
    } else {
        let shortlisted = shortlist_root_ids(app, &connection, &normalized)?;
        if shortlisted.is_empty() {
            all_roots
        } else {
            shortlisted
        }
    };
    let mut merged_hits = Vec::new();
    let mut merged_telemetry = LexicalSearchTelemetry::default();

    for root_id in root_ids {
        let (mut hits, telemetry) = search_for_root(
            app,
            &connection,
            root_id,
            &normalized,
            limit,
            file_name_only,
        )?;
        merge_telemetry(&mut merged_telemetry, &telemetry);
        merged_hits.append(&mut hits);
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for hit in merged_hits {
        let key = dedupe_key(&hit);
        if !seen.insert(key) {
            continue;
        }
        deduped.push(hit);
    }

    deduped.sort_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(Ordering::Equal)
            .then(left.relative_path.cmp(&right.relative_path))
            .then(
                left.heading_order
                    .unwrap_or(0)
                    .cmp(&right.heading_order.unwrap_or(0)),
            )
            .then(left.kind.cmp(&right.kind))
    });
    deduped.truncate(limit.clamp(10, 400));

    if started.elapsed().as_millis() > 80 {
        eprintln!(
            "Lexical search exceeded 80ms budget: {}ms query='{}'",
            started.elapsed().as_millis(),
            normalized
        );
    }

    Ok((deduped, merged_telemetry))
}

pub(crate) fn search(
    app: &AppHandle,
    query: &str,
    requested_root_id: Option<i64>,
    limit: usize,
    file_name_only: bool,
) -> CommandResult<Vec<SearchHit>> {
    let (results, _) = search_with_telemetry(app, query, requested_root_id, limit, file_name_only)?;
    Ok(results)
}

pub(crate) fn root_meta_doc_count(app: &AppHandle, root_id: i64) -> CommandResult<u64> {
    if root_id <= 0 {
        return Ok(0);
    }

    let root_runtime = root_runtime(app, root_id)?;

    let count = root_runtime
        .meta_reader
        .searcher()
        .search(&tantivy::query::AllQuery, &Count)
        .map_err(|error| format!("Could not count lexical meta docs for root: {error}"))?;
    Ok(u64::try_from(count).unwrap_or(0))
}
