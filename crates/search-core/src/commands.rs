use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::runtime::AppHandle;
use rayon::prelude::*;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::chunking::build_chunks;
use crate::db::{
    add_or_get_root_id, configure_database_for_bulk_index, index_lexical_dir, load_existing_files,
    open_database, restore_database_after_bulk_index, root_id,
};
use crate::docx_capture::{
    append_capture_to_docx, ensure_valid_capture_docx, extract_styled_section, paragraph_xml_heading,
};
use crate::docx_parse::{build_heading_ranges, has_tag, parse_docx_paragraphs, read_docx_part};
use crate::indexer::{
    apply_lexical_index_file_changes_for_root_with_options,
    rebuild_lexical_index_for_root_with_options,
};
use docx::rewrite_docx_with_parts;
use crate::lexical;
use crate::preview::{extract_heading_preview_html, extract_preview_content};
use crate::query_engine;
use crate::search::normalize_for_search;
use crate::search_engine;
use crate::semantic;
use crate::types::*;
use crate::util::*;
use crate::CommandResult;
use crate::DEFAULT_CAPTURE_TARGET;

use crate::docx_capture::{fallback_body_insertion_index, insertion_index_after_paragraph_count};

use roxmltree::{Document, Node};

const ROOT_SHARDS_DIR_NAME: &str = "roots";
const SHARED_INDEX_DIR_NAME: &str = ".blockvault-shared-index-v3";
const SHARED_INDEX_STAGING_DIR_NAME: &str = ".blockvault-shared-index-v3.tmp";
const SHARED_INDEX_DB_FILE_NAME: &str = "root.sqlite3";
const SHARED_INDEX_LEXICAL_DIR_NAME: &str = "lexical";
const SHARED_INDEX_MANIFEST_FILE_NAME: &str = "manifest.json";
const SHARED_INDEX_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedIndexManifest {
    version: u32,
    root_path: String,
    exported_at_ms: i64,
    file_count: usize,
    heading_count: usize,
    author_count: usize,
    chunk_count: usize,
}

fn folder_name_from_path(path: &str) -> String {
    let base = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_default();
    if !base.is_empty() {
        return base;
    }
    path.to_string()
}

fn file_or_directory_size_bytes(path: &Path) -> CommandResult<i64> {
    if !path.exists() {
        return Ok(0);
    }

    if path.is_file() {
        let metadata = fs::metadata(path).map_err(|error| {
            format!(
                "Could not read metadata for '{}': {error}",
                path_display(path)
            )
        })?;
        return Ok(i64::try_from(metadata.len()).unwrap_or(0));
    }

    let mut total_bytes = 0_i64;
    let entries = WalkDir::new(path).follow_links(false).into_iter();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        total_bytes = total_bytes.saturating_add(i64::try_from(metadata.len()).unwrap_or(0));
    }

    Ok(total_bytes)
}

fn local_root_shard_dir(app: &AppHandle, root_id: i64) -> CommandResult<PathBuf> {
    Ok(index_lexical_dir(app)?
        .join(ROOT_SHARDS_DIR_NAME)
        .join(root_id.to_string()))
}

fn shared_index_dir(canonical_root: &Path) -> PathBuf {
    canonical_root.join(SHARED_INDEX_DIR_NAME)
}

fn shared_index_staging_dir(canonical_root: &Path) -> PathBuf {
    canonical_root.join(SHARED_INDEX_STAGING_DIR_NAME)
}

fn shared_index_db_path(canonical_root: &Path) -> PathBuf {
    shared_index_dir(canonical_root).join(SHARED_INDEX_DB_FILE_NAME)
}

fn remove_path_if_exists(path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| {
            format!(
                "Could not remove directory '{}': {error}",
                path_display(path)
            )
        })?;
        return Ok(());
    }

    fs::remove_file(path)
        .map_err(|error| format!("Could not remove file '{}': {error}", path_display(path)))
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> CommandResult<()> {
    if !source.is_dir() {
        return Err(format!(
            "Missing source directory '{}'",
            path_display(source)
        ));
    }

    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "Could not create directory '{}': {error}",
            path_display(destination)
        )
    })?;

    let entries = fs::read_dir(source).map_err(|error| {
        format!(
            "Could not read directory '{}': {error}",
            path_display(source)
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Could not read directory entry in '{}': {error}",
                path_display(source)
            )
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory_recursive(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "Could not copy '{}' to '{}': {error}",
                    path_display(&source_path),
                    path_display(&destination_path)
                )
            })?;
        }
    }

    Ok(())
}

fn remove_local_root_shard(app: &AppHandle, root_id: i64) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }
    let shard_dir = local_root_shard_dir(app, root_id)?;
    remove_path_if_exists(&shard_dir)?;
    lexical::drop_root_runtime(root_id);
    Ok(())
}

fn clear_shared_root_state(connection: &Connection, root_id: i64) -> CommandResult<()> {
    connection
        .execute(
            "DELETE FROM shared_file_maps WHERE root_id = ?1",
            params![root_id],
        )
        .map_err(|error| format!("Could not clear shared file-id mappings: {error}"))?;
    connection
        .execute(
            "DELETE FROM shared_root_sources WHERE root_id = ?1",
            params![root_id],
        )
        .map_err(|error| format!("Could not clear shared root source metadata: {error}"))?;
    Ok(())
}

fn root_has_shared_source(connection: &Connection, root_id: i64) -> CommandResult<bool> {
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM shared_root_sources WHERE root_id = ?1",
            params![root_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not query shared root source metadata: {error}"))?;
    Ok(count > 0)
}

fn initialize_shared_bundle_database(connection: &Connection) -> CommandResult<()> {
    connection
        .execute_batch(
            "
            PRAGMA synchronous = NORMAL;
            PRAGMA temp_store = MEMORY;

            CREATE TABLE IF NOT EXISTS files (
              id INTEGER PRIMARY KEY,
              relative_path TEXT NOT NULL,
              modified_ms INTEGER NOT NULL,
              size INTEGER NOT NULL,
              file_hash TEXT NOT NULL,
              heading_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS headings (
              id INTEGER PRIMARY KEY,
              file_id INTEGER NOT NULL,
              heading_order INTEGER NOT NULL,
              level INTEGER NOT NULL,
              text TEXT NOT NULL,
              normalized TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS authors (
              id INTEGER PRIMARY KEY,
              file_id INTEGER NOT NULL,
              author_order INTEGER NOT NULL,
              text TEXT NOT NULL,
              normalized TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
              id INTEGER PRIMARY KEY,
              file_id INTEGER NOT NULL,
              chunk_order INTEGER NOT NULL,
              heading_order INTEGER,
              heading_level INTEGER,
              heading_text TEXT,
              author_text TEXT,
              chunk_text TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_shared_files_relative ON files(relative_path);
            CREATE INDEX IF NOT EXISTS idx_shared_headings_file_order ON headings(file_id, heading_order);
            CREATE INDEX IF NOT EXISTS idx_shared_authors_file_order ON authors(file_id, author_order);
            CREATE INDEX IF NOT EXISTS idx_shared_chunks_file_order ON chunks(file_id, chunk_order);
            ",
        )
        .map_err(|error| format!("Could not initialize shared index bundle database: {error}"))?;
    Ok(())
}

fn export_shared_bundle_database(
    source_connection: &Connection,
    root_id: i64,
    root_path: &str,
    destination_db_path: &Path,
) -> CommandResult<SharedIndexManifest> {
    remove_path_if_exists(destination_db_path)?;
    let mut shared_connection = Connection::open(destination_db_path).map_err(|error| {
        format!(
            "Could not create shared index bundle database '{}': {error}",
            path_display(destination_db_path)
        )
    })?;
    initialize_shared_bundle_database(&shared_connection)?;

    let transaction = shared_connection
        .transaction()
        .map_err(|error| format!("Could not start shared index bundle transaction: {error}"))?;

    let mut file_count = 0_usize;
    let mut heading_count = 0_usize;
    let mut author_count = 0_usize;
    let mut chunk_count = 0_usize;

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO files(id, relative_path, modified_ms, size, file_hash, heading_count)
                VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )
            .map_err(|error| format!("Could not prepare shared files insert statement: {error}"))?;
        let mut select_statement = source_connection
            .prepare(
                "
                SELECT id, relative_path, modified_ms, size, file_hash, heading_count
                FROM files
                WHERE root_id = ?1
                ORDER BY id ASC
                ",
            )
            .map_err(|error| {
                format!("Could not prepare source files query for shared bundle: {error}")
            })?;
        let rows = select_statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| format!("Could not read source files for shared bundle: {error}"))?;

        for row in rows {
            let (file_id, relative_path, modified_ms, size, file_hash, heading_count_value) = row
                .map_err(
                |error| format!("Could not parse source file row for shared bundle: {error}"),
            )?;
            insert_statement
                .execute(params![
                    file_id,
                    relative_path,
                    modified_ms,
                    size,
                    file_hash,
                    heading_count_value
                ])
                .map_err(|error| format!("Could not insert shared bundle file row: {error}"))?;
            file_count = file_count.saturating_add(1);
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO headings(
                  id,
                  file_id,
                  heading_order,
                  level,
                  text,
                  normalized
                )
                VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare shared headings insert statement: {error}")
            })?;
        let mut select_statement = source_connection
            .prepare(
                "
                SELECT
                  h.id,
                  h.file_id,
                  h.heading_order,
                  h.level,
                  h.text,
                  h.normalized
                FROM headings h
                JOIN files f ON f.id = h.file_id
                WHERE f.root_id = ?1
                ORDER BY h.file_id ASC, h.heading_order ASC
                ",
            )
            .map_err(|error| {
                format!("Could not prepare source headings query for shared bundle: {error}")
            })?;
        let rows = select_statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|error| {
                format!("Could not read source headings for shared bundle: {error}")
            })?;

        for row in rows {
            let (heading_id, file_id, heading_order, level, text, normalized) =
                row.map_err(|error| {
                    format!("Could not parse source heading row for shared bundle: {error}")
                })?;
            insert_statement
                .execute(params![
                    heading_id,
                    file_id,
                    heading_order,
                    level,
                    text,
                    normalized
                ])
                .map_err(|error| format!("Could not insert shared bundle heading row: {error}"))?;
            heading_count = heading_count.saturating_add(1);
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO authors(
                  id,
                  file_id,
                  author_order,
                  text,
                  normalized
                )
                VALUES(?1, ?2, ?3, ?4, ?5)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare shared authors insert statement: {error}")
            })?;
        let mut select_statement = source_connection
            .prepare(
                "
                SELECT
                  a.id,
                  a.file_id,
                  a.author_order,
                  a.text,
                  a.normalized
                FROM authors a
                JOIN files f ON f.id = a.file_id
                WHERE f.root_id = ?1
                ORDER BY a.file_id ASC, a.author_order ASC
                ",
            )
            .map_err(|error| {
                format!("Could not prepare source authors query for shared bundle: {error}")
            })?;
        let rows = select_statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| format!("Could not read source authors for shared bundle: {error}"))?;

        for row in rows {
            let (author_id, file_id, author_order, text, normalized) = row.map_err(|error| {
                format!("Could not parse source author row for shared bundle: {error}")
            })?;
            insert_statement
                .execute(params![author_id, file_id, author_order, text, normalized])
                .map_err(|error| format!("Could not insert shared bundle author row: {error}"))?;
            author_count = author_count.saturating_add(1);
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO chunks(
                  id,
                  file_id,
                  chunk_order,
                  heading_order,
                  heading_level,
                  heading_text,
                  author_text,
                  chunk_text
                )
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare shared chunks insert statement: {error}")
            })?;
        let mut select_statement = source_connection
            .prepare(
                "
                SELECT
                  id,
                  file_id,
                  chunk_order,
                  heading_order,
                  heading_level,
                  heading_text,
                  author_text,
                  chunk_text
                FROM chunks
                WHERE root_id = ?1
                ORDER BY file_id ASC, chunk_order ASC
                ",
            )
            .map_err(|error| {
                format!("Could not prepare source chunks query for shared bundle: {error}")
            })?;
        let rows = select_statement
            .query_map(params![root_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|error| format!("Could not read source chunks for shared bundle: {error}"))?;

        for row in rows {
            let (
                chunk_id,
                file_id,
                chunk_order,
                heading_order,
                heading_level,
                heading_text,
                author_text,
                chunk_text,
            ) = row.map_err(|error| {
                format!("Could not parse source chunk row for shared bundle: {error}")
            })?;
            insert_statement
                .execute(params![
                    chunk_id,
                    file_id,
                    chunk_order,
                    heading_order,
                    heading_level,
                    heading_text,
                    author_text,
                    chunk_text,
                ])
                .map_err(|error| format!("Could not insert shared bundle chunk row: {error}"))?;
            chunk_count = chunk_count.saturating_add(1);
        }
    }

    transaction
        .commit()
        .map_err(|error| format!("Could not commit shared index bundle transaction: {error}"))?;

    Ok(SharedIndexManifest {
        version: SHARED_INDEX_MANIFEST_VERSION,
        root_path: root_path.to_string(),
        exported_at_ms: now_ms(),
        file_count,
        heading_count,
        author_count,
        chunk_count,
    })
}

fn publish_shared_index_bundle(
    app: &AppHandle,
    connection: &Connection,
    canonical_root: &Path,
    root_id: i64,
    root_path: &str,
) -> CommandResult<()> {
    if root_id <= 0 {
        return Ok(());
    }

    let local_shard_dir = local_root_shard_dir(app, root_id)?;
    if !local_shard_dir.is_dir() {
        return Ok(());
    }

    let staging_dir = shared_index_staging_dir(canonical_root);
    let final_dir = shared_index_dir(canonical_root);
    remove_path_if_exists(&staging_dir)?;
    fs::create_dir_all(&staging_dir).map_err(|error| {
        format!(
            "Could not create shared index staging directory '{}': {error}",
            path_display(&staging_dir)
        )
    })?;

    let shared_db_path = staging_dir.join(SHARED_INDEX_DB_FILE_NAME);
    let manifest = export_shared_bundle_database(connection, root_id, root_path, &shared_db_path)?;

    let shared_lexical_dir = staging_dir.join(SHARED_INDEX_LEXICAL_DIR_NAME);
    copy_directory_recursive(&local_shard_dir, &shared_lexical_dir)?;

    let manifest_content = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize shared index manifest JSON: {error}"))?;
    let manifest_path = staging_dir.join(SHARED_INDEX_MANIFEST_FILE_NAME);
    fs::write(&manifest_path, manifest_content).map_err(|error| {
        format!(
            "Could not write shared index manifest '{}': {error}",
            path_display(&manifest_path)
        )
    })?;

    remove_path_if_exists(&final_dir)?;
    fs::rename(&staging_dir, &final_dir).map_err(|error| {
        format!(
            "Could not finalize shared index bundle '{}' -> '{}': {error}",
            path_display(&staging_dir),
            path_display(&final_dir)
        )
    })?;

    Ok(())
}

fn read_shared_manifest(canonical_root: &Path) -> CommandResult<Option<SharedIndexManifest>> {
    let manifest_path = shared_index_dir(canonical_root).join(SHARED_INDEX_MANIFEST_FILE_NAME);
    if !manifest_path.is_file() {
        return Ok(None);
    }

    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "Could not read shared index manifest '{}': {error}",
            path_display(&manifest_path)
        )
    })?;
    let manifest: SharedIndexManifest = serde_json::from_str(&content).map_err(|error| {
        format!(
            "Could not parse shared index manifest '{}': {error}",
            path_display(&manifest_path)
        )
    })?;
    Ok(Some(manifest))
}

fn try_import_shared_index_bundle(
    app: &AppHandle,
    connection: &mut Connection,
    canonical_root: &Path,
    root_id: i64,
    root_path: &str,
    started_at: i64,
    progress: &mut IndexProgress,
    last_progress_emit_ms: &mut i64,
) -> CommandResult<Option<IndexStats>> {
    let shared_dir = shared_index_dir(canonical_root);
    if !shared_dir.is_dir() {
        return Ok(None);
    }

    if let Some(manifest) = read_shared_manifest(canonical_root)? {
        if manifest.version != SHARED_INDEX_MANIFEST_VERSION {
            return Ok(None);
        }
    }

    let shared_db_path = shared_index_db_path(canonical_root);
    if !shared_db_path.is_file() {
        return Ok(None);
    }

    let shared_connection = Connection::open(&shared_db_path).map_err(|error| {
        format!(
            "Could not open shared index database '{}': {error}",
            path_display(&shared_db_path)
        )
    })?;
    let shared_file_count = shared_connection
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Could not read shared file count: {error}"))?;
    if shared_file_count <= 0 {
        return Ok(None);
    }

    progress.phase = "discovering".to_string();
    progress.discovered = usize::try_from(shared_file_count).unwrap_or(0);
    progress.changed = progress.discovered;
    progress.current_file = Some("Found shared index bundle".to_string());
    emit_index_progress(app, started_at, progress, last_progress_emit_ms, true);

    progress.phase = "indexing".to_string();
    progress.current_file = Some("Importing shared index metadata".to_string());
    emit_index_progress(app, started_at, progress, last_progress_emit_ms, true);

    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start shared index import transaction: {error}"))?;

    transaction
        .execute("DELETE FROM chunks WHERE root_id = ?1", params![root_id])
        .map_err(|error| format!("Could not clear root chunks before shared import: {error}"))?;
    transaction
        .execute("DELETE FROM files WHERE root_id = ?1", params![root_id])
        .map_err(|error| format!("Could not clear root files before shared import: {error}"))?;
    clear_shared_root_state(&transaction, root_id)?;

    let mut imported_files = 0_usize;
    let mut imported_headings = 0_usize;
    let mut file_id_map = HashMap::<i64, (i64, String, String)>::new();

    {
        let mut insert_file_statement = transaction
            .prepare(
                "
                INSERT INTO files(root_id, relative_path, absolute_path, modified_ms, size, file_hash, heading_count)
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
            )
            .map_err(|error| format!("Could not prepare local files insert statement: {error}"))?;
        let mut insert_map_statement = transaction
            .prepare(
                "
                INSERT INTO shared_file_maps(root_id, shared_file_id, local_file_id)
                VALUES(?1, ?2, ?3)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare shared file-id map insert statement: {error}")
            })?;
        let mut select_statement = shared_connection
            .prepare(
                "
                SELECT id, relative_path, modified_ms, size, file_hash, heading_count
                FROM files
                ORDER BY id ASC
                ",
            )
            .map_err(|error| format!("Could not prepare shared files query: {error}"))?;
        let rows = select_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| format!("Could not execute shared files query: {error}"))?;

        for row in rows {
            let (shared_file_id, relative_path, modified_ms, size, file_hash, heading_count_value) =
                row.map_err(|error| format!("Could not parse shared file row: {error}"))?;
            let absolute_path = path_display(&canonical_root.join(&relative_path));
            insert_file_statement
                .execute(params![
                    root_id,
                    relative_path.as_str(),
                    absolute_path.as_str(),
                    modified_ms,
                    size,
                    file_hash.as_str(),
                    heading_count_value
                ])
                .map_err(|error| {
                    format!("Could not insert local file row from shared bundle: {error}")
                })?;
            let local_file_id = transaction.last_insert_rowid();
            insert_map_statement
                .execute(params![root_id, shared_file_id, local_file_id])
                .map_err(|error| format!("Could not insert shared file-id mapping row: {error}"))?;
            file_id_map.insert(
                shared_file_id,
                (local_file_id, relative_path.clone(), absolute_path),
            );
            imported_files = imported_files.saturating_add(1);
            progress.processed = imported_files;
            progress.updated = imported_files;
            progress.current_file = Some(relative_path);
            emit_index_progress(app, started_at, progress, last_progress_emit_ms, false);
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO headings(file_id, heading_order, level, text, normalized)
                VALUES(?1, ?2, ?3, ?4, ?5)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare local headings insert statement: {error}")
            })?;
        let mut select_statement = shared_connection
            .prepare(
                "
                SELECT file_id, heading_order, level, text, normalized
                FROM headings
                ORDER BY file_id ASC, heading_order ASC
                ",
            )
            .map_err(|error| format!("Could not prepare shared headings query: {error}"))?;
        let rows = select_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| format!("Could not execute shared headings query: {error}"))?;

        for row in rows {
            let (shared_file_id, heading_order, level, text, normalized) =
                row.map_err(|error| format!("Could not parse shared heading row: {error}"))?;
            let Some((local_file_id, _, _)) = file_id_map.get(&shared_file_id) else {
                continue;
            };
            insert_statement
                .execute(params![
                    *local_file_id,
                    heading_order,
                    level,
                    text,
                    normalized
                ])
                .map_err(|error| {
                    format!("Could not insert local heading row from shared bundle: {error}")
                })?;
            imported_headings = imported_headings.saturating_add(1);
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO authors(file_id, author_order, text, normalized)
                VALUES(?1, ?2, ?3, ?4)
                ",
            )
            .map_err(|error| {
                format!("Could not prepare local authors insert statement: {error}")
            })?;
        let mut select_statement = shared_connection
            .prepare(
                "
                SELECT file_id, author_order, text, normalized
                FROM authors
                ORDER BY file_id ASC, author_order ASC
                ",
            )
            .map_err(|error| format!("Could not prepare shared authors query: {error}"))?;
        let rows = select_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("Could not execute shared authors query: {error}"))?;

        for row in rows {
            let (shared_file_id, author_order, text, normalized) =
                row.map_err(|error| format!("Could not parse shared author row: {error}"))?;
            let Some((local_file_id, _, _)) = file_id_map.get(&shared_file_id) else {
                continue;
            };
            insert_statement
                .execute(params![*local_file_id, author_order, text, normalized])
                .map_err(|error| {
                    format!("Could not insert local author row from shared bundle: {error}")
                })?;
        }
    }

    {
        let mut insert_statement = transaction
            .prepare(
                "
                INSERT INTO chunks(
                  chunk_id,
                  root_id,
                  file_id,
                  chunk_order,
                  heading_order,
                  heading_level,
                  heading_text,
                  author_text,
                  chunk_text
                )
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
            )
            .map_err(|error| format!("Could not prepare local chunks insert statement: {error}"))?;
        let mut select_statement = shared_connection
            .prepare(
                "
                SELECT
                  file_id,
                  chunk_order,
                  heading_order,
                  heading_level,
                  heading_text,
                  author_text,
                  chunk_text
                FROM chunks
                ORDER BY file_id ASC, chunk_order ASC
                ",
            )
            .map_err(|error| format!("Could not prepare shared chunks query: {error}"))?;
        let rows = select_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| format!("Could not execute shared chunks query: {error}"))?;

        for row in rows {
            let (
                shared_file_id,
                chunk_order,
                heading_order,
                heading_level,
                heading_text,
                author_text,
                chunk_text,
            ) = row.map_err(|error| format!("Could not parse shared chunk row: {error}"))?;
            let Some((local_file_id, _, _)) = file_id_map.get(&shared_file_id) else {
                continue;
            };
            let chunk_id = format!("{root_id}:{local_file_id}:{chunk_order}");
            insert_statement
                .execute(params![
                    chunk_id,
                    root_id,
                    *local_file_id,
                    chunk_order,
                    heading_order,
                    heading_level,
                    heading_text,
                    author_text,
                    chunk_text,
                ])
                .map_err(|error| {
                    format!("Could not insert local chunk row from shared bundle: {error}")
                })?;
        }
    }

    let indexed_at_ms = now_ms();
    transaction
        .execute(
            "UPDATE roots SET last_indexed_ms = ?1 WHERE id = ?2",
            params![indexed_at_ms, root_id],
        )
        .map_err(|error| format!("Could not update root timestamp after shared import: {error}"))?;
    transaction
        .execute(
            "
            INSERT INTO shared_root_sources(root_id, source_path, imported_at_ms)
            VALUES(?1, ?2, ?3)
            ON CONFLICT(root_id)
            DO UPDATE SET source_path = excluded.source_path, imported_at_ms = excluded.imported_at_ms
            ",
            params![root_id, path_display(canonical_root), indexed_at_ms],
        )
        .map_err(|error| format!("Could not store shared root source metadata: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not commit shared index import transaction: {error}"))?;

    progress.phase = "lexical".to_string();
    progress.current_file = Some("Rebuilding lexical shard from imported metadata".to_string());
    emit_index_progress(app, started_at, progress, last_progress_emit_ms, true);
    rebuild_lexical_index_for_root_with_options(app, root_id, false)?;

    query_engine::clear_query_cache();
    query_engine::set_cached_root_id(root_path.to_string(), Some(root_id));
    write_root_index_marker(canonical_root, indexed_at_ms)?;

    progress.phase = "complete".to_string();
    progress.current_file = None;
    progress.discovered = imported_files;
    progress.changed = imported_files;
    progress.processed = imported_files;
    progress.updated = imported_files;
    progress.skipped = 0;
    progress.removed = 0;
    emit_index_progress(app, started_at, progress, last_progress_emit_ms, true);

    Ok(Some(IndexStats {
        scanned: usize::try_from(shared_file_count).unwrap_or(imported_files),
        updated: imported_files,
        skipped: 0,
        removed: 0,
        headings_extracted: imported_headings,
        elapsed_ms: now_ms() - started_at,
    }))
}

pub(crate) fn add_root(app: AppHandle, path: String) -> CommandResult<String> {
    let canonical = canonicalize_folder(&path)?;
    let canonical_string = path_display(&canonical);

    let connection = open_database(&app)?;
    let resolved_root_id = add_or_get_root_id(&connection, &canonical_string)?;
    query_engine::invalidate_cached_root_id(&canonical_string);
    query_engine::set_cached_root_id(canonical_string.clone(), Some(resolved_root_id));
    write_root_index_marker(&canonical, 0)?;
    Ok(canonical_string)
}

pub(crate) fn remove_root(app: AppHandle, path: String) -> CommandResult<()> {
    let canonical_path = canonicalize_folder(&path).ok();
    let canonical_string = canonical_path
        .as_ref()
        .map(|path| path_display(path))
        .unwrap_or(path);
    let connection = open_database(&app)?;
    let removed_root_id = root_id(&connection, &canonical_string)?;
    if let Some(root_id_value) = removed_root_id {
        connection
            .execute("DELETE FROM roots WHERE id = ?1", params![root_id_value])
            .map_err(|error| format!("Could not remove root: {error}"))?;
        remove_local_root_shard(&app, root_id_value)?;
        lexical::remove_root_catalog_document(&app, root_id_value)?;
        crate::async_runtime::block_on(semantic::purge_semantic_root(&app, root_id_value))?;
        let _ = search_engine::remove_root_documents(&app, &canonical_string);
    } else {
        connection
            .execute(
                "DELETE FROM roots WHERE path = ?1",
                params![&canonical_string],
            )
            .map_err(|error| format!("Could not remove root: {error}"))?;
    }

    query_engine::invalidate_cached_root_id(&canonical_string);
    query_engine::set_cached_root_id(canonical_string.clone(), None);
    query_engine::clear_query_cache();

    if let Some(root_path) = canonical_path {
        let marker_path = root_index_marker_path(&root_path);
        let _ = fs::remove_file(marker_path);
    }
    Ok(())
}

pub(crate) fn insert_capture(
    app: AppHandle,
    root_path: String,
    source_path: String,
    section_title: String,
    content: String,
    paragraph_xml: Option<Vec<String>>,
    target_path: Option<String>,
    heading_level: Option<i64>,
    heading_order: Option<i64>,
    selected_target_heading_order: Option<i64>,
) -> CommandResult<CaptureInsertResult> {
    let content_value = content;
    if content_value.trim().is_empty() {
        return Err("Cannot insert empty content into capture file.".to_string());
    }

    let canonical_root = canonicalize_folder(&root_path)?;
    let target_relative_path = normalize_capture_target_path(target_path.as_deref())?;
    let normalized_heading_level = heading_level.filter(|level| (1..=9).contains(level));
    let normalized_target_heading_order = selected_target_heading_order.filter(|value| *value > 0);
    let root_path_string = path_display(&canonical_root);
    let connection = open_database(&app)?;
    let root_id = add_or_get_root_id(&connection, &root_path_string)?;

    let created_at_ms = now_ms();
    connection
        .execute(
            "
            INSERT INTO captures(
              root_id,
              source_path,
              section_title,
              target_relative_path,
              heading_level,
              content,
              created_at_ms
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                root_id,
                &source_path,
                &section_title,
                &target_relative_path,
                normalized_heading_level,
                &content_value,
                created_at_ms
            ],
        )
        .map_err(|error| format!("Could not insert capture entry: {error}"))?;

    let capture_id = connection.last_insert_rowid();
    let capture_path = capture_docx_path(&canonical_root, &target_relative_path);
    let source_file_path = Path::new(&source_path);
    let styled_section = paragraph_xml
        .and_then(|entries| {
            let cleaned = entries
                .into_iter()
                .map(|entry| entry.trim().to_string())
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<String>>();
            if cleaned.is_empty() {
                None
            } else {
                Some(StyledSection {
                    paragraph_xml: cleaned,
                    style_ids: HashSet::new(),
                    relationship_ids: HashSet::new(),
                    used_source_xml: false,
                })
            }
        })
        .unwrap_or_else(|| extract_styled_section(source_file_path, heading_order, &content_value));
    append_capture_to_docx(
        &capture_path,
        source_file_path,
        normalized_heading_level,
        normalized_target_heading_order,
        &styled_section,
    )?;

    Ok(CaptureInsertResult {
        capture_path: path_display(&capture_path),
        marker: capture_marker(capture_id),
        target_relative_path,
    })
}

pub(crate) fn list_capture_targets(
    app: AppHandle,
    root_path: String,
) -> CommandResult<Vec<CaptureTarget>> {
    let canonical_root = canonicalize_folder(&root_path)?;
    let root_path_string = path_display(&canonical_root);
    let connection = open_database(&app)?;
    let root_id = add_or_get_root_id(&connection, &root_path_string)?;

    let mut by_target = HashMap::<String, i64>::new();
    by_target.insert(DEFAULT_CAPTURE_TARGET.to_string(), 0);

    let mut statement = connection
        .prepare(
            "
            SELECT target_relative_path, COUNT(*)
            FROM captures
            WHERE root_id = ?1
            GROUP BY target_relative_path
            ORDER BY target_relative_path ASC
            ",
        )
        .map_err(|error| format!("Could not prepare capture targets query: {error}"))?;

    let rows = statement
        .query_map(params![root_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| format!("Could not iterate capture targets query: {error}"))?;

    for row in rows {
        let (target, count) =
            row.map_err(|error| format!("Could not parse capture target row: {error}"))?;
        by_target.insert(target, count);
    }

    let mut targets = by_target
        .into_iter()
        .map(|(relative_path, entry_count)| {
            let absolute_path = capture_docx_path(&canonical_root, &relative_path);
            CaptureTarget {
                relative_path,
                absolute_path: path_display(&absolute_path),
                exists: absolute_path.is_file(),
                entry_count,
            }
        })
        .collect::<Vec<CaptureTarget>>();

    targets.sort_by(|left, right| {
        (left.relative_path != DEFAULT_CAPTURE_TARGET)
            .cmp(&(right.relative_path != DEFAULT_CAPTURE_TARGET))
            .then(left.relative_path.cmp(&right.relative_path))
    });

    Ok(targets)
}

fn capture_target_preview_for_path(
    canonical_root: &Path,
    normalized_target: &str,
) -> CaptureTargetPreview {
    let absolute_path = capture_docx_path(canonical_root, normalized_target);

    if !absolute_path.is_file() {
        return CaptureTargetPreview {
            relative_path: normalized_target.to_string(),
            absolute_path: path_display(&absolute_path),
            exists: false,
            heading_count: 0,
            headings: Vec::new(),
        };
    }

    let (mut headings, _) = extract_preview_content(&absolute_path).unwrap_or_default();
    headings.sort_by(|left, right| left.order.cmp(&right.order));

    CaptureTargetPreview {
        relative_path: normalized_target.to_string(),
        absolute_path: path_display(&absolute_path),
        exists: true,
        heading_count: i64::try_from(headings.len()).unwrap_or(0),
        headings,
    }
}

pub(crate) fn get_capture_target_preview(
    _app: AppHandle,
    root_path: String,
    target_path: String,
) -> CommandResult<CaptureTargetPreview> {
    let canonical_root = canonicalize_folder(&root_path)?;
    let normalized_target = normalize_capture_target_path(Some(&target_path))?;
    Ok(capture_target_preview_for_path(
        &canonical_root,
        &normalized_target,
    ))
}

pub(crate) fn delete_capture_heading(
    _app: AppHandle,
    root_path: String,
    target_path: String,
    heading_order: i64,
) -> CommandResult<CaptureTargetPreview> {
    let canonical_root = canonicalize_folder(&root_path)?;
    let normalized_target = normalize_capture_target_path(Some(&target_path))?;
    let absolute_path = capture_docx_path(&canonical_root, &normalized_target);

    if !absolute_path.is_file() {
        return Err(format!(
            "Target capture file does not exist: {}",
            path_display(&absolute_path)
        ));
    }

    ensure_valid_capture_docx(&absolute_path)?;
    let paragraphs = parse_docx_paragraphs(&absolute_path)?;
    let heading_ranges = build_heading_ranges(&paragraphs);
    let target_range = heading_ranges
        .iter()
        .find(|range| range.order == heading_order)
        .cloned()
        .ok_or_else(|| format!("Heading order {heading_order} not found in target document."))?;

    let document_xml = read_docx_part(&absolute_path, "word/document.xml")?.ok_or_else(|| {
        format!(
            "Missing word/document.xml in '{}'",
            path_display(&absolute_path)
        )
    })?;
    let document = Document::parse(&document_xml).map_err(|error| {
        format!(
            "Could not parse destination document XML '{}': {error}",
            path_display(&absolute_path)
        )
    })?;
    let paragraph_nodes = document
        .descendants()
        .filter(|node| has_tag(*node, "p"))
        .collect::<Vec<Node<'_, '_>>>();

    if target_range.start_index >= paragraph_nodes.len()
        || target_range.end_index == 0
        || target_range.end_index > paragraph_nodes.len()
    {
        return Err("Heading range is out of bounds in destination document.".to_string());
    }

    let start = paragraph_nodes[target_range.start_index].range().start;
    let end = paragraph_nodes[target_range.end_index - 1].range().end;
    if start >= end || end > document_xml.len() {
        return Err("Could not resolve heading XML range in destination document.".to_string());
    }

    let mut updated_document_xml =
        String::with_capacity(document_xml.len().saturating_sub(end.saturating_sub(start)));
    updated_document_xml.push_str(&document_xml[..start]);
    updated_document_xml.push_str(&document_xml[end..]);

    let mut replacements = HashMap::new();
    replacements.insert(
        "word/document.xml".to_string(),
        updated_document_xml.into_bytes(),
    );
    rewrite_docx_with_parts(&absolute_path, &replacements)?;

    Ok(capture_target_preview_for_path(
        &canonical_root,
        &normalized_target,
    ))
}

pub(crate) fn move_capture_heading(
    _app: AppHandle,
    root_path: String,
    target_path: String,
    source_heading_order: i64,
    target_heading_order: i64,
) -> CommandResult<CaptureTargetPreview> {
    let canonical_root = canonicalize_folder(&root_path)?;
    let normalized_target = normalize_capture_target_path(Some(&target_path))?;
    let absolute_path = capture_docx_path(&canonical_root, &normalized_target);

    if source_heading_order == target_heading_order {
        return Ok(capture_target_preview_for_path(
            &canonical_root,
            &normalized_target,
        ));
    }

    if !absolute_path.is_file() {
        return Err(format!(
            "Target capture file does not exist: {}",
            path_display(&absolute_path)
        ));
    }

    ensure_valid_capture_docx(&absolute_path)?;
    let paragraphs = parse_docx_paragraphs(&absolute_path)?;
    let heading_ranges = build_heading_ranges(&paragraphs);

    let source_range = heading_ranges
        .iter()
        .find(|range| range.order == source_heading_order)
        .cloned()
        .ok_or_else(|| {
            format!("Source heading order {source_heading_order} not found in target document.")
        })?;
    let target_range = heading_ranges
        .iter()
        .find(|range| range.order == target_heading_order)
        .cloned()
        .ok_or_else(|| {
            format!("Target heading order {target_heading_order} not found in target document.")
        })?;

    if target_range.start_index >= source_range.start_index
        && target_range.start_index < source_range.end_index
    {
        return Err("Cannot move a heading into its own subtree.".to_string());
    }

    let document_xml = read_docx_part(&absolute_path, "word/document.xml")?.ok_or_else(|| {
        format!(
            "Missing word/document.xml in '{}'",
            path_display(&absolute_path)
        )
    })?;
    let document = Document::parse(&document_xml).map_err(|error| {
        format!(
            "Could not parse destination document XML '{}': {error}",
            path_display(&absolute_path)
        )
    })?;
    let paragraph_nodes = document
        .descendants()
        .filter(|node| has_tag(*node, "p"))
        .collect::<Vec<Node<'_, '_>>>();

    if source_range.start_index >= paragraph_nodes.len()
        || source_range.end_index == 0
        || source_range.end_index > paragraph_nodes.len()
        || target_range.start_index >= paragraph_nodes.len()
        || target_range.end_index == 0
        || target_range.end_index > paragraph_nodes.len()
    {
        return Err("Heading range is out of bounds in destination document.".to_string());
    }

    let source_start = paragraph_nodes[source_range.start_index].range().start;
    let source_end = paragraph_nodes[source_range.end_index - 1].range().end;
    if source_start >= source_end || source_end > document_xml.len() {
        return Err("Could not resolve source heading XML range.".to_string());
    }

    let moved_fragment = document_xml[source_start..source_end].to_string();
    let mut without_source =
        String::with_capacity(document_xml.len() - (source_end - source_start));
    without_source.push_str(&document_xml[..source_start]);
    without_source.push_str(&document_xml[source_end..]);

    let source_len = source_range
        .end_index
        .saturating_sub(source_range.start_index);
    let mut insertion_paragraph_count = target_range.end_index;
    if source_range.start_index < target_range.end_index {
        insertion_paragraph_count = insertion_paragraph_count.saturating_sub(source_len);
    }

    let insertion_index =
        insertion_index_after_paragraph_count(&without_source, insertion_paragraph_count)
            .unwrap_or(fallback_body_insertion_index(&without_source)?);

    let mut updated_document_xml =
        String::with_capacity(without_source.len().saturating_add(moved_fragment.len()));
    updated_document_xml.push_str(&without_source[..insertion_index]);
    updated_document_xml.push_str(&moved_fragment);
    updated_document_xml.push_str(&without_source[insertion_index..]);

    let mut replacements = HashMap::new();
    replacements.insert(
        "word/document.xml".to_string(),
        updated_document_xml.into_bytes(),
    );
    rewrite_docx_with_parts(&absolute_path, &replacements)?;

    Ok(capture_target_preview_for_path(
        &canonical_root,
        &normalized_target,
    ))
}

pub(crate) fn add_capture_heading(
    _app: AppHandle,
    root_path: String,
    target_path: String,
    heading_level: i64,
    heading_text: String,
    selected_target_heading_order: Option<i64>,
) -> CommandResult<CaptureTargetPreview> {
    if !(1..=4).contains(&heading_level) {
        return Err("Heading level must be H1, H2, H3, or H4.".to_string());
    }

    let trimmed_text = heading_text.trim();
    if trimmed_text.is_empty() {
        return Err("Heading name cannot be empty.".to_string());
    }

    let canonical_root = canonicalize_folder(&root_path)?;
    let normalized_target = normalize_capture_target_path(Some(&target_path))?;
    let absolute_path = capture_docx_path(&canonical_root, &normalized_target);

    let styled_section = StyledSection {
        paragraph_xml: vec![paragraph_xml_heading(heading_level, trimmed_text)],
        style_ids: HashSet::new(),
        relationship_ids: HashSet::new(),
        used_source_xml: false,
    };

    append_capture_to_docx(
        &absolute_path,
        &absolute_path,
        Some(heading_level),
        selected_target_heading_order.filter(|value| *value > 0),
        &styled_section,
    )?;

    Ok(capture_target_preview_for_path(
        &canonical_root,
        &normalized_target,
    ))
}

pub(crate) fn list_roots(app: AppHandle) -> CommandResult<Vec<RootSummary>> {
    let connection = open_database(&app)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
              r.path,
              r.added_at_ms,
              r.last_indexed_ms,
              (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id) AS file_count,
              (
                SELECT COUNT(*)
                FROM headings h
                JOIN files f ON f.id = h.file_id
                WHERE f.root_id = r.id
              ) AS heading_count
            FROM roots r
            ORDER BY r.path
            ",
        )
        .map_err(|error| format!("Could not prepare roots query: {error}"))?;

    let rows = statement
        .query_map([], |row| {
            Ok(RootSummary {
                path: row.get(0)?,
                added_at_ms: row.get(1)?,
                last_indexed_ms: row.get(2)?,
                file_count: row.get(3)?,
                heading_count: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not iterate roots query: {error}"))?;

    let mut roots = Vec::new();
    for row in rows {
        roots.push(row.map_err(|error| format!("Could not parse roots row: {error}"))?);
    }

    Ok(roots)
}

pub(crate) fn list_root_indexes(app: AppHandle) -> CommandResult<Vec<RootIndexEntry>> {
    let connection = open_database(&app)?;
    let lexical_root = index_lexical_dir(&app)?.join(ROOT_SHARDS_DIR_NAME);

    let mut statement = connection
        .prepare(
            "
            SELECT
              r.id,
              r.path,
              r.last_indexed_ms,
              (SELECT COUNT(*) FROM files f WHERE f.root_id = r.id) AS file_count,
              (
                SELECT COUNT(*)
                FROM headings h
                JOIN files f ON f.id = h.file_id
                WHERE f.root_id = r.id
              ) AS heading_count
            FROM roots r
            ORDER BY r.path
            ",
        )
        .map_err(|error| format!("Could not prepare root index query: {error}"))?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| format!("Could not iterate root index query: {error}"))?;

    let mut entries = Vec::new();
    for row in rows {
        let (root_id, root_path, last_indexed_ms, file_count, heading_count) =
            row.map_err(|error| format!("Could not parse root index row: {error}"))?;
        let index_path = lexical_root.join(root_id.to_string());
        let index_size_bytes = file_or_directory_size_bytes(&index_path)?;
        entries.push(RootIndexEntry {
            root_id,
            root_path: root_path.clone(),
            folder_name: folder_name_from_path(&root_path),
            index_path: path_display(&index_path),
            index_size_bytes,
            file_count,
            heading_count,
            last_indexed_ms,
        });
    }

    Ok(entries)
}

fn refresh_index_progress_metrics(
    progress: &mut IndexProgress,
    started_at_ms: i64,
    phase_started_at_ms: i64,
    log_path: &str,
) {
    let now = now_ms();
    let elapsed_ms = now.saturating_sub(started_at_ms);
    let phase_elapsed_ms = now.saturating_sub(phase_started_at_ms);

    progress.elapsed_ms = elapsed_ms;
    progress.phase_elapsed_ms = phase_elapsed_ms;
    progress.log_path = Some(log_path.to_string());
    progress.scan_rate_per_sec = if elapsed_ms > 0 {
        (progress.discovered as f64 * 1000.0) / elapsed_ms as f64
    } else {
        0.0
    };
    progress.process_rate_per_sec = if progress.phase == "indexing" && phase_elapsed_ms > 0 {
        (progress.processed as f64 * 1000.0) / phase_elapsed_ms as f64
    } else {
        0.0
    };
    progress.eta_ms = if progress.phase == "indexing"
        && progress.processed > 0
        && progress.changed > progress.processed
        && phase_elapsed_ms > 0
    {
        let remaining = progress.changed - progress.processed;
        Some(
            ((phase_elapsed_ms as f64 / progress.processed as f64) * remaining as f64).round()
                as i64,
        )
    } else {
        None
    };
}

fn emit_index_progress_with_observability(
    app: &AppHandle,
    started_at_ms: i64,
    phase_started_at_ms: i64,
    progress: &mut IndexProgress,
    last_progress_emit_ms: &mut i64,
    logger: &IndexRunLogger,
    last_progress_log_ms: &mut i64,
    force: bool,
) {
    let log_path = logger.path_string();
    refresh_index_progress_metrics(progress, started_at_ms, phase_started_at_ms, &log_path);
    emit_index_progress(app, started_at_ms, progress, last_progress_emit_ms, force);

    let now = now_ms();
    if force || now - *last_progress_log_ms >= INDEX_LOG_HEARTBEAT_INTERVAL_MS {
        let message = format!(
            "progress discovered={} changed={} processed={} updated={} skipped={} removed={} elapsedMs={} phaseElapsedMs={} scanRatePerSec={:.2} processRatePerSec={:.2} etaMs={} currentFile=\"{}\"",
            progress.discovered,
            progress.changed,
            progress.processed,
            progress.updated,
            progress.skipped,
            progress.removed,
            progress.elapsed_ms,
            progress.phase_elapsed_ms,
            progress.scan_rate_per_sec,
            progress.process_rate_per_sec,
            progress.eta_ms.unwrap_or(-1),
            progress.current_file.as_deref().unwrap_or("")
        );
        let _ = logger.append("INFO", &progress.phase, &message);
        *last_progress_log_ms = now;
    }
}

fn set_index_phase(
    app: &AppHandle,
    progress: &mut IndexProgress,
    next_phase: &str,
    detail: &str,
    started_at_ms: i64,
    phase_started_at_ms: &mut i64,
    last_progress_emit_ms: &mut i64,
    logger: &IndexRunLogger,
    last_progress_log_ms: &mut i64,
) {
    progress.phase = next_phase.to_string();
    progress.current_file = None;
    *phase_started_at_ms = now_ms();
    let _ = logger.append("INFO", next_phase, detail);
    emit_index_progress_with_observability(
        app,
        started_at_ms,
        *phase_started_at_ms,
        progress,
        last_progress_emit_ms,
        logger,
        last_progress_log_ms,
        true,
    );
}

pub(crate) fn index_root(app: AppHandle, path: String) -> CommandResult<IndexStats> {
    let started_at = now_ms();
    let canonical_root = canonicalize_folder(&path)?;
    let root_path = path_display(&canonical_root);
    let logger = IndexRunLogger::create(&app, &root_path, started_at)?;

    let result = (|| -> CommandResult<IndexStats> {
        let mut connection = open_database(&app)?;
        let root_id = add_or_get_root_id(&connection, &root_path)?;
        let existing_files = load_existing_files(&connection, root_id)?;

        let mut scanned = 0_usize;
        let mut updated = 0_usize;
        let mut skipped = 0_usize;
        let mut removed = 0_usize;
        let mut headings_extracted = 0_usize;
        let mut updated_file_ids = Vec::new();
        let mut removed_file_ids = Vec::new();
        let mut seen_relative_paths = HashSet::new();
        let mut indexing_candidates = Vec::new();

        let log_path = logger.path_string();
        let mut progress = IndexProgress {
            root_path: root_path.clone(),
            phase: "discovering".to_string(),
            discovered: 0,
            changed: 0,
            processed: 0,
            updated: 0,
            skipped: 0,
            removed: 0,
            elapsed_ms: 0,
            phase_elapsed_ms: 0,
            scan_rate_per_sec: 0.0,
            process_rate_per_sec: 0.0,
            eta_ms: None,
            log_path: Some(log_path.clone()),
            current_file: None,
        };
        let mut last_progress_emit_ms = 0_i64;
        let mut last_progress_log_ms = 0_i64;
        let mut phase_started_at = started_at;
        let _ = logger.append(
            "INFO",
            "discovering",
            &format!("index run started root=\"{}\"", root_path),
        );
        emit_index_progress_with_observability(
            &app,
            started_at,
            phase_started_at,
            &mut progress,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
            true,
        );

        if existing_files.is_empty() {
            match try_import_shared_index_bundle(
                &app,
                &mut connection,
                &canonical_root,
                root_id,
                &root_path,
                started_at,
                &mut progress,
                &mut last_progress_emit_ms,
            ) {
                Ok(Some(imported_stats)) => {
                    let _ = logger.append("INFO", "discovering", "shared index bundle imported");
                    let _ = search_engine::mark_pending_update(&app);
                    search_engine::request_background_rebuild(app.clone());
                    return Ok(imported_stats);
                }
                Ok(None) => {
                    let _ =
                        logger.append("INFO", "discovering", "no shared index bundle available");
                }
                Err(error) => {
                    let _ = logger.append(
                        "WARN",
                        "discovering",
                        &format!("shared index import failed error=\"{}\"", error),
                    );
                    eprintln!(
                        "[index_root] Shared index import failed for '{}': {}",
                        root_path, error
                    );
                }
            }
        }

        let has_shared_source = root_has_shared_source(&connection, root_id)?;
        if !has_shared_source {
            clear_shared_root_state(&connection, root_id)?;
        }

        configure_database_for_bulk_index(&connection)?;
        let _ = logger.append(
            "INFO",
            "discovering",
            "bulk index database settings enabled",
        );

        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(is_visible_entry)
        {
            let Ok(entry) = entry else {
                continue;
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let is_docx = entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.eq_ignore_ascii_case("docx"))
                .unwrap_or(false);
            if !is_docx {
                continue;
            }

            scanned += 1;
            let absolute_path = entry.path().to_path_buf();
            let relative_path_value = relative_path(&canonical_root, &absolute_path)?;
            seen_relative_paths.insert(relative_path_value.clone());

            let metadata = fs::metadata(&absolute_path).map_err(|error| {
                format!(
                    "Could not read metadata for '{}': {error}",
                    path_display(&absolute_path)
                )
            })?;
            let modified_ms = metadata.modified().map(epoch_ms).unwrap_or(0);
            let size = i64::try_from(metadata.len()).unwrap_or(0);

            if let Some(existing) = existing_files.get(&relative_path_value) {
                if existing.modified_ms == modified_ms
                    && existing.size == size
                    && !existing.file_hash.is_empty()
                {
                    skipped += 1;
                } else {
                    indexing_candidates.push(IndexCandidate {
                        existing_file_id: Some(existing.id),
                        existing_file_hash: (!existing.file_hash.is_empty())
                            .then(|| existing.file_hash.clone()),
                        relative_path: relative_path_value.clone(),
                        absolute_path,
                        modified_ms,
                        size,
                    });
                }
            } else {
                indexing_candidates.push(IndexCandidate {
                    existing_file_id: None,
                    existing_file_hash: None,
                    relative_path: relative_path_value.clone(),
                    absolute_path,
                    modified_ms,
                    size,
                });
            }

            progress.discovered = scanned;
            progress.changed = indexing_candidates.len();
            progress.skipped = skipped;
            progress.current_file = Some(relative_path_value);
            emit_index_progress_with_observability(
                &app,
                started_at,
                phase_started_at,
                &mut progress,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
                false,
            );
        }

        let stale_entries = existing_files
            .iter()
            .filter_map(|(relative_path, existing)| {
                (!seen_relative_paths.contains(relative_path))
                    .then_some((relative_path.clone(), existing.id))
            })
            .collect::<Vec<(String, i64)>>();

        progress.discovered = scanned;
        progress.changed = indexing_candidates.len();
        progress.skipped = skipped;
        set_index_phase(
            &app,
            &mut progress,
            "indexing",
            &format!(
                "starting parse/write loop changed={} skipped={} stale={}",
                indexing_candidates.len(),
                skipped,
                stale_entries.len()
            ),
            started_at,
            &mut phase_started_at,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
        );

        let parse_chunk_size = suggested_parse_chunk_size();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Could not start bulk index transaction: {error}"))?;
        let mut update_file_stmt = transaction
            .prepare_cached(
                "UPDATE files
                 SET absolute_path = ?1, modified_ms = ?2, size = ?3, file_hash = ?4, heading_count = ?5
                 WHERE id = ?6",
            )
            .map_err(|error| format!("Could not prepare indexed file update statement: {error}"))?;
        let mut insert_file_stmt = transaction
            .prepare_cached(
                "INSERT INTO files(root_id, relative_path, absolute_path, modified_ms, size, file_hash, heading_count)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|error| format!("Could not prepare indexed file insert statement: {error}"))?;
        let mut delete_headings_stmt = transaction
            .prepare_cached("DELETE FROM headings WHERE file_id = ?1")
            .map_err(|error| format!("Could not prepare heading cleanup statement: {error}"))?;
        let mut delete_authors_stmt = transaction
            .prepare_cached("DELETE FROM authors WHERE file_id = ?1")
            .map_err(|error| format!("Could not prepare author cleanup statement: {error}"))?;
        let mut delete_chunks_stmt = transaction
            .prepare_cached("DELETE FROM chunks WHERE file_id = ?1")
            .map_err(|error| format!("Could not prepare chunk cleanup statement: {error}"))?;
        let mut insert_heading_stmt = transaction
            .prepare_cached(
                "INSERT INTO headings(file_id, heading_order, level, text, normalized)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|error| format!("Could not prepare heading insert statement: {error}"))?;
        let mut insert_author_stmt = transaction
            .prepare_cached(
                "INSERT INTO authors(file_id, author_order, text, normalized)
                 VALUES(?1, ?2, ?3, ?4)",
            )
            .map_err(|error| format!("Could not prepare author insert statement: {error}"))?;
        let mut insert_chunk_stmt = transaction
            .prepare_cached(
                "
                INSERT INTO chunks(
                  chunk_id,
                  root_id,
                  file_id,
                  chunk_order,
                  heading_order,
                  heading_level,
                  heading_text,
                  author_text,
                  chunk_text
                )
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
            )
            .map_err(|error| format!("Could not prepare chunk insert statement: {error}"))?;
        let mut delete_stale_file_stmt = transaction
            .prepare_cached("DELETE FROM files WHERE id = ?1")
            .map_err(|error| format!("Could not prepare stale file delete statement: {error}"))?;
        let mut update_root_indexed_stmt = transaction
            .prepare_cached("UPDATE roots SET last_indexed_ms = ?1 WHERE id = ?2")
            .map_err(|error| format!("Could not prepare root timestamp statement: {error}"))?;

        for chunk in indexing_candidates.chunks(parse_chunk_size) {
            let prepared_chunk = chunk
                .par_iter()
                .map(|candidate| {
                    let file_hash = fast_file_hash(&candidate.absolute_path)?;
                    if candidate.existing_file_hash.as_deref() == Some(file_hash.as_str()) {
                        return Ok(PreparedIndexCandidate::Unchanged(candidate.clone()));
                    }

                    let paragraphs =
                        parse_docx_paragraphs(&candidate.absolute_path).unwrap_or_default();
                    let headings = paragraphs
                        .iter()
                        .filter_map(|paragraph| {
                            paragraph.heading_level.map(|level| ParsedHeading {
                                order: paragraph.order,
                                level,
                                text: paragraph.text.clone(),
                            })
                        })
                        .collect::<Vec<ParsedHeading>>();
                    let authors = extract_author_candidates(&paragraphs);
                    let chunks = build_chunks(&paragraphs);
                    Ok(PreparedIndexCandidate::Parsed(ParsedIndexCandidate {
                        candidate: candidate.clone(),
                        file_hash,
                        headings,
                        authors,
                        chunks,
                    }))
                })
                .collect::<Vec<CommandResult<PreparedIndexCandidate>>>();

            let prepared_chunk = prepared_chunk
                .into_iter()
                .collect::<CommandResult<Vec<PreparedIndexCandidate>>>()?;
            let mut parsed_chunk = Vec::new();
            for prepared in prepared_chunk {
                match prepared {
                    PreparedIndexCandidate::Parsed(parsed) => parsed_chunk.push(parsed),
                    PreparedIndexCandidate::Unchanged(candidate) => {
                        skipped += 1;
                        progress.changed = progress.changed.saturating_sub(1);
                        progress.skipped = skipped;
                        progress.current_file = Some(candidate.relative_path);
                        emit_index_progress_with_observability(
                            &app,
                            started_at,
                            phase_started_at,
                            &mut progress,
                            &mut last_progress_emit_ms,
                            &logger,
                            &mut last_progress_log_ms,
                            false,
                        );
                    }
                }
            }

            for parsed in parsed_chunk {
                let relative_path_value = parsed.candidate.relative_path;
                let absolute_path_string = path_display(&parsed.candidate.absolute_path);
                let modified_ms = parsed.candidate.modified_ms;
                let size = parsed.candidate.size;
                let heading_count = i64::try_from(parsed.headings.len()).unwrap_or(0);
                headings_extracted += parsed.headings.len();

                let file_id = if let Some(existing_id) = parsed.candidate.existing_file_id {
                    update_file_stmt
                        .execute(params![
                            absolute_path_string,
                            modified_ms,
                            size,
                            parsed.file_hash.as_str(),
                            heading_count,
                            existing_id
                        ])
                        .map_err(|error| {
                            format!(
                                "Could not update indexed file '{}': {error}",
                                relative_path_value
                            )
                        })?;
                    existing_id
                } else {
                    insert_file_stmt
                        .execute(params![
                            root_id,
                            relative_path_value.as_str(),
                            absolute_path_string,
                            modified_ms,
                            size,
                            parsed.file_hash.as_str(),
                            heading_count
                        ])
                        .map_err(|error| {
                            format!(
                                "Could not insert indexed file '{}': {error}",
                                relative_path_value
                            )
                        })?;
                    transaction.last_insert_rowid()
                };
                updated_file_ids.push(file_id);

                delete_headings_stmt
                    .execute(params![file_id])
                    .map_err(|error| {
                        format!(
                            "Could not clear old headings for '{}': {error}",
                            relative_path_value
                        )
                    })?;

                delete_authors_stmt
                    .execute(params![file_id])
                    .map_err(|error| {
                        format!(
                            "Could not clear old author rows for '{}': {error}",
                            relative_path_value
                        )
                    })?;

                delete_chunks_stmt
                    .execute(params![file_id])
                    .map_err(|error| {
                        format!(
                            "Could not clear old chunks for '{}': {error}",
                            relative_path_value
                        )
                    })?;

                for heading in parsed.headings {
                    let normalized = normalize_for_search(&heading.text);
                    insert_heading_stmt
                        .execute(params![
                            file_id,
                            heading.order,
                            heading.level,
                            heading.text,
                            normalized
                        ])
                        .map_err(|error| {
                            format!(
                                "Could not insert heading for '{}': {error}",
                                relative_path_value
                            )
                        })?;
                }

                for (author_order, author_text) in parsed.authors {
                    let normalized_author = normalize_for_search(&author_text);
                    insert_author_stmt
                        .execute(params![
                            file_id,
                            author_order,
                            author_text,
                            normalized_author
                        ])
                        .map_err(|error| {
                            format!(
                                "Could not insert author metadata for '{}': {error}",
                                relative_path_value
                            )
                        })?;
                }

                for chunk in parsed.chunks {
                    let chunk_id = format!("{}:{}:{}", root_id, file_id, chunk.chunk_order);
                    insert_chunk_stmt
                        .execute(params![
                            chunk_id,
                            root_id,
                            file_id,
                            chunk.chunk_order,
                            chunk.heading_order,
                            chunk.heading_level,
                            chunk.heading_text,
                            chunk.author_text,
                            chunk.chunk_text,
                        ])
                        .map_err(|error| {
                            format!(
                                "Could not insert chunk row for '{}': {error}",
                                relative_path_value
                            )
                        })?;
                }

                updated += 1;
                progress.processed = updated;
                progress.updated = updated;
                progress.current_file = Some(relative_path_value);
                emit_index_progress_with_observability(
                    &app,
                    started_at,
                    phase_started_at,
                    &mut progress,
                    &mut last_progress_emit_ms,
                    &logger,
                    &mut last_progress_log_ms,
                    false,
                );
            }
        }

        set_index_phase(
            &app,
            &mut progress,
            "cleaning",
            &format!("removing stale entries count={}", stale_entries.len()),
            started_at,
            &mut phase_started_at,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
        );

        let indexed_at_ms = now_ms();
        for (relative_path_value, file_id) in stale_entries {
            delete_stale_file_stmt
                .execute(params![file_id])
                .map_err(|error| {
                    format!(
                        "Could not remove stale index row '{}': {error}",
                        relative_path_value
                    )
                })?;
            removed += 1;
            removed_file_ids.push(file_id);

            progress.removed = removed;
            progress.current_file = Some(relative_path_value);
            emit_index_progress_with_observability(
                &app,
                started_at,
                phase_started_at,
                &mut progress,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
                false,
            );
        }

        update_root_indexed_stmt
            .execute(params![indexed_at_ms, root_id])
            .map_err(|error| format!("Could not update root index timestamp: {error}"))?;

        drop(update_file_stmt);
        drop(insert_file_stmt);
        drop(delete_headings_stmt);
        drop(delete_authors_stmt);
        drop(delete_chunks_stmt);
        drop(insert_heading_stmt);
        drop(insert_author_stmt);
        drop(insert_chunk_stmt);
        drop(delete_stale_file_stmt);
        drop(update_root_indexed_stmt);

        set_index_phase(
            &app,
            &mut progress,
            "committing",
            "committing sqlite transaction to disk",
            started_at,
            &mut phase_started_at,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
        );

        transaction
            .commit()
            .map_err(|error| format!("Could not commit final index transaction: {error}"))?;

        let _ = logger.append(
            "INFO",
            "committing",
            &format!(
                "database transaction committed updated={} removed={} headingsExtracted={}",
                updated, removed, headings_extracted
            ),
        );

        restore_database_after_bulk_index(&connection)?;
        let _ = logger.append(
            "INFO",
            "committing",
            "bulk index database settings restored",
        );

        write_root_index_marker(&canonical_root, indexed_at_ms)?;

        let indexed_root_docs = lexical::indexed_root_doc_count(&app, root_id).unwrap_or(0);
        let mut updated_local_lexical = false;
        if updated > 0 || removed > 0 || (scanned > 0 && indexed_root_docs == 0) {
            set_index_phase(
                &app,
                &mut progress,
                "lexical",
                &format!(
                    "updating lexical shards updated={} removed={} indexedRootDocs={}",
                    updated, removed, indexed_root_docs
                ),
                started_at,
                &mut phase_started_at,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
            );
            progress.discovered = scanned;
            progress.changed = indexing_candidates.len();
            progress.processed = updated;
            progress.updated = updated;
            progress.skipped = skipped;
            progress.removed = removed;
            emit_index_progress_with_observability(
                &app,
                started_at,
                phase_started_at,
                &mut progress,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
                true,
            );

            let wait_for_merges = false;
            if scanned > 0 && indexed_root_docs == 0 {
                rebuild_lexical_index_for_root_with_options(&app, root_id, wait_for_merges)?;
            } else {
                apply_lexical_index_file_changes_for_root_with_options(
                    &app,
                    root_id,
                    &updated_file_ids,
                    &removed_file_ids,
                    wait_for_merges,
                )?;
            }
            updated_local_lexical = true;
            let _ = logger.append("INFO", "lexical", "lexical shard update complete");
        }
        if updated_local_lexical {
            clear_shared_root_state(&connection, root_id)?;
        }

        let should_mark_search_pending =
            updated > 0 || removed > 0 || (scanned > 0 && existing_files.is_empty());
        if should_mark_search_pending {
            set_index_phase(
                &app,
                &mut progress,
                "search",
                "marking search index for lazy rebuild",
                started_at,
                &mut phase_started_at,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
            );
            progress.current_file = None;
            progress.discovered = scanned;
            progress.changed = indexing_candidates.len();
            progress.processed = updated;
            progress.updated = updated;
            progress.skipped = skipped;
            progress.removed = removed;
            emit_index_progress_with_observability(
                &app,
                started_at,
                phase_started_at,
                &mut progress,
                &mut last_progress_emit_ms,
                &logger,
                &mut last_progress_log_ms,
                true,
            );
            search_engine::mark_pending_update(&app)?;
            let _ = logger.append(
                "INFO",
                "search",
                "search index marked stale for lazy rebuild",
            );
            let background_app = app.clone();
            let background_logger = logger.clone();
            let background_updated_file_ids = updated_file_ids.clone();
            let background_removed_file_ids = removed_file_ids.clone();
            crate::async_runtime::spawn_blocking(move || {
                let _ = background_logger.append(
                    "INFO",
                    "search",
                    "background search index update started",
                );
                match open_database(&background_app).and_then(|background_connection| {
                    search_engine::apply_file_changes_from_connection(
                        &background_app,
                        &background_connection,
                        root_id,
                        &background_updated_file_ids,
                        &background_removed_file_ids,
                    )
                }) {
                    Ok(()) => {
                        let _ = background_logger.append(
                            "INFO",
                            "search",
                            "background search index update complete",
                        );
                    }
                    Err(error) => {
                        let _ = background_logger.append(
                            "WARN",
                            "search",
                            &format!("background search index update failed error=\"{}\"", error),
                        );
                    }
                }
            });
        }
        query_engine::set_cached_root_id(root_path.clone(), Some(root_id));
        query_engine::clear_query_cache();

        let should_publish_shared_bundle =
            updated > 0 || removed > 0 || (scanned > 0 && existing_files.is_empty());
        if should_publish_shared_bundle {
            let background_app = app.clone();
            let background_root_path = root_path.clone();
            let background_canonical_root = canonical_root.clone();
            let background_logger = logger.clone();
            crate::async_runtime::spawn_blocking(move || {
                let _ = background_logger.append(
                    "INFO",
                    "publish",
                    "background shared index publish started",
                );
                match open_database(&background_app).and_then(|background_connection| {
                    publish_shared_index_bundle(
                        &background_app,
                        &background_connection,
                        &background_canonical_root,
                        root_id,
                        &background_root_path,
                    )
                }) {
                    Ok(()) => {
                        let _ = background_logger.append(
                            "INFO",
                            "publish",
                            "background shared index publish complete",
                        );
                    }
                    Err(error) => {
                        let _ = background_logger.append(
                            "WARN",
                            "publish",
                            &format!("background shared index publish failed error=\"{}\"", error),
                        );
                    }
                }
            });
        }

        let finished_at_ms = now_ms();

        set_index_phase(
            &app,
            &mut progress,
            "complete",
            &format!(
                "index run complete scanned={} updated={} skipped={} removed={} headingsExtracted={}",
                scanned, updated, skipped, removed, headings_extracted
            ),
            started_at,
            &mut phase_started_at,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
        );
        progress.current_file = None;
        progress.discovered = scanned;
        progress.changed = indexing_candidates.len();
        progress.processed = updated;
        progress.updated = updated;
        progress.skipped = skipped;
        progress.removed = removed;
        emit_index_progress_with_observability(
            &app,
            started_at,
            phase_started_at,
            &mut progress,
            &mut last_progress_emit_ms,
            &logger,
            &mut last_progress_log_ms,
            true,
        );

        Ok(IndexStats {
            scanned,
            updated,
            skipped,
            removed,
            headings_extracted,
            elapsed_ms: finished_at_ms - started_at,
        })
    })();

    if let Err(error) = &result {
        let _ = logger.append(
            "ERROR",
            "failed",
            &format!("index run failed error=\"{}\"", error),
        );
    }

    result
}

fn ensure_folder_with_ancestors(folders: &mut HashMap<String, FolderEntry>, folder_path: &str) {
    let mut current = folder_path.to_string();

    loop {
        if !folders.contains_key(&current) {
            let parent_path = current
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_string());
            let name = if current.is_empty() {
                "Root".to_string()
            } else {
                current
                    .rsplit_once('/')
                    .map(|(_, name)| name.to_string())
                    .unwrap_or_else(|| current.clone())
            };
            let depth = if current.is_empty() {
                0
            } else {
                current.split('/').count()
            };

            folders.insert(
                current.clone(),
                FolderEntry {
                    path: current.clone(),
                    name,
                    parent_path,
                    depth,
                    file_count: 0,
                },
            );
        }

        if current.is_empty() {
            break;
        }

        current = current
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_default();
    }
}

pub(crate) fn get_index_snapshot(app: AppHandle, path: String) -> CommandResult<IndexSnapshot> {
    let canonical_path = canonicalize_folder(&path)
        .map(|canonical| path_display(&canonical))
        .unwrap_or(path);

    let connection = open_database(&app)?;
    let root_id = root_id(&connection, &canonical_path)?.ok_or_else(|| {
        format!(
            "No index found for '{}'. Add the folder first.",
            canonical_path
        )
    })?;

    let indexed_at_ms = connection
        .query_row(
            "SELECT last_indexed_ms FROM roots WHERE id = ?1",
            params![root_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not read root timestamp: {error}"))?;

    let mut statement = connection
        .prepare(
            "
            SELECT id, relative_path, modified_ms, heading_count
            FROM files
            WHERE root_id = ?1
            ORDER BY relative_path
            ",
        )
        .map_err(|error| format!("Could not prepare file snapshot query: {error}"))?;

    let rows = statement
        .query_map(params![root_id], |row| {
            Ok(FileRecord {
                id: row.get(0)?,
                relative_path: row.get(1)?,
                modified_ms: row.get(2)?,
                heading_count: row.get(3)?,
            })
        })
        .map_err(|error| format!("Could not iterate indexed files: {error}"))?;

    let mut files = Vec::new();
    let mut folders = HashMap::new();
    ensure_folder_with_ancestors(&mut folders, "");

    for row in rows {
        let record = row.map_err(|error| format!("Could not parse indexed file row: {error}"))?;
        let folder_path = folder_from_relative(&record.relative_path);
        ensure_folder_with_ancestors(&mut folders, &folder_path);

        let mut current_folder = folder_path.clone();
        loop {
            if let Some(folder_entry) = folders.get_mut(&current_folder) {
                folder_entry.file_count += 1;
            }

            if current_folder.is_empty() {
                break;
            }

            current_folder = current_folder
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_string())
                .unwrap_or_default();
        }

        files.push(IndexedFile {
            id: record.id,
            file_name: file_name_from_relative(&record.relative_path),
            relative_path: record.relative_path,
            folder_path,
            modified_ms: record.modified_ms,
            heading_count: record.heading_count,
        });
    }

    let mut folder_values = folders.into_values().collect::<Vec<FolderEntry>>();
    folder_values.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then(left.path.cmp(&right.path))
    });

    Ok(IndexSnapshot {
        root_path: canonical_path,
        indexed_at_ms,
        folders: folder_values,
        files,
    })
}

pub(crate) fn get_file_preview(app: AppHandle, file_id: i64) -> CommandResult<FilePreview> {
    let connection = open_database(&app)?;

    let (relative_path, absolute_path, heading_count) = connection
        .query_row(
            "SELECT relative_path, absolute_path, heading_count FROM files WHERE id = ?1",
            params![file_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(|error| format!("Could not load file preview metadata: {error}"))?;
    let mut headings_statement = connection
        .prepare(
            "
            SELECT id, heading_order, level, text
            FROM headings
            WHERE file_id = ?1
            ORDER BY heading_order ASC
            ",
        )
        .map_err(|error| format!("Could not prepare file heading preview query: {error}"))?;
    let heading_rows = headings_statement
        .query_map(params![file_id], |row| {
            let text = row.get::<_, String>(3)?;
            Ok(FileHeading {
                id: row.get::<_, i64>(0)?,
                order: row.get::<_, i64>(1)?,
                level: row.get::<_, i64>(2)?,
                copy_text: text.clone(),
                text,
            })
        })
        .map_err(|error| format!("Could not read file heading preview rows: {error}"))?;
    let mut headings = Vec::new();
    for row in heading_rows {
        headings.push(
            row.map_err(|error| format!("Could not parse file heading preview row: {error}"))?,
        );
    }

    let f8_cites = Vec::new();

    Ok(FilePreview {
        file_id,
        file_name: file_name_from_relative(&relative_path),
        relative_path,
        absolute_path,
        heading_count: i64::try_from(headings.len()).unwrap_or(heading_count),
        headings,
        f8_cites,
    })
}

pub(crate) fn get_heading_preview_html(
    app: AppHandle,
    file_id: i64,
    heading_order: i64,
) -> CommandResult<String> {
    if heading_order <= 0 {
        return Ok(String::new());
    }

    let connection = open_database(&app)?;
    let absolute_path = connection
        .query_row(
            "SELECT absolute_path FROM files WHERE id = ?1",
            params![file_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Could not load heading preview source file: {error}"))?;

    extract_heading_preview_html(Path::new(&absolute_path), heading_order)
}

pub(crate) async fn search_index(
    app: AppHandle,
    query: String,
    root_path: Option<String>,
    limit: Option<usize>,
) -> CommandResult<Vec<SearchHit>> {
    crate::async_runtime::spawn_blocking(move || {
        query_engine::search_lexical(&app, &query, root_path, limit)
    })
    .await
    .map_err(|error| format!("Lexical search command failed: {error}"))?
}

pub(crate) async fn search_index_semantic(
    app: AppHandle,
    query: String,
    root_path: Option<String>,
    limit: Option<usize>,
) -> CommandResult<Vec<SearchHit>> {
    query_engine::search_semantic(&app, &query, root_path, limit).await
}

pub(crate) async fn search_index_hybrid(
    app: AppHandle,
    query: String,
    root_path: Option<String>,
    limit: Option<usize>,
    file_name_only: Option<bool>,
    semantic_enabled: Option<bool>,
) -> CommandResult<Vec<SearchHit>> {
    let mode = if semantic_enabled.unwrap_or(true) {
        SearchMode::Mixed
    } else {
        SearchMode::Keyword
    };
    let response = search_engine::search(
        &app,
        SearchRequest {
            query,
            mode: Some(mode),
            root_paths: root_path.map(|path| vec![path]),
            limit,
            filters: Some(SearchFilters {
                file_name_only,
                ..SearchFilters::default()
            }),
            ..SearchRequest::default()
        },
    )?;
    Ok(response
        .results
        .into_iter()
        .map(|result| SearchHit {
            source: result.source,
            kind: result.kind,
            file_id: result.file_id,
            file_name: result.file_name,
            relative_path: result.relative_path,
            absolute_path: result.absolute_path,
            heading_level: result.heading_level,
            heading_text: result.heading_text,
            heading_order: result.heading_order,
            score: result.score,
        })
        .collect())
}

pub(crate) async fn search(
    app: AppHandle,
    request: SearchRequest,
) -> CommandResult<SearchResponse> {
    crate::async_runtime::spawn_blocking(move || search_engine::search(&app, request))
        .await
        .map_err(|error| format!("Search command failed: {error}"))?
}

pub(crate) async fn hydrate_search_results(
    app: AppHandle,
    request: SearchHydrateRequest,
) -> CommandResult<SearchHydrateResponse> {
    crate::async_runtime::spawn_blocking(move || search_engine::hydrate_results(&app, request))
        .await
        .map_err(|error| format!("Search hydration command failed: {error}"))?
}

pub(crate) fn search_warm(app: AppHandle) -> CommandResult<SearchWarmResult> {
    search_engine::warm(&app)
}

pub(crate) fn index_status(app: AppHandle) -> CommandResult<SearchIndexStatus> {
    search_engine::index_status(&app)
}

pub(crate) fn index_optimize(app: AppHandle) -> CommandResult<SearchWarmResult> {
    search_engine::optimize(&app)
}

pub(crate) fn semantic_install_status(app: AppHandle) -> CommandResult<SemanticInstallStatus> {
    Ok(search_engine::semantic_install_status(&app))
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    if sorted_samples.is_empty() {
        return 0.0;
    }
    let clamped = percentile.clamp(0.0, 1.0);
    let last = sorted_samples.len().saturating_sub(1);
    let index = ((last as f64) * clamped).round() as usize;
    sorted_samples[index.min(last)]
}

fn latency_stats(samples: &[f64]) -> BenchmarkLatencyStats {
    if samples.is_empty() {
        return BenchmarkLatencyStats::default();
    }

    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let sum = sorted.iter().copied().sum::<f64>();
    BenchmarkLatencyStats {
        runs: sorted.len(),
        min_ms: *sorted.first().unwrap_or(&0.0),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        max_ms: *sorted.last().unwrap_or(&0.0),
        mean_ms: sum / (sorted.len() as f64),
    }
}

fn build_task_result(
    enabled: bool,
    samples: &[f64],
    total_hits: usize,
    error: Option<String>,
) -> BenchmarkTaskResult {
    BenchmarkTaskResult {
        enabled,
        error,
        total_hits,
        latency: latency_stats(samples),
        tier_timings_ms: HashMap::new(),
        tier_hit_counts: HashMap::new(),
        doc_fetch_ms: 0.0,
        fallbacks_triggered: Vec::new(),
    }
}

#[derive(Default)]
struct TelemetryAccumulator {
    tier_timings_ms: HashMap<String, f64>,
    tier_hit_counts: HashMap<String, usize>,
    doc_fetch_ms_total: f64,
    fallbacks_triggered: HashSet<String>,
    samples: usize,
}

fn add_telemetry_sample(
    accumulator: &mut TelemetryAccumulator,
    telemetry: &LexicalSearchTelemetry,
) {
    for (tier, value) in &telemetry.tier_timings_ms {
        *accumulator
            .tier_timings_ms
            .entry(tier.clone())
            .or_insert(0.0) += value;
    }
    for (tier, value) in &telemetry.tier_hit_counts {
        *accumulator.tier_hit_counts.entry(tier.clone()).or_insert(0) += value;
    }
    accumulator.doc_fetch_ms_total += telemetry.doc_fetch_ms;
    for fallback in &telemetry.fallbacks_triggered {
        accumulator.fallbacks_triggered.insert(fallback.clone());
    }
    accumulator.samples = accumulator.samples.saturating_add(1);
}

fn finalize_telemetry(result: &mut BenchmarkTaskResult, accumulator: &TelemetryAccumulator) {
    if accumulator.samples == 0 {
        return;
    }
    let samples = accumulator.samples as f64;
    result.tier_timings_ms = accumulator
        .tier_timings_ms
        .iter()
        .map(|(tier, value)| (tier.clone(), *value / samples))
        .collect::<HashMap<String, f64>>();
    result.tier_hit_counts = accumulator
        .tier_hit_counts
        .iter()
        .map(|(tier, value)| (tier.clone(), *value / accumulator.samples))
        .collect::<HashMap<String, usize>>();
    result.doc_fetch_ms = accumulator.doc_fetch_ms_total / samples;
    result.fallbacks_triggered = accumulator
        .fallbacks_triggered
        .iter()
        .cloned()
        .collect::<Vec<String>>();
    result.fallbacks_triggered.sort();
}

fn query_candidates_from_text(text: &str) -> Vec<String> {
    let normalized = normalize_for_search(text);
    if normalized.is_empty() {
        return Vec::new();
    }

    let tokens = normalized
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_string())
        .collect::<Vec<String>>();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let head_three = tokens.iter().take(3).cloned().collect::<Vec<String>>();
    if !head_three.is_empty() {
        candidates.push(head_three.join(" "));
    }
    if tokens.len() >= 2 {
        candidates.push(
            tokens
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<String>>()
                .join(" "),
        );
    }
    candidates.push(tokens[0].clone());
    if tokens.len() >= 4 {
        let tail_two = tokens[tokens.len().saturating_sub(2)..]
            .iter()
            .cloned()
            .collect::<Vec<String>>()
            .join(" ");
        candidates.push(tail_two);
    }

    candidates
}

fn push_query_candidate(
    target: &mut Vec<String>,
    seen: &mut HashSet<String>,
    candidate: String,
    max_queries: usize,
) {
    let normalized = normalize_for_search(&candidate);
    if normalized.chars().count() < 2 {
        return;
    }
    if seen.insert(normalized.clone()) {
        target.push(normalized);
    }
    if target.len() > max_queries {
        target.truncate(max_queries);
    }
}

fn collect_benchmark_queries(
    connection: &Connection,
    root_id_value: i64,
    provided_queries: &[String],
    max_queries: usize,
) -> CommandResult<Vec<String>> {
    let mut queries = Vec::new();
    let mut seen = HashSet::new();

    for provided in provided_queries {
        for candidate in query_candidates_from_text(provided) {
            if queries.len() >= max_queries {
                return Ok(queries);
            }
            push_query_candidate(&mut queries, &mut seen, candidate, max_queries);
        }
    }

    let mut heading_statement = connection
        .prepare(
            "
            SELECT h.text
            FROM headings h
            JOIN files f ON f.id = h.file_id
            WHERE f.root_id = ?1
            ORDER BY length(h.text) DESC, h.id ASC
            LIMIT 240
            ",
        )
        .map_err(|error| format!("Could not prepare benchmark heading query source: {error}"))?;
    let heading_rows = heading_statement
        .query_map(params![root_id_value], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Could not load benchmark heading query source: {error}"))?;
    for row in heading_rows {
        if queries.len() >= max_queries {
            break;
        }
        let text =
            row.map_err(|error| format!("Could not parse benchmark heading text: {error}"))?;
        for candidate in query_candidates_from_text(&text) {
            if queries.len() >= max_queries {
                break;
            }
            push_query_candidate(&mut queries, &mut seen, candidate, max_queries);
        }
    }

    let mut author_statement = connection
        .prepare(
            "
            SELECT a.text
            FROM authors a
            JOIN files f ON f.id = a.file_id
            WHERE f.root_id = ?1
            ORDER BY a.id DESC
            LIMIT 120
            ",
        )
        .map_err(|error| format!("Could not prepare benchmark author query source: {error}"))?;
    let author_rows = author_statement
        .query_map(params![root_id_value], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Could not load benchmark author query source: {error}"))?;
    for row in author_rows {
        if queries.len() >= max_queries {
            break;
        }
        let text =
            row.map_err(|error| format!("Could not parse benchmark author text: {error}"))?;
        for candidate in query_candidates_from_text(&text) {
            if queries.len() >= max_queries {
                break;
            }
            push_query_candidate(&mut queries, &mut seen, candidate, max_queries);
        }
    }

    let mut file_statement = connection
        .prepare(
            "
            SELECT relative_path
            FROM files
            WHERE root_id = ?1
            ORDER BY heading_count DESC, modified_ms DESC
            LIMIT 180
            ",
        )
        .map_err(|error| format!("Could not prepare benchmark file query source: {error}"))?;
    let file_rows = file_statement
        .query_map(params![root_id_value], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Could not load benchmark file query source: {error}"))?;
    for row in file_rows {
        if queries.len() >= max_queries {
            break;
        }
        let relative_path_value =
            row.map_err(|error| format!("Could not parse benchmark file relative path: {error}"))?;
        let file_name = file_name_from_relative(&relative_path_value);
        for candidate in query_candidates_from_text(&file_name) {
            if queries.len() >= max_queries {
                break;
            }
            push_query_candidate(&mut queries, &mut seen, candidate, max_queries);
        }
    }

    if queries.is_empty() {
        for fallback in [
            "introduction",
            "method",
            "results",
            "discussion",
            "conclusion",
            "references",
        ] {
            push_query_candidate(&mut queries, &mut seen, fallback.to_string(), max_queries);
        }
    }

    Ok(queries)
}

fn sample_file_ids(
    connection: &Connection,
    root_id_value: i64,
    limit: usize,
) -> CommandResult<Vec<i64>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut statement = connection
        .prepare(
            "
            SELECT id
            FROM files
            WHERE root_id = ?1
            ORDER BY heading_count DESC, modified_ms DESC, id DESC
            LIMIT ?2
            ",
        )
        .map_err(|error| {
            format!("Could not prepare benchmark file preview sample query: {error}")
        })?;
    let rows = statement
        .query_map(params![root_id_value, limit_i64], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| format!("Could not run benchmark file preview sample query: {error}"))?;

    let mut output = Vec::new();
    for row in rows {
        output.push(row.map_err(|error| format!("Could not parse sampled file id: {error}"))?);
    }
    Ok(output)
}

fn sample_heading_refs(
    connection: &Connection,
    root_id_value: i64,
    limit: usize,
) -> CommandResult<Vec<(i64, i64)>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut statement = connection
        .prepare(
            "
            SELECT h.file_id, h.heading_order
            FROM headings h
            JOIN files f ON f.id = h.file_id
            WHERE f.root_id = ?1
            ORDER BY f.heading_count DESC, h.heading_order ASC
            LIMIT ?2
            ",
        )
        .map_err(|error| {
            format!("Could not prepare benchmark heading preview sample query: {error}")
        })?;
    let rows = statement
        .query_map(params![root_id_value, limit_i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| {
            format!("Could not run benchmark heading preview sample query: {error}")
        })?;

    let mut output = Vec::new();
    for row in rows {
        output.push(
            row.map_err(|error| format!("Could not parse sampled heading reference: {error}"))?,
        );
    }
    Ok(output)
}

async fn run_benchmark_search_summary(
    app: &AppHandle,
    root_path: &str,
    root_id_value: i64,
    benchmark_queries: &[String],
    benchmark_iterations: usize,
    benchmark_limit: usize,
    benchmark_include_semantic: bool,
) -> BenchmarkSearchSummary {
    let mut search = BenchmarkSearchSummary {
        query_count: benchmark_queries.len(),
        iterations: benchmark_iterations,
        limit: benchmark_limit,
        ..BenchmarkSearchSummary::default()
    };

    let mut lexical_raw_samples = Vec::new();
    let mut lexical_raw_hits = 0_usize;
    let mut lexical_raw_error: Option<String> = None;
    let mut lexical_raw_telemetry = TelemetryAccumulator::default();
    'lexical_raw: for _ in 0..benchmark_iterations {
        for query in benchmark_queries {
            let started = Instant::now();
            match lexical::search_with_telemetry(
                app,
                query,
                Some(root_id_value),
                benchmark_limit,
                false,
            ) {
                Ok((hits, telemetry)) => {
                    lexical_raw_samples.push(elapsed_ms(started));
                    lexical_raw_hits = lexical_raw_hits.saturating_add(hits.len());
                    add_telemetry_sample(&mut lexical_raw_telemetry, &telemetry);
                }
                Err(error) => {
                    lexical_raw_error = Some(error);
                    break 'lexical_raw;
                }
            }
        }
    }
    search.lexical_raw = build_task_result(
        true,
        &lexical_raw_samples,
        lexical_raw_hits,
        lexical_raw_error,
    );
    finalize_telemetry(&mut search.lexical_raw, &lexical_raw_telemetry);

    query_engine::clear_query_cache();
    for query in benchmark_queries {
        let _ = query_engine::search_lexical(
            app,
            query,
            Some(root_path.to_string()),
            Some(benchmark_limit),
        );
    }
    let mut lexical_cached_samples = Vec::new();
    let mut lexical_cached_hits = 0_usize;
    let mut lexical_cached_error: Option<String> = None;
    'lexical_cached: for _ in 0..benchmark_iterations {
        for query in benchmark_queries {
            let started = Instant::now();
            match query_engine::search_lexical(
                app,
                query,
                Some(root_path.to_string()),
                Some(benchmark_limit),
            ) {
                Ok(hits) => {
                    lexical_cached_samples.push(elapsed_ms(started));
                    lexical_cached_hits = lexical_cached_hits.saturating_add(hits.len());
                }
                Err(error) => {
                    lexical_cached_error = Some(error);
                    break 'lexical_cached;
                }
            }
        }
    }
    search.lexical_cached = build_task_result(
        true,
        &lexical_cached_samples,
        lexical_cached_hits,
        lexical_cached_error,
    );

    if benchmark_include_semantic {
        query_engine::clear_query_cache();
        for query in benchmark_queries {
            let _ = query_engine::search_hybrid(
                app,
                query,
                Some(root_path.to_string()),
                Some(benchmark_limit),
                false,
                true,
            )
            .await;
        }

        let mut hybrid_samples = Vec::new();
        let mut hybrid_hits = 0_usize;
        let mut hybrid_error: Option<String> = None;
        'hybrid: for _ in 0..benchmark_iterations {
            for query in benchmark_queries {
                let started = Instant::now();
                match query_engine::search_hybrid(
                    app,
                    query,
                    Some(root_path.to_string()),
                    Some(benchmark_limit),
                    false,
                    true,
                )
                .await
                {
                    Ok(hits) => {
                        hybrid_samples.push(elapsed_ms(started));
                        hybrid_hits = hybrid_hits.saturating_add(hits.len());
                    }
                    Err(error) => {
                        hybrid_error = Some(error);
                        break 'hybrid;
                    }
                }
            }
        }
        search.hybrid = build_task_result(true, &hybrid_samples, hybrid_hits, hybrid_error);

        let mut semantic_samples = Vec::new();
        let mut semantic_hits = 0_usize;
        let mut semantic_error: Option<String> = None;
        if let Some(warm_query) = benchmark_queries.first() {
            let _ = query_engine::search_semantic(
                app,
                warm_query,
                Some(root_path.to_string()),
                Some(benchmark_limit),
            )
            .await;
        }
        'semantic: for _ in 0..benchmark_iterations {
            for query in benchmark_queries {
                let started = Instant::now();
                match query_engine::search_semantic(
                    app,
                    query,
                    Some(root_path.to_string()),
                    Some(benchmark_limit),
                )
                .await
                {
                    Ok(hits) => {
                        semantic_samples.push(elapsed_ms(started));
                        semantic_hits = semantic_hits.saturating_add(hits.len());
                    }
                    Err(error) => {
                        semantic_error = Some(error);
                        break 'semantic;
                    }
                }
            }
        }
        search.semantic = build_task_result(true, &semantic_samples, semantic_hits, semantic_error);
    } else {
        search.hybrid = build_task_result(false, &[], 0, None);
        search.semantic = build_task_result(false, &[], 0, None);
    }

    search
}

pub(crate) async fn benchmark_root_performance(
    app: AppHandle,
    path: String,
    queries: Option<Vec<String>>,
    iterations: Option<usize>,
    limit: Option<usize>,
    include_semantic: Option<bool>,
    preview_samples: Option<usize>,
) -> CommandResult<BenchmarkReport> {
    let benchmark_started = Instant::now();
    let canonical_root = canonicalize_folder(&path)?;
    let root_path = path_display(&canonical_root);

    add_root(app.clone(), root_path.clone())?;
    let index_full = index_root(app.clone(), root_path.clone())?;
    let index_incremental = index_root(app.clone(), root_path.clone())?;

    let connection = open_database(&app)?;
    let root_id_value = root_id(&connection, &root_path)?.ok_or_else(|| {
        format!(
            "Benchmark root id missing for '{}'. Try indexing again.",
            root_path
        )
    })?;

    let benchmark_iterations = iterations.unwrap_or(3).clamp(1, 12);
    let benchmark_limit = limit.unwrap_or(80).clamp(10, 400);
    let benchmark_include_semantic = include_semantic.unwrap_or(true);
    let benchmark_preview_samples = preview_samples.unwrap_or(16).clamp(0, 240);
    let provided_queries = queries.unwrap_or_default();
    let benchmark_queries =
        collect_benchmark_queries(&connection, root_id_value, &provided_queries, 32)?;

    let search = run_benchmark_search_summary(
        &app,
        &root_path,
        root_id_value,
        &benchmark_queries,
        benchmark_iterations,
        benchmark_limit,
        benchmark_include_semantic,
    )
    .await;

    let snapshot_started = Instant::now();
    let _ = get_index_snapshot(app.clone(), root_path.clone())?;
    let mut preview = BenchmarkPreviewSummary {
        snapshot_ms: elapsed_ms(snapshot_started),
        ..BenchmarkPreviewSummary::default()
    };

    let sampled_file_ids = sample_file_ids(&connection, root_id_value, benchmark_preview_samples)?;
    let mut file_preview_samples = Vec::new();
    let mut file_preview_hits = 0_usize;
    let mut file_preview_error: Option<String> = None;
    for file_id in sampled_file_ids {
        let started = Instant::now();
        match get_file_preview(app.clone(), file_id) {
            Ok(file_preview) => {
                file_preview_samples.push(elapsed_ms(started));
                file_preview_hits = file_preview_hits
                    .saturating_add(usize::try_from(file_preview.heading_count).unwrap_or(0));
            }
            Err(error) => {
                file_preview_error = Some(error);
                break;
            }
        }
    }
    preview.file_preview = build_task_result(
        benchmark_preview_samples > 0,
        &file_preview_samples,
        file_preview_hits,
        file_preview_error,
    );

    let sampled_heading_refs =
        sample_heading_refs(&connection, root_id_value, benchmark_preview_samples)?;
    let mut heading_preview_samples = Vec::new();
    let mut heading_preview_hits = 0_usize;
    let mut heading_preview_error: Option<String> = None;
    for (file_id, heading_order) in sampled_heading_refs {
        let started = Instant::now();
        match get_heading_preview_html(app.clone(), file_id, heading_order) {
            Ok(html) => {
                heading_preview_samples.push(elapsed_ms(started));
                if !html.trim().is_empty() {
                    heading_preview_hits = heading_preview_hits.saturating_add(1);
                }
            }
            Err(error) => {
                heading_preview_error = Some(error);
                break;
            }
        }
    }
    preview.heading_preview_html = build_task_result(
        benchmark_preview_samples > 0,
        &heading_preview_samples,
        heading_preview_hits,
        heading_preview_error,
    );

    Ok(BenchmarkReport {
        root_path,
        index_full,
        index_incremental,
        queries: benchmark_queries,
        search,
        preview,
        generated_at_ms: now_ms(),
        elapsed_ms: elapsed_ms(benchmark_started).round() as i64,
    })
}

pub(crate) async fn benchmark_query_runtime(
    app: AppHandle,
    path: String,
    queries: Option<Vec<String>>,
    iterations: Option<usize>,
    limit: Option<usize>,
    include_semantic: Option<bool>,
) -> CommandResult<BenchmarkQueryRuntimeReport> {
    let benchmark_started = Instant::now();
    let canonical_root = canonicalize_folder(&path)?;
    let root_path = path_display(&canonical_root);

    add_root(app.clone(), root_path.clone())?;
    let connection = open_database(&app)?;
    let root_id_value = root_id(&connection, &root_path)?.ok_or_else(|| {
        format!(
            "Benchmark root id missing for '{}'. Try adding or indexing this root.",
            root_path
        )
    })?;

    let indexed_files = connection
        .query_row(
            "SELECT COUNT(*) FROM files WHERE root_id = ?1",
            params![root_id_value],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Could not count indexed files for runtime benchmark: {error}"))?;
    if indexed_files <= 0 {
        return Err(format!(
            "Runtime benchmark requires indexed files for '{}'. Run index_root first.",
            root_path
        ));
    }

    let benchmark_iterations = iterations.unwrap_or(3).clamp(1, 12);
    let benchmark_limit = limit.unwrap_or(80).clamp(10, 400);
    let benchmark_include_semantic = include_semantic.unwrap_or(true);
    let provided_queries = queries.unwrap_or_default();
    let benchmark_queries =
        collect_benchmark_queries(&connection, root_id_value, &provided_queries, 32)?;

    let search = run_benchmark_search_summary(
        &app,
        &root_path,
        root_id_value,
        &benchmark_queries,
        benchmark_iterations,
        benchmark_limit,
        benchmark_include_semantic,
    )
    .await;

    Ok(BenchmarkQueryRuntimeReport {
        root_path,
        queries: benchmark_queries,
        search,
        generated_at_ms: now_ms(),
        elapsed_ms: elapsed_ms(benchmark_started).round() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::{latency_stats, query_candidates_from_text};

    #[test]
    fn query_candidates_produces_multiple_usable_forms() {
        let candidates = query_candidates_from_text("The quick brown fox jumps over fences");

        assert!(candidates.contains(&"the quick brown".to_string()));
        assert!(candidates.contains(&"the quick".to_string()));
        assert!(candidates.contains(&"the".to_string()));
        assert!(candidates.contains(&"over fences".to_string()));
    }

    #[test]
    fn latency_stats_computes_expected_percentiles() {
        let stats = latency_stats(&[10.0, 20.0, 30.0, 40.0]);

        assert_eq!(stats.runs, 4);
        assert_eq!(stats.min_ms, 10.0);
        assert_eq!(stats.p50_ms, 30.0);
        assert_eq!(stats.p95_ms, 40.0);
        assert_eq!(stats.max_ms, 40.0);
        assert_eq!(stats.mean_ms, 25.0);
    }
}
