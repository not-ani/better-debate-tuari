use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, Schema, TextFieldIndexing, TextOptions, Value,
    STORED, STRING,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{doc, Index, IndexReader, ReloadPolicy, TantivyDocument, Term};

use crate::db::{index_lexical_dir, index_manifests_dir, index_payload_dir, open_database};
use crate::docx_parse::parse_docx_cards;
use crate::runtime::AppHandle;
use crate::search::normalize_for_search;
use crate::types::{
    HighlightSpan, HydratedSearchResult, ParsedCard, PlannerMode, SearchCandidateCounts,
    SearchCardParagraph, SearchCardPayload, SearchDiagnostics, SearchDocPayload, SearchEntityType,
    SearchFilters, SearchHydrateRequest, SearchHydrateResponse, SearchIndexStatus, SearchLatencyMs,
    SearchMode, SearchOutlineEntry, SearchRequest, SearchResponse, SearchResult, SearchWarmResult,
    SearchWarning, SemanticInstallStatus, SemanticStatus,
};
use crate::util::{file_name_from_relative, now_ms, path_display};
use crate::CommandResult;

const SEARCH_INDEX_DIR_NAME: &str = "global";
const SEARCH_MANIFEST_FILE_NAME: &str = "search.json";
const PAYLOAD_BIN_FILE_NAME: &str = "result_rows.bin";
const PAYLOAD_INDEX_FILE_NAME: &str = "result_rows.idx";
const PREFIX_TOKENIZER: &str = "search_prefix";
const RESCUE_TOKENIZER: &str = "search_rescue";
const WRITER_HEAP_BYTES: usize = 96 * 1024 * 1024;
const RESULT_FETCH_LIMIT_EXACT: usize = 100;
const RESULT_FETCH_LIMIT_BM25: usize = 250;
const RESULT_FETCH_LIMIT_PREFIX: usize = 80;
const RESULT_FETCH_LIMIT_RESCUE: usize = 80;
const VERBATIM_SYNONYMS: &str = include_str!("../resources/synonyms.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SearchStage {
    Exact,
    Bm25f,
    PrefixRescue,
    ChargramRescue,
}

#[derive(Clone)]
struct SearchFields {
    result_id: Field,
    root_id: Field,
    root_path: Field,
    entity_type: Field,
    file_id: Field,
    heading_level: Field,
    heading_order: Field,
    modified_ms: Field,
    cite_date_sort: Field,
    file_name_terms: Field,
    path_terms: Field,
    tag_terms: Field,
    tag_sub_terms: Field,
    highlighted_terms: Field,
    cite_terms: Field,
    body_terms: Field,
    prefix_terms: Field,
    rescue_terms: Field,
    file_name: Field,
    relative_path: Field,
    absolute_path: Field,
    heading_text: Field,
    cite: Field,
    cite_date: Field,
    snippet: Field,
    outline_json: Field,
    payload_json: Field,
    normalized_file_name: Field,
    normalized_path: Field,
    normalized_tag: Field,
    normalized_highlighted: Field,
    normalized_cite: Field,
    normalized_body: Field,
}

struct SearchRuntime {
    index: Index,
    reader: IndexReader,
    fields: SearchFields,
    writer_lock: Mutex<()>,
}

#[derive(Clone)]
struct StoredRecord {
    result: SearchResult,
    normalized_file_name: String,
    normalized_path: String,
    normalized_tag: String,
    normalized_tag_sub: String,
    normalized_highlighted: String,
    normalized_cite: String,
    normalized_body: String,
    payload_json: String,
    modified_ms: i64,
    cite_date_sort: i64,
}

#[derive(Clone)]
struct Candidate {
    record: StoredRecord,
    best_bm25: f64,
    matched_stages: HashSet<SearchStage>,
}

struct SearchPlan {
    planner_mode: PlannerMode,
    allow_prefix_rescue: bool,
    allow_chargram_rescue: bool,
    prefer_path: bool,
    prefer_names: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnsureReadyState {
    Ready,
    PendingRebuild,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchManifest {
    updated_at_ms: i64,
    doc_count: usize,
    #[serde(default)]
    pending: bool,
    engine: String,
}

type SynonymMap = HashMap<String, Vec<String>>;

static SEARCH_RUNTIME: OnceLock<Mutex<Option<Arc<SearchRuntime>>>> = OnceLock::new();
static SYNONYM_MAP: OnceLock<SynonymMap> = OnceLock::new();
static PAYLOAD_INDEX_CACHE: OnceLock<Mutex<Option<HashMap<i64, (u64, u64)>>>> = OnceLock::new();
static SEARCH_REBUILD_IN_FLIGHT: OnceLock<Mutex<bool>> = OnceLock::new();

fn synonym_map() -> &'static SynonymMap {
    SYNONYM_MAP.get_or_init(load_synonym_map)
}

fn search_runtime_cell() -> &'static Mutex<Option<Arc<SearchRuntime>>> {
    SEARCH_RUNTIME.get_or_init(|| Mutex::new(None))
}

fn payload_index_cache() -> &'static Mutex<Option<HashMap<i64, (u64, u64)>>> {
    PAYLOAD_INDEX_CACHE.get_or_init(|| Mutex::new(None))
}

fn search_rebuild_in_flight() -> &'static Mutex<bool> {
    SEARCH_REBUILD_IN_FLIGHT.get_or_init(|| Mutex::new(false))
}

fn search_index_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_lexical_dir(app)?.join(SEARCH_INDEX_DIR_NAME))
}

fn manifest_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_manifests_dir(app)?.join(SEARCH_MANIFEST_FILE_NAME))
}

fn read_manifest(app: &AppHandle) -> CommandResult<Option<SearchManifest>> {
    let path = manifest_path(app)?;
    if !path.is_file() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Could not read search manifest '{}': {error}",
            path_display(&path)
        )
    })?;
    let manifest = serde_json::from_str::<SearchManifest>(&raw).map_err(|error| {
        format!(
            "Could not parse search manifest '{}': {error}",
            path_display(&path)
        )
    })?;
    Ok(Some(manifest))
}

fn indexed_text_options(tokenizer: &str, stored: bool) -> TextOptions {
    let options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    if stored {
        options.set_stored()
    } else {
        options
    }
}

fn stored_text_options() -> TextOptions {
    TextOptions::default().set_stored()
}

fn build_schema() -> Schema {
    let numeric = NumericOptions::default()
        .set_fast()
        .set_stored()
        .set_indexed();
    let mut builder = Schema::builder();

    builder.add_u64_field("result_id", numeric.clone());
    builder.add_u64_field("root_id", numeric.clone());
    builder.add_text_field("root_path", STRING | STORED);
    builder.add_text_field("entity_type", STRING | STORED);
    builder.add_u64_field("file_id", numeric.clone());
    builder.add_i64_field("heading_level", numeric.clone());
    builder.add_i64_field("heading_order", numeric.clone());
    builder.add_i64_field("modified_ms", numeric.clone());
    builder.add_i64_field("cite_date_sort", numeric.clone());

    builder.add_text_field("file_name_terms", indexed_text_options("default", false));
    builder.add_text_field("path_terms", indexed_text_options("default", false));
    builder.add_text_field("tag_terms", indexed_text_options("default", false));
    builder.add_text_field("tag_sub_terms", indexed_text_options("default", false));
    builder.add_text_field("highlighted_terms", indexed_text_options("default", false));
    builder.add_text_field("cite_terms", indexed_text_options("default", false));
    builder.add_text_field("body_terms", indexed_text_options("default", false));
    builder.add_text_field(
        "prefix_terms",
        indexed_text_options(PREFIX_TOKENIZER, false),
    );
    builder.add_text_field(
        "rescue_terms",
        indexed_text_options(RESCUE_TOKENIZER, false),
    );

    builder.add_text_field("file_name", stored_text_options());
    builder.add_text_field("relative_path", stored_text_options());
    builder.add_text_field("absolute_path", stored_text_options());
    builder.add_text_field("heading_text", stored_text_options());
    builder.add_text_field("cite", stored_text_options());
    builder.add_text_field("cite_date", stored_text_options());
    builder.add_text_field("snippet", stored_text_options());
    builder.add_text_field("outline_json", stored_text_options());
    builder.add_text_field("payload_json", stored_text_options());
    builder.add_text_field("normalized_file_name", stored_text_options());
    builder.add_text_field("normalized_path", stored_text_options());
    builder.add_text_field("normalized_tag", stored_text_options());
    builder.add_text_field("normalized_highlighted", stored_text_options());
    builder.add_text_field("normalized_cite", stored_text_options());
    builder.add_text_field("normalized_body", stored_text_options());

    builder.build()
}

fn search_fields(schema: &Schema) -> CommandResult<SearchFields> {
    Ok(SearchFields {
        result_id: schema
            .get_field("result_id")
            .map_err(|e| format!("Missing result_id field: {e}"))?,
        root_id: schema
            .get_field("root_id")
            .map_err(|e| format!("Missing root_id field: {e}"))?,
        root_path: schema
            .get_field("root_path")
            .map_err(|e| format!("Missing root_path field: {e}"))?,
        entity_type: schema
            .get_field("entity_type")
            .map_err(|e| format!("Missing entity_type field: {e}"))?,
        file_id: schema
            .get_field("file_id")
            .map_err(|e| format!("Missing file_id field: {e}"))?,
        heading_level: schema
            .get_field("heading_level")
            .map_err(|e| format!("Missing heading_level field: {e}"))?,
        heading_order: schema
            .get_field("heading_order")
            .map_err(|e| format!("Missing heading_order field: {e}"))?,
        modified_ms: schema
            .get_field("modified_ms")
            .map_err(|e| format!("Missing modified_ms field: {e}"))?,
        cite_date_sort: schema
            .get_field("cite_date_sort")
            .map_err(|e| format!("Missing cite_date_sort field: {e}"))?,
        file_name_terms: schema
            .get_field("file_name_terms")
            .map_err(|e| format!("Missing file_name_terms field: {e}"))?,
        path_terms: schema
            .get_field("path_terms")
            .map_err(|e| format!("Missing path_terms field: {e}"))?,
        tag_terms: schema
            .get_field("tag_terms")
            .map_err(|e| format!("Missing tag_terms field: {e}"))?,
        tag_sub_terms: schema
            .get_field("tag_sub_terms")
            .map_err(|e| format!("Missing tag_sub_terms field: {e}"))?,
        highlighted_terms: schema
            .get_field("highlighted_terms")
            .map_err(|e| format!("Missing highlighted_terms field: {e}"))?,
        cite_terms: schema
            .get_field("cite_terms")
            .map_err(|e| format!("Missing cite_terms field: {e}"))?,
        body_terms: schema
            .get_field("body_terms")
            .map_err(|e| format!("Missing body_terms field: {e}"))?,
        prefix_terms: schema
            .get_field("prefix_terms")
            .map_err(|e| format!("Missing prefix_terms field: {e}"))?,
        rescue_terms: schema
            .get_field("rescue_terms")
            .map_err(|e| format!("Missing rescue_terms field: {e}"))?,
        file_name: schema
            .get_field("file_name")
            .map_err(|e| format!("Missing file_name field: {e}"))?,
        relative_path: schema
            .get_field("relative_path")
            .map_err(|e| format!("Missing relative_path field: {e}"))?,
        absolute_path: schema
            .get_field("absolute_path")
            .map_err(|e| format!("Missing absolute_path field: {e}"))?,
        heading_text: schema
            .get_field("heading_text")
            .map_err(|e| format!("Missing heading_text field: {e}"))?,
        cite: schema
            .get_field("cite")
            .map_err(|e| format!("Missing cite field: {e}"))?,
        cite_date: schema
            .get_field("cite_date")
            .map_err(|e| format!("Missing cite_date field: {e}"))?,
        snippet: schema
            .get_field("snippet")
            .map_err(|e| format!("Missing snippet field: {e}"))?,
        outline_json: schema
            .get_field("outline_json")
            .map_err(|e| format!("Missing outline_json field: {e}"))?,
        payload_json: schema
            .get_field("payload_json")
            .map_err(|e| format!("Missing payload_json field: {e}"))?,
        normalized_file_name: schema
            .get_field("normalized_file_name")
            .map_err(|e| format!("Missing normalized_file_name field: {e}"))?,
        normalized_path: schema
            .get_field("normalized_path")
            .map_err(|e| format!("Missing normalized_path field: {e}"))?,
        normalized_tag: schema
            .get_field("normalized_tag")
            .map_err(|e| format!("Missing normalized_tag field: {e}"))?,
        normalized_highlighted: schema
            .get_field("normalized_highlighted")
            .map_err(|e| format!("Missing normalized_highlighted field: {e}"))?,
        normalized_cite: schema
            .get_field("normalized_cite")
            .map_err(|e| format!("Missing normalized_cite field: {e}"))?,
        normalized_body: schema
            .get_field("normalized_body")
            .map_err(|e| format!("Missing normalized_body field: {e}"))?,
    })
}

fn register_tokenizers(index: &Index) -> CommandResult<()> {
    let prefix_tokenizer = NgramTokenizer::new(2, 18, true)
        .map_err(|e| format!("Could not build search prefix tokenizer: {e}"))?;
    let rescue_tokenizer = NgramTokenizer::new(3, 3, false)
        .map_err(|e| format!("Could not build search rescue tokenizer: {e}"))?;

    index.tokenizers().register(
        PREFIX_TOKENIZER,
        TextAnalyzer::builder(prefix_tokenizer)
            .filter(LowerCaser)
            .build(),
    );
    index.tokenizers().register(
        RESCUE_TOKENIZER,
        TextAnalyzer::builder(rescue_tokenizer)
            .filter(LowerCaser)
            .build(),
    );
    Ok(())
}

fn recreate_index(index_dir: &PathBuf) -> CommandResult<Index> {
    if index_dir.exists() {
        fs::remove_dir_all(index_dir).map_err(|error| {
            format!(
                "Could not remove stale search index '{}': {error}",
                path_display(index_dir)
            )
        })?;
    }
    fs::create_dir_all(index_dir).map_err(|error| {
        format!(
            "Could not create search index dir '{}': {error}",
            path_display(index_dir)
        )
    })?;
    let index = Index::create_in_dir(index_dir, build_schema())
        .map_err(|error| format!("Could not create search index: {error}"))?;
    register_tokenizers(&index)?;
    Ok(index)
}

fn open_or_create_index(index_dir: &PathBuf) -> CommandResult<Index> {
    if !index_dir.exists() {
        return recreate_index(index_dir);
    }
    let index = Index::open_in_dir(index_dir).map_err(|_| "schema reset needed".to_string());
    match index {
        Ok(index) => {
            register_tokenizers(&index)?;
            if search_fields(&index.schema()).is_err() {
                recreate_index(index_dir)
            } else {
                Ok(index)
            }
        }
        Err(_) => recreate_index(index_dir),
    }
}

fn search_runtime(app: &AppHandle) -> CommandResult<Arc<SearchRuntime>> {
    let mut runtime = search_runtime_cell()
        .lock()
        .map_err(|_| "Could not lock search runtime".to_string())?;
    if let Some(existing) = runtime.as_ref() {
        return Ok(Arc::clone(existing));
    }

    let index = open_or_create_index(&search_index_dir(app)?)?;
    let fields = search_fields(&index.schema())?;
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .map_err(|error| format!("Could not create search index reader: {error}"))?;
    let built = Arc::new(SearchRuntime {
        index,
        reader,
        fields,
        writer_lock: Mutex::new(()),
    });
    *runtime = Some(Arc::clone(&built));
    Ok(built)
}

fn field_text(document: &TantivyDocument, field: Field) -> String {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn field_u64(document: &TantivyDocument, field: Field) -> Option<u64> {
    document.get_first(field).and_then(|value| value.as_u64())
}

fn field_i64(document: &TantivyDocument, field: Field) -> Option<i64> {
    document.get_first(field).and_then(|value| value.as_i64())
}

fn semantic_resource_installed(app: &AppHandle, file_name: &str) -> bool {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join(file_name));
        candidates.push(resource_dir.join("resources").join(file_name));
    }
    let manifest_resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    candidates.push(manifest_resources.join(file_name));
    candidates.into_iter().any(|path| path.exists())
}

fn semantic_index_pending(connection: &Connection) -> CommandResult<bool> {
    let root_count = connection
        .query_row("SELECT COUNT(*) FROM roots", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not count roots for semantic status: {error}"))?;
    if root_count <= 0 {
        return Ok(false);
    }

    let stale_count = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM roots r
            LEFT JOIN semantic_root_state s ON s.root_id = r.id
            WHERE s.root_id IS NULL
               OR s.item_count <= 0
               OR s.embedding_dim <= 0
               OR s.last_indexed_ms < r.last_indexed_ms
            ",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not count stale semantic roots: {error}"))?;
    Ok(stale_count > 0)
}

fn semantic_install_status_for_app(app: &AppHandle) -> SemanticInstallStatus {
    let installed = semantic_resource_installed(app, "model.onnx")
        && semantic_resource_installed(app, "tokenizer.json");
    let status = if !installed {
        SemanticStatus::Unavailable
    } else {
        match open_database(app).and_then(|connection| semantic_index_pending(&connection)) {
            Ok(false) => SemanticStatus::Ready,
            Ok(true) | Err(_) => SemanticStatus::Stale,
        }
    };
    SemanticInstallStatus {
        installed,
        model_name: installed.then_some("bge-small-en-v1.5".to_string()),
        status,
    }
}

fn build_plan(raw_query: &str, normalized_query: &str, mode: SearchMode) -> SearchPlan {
    let token_count = normalized_query.split_whitespace().count();
    let has_path_markers =
        raw_query.contains('/') || raw_query.contains('\\') || raw_query.contains('.');
    let looks_like_identifier = normalized_query
        .chars()
        .any(|character| character.is_ascii_digit())
        && normalized_query.chars().count() <= 40;
    let quoted = raw_query.contains('"');
    let avg_token_len = if token_count == 0 {
        0.0
    } else {
        normalized_query
            .split_whitespace()
            .map(|token| token.chars().count() as f64)
            .sum::<f64>()
            / token_count as f64
    };
    let prefer_names = token_count <= 4
        && avg_token_len >= 4.0
        && !normalized_query
            .split_whitespace()
            .any(|token| token.len() <= 2);

    let planner_mode = if looks_like_identifier {
        PlannerMode::ExactIdLike
    } else if has_path_markers {
        PlannerMode::PathLike
    } else if quoted {
        PlannerMode::PhraseLike
    } else if prefer_names {
        PlannerMode::NameLike
    } else if normalized_query.split_whitespace().count() >= 5 {
        if mode == SearchMode::Mixed {
            PlannerMode::LongMixed
        } else {
            PlannerMode::PhraseLike
        }
    } else if normalized_query.chars().count() <= 18 && token_count <= 3 {
        PlannerMode::ShortKeyword
    } else {
        PlannerMode::LongMixed
    };

    SearchPlan {
        planner_mode,
        allow_prefix_rescue: !matches!(planner_mode, PlannerMode::ExactIdLike),
        allow_chargram_rescue: matches!(
            planner_mode,
            PlannerMode::ShortKeyword
                | PlannerMode::ExactIdLike
                | PlannerMode::PathLike
                | PlannerMode::NameLike
        ),
        prefer_path: matches!(
            planner_mode,
            PlannerMode::PathLike | PlannerMode::ExactIdLike
        ),
        prefer_names: matches!(planner_mode, PlannerMode::NameLike),
    }
}

fn unique_tokens(normalized: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tokens = Vec::new();
    for token in normalized.split_whitespace() {
        if seen.insert(token.to_string()) {
            tokens.push(token.to_string());
        }
    }
    tokens
}

fn chargrams(normalized: &str) -> Vec<String> {
    let compact = normalized.replace(' ', "");
    let chars = compact.chars().collect::<Vec<char>>();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut grams = Vec::new();
    for start in 0..=chars.len().saturating_sub(3) {
        let gram = chars[start..start + 3].iter().collect::<String>();
        if seen.insert(gram.clone()) {
            grams.push(gram);
        }
    }
    grams
}

fn prefix_tokens(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|token| {
            if token.len() >= 4 {
                Some(token[..token.len() - 1].to_string())
            } else {
                None
            }
        })
        .collect()
}

fn load_synonym_map() -> SynonymMap {
    let mut synonyms = HashMap::<String, Vec<String>>::new();
    for line in VERBATIM_SYNONYMS.lines() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let values = raw
            .split(',')
            .map(normalize_for_search)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        for value in &values {
            let mut peers = values
                .iter()
                .filter(|candidate| *candidate != value)
                .cloned()
                .collect::<Vec<_>>();
            peers.truncate(6);
            synonyms.insert(value.clone(), peers);
        }
    }
    synonyms
}

fn token_groups_for_segment(segment: &str) -> Vec<Vec<String>> {
    let tokens = unique_tokens(segment);
    tokens
        .into_iter()
        .map(|token| {
            let mut variants = vec![token.clone()];
            if let Some(synonyms) = synonym_map().get(&token) {
                variants.extend(synonyms.iter().cloned());
            }
            variants.truncate(6);
            variants.sort();
            variants.dedup();
            variants
        })
        .collect()
}

fn quoted_segments(raw_query: &str) -> (Vec<String>, Vec<String>) {
    let mut unquoted = Vec::new();
    let mut quoted = Vec::new();
    for (index, part) in raw_query.split('"').enumerate() {
        let normalized = normalize_for_search(part);
        if normalized.is_empty() {
            continue;
        }
        if index % 2 == 0 {
            unquoted.push(normalized);
        } else {
            quoted.push(normalized);
        }
    }
    (unquoted, quoted)
}

fn build_group_query(
    weighted_fields: &[(Field, f32)],
    groups: &[Vec<String>],
    conjunction: bool,
) -> Option<Box<dyn Query>> {
    if weighted_fields.is_empty() || groups.is_empty() {
        return None;
    }

    let mut clauses = Vec::new();
    for group in groups {
        let mut group_clauses = Vec::new();
        for variant in group {
            for (field, boost) in weighted_fields {
                let query = TermQuery::new(
                    Term::from_field_text(*field, variant),
                    IndexRecordOption::WithFreqs,
                );
                group_clauses.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(query), *boost)) as Box<dyn Query>,
                ));
            }
        }
        if group_clauses.is_empty() {
            continue;
        }
        clauses.push((
            if conjunction {
                Occur::Must
            } else {
                Occur::Should
            },
            Box::new(BooleanQuery::new(group_clauses)) as Box<dyn Query>,
        ));
    }

    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn tokenize_phrase(phrase: &str) -> Vec<String> {
    unique_tokens(phrase)
}

fn build_phrase_query(fields: &[Field], phrase: &str, boost: f32) -> Option<Box<dyn Query>> {
    let tokens = tokenize_phrase(phrase);
    if tokens.len() < 2 {
        return None;
    }
    let mut clauses = Vec::new();
    for field in fields {
        let terms = tokens
            .iter()
            .map(|token| Term::from_field_text(*field, token))
            .collect::<Vec<_>>();
        let query = PhraseQuery::new(terms);
        clauses.push((
            Occur::Should,
            Box::new(BoostQuery::new(Box::new(query), boost)) as Box<dyn Query>,
        ));
    }
    Some(Box::new(BooleanQuery::new(clauses)))
}

fn build_filter_query(
    fields: &SearchFields,
    root_paths: &[String],
    filters: Option<&SearchFilters>,
) -> Option<Box<dyn Query>> {
    let mut clauses = Vec::new();
    if !root_paths.is_empty() {
        let root_terms = root_paths
            .iter()
            .map(|path| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        Term::from_field_text(fields.root_path, path),
                        IndexRecordOption::Basic,
                    )) as Box<dyn Query>,
                )
            })
            .collect::<Vec<_>>();
        clauses.push((
            Occur::Must,
            Box::new(BooleanQuery::new(root_terms)) as Box<dyn Query>,
        ));
    }

    let entity_types = if filters
        .and_then(|value| value.file_name_only)
        .unwrap_or(false)
    {
        Some(vec![SearchEntityType::Doc])
    } else {
        filters.and_then(|value| value.entity_types.clone())
    };

    if let Some(entity_types) = entity_types {
        let mut entity_clauses = Vec::new();
        for entity_type in entity_types {
            let label = match entity_type {
                SearchEntityType::Doc => "doc",
                SearchEntityType::Card => "card",
            };
            entity_clauses.push((
                Occur::Should,
                Box::new(TermQuery::new(
                    Term::from_field_text(fields.entity_type, label),
                    IndexRecordOption::Basic,
                )) as Box<dyn Query>,
            ));
        }
        clauses.push((
            Occur::Must,
            Box::new(BooleanQuery::new(entity_clauses)) as Box<dyn Query>,
        ));
    }

    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn parse_outline(raw: &str) -> Vec<SearchOutlineEntry> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn stored_record_from_doc(document: &TantivyDocument, fields: &SearchFields) -> StoredRecord {
    let entity_type = if field_text(document, fields.entity_type) == "doc" {
        SearchEntityType::Doc
    } else {
        SearchEntityType::Card
    };
    let heading_text = field_text(document, fields.heading_text);
    let cite = field_text(document, fields.cite);
    let cite_date = field_text(document, fields.cite_date);
    let snippet = field_text(document, fields.snippet);
    let outline_json = field_text(document, fields.outline_json);

    let result = SearchResult {
        result_id: i64::try_from(field_u64(document, fields.result_id).unwrap_or(0)).unwrap_or(0),
        entity_type,
        root_path: field_text(document, fields.root_path),
        file_id: i64::try_from(field_u64(document, fields.file_id).unwrap_or(0)).unwrap_or(0),
        file_name: field_text(document, fields.file_name),
        relative_path: field_text(document, fields.relative_path),
        absolute_path: field_text(document, fields.absolute_path),
        heading_text: (!heading_text.is_empty()).then_some(heading_text),
        heading_level: field_i64(document, fields.heading_level).filter(|value| *value > 0),
        heading_order: field_i64(document, fields.heading_order).filter(|value| *value > 0),
        cite: (!cite.is_empty()).then_some(cite),
        cite_date: (!cite_date.is_empty()).then_some(cite_date),
        outline_path: parse_outline(&outline_json),
        snippet: (!snippet.is_empty()).then_some(snippet),
        highlights: Vec::new(),
        score: 0.0,
        source: "lexical".to_string(),
        kind: if entity_type == SearchEntityType::Doc {
            "file".to_string()
        } else {
            "heading".to_string()
        },
    };

    StoredRecord {
        result,
        normalized_file_name: field_text(document, fields.normalized_file_name),
        normalized_path: field_text(document, fields.normalized_path),
        normalized_tag: field_text(document, fields.normalized_tag),
        normalized_tag_sub: String::new(),
        normalized_highlighted: field_text(document, fields.normalized_highlighted),
        normalized_cite: field_text(document, fields.normalized_cite),
        normalized_body: field_text(document, fields.normalized_body),
        payload_json: field_text(document, fields.payload_json),
        modified_ms: field_i64(document, fields.modified_ms).unwrap_or(0),
        cite_date_sort: field_i64(document, fields.cite_date_sort).unwrap_or(0),
    }
}

fn preview_text(text: &str) -> String {
    let compact = text.trim();
    if compact.chars().count() <= 220 {
        return compact.to_string();
    }
    compact
        .chars()
        .take(220)
        .collect::<String>()
        .trim()
        .to_string()
}

fn highlight_span(text: &str, normalized_query: &str, field: &str) -> Option<HighlightSpan> {
    if text.is_empty() || normalized_query.is_empty() {
        return None;
    }
    let lowered = normalize_for_search(text);
    lowered.find(normalized_query).map(|start| HighlightSpan {
        field: field.to_string(),
        start,
        end: start + normalized_query.len(),
    })
}

fn rerank_candidate(
    candidate: &Candidate,
    normalized_query: &str,
    query_tokens: &[String],
    quoted_phrases: &[String],
    plan: &SearchPlan,
) -> (f64, Vec<HighlightSpan>) {
    let record = &candidate.record;
    let tag = record.normalized_tag.as_str();
    let highlighted = record.normalized_highlighted.as_str();
    let cite = record.normalized_cite.as_str();
    let body = record.normalized_body.as_str();
    let path = record.normalized_path.as_str();
    let file_name = record.normalized_file_name.as_str();

    let mut score = candidate.best_bm25 * 8.0;

    if candidate.matched_stages.contains(&SearchStage::Exact) {
        score += 14.0;
    }
    if candidate
        .matched_stages
        .contains(&SearchStage::PrefixRescue)
    {
        score -= 1.0;
    }
    if candidate
        .matched_stages
        .contains(&SearchStage::ChargramRescue)
        && !candidate.matched_stages.contains(&SearchStage::Exact)
    {
        score -= 4.0;
    }

    if [tag, highlighted, cite, body, path, file_name]
        .iter()
        .any(|value| !value.is_empty() && value.contains(normalized_query))
    {
        score += 28.0;
    }

    for phrase in quoted_phrases {
        if tag.contains(phrase) {
            score += 20.0;
        }
        if highlighted.contains(phrase) {
            score += 16.0;
        }
        if cite.contains(phrase) {
            score += 16.0;
        }
        if body.contains(phrase) {
            score += 12.0;
        }
    }

    let mut matched = 0_usize;
    let mut rare_matched = 0_usize;
    for token in query_tokens {
        let in_file = file_name.contains(token);
        let in_path = path.contains(token);
        let in_tag = tag.contains(token);
        let in_highlighted = highlighted.contains(token);
        let in_cite = cite.contains(token);
        let in_body = body.contains(token);
        if in_file || in_path || in_tag || in_highlighted || in_cite || in_body {
            matched += 1;
            if token.len() >= 7 {
                rare_matched += 1;
            }
        }
        if in_tag {
            score += 8.0;
        } else if in_highlighted {
            score += 6.0;
        } else if in_cite {
            score += 6.0;
        } else if in_file {
            score += 5.0;
        } else if in_path {
            score += 4.0;
        } else if in_body {
            score += 2.5;
        }
    }

    if !query_tokens.is_empty() {
        score += (matched as f64 / query_tokens.len() as f64) * 22.0;
        let rare_total = query_tokens.iter().filter(|token| token.len() >= 7).count();
        if rare_total > 0 {
            score += (rare_matched as f64 / rare_total as f64) * 10.0;
        }
    }

    if plan.prefer_path && (path.contains(normalized_query) || file_name.contains(normalized_query))
    {
        score += 12.0;
    }
    if plan.prefer_names && (!tag.is_empty() || !cite.is_empty()) {
        score += 4.0;
    }

    if candidate.record.result.entity_type == SearchEntityType::Card {
        score += 3.0;
        if let Some(level) = candidate.record.result.heading_level {
            score += (7_i64.saturating_sub(level.clamp(1, 6))) as f64 * 1.1;
        }
    } else if !plan.prefer_path {
        score -= 3.0;
    }

    score += (candidate.record.modified_ms.max(0) as f64) / 1_000_000_000_000.0;

    let mut highlights = Vec::new();
    if let Some(span) = highlight_span(
        &candidate.record.result.file_name,
        normalized_query,
        "fileName",
    ) {
        highlights.push(span);
    }
    if let Some(text) = candidate.record.result.heading_text.as_deref() {
        if let Some(span) = highlight_span(text, normalized_query, "headingText") {
            highlights.push(span);
        }
    }
    if let Some(text) = candidate.record.result.cite.as_deref() {
        if let Some(span) = highlight_span(text, normalized_query, "cite") {
            highlights.push(span);
        }
    }
    if let Some(text) = candidate.record.result.snippet.as_deref() {
        if let Some(span) = highlight_span(text, normalized_query, "snippet") {
            highlights.push(span);
        }
    }

    (score, highlights)
}

fn filtered_out(result: &SearchResult, filters: Option<&SearchFilters>) -> bool {
    let Some(filters) = filters else {
        return false;
    };
    if let Some(path_prefixes) = filters.path_prefixes.as_ref() {
        if !path_prefixes.is_empty()
            && !path_prefixes
                .iter()
                .any(|prefix| result.relative_path.starts_with(prefix))
        {
            return true;
        }
    }
    if let Some(from) = filters.cite_date_from.as_deref() {
        if let Some(cite_date) = result.cite_date.as_deref() {
            if cite_date < from {
                return true;
            }
        }
    }
    if let Some(to) = filters.cite_date_to.as_deref() {
        if let Some(cite_date) = result.cite_date.as_deref() {
            if cite_date > to {
                return true;
            }
        }
    }
    false
}

fn execute_stage(
    searcher: &tantivy::Searcher,
    fields: &SearchFields,
    stage: SearchStage,
    query: Box<dyn Query>,
    filter_query: Option<Box<dyn Query>>,
    fetch_limit: usize,
    candidates: &mut HashMap<i64, Candidate>,
) -> CommandResult<usize> {
    let final_query: Box<dyn Query> = if let Some(filter_query) = filter_query {
        Box::new(BooleanQuery::new(vec![
            (Occur::Must, query),
            (Occur::Must, filter_query),
        ]))
    } else {
        query
    };
    let docs = searcher
        .search(&final_query, &TopDocs::with_limit(fetch_limit))
        .map_err(|error| format!("Search stage execution failed: {error}"))?;
    let mut accepted = 0_usize;
    for (bm25, address) in docs {
        let document = searcher
            .doc::<TantivyDocument>(address)
            .map_err(|error| format!("Could not read search candidate: {error}"))?;
        let record = stored_record_from_doc(&document, fields);
        let entry = candidates
            .entry(record.result.result_id)
            .or_insert_with(|| Candidate {
                record: record.clone(),
                best_bm25: f64::from(bm25),
                matched_stages: HashSet::new(),
            });
        entry.best_bm25 = entry.best_bm25.max(f64::from(bm25));
        entry.record = record;
        entry.matched_stages.insert(stage);
        accepted += 1;
    }
    Ok(accepted)
}

fn is_lock_busy_error(error: &str) -> bool {
    error.contains("LockBusy") || error.contains("Failed to acquire index lock")
}

fn indexed_file_count(connection: &Connection) -> CommandResult<usize> {
    let count = connection
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not count indexed files for search: {error}"))?;
    Ok(usize::try_from(count.max(0)).unwrap_or(0))
}

fn search_doc_count(runtime: &SearchRuntime) -> CommandResult<usize> {
    let count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| format!("Could not count search docs: {error}"))?;
    Ok(count)
}

fn search_index_incomplete(
    connection: &Connection,
    runtime: &SearchRuntime,
    manifest: Option<&SearchManifest>,
) -> CommandResult<bool> {
    let indexed_files = indexed_file_count(connection)?;
    if indexed_files == 0 {
        return Ok(false);
    }

    let doc_count = search_doc_count(runtime)?;
    if doc_count < indexed_files {
        return Ok(true);
    }

    Ok(manifest
        .map(|value| value.pending)
        .unwrap_or(doc_count == 0))
}

fn ensure_ready(app: &AppHandle) -> CommandResult<EnsureReadyState> {
    let runtime = search_runtime(app)?;
    let manifest = read_manifest(app)?;
    let connection = open_database(app)?;
    if !search_index_incomplete(&connection, &runtime, manifest.as_ref())? {
        return Ok(EnsureReadyState::Ready);
    }

    let indexed_files = indexed_file_count(&connection)?;
    if indexed_files == 0 {
        write_manifest(app, 0)?;
        return Ok(EnsureReadyState::Ready);
    }

    request_background_rebuild(app.clone());
    Ok(EnsureReadyState::PendingRebuild)
}

fn entity_label(entity_type: SearchEntityType) -> &'static str {
    match entity_type {
        SearchEntityType::Doc => "doc",
        SearchEntityType::Card => "card",
    }
}

fn store_document(fields: &SearchFields, record: &StoredRecord, root_id: i64) -> TantivyDocument {
    doc!(
        fields.result_id => u64::try_from(record.result.result_id.max(0)).unwrap_or(0),
        fields.root_id => u64::try_from(root_id.max(0)).unwrap_or(0),
        fields.root_path => record.result.root_path.clone(),
        fields.entity_type => entity_label(record.result.entity_type).to_string(),
        fields.file_id => u64::try_from(record.result.file_id.max(0)).unwrap_or(0),
        fields.heading_level => record.result.heading_level.unwrap_or(0),
        fields.heading_order => record.result.heading_order.unwrap_or(0),
        fields.modified_ms => record.modified_ms,
        fields.cite_date_sort => record.cite_date_sort,
        fields.file_name_terms => record.normalized_file_name.clone(),
        fields.path_terms => record.normalized_path.clone(),
        fields.tag_terms => record.normalized_tag.clone(),
        fields.tag_sub_terms => record.normalized_tag_sub.clone(),
        fields.highlighted_terms => record.normalized_highlighted.clone(),
        fields.cite_terms => record.normalized_cite.clone(),
        fields.body_terms => record.normalized_body.clone(),
        fields.prefix_terms => format!("{} {} {}", record.normalized_file_name, record.normalized_path, record.normalized_tag),
        fields.rescue_terms => format!(
            "{} {} {} {} {} {}",
            record.normalized_file_name,
            record.normalized_path,
            record.normalized_tag,
            record.normalized_tag_sub,
            record.normalized_cite,
            preview_text(&record.normalized_body)
        ),
        fields.file_name => record.result.file_name.clone(),
        fields.relative_path => record.result.relative_path.clone(),
        fields.absolute_path => record.result.absolute_path.clone(),
        fields.heading_text => record.result.heading_text.clone().unwrap_or_default(),
        fields.cite => record.result.cite.clone().unwrap_or_default(),
        fields.cite_date => record.result.cite_date.clone().unwrap_or_default(),
        fields.snippet => record.result.snippet.clone().unwrap_or_default(),
        fields.outline_json => serde_json::to_string(&record.result.outline_path).unwrap_or_else(|_| "[]".to_string()),
        fields.payload_json => record.payload_json.clone(),
        fields.normalized_file_name => record.normalized_file_name.clone(),
        fields.normalized_path => record.normalized_path.clone(),
        fields.normalized_tag => record.normalized_tag.clone(),
        fields.normalized_highlighted => record.normalized_highlighted.clone(),
        fields.normalized_cite => record.normalized_cite.clone(),
        fields.normalized_body => record.normalized_body.clone(),
    )
}

fn doc_result_id(file_id: i64) -> i64 {
    ((file_id as u64) << 32) as i64
}

fn card_result_id(file_id: i64, heading_order: i64) -> i64 {
    ((((file_id as u64) << 32) | (heading_order.max(1) as u64)) as i64).saturating_add(1)
}

fn load_root_path(connection: &Connection, root_id: i64) -> CommandResult<String> {
    connection
        .query_row(
            "SELECT path FROM roots WHERE id = ?1",
            params![root_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Could not load root path for search sync: {error}"))
}

fn payload_bin_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_payload_dir(app)?.join(PAYLOAD_BIN_FILE_NAME))
}

fn payload_index_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(index_payload_dir(app)?.join(PAYLOAD_INDEX_FILE_NAME))
}

fn load_file_metadata(
    connection: &Connection,
    root_id: i64,
    file_id: i64,
) -> CommandResult<Option<(String, String, i64)>> {
    connection
        .query_row(
            "SELECT relative_path, absolute_path, modified_ms FROM files WHERE root_id = ?1 AND id = ?2",
            params![root_id, file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        )
        .optional()
        .map_err(|error| format!("Could not load file metadata for search: {error}"))
}

fn load_headings(connection: &Connection, file_id: i64) -> CommandResult<Vec<SearchOutlineEntry>> {
    let mut statement = connection
        .prepare("SELECT heading_order, level, text FROM headings WHERE file_id = ?1 ORDER BY heading_order ASC")
        .map_err(|error| format!("Could not prepare headings query: {error}"))?;
    let rows = statement
        .query_map(params![file_id], |row| {
            Ok(SearchOutlineEntry {
                order: row.get::<_, i64>(0)?,
                level: row.get::<_, i64>(1)?,
                text: row.get::<_, String>(2)?,
            })
        })
        .map_err(|error| format!("Could not execute headings query: {error}"))?;
    let mut headings = Vec::new();
    for row in rows {
        headings.push(row.map_err(|error| format!("Could not parse heading row: {error}"))?);
    }
    Ok(headings)
}

fn heading_trail(headings: &[SearchOutlineEntry], heading_order: i64) -> Vec<SearchOutlineEntry> {
    let mut trail = Vec::new();
    for heading in headings
        .iter()
        .filter(|heading| heading.order <= heading_order)
    {
        while trail
            .last()
            .map(|last: &SearchOutlineEntry| last.level >= heading.level)
            .unwrap_or(false)
        {
            trail.pop();
        }
        trail.push(heading.clone());
    }
    trail
}

fn cite_date_sort(value: Option<&str>) -> i64 {
    value
        .map(|date| date.replace('-', ""))
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(0)
}

fn fallback_card_records(
    connection: &Connection,
    root_id: i64,
    root_path: &str,
    file_id: i64,
    relative_path: &str,
    absolute_path: &str,
    modified_ms: i64,
    headings: &[SearchOutlineEntry],
) -> CommandResult<Vec<StoredRecord>> {
    let file_name = file_name_from_relative(relative_path);
    let normalized_file_name = normalize_for_search(&file_name);
    let normalized_path = normalize_for_search(relative_path);
    let mut statement = connection
        .prepare(
            "SELECT heading_level, heading_order, heading_text, author_text, chunk_text FROM chunks WHERE root_id = ?1 AND file_id = ?2 ORDER BY chunk_order ASC",
        )
        .map_err(|error| format!("Could not prepare fallback chunk query: {error}"))?;
    let rows = statement
        .query_map(params![root_id, file_id], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| format!("Could not execute fallback chunk query: {error}"))?;

    let mut records = Vec::new();
    for row in rows {
        let (heading_level, heading_order, heading_text, author_text, chunk_text) =
            row.map_err(|error| format!("Could not parse fallback chunk row: {error}"))?;
        let outline_path = heading_order
            .map(|order| heading_trail(headings, order))
            .unwrap_or_default();
        let snippet = preview_text(&chunk_text);
        let cite = author_text.unwrap_or_default();
        let payload = HydratedSearchResult::Card(SearchCardPayload {
            result_id: card_result_id(file_id, heading_order.unwrap_or(1)),
            entity_type: SearchEntityType::Card,
            root_path: root_path.to_string(),
            file_id,
            file_name: file_name.clone(),
            relative_path: relative_path.to_string(),
            absolute_path: absolute_path.to_string(),
            card_id: format!("fallback:{file_id}:{}", heading_order.unwrap_or(1)),
            tag: heading_text.clone().unwrap_or_else(|| file_name.clone()),
            tag_sub: String::new(),
            cite: cite.clone(),
            cite_date: None,
            heading_order: heading_order.unwrap_or(1),
            heading_level: heading_level.unwrap_or(1),
            heading_trail: outline_path.clone(),
            body: vec![SearchCardParagraph {
                paragraph_index: 0,
                text: chunk_text.clone(),
                spans: Vec::new(),
            }],
        });
        let payload_json = serde_json::to_string(&payload)
            .map_err(|error| format!("Could not serialize fallback payload: {error}"))?;

        records.push(StoredRecord {
            result: SearchResult {
                result_id: card_result_id(file_id, heading_order.unwrap_or(1)),
                entity_type: SearchEntityType::Card,
                root_path: root_path.to_string(),
                file_id,
                file_name: file_name.clone(),
                relative_path: relative_path.to_string(),
                absolute_path: absolute_path.to_string(),
                heading_text: heading_text.clone(),
                heading_level,
                heading_order,
                cite: (!cite.is_empty()).then_some(cite.clone()),
                cite_date: None,
                outline_path,
                snippet: (!snippet.is_empty()).then_some(snippet),
                highlights: Vec::new(),
                score: 0.0,
                source: "lexical".to_string(),
                kind: "heading".to_string(),
            },
            normalized_file_name: normalized_file_name.clone(),
            normalized_path: normalized_path.clone(),
            normalized_tag: normalize_for_search(
                heading_text.as_deref().unwrap_or(file_name.as_str()),
            ),
            normalized_tag_sub: String::new(),
            normalized_highlighted: String::new(),
            normalized_cite: normalize_for_search(&cite),
            normalized_body: normalize_for_search(&chunk_text),
            payload_json,
            modified_ms,
            cite_date_sort: 0,
        });
    }
    Ok(records)
}

fn load_doc_record(
    connection: &Connection,
    root_id: i64,
    root_path: &str,
    file_id: i64,
) -> CommandResult<Option<StoredRecord>> {
    let Some((relative_path, absolute_path, modified_ms)) =
        load_file_metadata(connection, root_id, file_id)?
    else {
        return Ok(None);
    };
    let headings = load_headings(connection, file_id)?;
    let file_name = file_name_from_relative(&relative_path);
    let normalized_file_name = normalize_for_search(&file_name);
    let normalized_path = normalize_for_search(&relative_path);
    let payload = HydratedSearchResult::Doc(SearchDocPayload {
        result_id: doc_result_id(file_id),
        entity_type: SearchEntityType::Doc,
        root_path: root_path.to_string(),
        file_id,
        file_name: file_name.clone(),
        relative_path: relative_path.clone(),
        absolute_path: absolute_path.clone(),
        headings: headings.clone(),
    });
    let payload_json = serde_json::to_string(&payload)
        .map_err(|error| format!("Could not serialize doc payload: {error}"))?;

    Ok(Some(StoredRecord {
        result: SearchResult {
            result_id: doc_result_id(file_id),
            entity_type: SearchEntityType::Doc,
            root_path: root_path.to_string(),
            file_id,
            file_name,
            relative_path,
            absolute_path,
            heading_text: headings.first().map(|heading| heading.text.clone()),
            heading_level: None,
            heading_order: None,
            cite: None,
            cite_date: None,
            outline_path: headings.clone(),
            snippet: headings
                .first()
                .map(|heading| preview_text(&heading.text))
                .filter(|value| !value.is_empty()),
            highlights: Vec::new(),
            score: 0.0,
            source: "lexical".to_string(),
            kind: "file".to_string(),
        },
        normalized_file_name,
        normalized_path,
        normalized_tag: String::new(),
        normalized_tag_sub: String::new(),
        normalized_highlighted: String::new(),
        normalized_cite: String::new(),
        normalized_body: normalize_for_search(
            &headings
                .iter()
                .map(|heading| heading.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        payload_json,
        modified_ms,
        cite_date_sort: 0,
    }))
}

fn parsed_card_to_payload(
    root_path: &str,
    file_id: i64,
    file_name: &str,
    relative_path: &str,
    absolute_path: &str,
    card: &ParsedCard,
    headings: &[SearchOutlineEntry],
) -> SearchCardPayload {
    let trail = heading_trail(headings, card.heading_order);
    SearchCardPayload {
        result_id: card_result_id(file_id, card.heading_order),
        entity_type: SearchEntityType::Card,
        root_path: root_path.to_string(),
        file_id,
        file_name: file_name.to_string(),
        relative_path: relative_path.to_string(),
        absolute_path: absolute_path.to_string(),
        card_id: format!("{file_id}:{}", card.heading_order),
        tag: card.tag.clone(),
        tag_sub: card.tag_sub.clone(),
        cite: card.cite.clone(),
        cite_date: card.cite_date.clone(),
        heading_order: card.heading_order,
        heading_level: card.heading_level,
        heading_trail: trail,
        body: card
            .body
            .iter()
            .enumerate()
            .map(|(index, paragraph)| SearchCardParagraph {
                paragraph_index: index,
                text: paragraph.text.clone(),
                spans: paragraph.spans.clone(),
            })
            .collect(),
    }
}

fn load_card_records(
    connection: &Connection,
    root_id: i64,
    root_path: &str,
    file_id: i64,
) -> CommandResult<Vec<StoredRecord>> {
    let Some((relative_path, absolute_path, modified_ms)) =
        load_file_metadata(connection, root_id, file_id)?
    else {
        return Ok(Vec::new());
    };
    let headings = load_headings(connection, file_id)?;
    let file_name = file_name_from_relative(&relative_path);
    let normalized_file_name = normalize_for_search(&file_name);
    let normalized_path = normalize_for_search(&relative_path);
    let parsed_cards = parse_docx_cards(Path::new(&absolute_path)).unwrap_or_default();
    if parsed_cards.is_empty() {
        return fallback_card_records(
            connection,
            root_id,
            root_path,
            file_id,
            &relative_path,
            &absolute_path,
            modified_ms,
            &headings,
        );
    }

    let mut records = Vec::new();
    for card in parsed_cards {
        let payload = parsed_card_to_payload(
            root_path,
            file_id,
            &file_name,
            &relative_path,
            &absolute_path,
            &card,
            &headings,
        );
        let payload_json = serde_json::to_string(&HydratedSearchResult::Card(payload.clone()))
            .map_err(|error| format!("Could not serialize card payload: {error}"))?;
        let body_text = card
            .body
            .iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let snippet = preview_text(&format!("{} {}", card.cite, body_text));
        records.push(StoredRecord {
            result: SearchResult {
                result_id: payload.result_id,
                entity_type: SearchEntityType::Card,
                root_path: root_path.to_string(),
                file_id,
                file_name: file_name.clone(),
                relative_path: relative_path.clone(),
                absolute_path: absolute_path.clone(),
                heading_text: Some(card.tag.clone()),
                heading_level: Some(card.heading_level),
                heading_order: Some(card.heading_order),
                cite: Some(card.cite.clone()),
                cite_date: card.cite_date.clone(),
                outline_path: payload.heading_trail.clone(),
                snippet: (!snippet.is_empty()).then_some(snippet),
                highlights: Vec::new(),
                score: 0.0,
                source: "lexical".to_string(),
                kind: "heading".to_string(),
            },
            normalized_file_name: normalized_file_name.clone(),
            normalized_path: normalized_path.clone(),
            normalized_tag: normalize_for_search(&card.tag),
            normalized_tag_sub: normalize_for_search(&card.tag_sub),
            normalized_highlighted: normalize_for_search(&card.highlighted_text),
            normalized_cite: normalize_for_search(&card.cite),
            normalized_body: normalize_for_search(&format!("{} {}", card.tag_sub, body_text)),
            payload_json,
            modified_ms,
            cite_date_sort: cite_date_sort(card.cite_date.as_deref()),
        });
    }

    Ok(records)
}

fn write_manifest(app: &AppHandle, doc_count: usize) -> CommandResult<()> {
    write_manifest_with_state(app, doc_count, false)
}

fn write_manifest_with_state(
    app: &AppHandle,
    doc_count: usize,
    pending: bool,
) -> CommandResult<()> {
    let path = manifest_path(app)?;
    let payload = SearchManifest {
        updated_at_ms: now_ms(),
        doc_count,
        pending,
        engine: "logos-local-card-search".to_string(),
    };
    fs::write(
        &path,
        serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("Could not serialize search manifest: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "Could not write search manifest '{}': {error}",
            path_display(&path)
        )
    })
}

fn invalidate_payload_cache() {
    if let Ok(mut writer) = payload_index_cache().lock() {
        *writer = None;
    }
}

fn write_payload_sidecar(app: &AppHandle, runtime: &SearchRuntime) -> CommandResult<()> {
    let _ = (app, runtime);
    invalidate_payload_cache();
    Ok(())
}

fn read_payload_json(app: &AppHandle, result_id: i64) -> CommandResult<Option<String>> {
    let _ = (app, result_id);
    Ok(None)
}

pub(crate) fn rebuild_all_from_connection(
    app: &AppHandle,
    connection: &Connection,
) -> CommandResult<()> {
    let runtime = search_runtime(app)?;
    let _guard = runtime
        .writer_lock
        .lock()
        .map_err(|_| "Could not lock search writer".to_string())?;
    let mut writer: tantivy::IndexWriter<TantivyDocument> = runtime
        .index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|error| format!("Could not open search writer: {error}"))?;
    writer
        .delete_all_documents()
        .map_err(|error| format!("Could not clear search index before rebuild: {error}"))?;

    let mut statement = connection
        .prepare("SELECT id FROM roots ORDER BY id ASC")
        .map_err(|error| format!("Could not prepare search root rebuild query: {error}"))?;
    let root_rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not execute search root rebuild query: {error}"))?;

    for root_row in root_rows {
        let root_id =
            root_row.map_err(|error| format!("Could not parse root rebuild row: {error}"))?;
        let root_path = load_root_path(connection, root_id)?;
        let mut file_statement = connection
            .prepare("SELECT id FROM files WHERE root_id = ?1 ORDER BY id ASC")
            .map_err(|error| format!("Could not prepare search file rebuild query: {error}"))?;
        let file_rows = file_statement
            .query_map(params![root_id], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("Could not execute search file rebuild query: {error}"))?;
        for file_row in file_rows {
            let file_id =
                file_row.map_err(|error| format!("Could not parse file rebuild row: {error}"))?;
            if let Some(doc_record) = load_doc_record(connection, root_id, &root_path, file_id)? {
                writer
                    .add_document(store_document(&runtime.fields, &doc_record, root_id))
                    .map_err(|error| format!("Could not add search doc row: {error}"))?;
            }
            for record in load_card_records(connection, root_id, &root_path, file_id)? {
                writer
                    .add_document(store_document(&runtime.fields, &record, root_id))
                    .map_err(|error| format!("Could not add search card row: {error}"))?;
            }
        }
    }

    writer
        .commit()
        .map_err(|error| format!("Could not commit search rebuild: {error}"))?;
    runtime
        .reader
        .reload()
        .map_err(|error| format!("Could not reload search reader after rebuild: {error}"))?;
    let doc_count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| format!("Could not count search docs after rebuild: {error}"))?;
    write_manifest(app, doc_count)
}

pub(crate) fn apply_file_changes_from_connection(
    app: &AppHandle,
    connection: &Connection,
    root_id: i64,
    updated_file_ids: &[i64],
    removed_file_ids: &[i64],
) -> CommandResult<()> {
    let runtime = search_runtime(app)?;
    let manifest = read_manifest(app)?;
    if search_index_incomplete(connection, &runtime, manifest.as_ref())? {
        return rebuild_all_from_connection(app, connection);
    }
    let _guard = runtime
        .writer_lock
        .lock()
        .map_err(|_| "Could not lock search writer".to_string())?;
    let mut writer: tantivy::IndexWriter<TantivyDocument> = runtime
        .index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|error| format!("Could not open search writer: {error}"))?;

    let mut touched = false;
    for file_id in updated_file_ids.iter().chain(removed_file_ids.iter()) {
        writer.delete_term(Term::from_field_u64(
            runtime.fields.file_id,
            u64::try_from(*file_id).unwrap_or(0),
        ));
        touched = true;
    }

    if !updated_file_ids.is_empty() {
        let root_path = load_root_path(connection, root_id)?;
        for file_id in updated_file_ids {
            if let Some(doc_record) = load_doc_record(connection, root_id, &root_path, *file_id)? {
                writer
                    .add_document(store_document(&runtime.fields, &doc_record, root_id))
                    .map_err(|error| format!("Could not add search doc update: {error}"))?;
            }
            for record in load_card_records(connection, root_id, &root_path, *file_id)? {
                writer
                    .add_document(store_document(&runtime.fields, &record, root_id))
                    .map_err(|error| format!("Could not add search card update: {error}"))?;
            }
        }
    }

    if !touched {
        return Ok(());
    }

    writer
        .commit()
        .map_err(|error| format!("Could not commit search updates: {error}"))?;
    runtime
        .reader
        .reload()
        .map_err(|error| format!("Could not reload search reader after updates: {error}"))?;
    let doc_count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| format!("Could not count search docs after update: {error}"))?;
    write_manifest(app, doc_count)
}

pub(crate) fn remove_root_documents(app: &AppHandle, root_path: &str) -> CommandResult<()> {
    let runtime = search_runtime(app)?;
    let _guard = runtime
        .writer_lock
        .lock()
        .map_err(|_| "Could not lock search writer".to_string())?;
    let mut writer: tantivy::IndexWriter<TantivyDocument> = runtime
        .index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|error| format!("Could not open search writer: {error}"))?;
    writer.delete_term(Term::from_field_text(runtime.fields.root_path, root_path));
    writer
        .commit()
        .map_err(|error| format!("Could not commit search root delete: {error}"))?;
    runtime
        .reader
        .reload()
        .map_err(|error| format!("Could not reload search reader after root delete: {error}"))?;
    let doc_count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| format!("Could not count search docs after root delete: {error}"))?;
    write_manifest(app, doc_count)
}

pub(crate) fn mark_pending_update(app: &AppHandle) -> CommandResult<()> {
    let runtime = search_runtime(app)?;
    let doc_count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| {
            format!("Could not count search docs while marking search index stale: {error}")
        })?;
    write_manifest_with_state(app, doc_count, true)
}

pub(crate) fn request_background_rebuild(app: AppHandle) {
    let Ok(mut in_flight) = search_rebuild_in_flight().lock() else {
        return;
    };
    if *in_flight {
        return;
    }
    *in_flight = true;
    drop(in_flight);

    crate::async_runtime::spawn_blocking(move || {
        let result = open_database(&app)
            .and_then(|connection| rebuild_all_from_connection(&app, &connection));
        if let Err(error) = result {
            eprintln!("[search] background rebuild failed: {error}");
        }
        if let Ok(mut in_flight) = search_rebuild_in_flight().lock() {
            *in_flight = false;
        }
    });
}

pub(crate) fn warm(app: &AppHandle) -> CommandResult<SearchWarmResult> {
    let ready_state = ensure_ready(app)?;
    let runtime = search_runtime(app)?;
    let doc_count = runtime
        .reader
        .searcher()
        .search(&AllQuery, &Count)
        .map_err(|error| format!("Could not count search docs during warm: {error}"))?;
    Ok(SearchWarmResult {
        ready: doc_count > 0 && ready_state == EnsureReadyState::Ready,
        doc_count,
    })
}

pub(crate) fn index_status(app: &AppHandle) -> CommandResult<SearchIndexStatus> {
    let runtime = search_runtime(app)?;
    let doc_count = search_doc_count(&runtime)?;
    let manifest = read_manifest(app)?;
    let connection = open_database(app)?;
    let pending = search_index_incomplete(&connection, &runtime, manifest.as_ref())?;
    Ok(SearchIndexStatus {
        layout_version: crate::db::INDEX_LAYOUT_VERSION,
        ready: doc_count > 0 && !pending,
        doc_count,
        semantic_status: semantic_install_status_for_app(app).status,
    })
}

pub(crate) fn optimize(app: &AppHandle) -> CommandResult<SearchWarmResult> {
    warm(app)
}

pub(crate) fn semantic_install_status(app: &AppHandle) -> SemanticInstallStatus {
    semantic_install_status_for_app(app)
}

fn exact_stage_query(
    runtime: &SearchRuntime,
    plan: &SearchPlan,
    unquoted_segments: &[String],
    quoted_phrases: &[String],
) -> Option<Box<dyn Query>> {
    let exact_fields = if plan.prefer_path {
        vec![
            (runtime.fields.path_terms, 4.0),
            (runtime.fields.file_name_terms, 3.6),
            (runtime.fields.tag_terms, 2.6),
            (runtime.fields.tag_sub_terms, 1.8),
        ]
    } else if plan.prefer_names {
        vec![
            (runtime.fields.tag_terms, 4.0),
            (runtime.fields.tag_sub_terms, 2.2),
            (runtime.fields.cite_terms, 3.0),
            (runtime.fields.highlighted_terms, 2.6),
            (runtime.fields.body_terms, 1.2),
            (runtime.fields.file_name_terms, 3.2),
            (runtime.fields.path_terms, 2.6),
        ]
    } else {
        vec![
            (runtime.fields.tag_terms, 4.0),
            (runtime.fields.tag_sub_terms, 2.2),
            (runtime.fields.highlighted_terms, 3.0),
            (runtime.fields.cite_terms, 3.0),
            (runtime.fields.body_terms, 1.0),
            (runtime.fields.file_name_terms, 1.5),
            (runtime.fields.path_terms, 1.2),
        ]
    };

    let mut clauses = Vec::new();
    for segment in unquoted_segments {
        let groups = token_groups_for_segment(segment);
        if let Some(query) = build_group_query(&exact_fields, &groups, true) {
            clauses.push((Occur::Must, query));
        }
    }
    for phrase in quoted_phrases {
        if let Some(query) = build_phrase_query(
            &[
                runtime.fields.tag_terms,
                runtime.fields.highlighted_terms,
                runtime.fields.cite_terms,
                runtime.fields.body_terms,
            ],
            phrase,
            4.0,
        ) {
            clauses.push((Occur::Must, query));
        }
    }
    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn bm25_stage_query(
    runtime: &SearchRuntime,
    plan: &SearchPlan,
    unquoted_segments: &[String],
) -> Option<Box<dyn Query>> {
    let fields = if plan.prefer_path {
        vec![
            (runtime.fields.file_name_terms, 4.2),
            (runtime.fields.path_terms, 4.0),
            (runtime.fields.tag_terms, 2.2),
            (runtime.fields.tag_sub_terms, 1.6),
            (runtime.fields.body_terms, 0.8),
        ]
    } else if plan.prefer_names {
        vec![
            (runtime.fields.tag_terms, 4.0),
            (runtime.fields.tag_sub_terms, 2.2),
            (runtime.fields.highlighted_terms, 3.0),
            (runtime.fields.cite_terms, 3.0),
            (runtime.fields.body_terms, 1.2),
            (runtime.fields.file_name_terms, 3.4),
            (runtime.fields.path_terms, 2.8),
        ]
    } else {
        vec![
            (runtime.fields.tag_terms, 4.0),
            (runtime.fields.tag_sub_terms, 2.2),
            (runtime.fields.highlighted_terms, 3.0),
            (runtime.fields.cite_terms, 3.0),
            (runtime.fields.body_terms, 1.2),
            (runtime.fields.file_name_terms, 1.8),
            (runtime.fields.path_terms, 1.4),
        ]
    };
    let mut clauses = Vec::new();
    for segment in unquoted_segments {
        let groups = token_groups_for_segment(segment);
        if let Some(query) = build_group_query(
            &fields,
            &groups,
            !matches!(plan.planner_mode, PlannerMode::LongMixed),
        ) {
            clauses.push((Occur::Must, query));
        }
    }
    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

pub(crate) fn search(app: &AppHandle, request: SearchRequest) -> CommandResult<SearchResponse> {
    let ready_state = ensure_ready(app)?;
    let started = Instant::now();
    let runtime = search_runtime(app)?;
    let searcher = runtime.reader.searcher();

    let normalized_query = normalize_for_search(&request.query);
    if normalized_query.is_empty() {
        return Ok(SearchResponse {
            results: Vec::new(),
            total_approx: Some(0),
            diagnostics: request
                .diagnostics
                .unwrap_or(false)
                .then_some(SearchDiagnostics {
                    planner_mode: PlannerMode::ShortKeyword,
                    latency_ms: SearchLatencyMs::default(),
                    candidate_counts: SearchCandidateCounts::default(),
                    semantic_status: Some(SemanticStatus::Unavailable),
                }),
            warnings: Vec::new(),
        });
    }

    let mode = request.mode.unwrap_or_default();
    let offset = request.offset.unwrap_or(0).min(5_000);
    let limit = request.limit.unwrap_or(50).clamp(1, 250);
    let filters = request.filters.as_ref();
    let root_paths = request.root_paths.clone().unwrap_or_default();
    let plan = build_plan(&request.query, &normalized_query, mode);
    let (unquoted_segments, quoted_phrases) = quoted_segments(&request.query);
    let query_tokens = unique_tokens(&normalized_query);

    let lexical_started = Instant::now();
    let mut candidates = HashMap::<i64, Candidate>::new();
    let mut counts = SearchCandidateCounts::default();

    if let Some(query) = exact_stage_query(&runtime, &plan, &unquoted_segments, &quoted_phrases) {
        counts.exact = execute_stage(
            &searcher,
            &runtime.fields,
            SearchStage::Exact,
            query,
            build_filter_query(&runtime.fields, &root_paths, filters),
            RESULT_FETCH_LIMIT_EXACT,
            &mut candidates,
        )?;
    }

    if let Some(query) = bm25_stage_query(&runtime, &plan, &unquoted_segments) {
        counts.bm25f = execute_stage(
            &searcher,
            &runtime.fields,
            SearchStage::Bm25f,
            query,
            build_filter_query(&runtime.fields, &root_paths, filters),
            RESULT_FETCH_LIMIT_BM25,
            &mut candidates,
        )?;
    }

    if plan.allow_prefix_rescue && counts.exact + counts.bm25f < 40 {
        let prefix_tokens = prefix_tokens(&query_tokens);
        let groups = prefix_tokens
            .into_iter()
            .map(|token| vec![token])
            .collect::<Vec<_>>();
        if let Some(query) = build_group_query(
            &[
                (runtime.fields.prefix_terms, 2.4),
                (runtime.fields.file_name_terms, 1.8),
                (runtime.fields.path_terms, 1.8),
            ],
            &groups,
            false,
        ) {
            counts.prefix_rescue = execute_stage(
                &searcher,
                &runtime.fields,
                SearchStage::PrefixRescue,
                query,
                build_filter_query(&runtime.fields, &root_paths, filters),
                RESULT_FETCH_LIMIT_PREFIX,
                &mut candidates,
            )?;
        }
    }

    if plan.allow_chargram_rescue && candidates.len() < 60 {
        let rescue_tokens = chargrams(&normalized_query)
            .into_iter()
            .map(|token| vec![token])
            .collect::<Vec<_>>();
        if let Some(query) =
            build_group_query(&[(runtime.fields.rescue_terms, 1.6)], &rescue_tokens, false)
        {
            counts.chargram_rescue = execute_stage(
                &searcher,
                &runtime.fields,
                SearchStage::ChargramRescue,
                query,
                build_filter_query(&runtime.fields, &root_paths, filters),
                RESULT_FETCH_LIMIT_RESCUE,
                &mut candidates,
            )?;
        }
    }

    let lexical_ms = lexical_started.elapsed().as_secs_f64() * 1000.0;
    let semantic_install = semantic_install_status_for_app(app);
    let semantic_status = if mode == SearchMode::Mixed {
        Some(semantic_install.status)
    } else {
        None
    };
    let mut warnings = match (mode, semantic_install.status) {
        (SearchMode::Mixed, SemanticStatus::Unavailable) => vec![SearchWarning {
            code: "semantic_unavailable".to_string(),
            message: "Mixed mode is using lexical-only results because semantic resources are not installed.".to_string(),
        }],
        (SearchMode::Mixed, SemanticStatus::Stale) => vec![SearchWarning {
            code: "semantic_stale".to_string(),
            message: "Semantic search is still building. Results are temporarily lexical-only until embeddings finish updating.".to_string(),
        }],
        _ => Vec::new(),
    };
    if ready_state == EnsureReadyState::PendingRebuild {
        warnings.push(SearchWarning {
            code: "index_busy".to_string(),
            message: "Search index is updating right now. Results may be temporarily incomplete until indexing finishes.".to_string(),
        });
    }

    let rerank_started = Instant::now();
    let mut ranked = candidates
        .into_values()
        .filter_map(|candidate| {
            if filtered_out(&candidate.record.result, filters) {
                return None;
            }
            let (score, highlights) = rerank_candidate(
                &candidate,
                &normalized_query,
                &query_tokens,
                &quoted_phrases,
                &plan,
            );
            let mut result = candidate.record.result.clone();
            result.score = score;
            result.highlights = highlights;
            result.source = "lexical".to_string();
            Some(result)
        })
        .collect::<Vec<_>>();
    counts.reranked = ranked.len();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then(left.relative_path.cmp(&right.relative_path))
            .then(
                left.heading_order
                    .unwrap_or(0)
                    .cmp(&right.heading_order.unwrap_or(0)),
            )
            .then(left.kind.cmp(&right.kind))
    });
    let rerank_ms = rerank_started.elapsed().as_secs_f64() * 1000.0;

    let total_approx = ranked.len();
    let results = ranked
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;

    Ok(SearchResponse {
        results,
        total_approx: Some(total_approx),
        diagnostics: request
            .diagnostics
            .unwrap_or(false)
            .then_some(SearchDiagnostics {
                planner_mode: plan.planner_mode,
                latency_ms: SearchLatencyMs {
                    total: total_ms,
                    lexical: lexical_ms,
                    semantic: None,
                    rerank: rerank_ms,
                    payload: 0.0,
                },
                candidate_counts: SearchCandidateCounts {
                    semantic: Some(0).filter(|_| mode == SearchMode::Mixed),
                    ..counts
                },
                semantic_status,
            }),
        warnings,
    })
}

pub(crate) fn hydrate_results(
    app: &AppHandle,
    request: SearchHydrateRequest,
) -> CommandResult<SearchHydrateResponse> {
    let _ = ensure_ready(app)?;
    let runtime = search_runtime(app)?;
    let searcher = runtime.reader.searcher();
    let mut results = Vec::new();

    for result_id in request.result_ids {
        if let Some(payload_json) = read_payload_json(app, result_id)? {
            let hydrated =
                serde_json::from_str::<HydratedSearchResult>(&payload_json).map_err(|error| {
                    format!("Could not decode hydrated search payload {result_id}: {error}")
                })?;
            results.push(hydrated);
            continue;
        }
        let query = TermQuery::new(
            Term::from_field_u64(
                runtime.fields.result_id,
                u64::try_from(result_id.max(0)).unwrap_or(0),
            ),
            IndexRecordOption::Basic,
        );
        let docs = searcher
            .search(&query, &TopDocs::with_limit(1))
            .map_err(|error| format!("Could not hydrate search result {result_id}: {error}"))?;
        let Some((_, address)) = docs.into_iter().next() else {
            continue;
        };
        let document = searcher.doc::<TantivyDocument>(address).map_err(|error| {
            format!("Could not read hydrated search result {result_id}: {error}")
        })?;
        let payload_json = field_text(&document, runtime.fields.payload_json);
        let hydrated =
            serde_json::from_str::<HydratedSearchResult>(&payload_json).map_err(|error| {
                format!("Could not decode hydrated search payload {result_id}: {error}")
            })?;
        results.push(hydrated);
    }

    Ok(SearchHydrateResponse { results })
}

#[cfg(test)]
mod tests {
    use super::{build_plan, PlannerMode, SearchMode};

    #[test]
    fn path_markers_use_raw_query() {
        let plan = build_plan(
            "Case Negs/Case Neg - NOAA - DDI 2025 HKL.docx",
            "case negs case neg noaa ddi 2025 hkl docx",
            SearchMode::Keyword,
        );
        assert_eq!(plan.planner_mode, PlannerMode::PathLike);
        assert!(plan.prefer_path);
    }

    #[test]
    fn quoted_queries_use_phrase_mode() {
        let plan = build_plan(
            "\"traditional ecological knowledge\"",
            "traditional ecological knowledge",
            SearchMode::Keyword,
        );
        assert_eq!(plan.planner_mode, PlannerMode::PhraseLike);
    }

    #[test]
    fn name_like_queries_keep_typo_rescue_enabled() {
        let plan = build_plan("identit", "identit", SearchMode::Keyword);
        assert_eq!(plan.planner_mode, PlannerMode::NameLike);
        assert!(plan.allow_chargram_rescue);
    }
}
