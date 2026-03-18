use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ort::session::Session as OrtSession;
use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootSummary {
    pub path: String,
    pub file_count: i64,
    pub heading_count: i64,
    pub added_at_ms: i64,
    pub last_indexed_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootIndexEntry {
    pub root_id: i64,
    pub root_path: String,
    pub folder_name: String,
    pub index_path: String,
    pub index_size_bytes: i64,
    pub file_count: i64,
    pub heading_count: i64,
    pub last_indexed_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexStats {
    pub scanned: usize,
    pub updated: usize,
    pub skipped: usize,
    pub removed: usize,
    pub headings_extracted: usize,
    pub elapsed_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderEntry {
    pub path: String,
    pub name: String,
    pub parent_path: Option<String>,
    pub depth: usize,
    pub file_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexedFile {
    pub id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub folder_path: String,
    pub modified_ms: i64,
    pub heading_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexSnapshot {
    pub root_path: String,
    pub indexed_at_ms: i64,
    pub folders: Vec<FolderEntry>,
    pub files: Vec<IndexedFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileHeading {
    pub id: i64,
    pub order: i64,
    pub level: i64,
    pub text: String,
    pub copy_text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaggedBlock {
    pub order: i64,
    pub style_label: String,
    pub text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilePreview {
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub heading_count: i64,
    pub headings: Vec<FileHeading>,
    pub f8_cites: Vec<TaggedBlock>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchHit {
    pub source: String,
    pub kind: String,
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub heading_level: Option<i64>,
    pub heading_text: Option<String>,
    pub heading_order: Option<i64>,
    pub score: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchMode {
    Keyword,
    Mixed,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Keyword
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlannerMode {
    ExactIdLike,
    ShortKeyword,
    PhraseLike,
    PathLike,
    NameLike,
    LongMixed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SearchEntityType {
    Doc,
    Card,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SemanticStatus {
    Ready,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HighlightSpan {
    pub field: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResult {
    pub result_id: i64,
    pub entity_type: SearchEntityType,
    pub root_path: String,
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub heading_text: Option<String>,
    pub heading_level: Option<i64>,
    pub heading_order: Option<i64>,
    pub cite: Option<String>,
    pub cite_date: Option<String>,
    pub outline_path: Vec<SearchOutlineEntry>,
    pub snippet: Option<String>,
    pub highlights: Vec<HighlightSpan>,
    pub score: f64,
    pub source: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchOutlineEntry {
    pub order: i64,
    pub level: i64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchLatencyMs {
    pub total: f64,
    pub lexical: f64,
    pub semantic: Option<f64>,
    pub rerank: f64,
    pub payload: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchCandidateCounts {
    pub exact: usize,
    pub bm25f: usize,
    pub prefix_rescue: usize,
    pub chargram_rescue: usize,
    pub semantic: Option<usize>,
    pub reranked: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchDiagnostics {
    pub planner_mode: PlannerMode,
    pub latency_ms: SearchLatencyMs,
    pub candidate_counts: SearchCandidateCounts,
    pub semantic_status: Option<SemanticStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total_approx: Option<usize>,
    pub diagnostics: Option<SearchDiagnostics>,
    pub warnings: Vec<SearchWarning>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchRequest {
    pub query: String,
    pub mode: Option<SearchMode>,
    pub root_paths: Option<Vec<String>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub filters: Option<SearchFilters>,
    pub diagnostics: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchFilters {
    pub file_name_only: Option<bool>,
    pub entity_types: Option<Vec<SearchEntityType>>,
    pub path_prefixes: Option<Vec<String>>,
    pub cite_date_from: Option<String>,
    pub cite_date_to: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchHydrateRequest {
    pub result_ids: Vec<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchHydrateResponse {
    pub results: Vec<HydratedSearchResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchInlineSpan {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchCardParagraph {
    pub paragraph_index: usize,
    pub text: String,
    pub spans: Vec<SearchInlineSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchCardPayload {
    pub result_id: i64,
    pub entity_type: SearchEntityType,
    pub root_path: String,
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub card_id: String,
    pub tag: String,
    pub tag_sub: String,
    pub cite: String,
    pub cite_date: Option<String>,
    pub heading_order: i64,
    pub heading_level: i64,
    pub heading_trail: Vec<SearchOutlineEntry>,
    pub body: Vec<SearchCardParagraph>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchDocPayload {
    pub result_id: i64,
    pub entity_type: SearchEntityType,
    pub root_path: String,
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub headings: Vec<SearchOutlineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum HydratedSearchResult {
    Doc(SearchDocPayload),
    Card(SearchCardPayload),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchWarmResult {
    pub ready: bool,
    pub doc_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchIndexStatus {
    pub layout_version: i64,
    pub ready: bool,
    pub doc_count: usize,
    pub semantic_status: SemanticStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticInstallStatus {
    pub installed: bool,
    pub model_name: Option<String>,
    pub status: SemanticStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureInsertResult {
    pub capture_path: String,
    pub marker: String,
    pub target_relative_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureTarget {
    pub relative_path: String,
    pub absolute_path: String,
    pub exists: bool,
    pub entry_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureTargetPreview {
    pub relative_path: String,
    pub absolute_path: String,
    pub exists: bool,
    pub heading_count: i64,
    pub headings: Vec<FileHeading>,
}

#[derive(Clone)]
pub(crate) struct ExistingFileMeta {
    pub id: i64,
    pub modified_ms: i64,
    pub size: i64,
    pub file_hash: String,
}

#[derive(Clone)]
pub(crate) struct ParsedHeading {
    pub order: i64,
    pub level: i64,
    pub text: String,
}

#[derive(Clone)]
pub(crate) struct ParsedParagraph {
    pub order: i64,
    pub text: String,
    pub heading_level: Option<i64>,
    pub style_label: Option<String>,
    pub is_f8_cite: bool,
}

#[derive(Clone)]
pub(crate) struct ParsedCard {
    pub tag: String,
    pub tag_sub: String,
    pub cite: String,
    pub cite_date: Option<String>,
    pub body: Vec<ParsedCardParagraph>,
    pub highlighted_text: String,
    pub heading_order: i64,
    pub heading_level: i64,
}

#[derive(Clone)]
pub(crate) struct ParsedCardParagraph {
    pub text: String,
    pub spans: Vec<SearchInlineSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexTextRun {
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub small_caps: bool,
    #[serde(default)]
    pub highlight_color: Option<String>,
    #[serde(default)]
    pub style_id: Option<String>,
    #[serde(default)]
    pub style_name: Option<String>,
    #[serde(default)]
    pub is_f8_cite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexBlock {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub runs: Vec<TexTextRun>,
    pub level: Option<i64>,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
    #[serde(default)]
    pub is_f8_cite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexDocumentPayload {
    pub file_path: String,
    pub file_name: String,
    pub paragraph_count: i64,
    pub blocks: Vec<TexBlock>,
}

#[derive(Clone)]
pub(crate) struct HeadingRange {
    pub order: i64,
    pub level: i64,
    pub start_index: usize,
    pub end_index: usize,
}

#[derive(Clone)]
pub(crate) struct FileRecord {
    pub id: i64,
    pub relative_path: String,
    pub modified_ms: i64,
    pub heading_count: i64,
}

#[derive(Clone)]
pub(crate) struct IndexCandidate {
    pub existing_file_id: Option<i64>,
    pub existing_file_hash: Option<String>,
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub modified_ms: i64,
    pub size: i64,
}

pub(crate) struct ParsedIndexCandidate {
    pub candidate: IndexCandidate,
    pub file_hash: String,
    pub headings: Vec<ParsedHeading>,
    pub authors: Vec<(i64, String)>,
    pub chunks: Vec<ParsedChunk>,
}

pub(crate) enum PreparedIndexCandidate {
    Parsed(ParsedIndexCandidate),
    Unchanged(IndexCandidate),
}

#[derive(Clone)]
pub(crate) struct ParsedChunk {
    pub chunk_order: i64,
    pub heading_order: Option<i64>,
    pub heading_level: Option<i64>,
    pub heading_text: Option<String>,
    pub author_text: Option<String>,
    pub chunk_text: String,
}

#[derive(Clone)]
pub(crate) struct SemanticCandidate {
    pub semantic_key: String,
    pub semantic_id: i64,
    pub root_id: i64,
    pub kind: String,
    pub file_id: i64,
    pub source_row_id: i64,
    pub heading_level: Option<i64>,
    pub heading_order: Option<i64>,
    pub semantic_text: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticRootIndexState {
    pub root_id: i64,
    pub last_indexed_ms: i64,
    pub item_count: usize,
    pub embedding_dim: usize,
    pub updated_at_ms: i64,
}

pub(crate) struct SemanticRuntime {
    pub tokenizer: Tokenizer,
    pub session: OrtSession,
    pub output_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexProgress {
    pub root_path: String,
    pub phase: String,
    pub discovered: usize,
    pub changed: usize,
    pub processed: usize,
    pub updated: usize,
    pub skipped: usize,
    pub removed: usize,
    pub elapsed_ms: i64,
    pub phase_elapsed_ms: i64,
    pub scan_rate_per_sec: f64,
    pub process_rate_per_sec: f64,
    pub eta_ms: Option<i64>,
    pub log_path: Option<String>,
    pub current_file: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkLatencyStats {
    pub runs: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkTaskResult {
    pub enabled: bool,
    pub error: Option<String>,
    pub total_hits: usize,
    pub latency: BenchmarkLatencyStats,
    pub tier_timings_ms: HashMap<String, f64>,
    pub tier_hit_counts: HashMap<String, usize>,
    pub doc_fetch_ms: f64,
    pub fallbacks_triggered: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LexicalSearchTelemetry {
    pub tier_timings_ms: HashMap<String, f64>,
    pub tier_hit_counts: HashMap<String, usize>,
    pub doc_fetch_ms: f64,
    pub fallbacks_triggered: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkSearchSummary {
    pub query_count: usize,
    pub iterations: usize,
    pub limit: usize,
    pub lexical_raw: BenchmarkTaskResult,
    pub lexical_cached: BenchmarkTaskResult,
    pub hybrid: BenchmarkTaskResult,
    pub semantic: BenchmarkTaskResult,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkPreviewSummary {
    pub snapshot_ms: f64,
    pub file_preview: BenchmarkTaskResult,
    pub heading_preview_html: BenchmarkTaskResult,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkReport {
    pub root_path: String,
    pub index_full: IndexStats,
    pub index_incremental: IndexStats,
    pub queries: Vec<String>,
    pub search: BenchmarkSearchSummary,
    pub preview: BenchmarkPreviewSummary,
    pub generated_at_ms: i64,
    pub elapsed_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkQueryRuntimeReport {
    pub root_path: String,
    pub queries: Vec<String>,
    pub search: BenchmarkSearchSummary,
    pub generated_at_ms: i64,
    pub elapsed_ms: i64,
}

pub(crate) struct StyledSection {
    pub paragraph_xml: Vec<String>,
    pub style_ids: HashSet<String>,
    pub relationship_ids: HashSet<String>,
    pub used_source_xml: bool,
}

pub(crate) struct SourceStyleDefinition {
    pub xml: String,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RelationshipDef {
    pub rel_type: String,
    pub target: String,
    pub target_mode: Option<String>,
}
