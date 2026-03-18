# @better-debate/core

A Rust native library that powers the BlockVault desktop app and core utility. It provides document indexing, hybrid search (lexical + semantic), DOCX capture/editing, and preview generation. The library is exposed to JavaScript/TypeScript via Bun FFI as a dynamic library (`libcore.dylib` / `core.dll` / `libcore.so`).

## Structure

```
packages/core/
├── src/                    # Rust source
│   ├── lib.rs              # Entry point, FFI exports, command dispatch
│   ├── commands.rs        # High-level command handlers (add_root, index_root, search, etc.)
│   ├── chunking.rs        # Text chunking for indexing (sentence-aware, overlap)
│   ├── db.rs              # SQLite schema, migrations, index layout
│   ├── docx_capture.rs    # DOCX capture: append, insert, delete, move headings
│   ├── docx_parse.rs      # DOCX parsing: paragraphs, headings, styles, XML
│   ├── indexer.rs         # Rebuilds Tantivy lexical index from SQLite
│   ├── lexical.rs        # Tantivy full-text search (prefix, ngram tokenizers)
│   ├── preview.rs         # HTML preview extraction from DOCX
│   ├── query_engine.rs    # Hybrid search orchestration, caching
│   ├── search.rs          # Query normalization, text utilities
│   ├── semantic.rs        # LanceDB + ONNX embeddings, vector search
│   ├── types.rs           # Shared types, ONNX/tokenizer setup
│   ├── util.rs            # Paths, hashing, author extraction, progress events
│   └── vector.rs          # Thin wrapper over semantic search
├── ffi/
│   └── index.ts           # Bun FFI bindings, loadCore(), invoke()
├── scripts/
│   ├── benchmark.ts       # Performance benchmark runner
│   └── copy-artifact.ts   # Copies built .dylib/.dll/.so to app bundle
├── Cargo.toml
└── package.json
```

## Core Services

### 1. **Root & Index Management**

- **add_root** — Registers a folder as an index root, writes `.blockfile-index.json` marker.
- **list_roots** — Returns all registered roots with file/heading counts.
- **get_index_snapshot** — Returns folder tree and indexed files for a root.
- **index_root** — Scans DOCX files, parses headings/chunks/authors, updates SQLite and Tantivy. Emits `index-progress` events during indexing. Triggers async vector index rebuild when done.

Index layout (v2) lives under app data:

- `index-v2/meta/` — SQLite (`blockfile-meta-v2.sqlite3`), semantic meta JSON
- `index-v2/lexical/` — Tantivy index
- `index-v2/vector/` — LanceDB tables for embeddings

### 2. **Search (Hybrid)**

- **search_index_hybrid** — Combines lexical (Tantivy) and semantic (LanceDB + ONNX) search. Uses a query cache (TTL 2 min, 480 entries). Supports `root_path`, `limit`, `file_name_only`, `semantic_enabled`.
- **search_index** — Lexical-only.
- **search_index_semantic** — Semantic-only.

**Lexical** (`lexical.rs`): Tantivy with prefix and ngram tokenizers for fuzzy matching. Indexes headings, authors, and chunk text.

**Semantic** (`semantic.rs`): ONNX embedding model (`model.onnx` + `tokenizer.json`) + LanceDB. Embeddings are built asynchronously after indexing. Requires `resources/model.onnx` and `resources/tokenizer.json`.

### 3. **DOCX Capture**

- **list_capture_targets** — Lists capture DOCX files and entry counts.
- **get_capture_target_preview** — Returns headings for a capture file.
- **insert_capture** — Appends a styled section to a capture DOCX (or creates it). Preserves source formatting when possible.
- **add_capture_heading** — Inserts a new heading (H1–H4) into a capture file.
- **delete_capture_heading** — Removes a heading and its content.
- **move_capture_heading** — Moves a heading block to a new position.

Capture files default to `BlockFile-Captures.docx` in the root. `docx_capture` and `docx_parse` handle OOXML (word/document.xml, styles, relationships) directly.

### 4. **Preview**

- **get_file_preview** — Returns file metadata, headings, and F8 citation blocks.
- **get_heading_preview_html** — Returns HTML for a single heading’s content (bold, italic, underline, highlights preserved).

### 5. **Benchmark**

- **benchmark_root_performance** — Runs full + incremental index, lexical (raw/cached), hybrid, semantic, snapshot, file preview, and heading preview benchmarks. Produces latency stats (min, p50, p95, max, mean).

## FFI Interface

The Rust library exposes a C ABI:

- `core_configure(app_data_dir, resource_dir)` — Initialize app paths.
- `core_set_event_callback(callback)` — Register event callback (e.g. `index-progress`).
- `core_invoke_json(request)` — Execute a command. Request: `{ command, args }`. Response: `{ ok, value?, error? }`.
- `core_free_str(ptr)` — Free returned C string.

`ffi/index.ts` uses Bun’s `dlopen` to load the native library and provides:

```ts
const core = loadCore({
  appDataDir: "...",
  resourceDir: "...",
  onEvent: (eventName, payload) => { ... },
});

const result = core.invoke<MyResult>("search_index_hybrid", {
  query: "climate change",
  rootPath: "/path/to/root",
  limit: 80,
});
```

## Build & Run

```bash
# Development (debug build)
bun run build:dev

# Release
bun run build

# Tests
bun run test        # cargo test

# Benchmark (builds dev, then runs benchmark script)
bun run bench
```

Release builds use LTO and single codegen unit for smaller binaries.

## Dependencies (Rust)

| Crate              | Purpose                                |
| ------------------ | -------------------------------------- |
| `rusqlite`         | SQLite (bundled) for metadata          |
| `tantivy`          | Full-text search index                 |
| `lancedb`          | Vector store for embeddings            |
| `ort`              | ONNX Runtime for embedding model       |
| `tokenizers`       | HuggingFace tokenizer for embeddings   |
| `docx-rs`          | DOCX creation                          |
| `roxmltree`, `zip` | DOCX parsing (OOXML)                   |
| `blake3`           | Fast file hashing for change detection |
| `rayon`            | Parallel indexing                      |
| `tokio`            | Async runtime for semantic search      |
| `walkdir`          | Directory traversal                    |
