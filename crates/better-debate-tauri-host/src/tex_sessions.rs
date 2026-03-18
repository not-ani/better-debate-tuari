use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use better_debate_core::{open_tex_document, save_tex_document, TexBlock, TexDocumentPayload};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

const SESSIONS_DIR_NAME: &str = "tex-sessions";
const DETACHED_WINDOW_PREFIX: &str = "doc-";
const DETACHED_WINDOWS_CHANGED_EVENT: &str = "tex://detached-windows-changed";
const RECOVERABLE_SESSIONS_CHANGED_EVENT: &str = "tex://recoverable-sessions-changed";
const SESSION_POPOUT_ATTACHED_EVENT: &str = "tex://session-popout-attached";
const SESSION_UPDATED_EVENT: &str = "tex://session-updated";
const SPEECH_TARGET_CHANGED_EVENT: &str = "tex://speech-target-changed";

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

pub type SharedTexSessionStore = Arc<Mutex<TexSessionStore>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexSessionSnapshot {
    pub session_id: String,
    pub version: u64,
    pub dirty: bool,
    pub file_path: String,
    pub file_name: String,
    pub paragraph_count: i64,
    pub blocks: Vec<TexBlock>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexSessionSummary {
    pub session_id: String,
    pub file_path: String,
    pub file_name: String,
    pub version: u64,
    pub dirty: bool,
    pub paragraph_count: i64,
    pub updated_at_ms: i64,
}

pub type TexRecoverableSession = TexSessionSummary;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TexSessionOpenResult {
    Opened {
        snapshot: TexSessionSnapshot,
    },
    OwnerConflict {
        session_id: String,
        file_path: String,
        file_name: String,
        owner_window_label: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TexSessionAttachResult {
    Attached {
        snapshot: TexSessionSnapshot,
    },
    OwnerConflict {
        session_id: String,
        file_path: String,
        file_name: String,
        owner_window_label: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexPopoutTarget {
    pub session_id: String,
    pub file_path: String,
    pub file_name: String,
    pub window_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachedWindowEntry {
    pub label: String,
    pub session_id: String,
    pub file_path: String,
    pub file_name: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TexRouteHeading {
    pub block_index: i64,
    pub level: i64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TexSessionRouteTarget {
    pub session_id: String,
    pub file_path: String,
    pub file_name: String,
    pub owner_window_label: Option<String>,
    pub dirty: bool,
    pub headings: Vec<TexRouteHeading>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TexSpeechTargetState {
    pub target_session_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TexSendInsertMode {
    Below,
    Under,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexSendResult {
    pub target_session_id: String,
    pub snapshot: TexSessionSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexSessionUpdatedEvent {
    pub snapshot: TexSessionSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPopoutAttachedEvent {
    pub session_id: String,
    pub from_window_label: String,
    pub to_window_label: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TexSessionRequestRole {
    Writer,
    Observer,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTexSessionArgs {
    pub file_path: String,
    pub window_label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTexSessionArgs {
    pub file_path: String,
    pub window_label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachTexSessionArgs {
    pub session_id: String,
    pub window_label: String,
    pub request_role: TexSessionRequestRole,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTexSessionArgs {
    pub session_id: String,
    pub window_label: String,
    pub base_version: u64,
    pub document: TexDocumentPayload,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTexSessionArgs {
    pub session_id: String,
    pub window_label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareTexPopoutArgs {
    pub session_id: String,
    pub from_window_label: String,
    pub to_window_label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseTexSessionArgs {
    pub session_id: String,
    pub window_label: String,
    pub discard_unsaved: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdArgs {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSpeechTargetArgs {
    pub target_session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendToSessionArgs {
    pub target_session_id: String,
    pub target_block_index: Option<i64>,
    pub insert_mode: TexSendInsertMode,
    pub source_blocks: Vec<TexBlock>,
    pub source_root_level: i64,
    pub source_max_relative_depth: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTexSession {
    snapshot: TexSessionSnapshot,
    updated_at_ms: i64,
}

#[derive(Clone, Debug)]
struct TexSessionRecord {
    snapshot: TexSessionSnapshot,
    owner_window_label: Option<String>,
    attached_windows: HashSet<String>,
    updated_at_ms: i64,
    persistence_path: PathBuf,
    pending_popout_target: Option<String>,
}

impl TexSessionRecord {
    fn summary(&self) -> TexSessionSummary {
        TexSessionSummary {
            session_id: self.snapshot.session_id.clone(),
            file_path: self.snapshot.file_path.clone(),
            file_name: self.snapshot.file_name.clone(),
            version: self.snapshot.version,
            dirty: self.snapshot.dirty,
            paragraph_count: self.snapshot.paragraph_count,
            updated_at_ms: self.updated_at_ms,
        }
    }

    fn to_detached_entries(&self) -> Vec<DetachedWindowEntry> {
        self.attached_windows
            .iter()
            .filter(|label| is_detached_window_label(label))
            .map(|label| DetachedWindowEntry {
                label: label.clone(),
                session_id: self.snapshot.session_id.clone(),
                file_path: self.snapshot.file_path.clone(),
                file_name: self.snapshot.file_name.clone(),
                updated_at_ms: self.updated_at_ms,
            })
            .collect()
    }
}

pub struct TexSessionStore {
    sessions_dir: PathBuf,
    sessions: HashMap<String, TexSessionRecord>,
    file_to_session: HashMap<String, String>,
    active_speech_target_session_id: Option<String>,
}

impl TexSessionStore {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        let sessions_dir = app_data_dir.join(SESSIONS_DIR_NAME);
        fs::create_dir_all(&sessions_dir)
            .map_err(|error| format!("Could not create Tex sessions directory: {error}"))?;

        Ok(Self {
            sessions_dir,
            sessions: HashMap::new(),
            file_to_session: HashMap::new(),
            active_speech_target_session_id: None,
        })
    }

    fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{session_id}.json"))
    }

    fn persist_record(&self, record: &TexSessionRecord) -> Result<(), String> {
        let persisted = PersistedTexSession {
            snapshot: record.snapshot.clone(),
            updated_at_ms: record.updated_at_ms,
        };
        let payload = serde_json::to_vec_pretty(&persisted).map_err(|error| {
            format!(
                "Could not serialize Tex session '{}': {error}",
                record.snapshot.session_id
            )
        })?;
        let temp_path = record.persistence_path.with_extension("json.tmp");
        fs::write(&temp_path, payload).map_err(|error| {
            format!(
                "Could not write temporary Tex session '{}': {error}",
                record.snapshot.session_id
            )
        })?;
        fs::rename(&temp_path, &record.persistence_path).map_err(|error| {
            format!(
                "Could not finalize Tex session '{}': {error}",
                record.snapshot.session_id
            )
        })?;
        Ok(())
    }

    fn delete_persisted_path(path: &Path) -> Result<(), String> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not delete persisted Tex session: {error}")),
        }
    }

    fn load_persisted(&self, session_id: &str) -> Result<Option<TexSessionRecord>, String> {
        let path = self.session_path(session_id);
        let payload = match fs::read(&path) {
            Ok(payload) => payload,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Could not read persisted Tex session '{session_id}': {error}"
                ))
            }
        };

        let persisted: PersistedTexSession = serde_json::from_slice(&payload).map_err(|error| {
            format!("Could not parse persisted Tex session '{session_id}': {error}")
        })?;

        Ok(Some(TexSessionRecord {
            snapshot: persisted.snapshot,
            owner_window_label: None,
            attached_windows: HashSet::new(),
            updated_at_ms: persisted.updated_at_ms,
            persistence_path: path,
            pending_popout_target: None,
        }))
    }

    fn ensure_loaded(&mut self, session_id: &str) -> Result<(), String> {
        if self.sessions.contains_key(session_id) {
            return Ok(());
        }

        let Some(record) = self.load_persisted(session_id)? else {
            return Err(format!("Tex session '{session_id}' was not found."));
        };

        self.file_to_session.insert(
            record.snapshot.file_path.clone(),
            record.snapshot.session_id.clone(),
        );
        self.sessions.insert(session_id.to_string(), record);
        Ok(())
    }

    fn register_opened_document(
        &mut self,
        document: TexDocumentPayload,
        window_label: String,
    ) -> TexSessionOpenResult {
        let session_id = generate_session_id(&document.file_path);
        let updated_at_ms = now_ms();
        let snapshot = snapshot_from_document(session_id.clone(), 0, false, document);
        let record = TexSessionRecord {
            persistence_path: self.session_path(&session_id),
            snapshot: snapshot.clone(),
            owner_window_label: Some(window_label.clone()),
            attached_windows: HashSet::from([window_label]),
            updated_at_ms,
            pending_popout_target: None,
        };

        self.file_to_session
            .insert(snapshot.file_path.clone(), session_id.clone());
        self.sessions.insert(session_id, record);

        TexSessionOpenResult::Opened { snapshot }
    }

    fn open_session_from_file(
        &mut self,
        file_path: String,
        window_label: String,
    ) -> Result<TexSessionOpenResult, String> {
        if let Some(existing_session_id) = self.file_to_session.get(&file_path).cloned() {
            if let Some(existing) = self.sessions.get_mut(&existing_session_id) {
                if let Some(owner) = existing.owner_window_label.as_ref() {
                    if owner != &window_label {
                        return Ok(TexSessionOpenResult::OwnerConflict {
                            session_id: existing.snapshot.session_id.clone(),
                            file_path: existing.snapshot.file_path.clone(),
                            file_name: existing.snapshot.file_name.clone(),
                            owner_window_label: owner.clone(),
                        });
                    }
                }

                existing.attached_windows.insert(window_label.clone());
                existing.owner_window_label = Some(window_label);
                existing.updated_at_ms = now_ms();
                return Ok(TexSessionOpenResult::Opened {
                    snapshot: existing.snapshot.clone(),
                });
            }

            self.file_to_session.remove(&file_path);
        }

        let document = open_tex_document(Path::new(&file_path))?;
        Ok(self.register_opened_document(document, window_label))
    }

    fn create_session_at_path(
        &mut self,
        file_path: String,
        window_label: String,
    ) -> Result<TexSessionOpenResult, String> {
        let normalized_path = normalize_docx_path(&file_path)?;
        let path = Path::new(&normalized_path);
        if path.exists() {
            return Err(format!(
                "'{}' already exists. Use Open to edit an existing document.",
                path.display()
            ));
        }

        let document = save_tex_document(path, &[])?;
        Ok(self.register_opened_document(document, window_label))
    }

    fn attach_session(
        &mut self,
        session_id: String,
        window_label: String,
        request_role: TexSessionRequestRole,
    ) -> Result<(TexSessionAttachResult, Option<SessionPopoutAttachedEvent>), String> {
        self.ensure_loaded(&session_id)?;
        let record = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("Tex session '{session_id}' was not found."))?;

        let mut popout_event = None;
        if request_role == TexSessionRequestRole::Writer {
            if let Some(owner) = record.owner_window_label.as_ref() {
                if owner != &window_label {
                    if record.pending_popout_target.as_deref() == Some(window_label.as_str()) {
                        popout_event = Some(SessionPopoutAttachedEvent {
                            session_id: session_id.clone(),
                            from_window_label: owner.clone(),
                            to_window_label: window_label.clone(),
                        });
                    } else {
                        return Ok((
                            TexSessionAttachResult::OwnerConflict {
                                session_id: record.snapshot.session_id.clone(),
                                file_path: record.snapshot.file_path.clone(),
                                file_name: record.snapshot.file_name.clone(),
                                owner_window_label: owner.clone(),
                            },
                            None,
                        ));
                    }
                }
            }

            record.owner_window_label = Some(window_label.clone());
            record.pending_popout_target = None;
        }

        record.attached_windows.insert(window_label);
        record.updated_at_ms = now_ms();

        Ok((
            TexSessionAttachResult::Attached {
                snapshot: record.snapshot.clone(),
            },
            popout_event,
        ))
    }

    fn update_session(
        &mut self,
        session_id: String,
        window_label: String,
        base_version: u64,
        document: TexDocumentPayload,
    ) -> Result<TexSessionSnapshot, String> {
        self.ensure_loaded(&session_id)?;
        let record_clone = {
            let record = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| format!("Tex session '{session_id}' was not found."))?;

            let owner = record.owner_window_label.as_ref().ok_or_else(|| {
                format!(
                    "Tex session '{}' does not have a writer.",
                    record.snapshot.session_id
                )
            })?;
            if owner != &window_label {
                return Err(format!(
                    "Window '{window_label}' does not own Tex session '{}'.",
                    record.snapshot.session_id
                ));
            }
            if record.snapshot.version != base_version {
                return Err(format!(
                    "Tex session '{}' is stale. Expected version {}, received {}.",
                    record.snapshot.session_id, record.snapshot.version, base_version
                ));
            }

            record.snapshot = snapshot_from_document(
                record.snapshot.session_id.clone(),
                base_version + 1,
                true,
                document,
            );
            record.updated_at_ms = now_ms();
            record.clone()
        };

        self.persist_record(&record_clone)?;
        Ok(record_clone.snapshot)
    }

    fn save_session(
        &mut self,
        session_id: String,
        window_label: String,
    ) -> Result<TexSessionSnapshot, String> {
        self.ensure_loaded(&session_id)?;
        let record = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("Tex session '{session_id}' was not found."))?;

        let owner = record.owner_window_label.as_ref().ok_or_else(|| {
            format!(
                "Tex session '{}' does not have a writer.",
                record.snapshot.session_id
            )
        })?;
        if owner != &window_label {
            return Err(format!(
                "Window '{window_label}' does not own Tex session '{}'.",
                record.snapshot.session_id
            ));
        }

        let saved_document = save_tex_document(
            Path::new(&record.snapshot.file_path),
            &record.snapshot.blocks,
        )?;
        record.snapshot = snapshot_from_document(
            record.snapshot.session_id.clone(),
            record.snapshot.version + 1,
            false,
            saved_document,
        );
        record.updated_at_ms = now_ms();
        Self::delete_persisted_path(&record.persistence_path)?;
        Ok(record.snapshot.clone())
    }

    fn prepare_popout(
        &mut self,
        session_id: String,
        from_window_label: String,
        to_window_label: String,
    ) -> Result<TexPopoutTarget, String> {
        self.ensure_loaded(&session_id)?;
        let record = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("Tex session '{session_id}' was not found."))?;

        let owner = record.owner_window_label.as_ref().ok_or_else(|| {
            format!(
                "Tex session '{}' does not have a writer.",
                record.snapshot.session_id
            )
        })?;
        if owner != &from_window_label {
            return Err(format!(
                "Window '{from_window_label}' does not own Tex session '{}'.",
                record.snapshot.session_id
            ));
        }

        record.pending_popout_target = Some(to_window_label.clone());
        Ok(TexPopoutTarget {
            session_id: record.snapshot.session_id.clone(),
            file_path: record.snapshot.file_path.clone(),
            file_name: record.snapshot.file_name.clone(),
            window_label: to_window_label,
        })
    }

    fn release_session(
        &mut self,
        session_id: String,
        window_label: String,
        discard_unsaved: bool,
    ) -> Result<(), String> {
        self.ensure_loaded(&session_id)?;
        let Some(mut record) = self.sessions.remove(&session_id) else {
            return Ok(());
        };

        record.attached_windows.remove(&window_label);
        if record.owner_window_label.as_deref() == Some(window_label.as_str()) {
            record.owner_window_label = None;
        }
        if record.pending_popout_target.as_deref() == Some(window_label.as_str()) {
            record.pending_popout_target = None;
        }
        record.updated_at_ms = now_ms();
        self.clear_active_speech_target_if_matches(&session_id);

        if discard_unsaved {
            Self::delete_persisted_path(&record.persistence_path)?;
            if self.file_to_session.get(&record.snapshot.file_path) == Some(&session_id) {
                self.file_to_session.remove(&record.snapshot.file_path);
            }
            return Ok(());
        }

        if record.attached_windows.is_empty() {
            if record.snapshot.dirty {
                if self.file_to_session.get(&record.snapshot.file_path) == Some(&session_id) {
                    self.file_to_session.remove(&record.snapshot.file_path);
                }
                self.persist_record(&record)?;
                return Ok(());
            }

            Self::delete_persisted_path(&record.persistence_path)?;
            if self.file_to_session.get(&record.snapshot.file_path) == Some(&session_id) {
                self.file_to_session.remove(&record.snapshot.file_path);
            }
            return Ok(());
        }

        self.sessions.insert(session_id, record);
        Ok(())
    }

    fn discard_recoverable_session(&mut self, session_id: String) -> Result<(), String> {
        if let Some(record) = self.sessions.remove(&session_id) {
            self.clear_active_speech_target_if_matches(&session_id);
            Self::delete_persisted_path(&record.persistence_path)?;
            if self.file_to_session.get(&record.snapshot.file_path) == Some(&session_id) {
                self.file_to_session.remove(&record.snapshot.file_path);
            }
            return Ok(());
        }

        let path = self.session_path(&session_id);
        self.clear_active_speech_target_if_matches(&session_id);
        Self::delete_persisted_path(&path)
    }

    fn list_open_sessions(&self) -> Vec<TexSessionRouteTarget> {
        let mut entries = self
            .sessions
            .values()
            .map(|record| TexSessionRouteTarget {
                session_id: record.snapshot.session_id.clone(),
                file_path: record.snapshot.file_path.clone(),
                file_name: record.snapshot.file_name.clone(),
                owner_window_label: record.owner_window_label.clone(),
                dirty: record.snapshot.dirty,
                headings: route_headings(&record.snapshot.blocks),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        entries
    }

    fn active_speech_target_state(&self) -> TexSpeechTargetState {
        TexSpeechTargetState {
            target_session_id: self
                .active_speech_target_session_id
                .clone()
                .filter(|session_id| self.sessions.contains_key(session_id)),
        }
    }

    fn set_active_speech_target(
        &mut self,
        target_session_id: Option<String>,
    ) -> Result<TexSpeechTargetState, String> {
        if let Some(session_id) = target_session_id.as_ref() {
            self.ensure_loaded(session_id)?;
            if !self.sessions.contains_key(session_id) {
                return Err(format!("Tex session '{session_id}' was not found."));
            }
        }

        self.active_speech_target_session_id = target_session_id;
        Ok(self.active_speech_target_state())
    }

    fn send_to_session(&mut self, args: SendToSessionArgs) -> Result<TexSendResult, String> {
        self.ensure_loaded(&args.target_session_id)?;
        let record = self
            .sessions
            .get_mut(&args.target_session_id)
            .ok_or_else(|| format!("Tex session '{}' was not found.", args.target_session_id))?;

        let insertion = build_send_insertion(
            &record.snapshot.blocks,
            args.target_block_index,
            args.insert_mode,
            &args.source_blocks,
            args.source_root_level,
            args.source_max_relative_depth,
        )?;

        let mut next_blocks = record.snapshot.blocks.clone();
        next_blocks.splice(insertion.index..insertion.index, insertion.blocks);

        record.snapshot.blocks = next_blocks;
        record.snapshot.paragraph_count =
            i64::try_from(record.snapshot.blocks.len()).unwrap_or(i64::MAX);
        record.snapshot.version = record.snapshot.version.saturating_add(1);
        record.snapshot.dirty = true;
        record.updated_at_ms = now_ms();

        let record_clone = record.clone();
        self.persist_record(&record_clone)?;

        Ok(TexSendResult {
            target_session_id: args.target_session_id,
            snapshot: record_clone.snapshot,
        })
    }

    fn clear_active_speech_target_if_matches(&mut self, session_id: &str) {
        if self.active_speech_target_session_id.as_deref() == Some(session_id) {
            self.active_speech_target_session_id = None;
        }
    }

    fn detached_windows(&self) -> Vec<DetachedWindowEntry> {
        let mut entries = self
            .sessions
            .values()
            .flat_map(TexSessionRecord::to_detached_entries)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| right.updated_at_ms.cmp(&left.updated_at_ms));
        entries
    }

    fn recoverable_sessions(&self) -> Result<Vec<TexRecoverableSession>, String> {
        let mut recoverable = Vec::new();
        let read_dir = match fs::read_dir(&self.sessions_dir) {
            Ok(read_dir) => read_dir,
            Err(error) => {
                return Err(format!(
                    "Could not list persisted Tex sessions in '{}': {error}",
                    self.sessions_dir.display()
                ))
            }
        };

        for entry in read_dir {
            let entry = entry
                .map_err(|error| format!("Could not read persisted Tex session entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let payload = fs::read(&path).map_err(|error| {
                format!(
                    "Could not read persisted Tex session '{}': {error}",
                    path.display()
                )
            })?;
            let persisted: PersistedTexSession =
                serde_json::from_slice(&payload).map_err(|error| {
                    format!(
                        "Could not parse persisted Tex session '{}': {error}",
                        path.display()
                    )
                })?;

            if !persisted.snapshot.dirty {
                continue;
            }
            if self.sessions.contains_key(&persisted.snapshot.session_id) {
                continue;
            }

            recoverable.push(TexSessionSummary {
                session_id: persisted.snapshot.session_id,
                file_path: persisted.snapshot.file_path,
                file_name: persisted.snapshot.file_name,
                version: persisted.snapshot.version,
                dirty: persisted.snapshot.dirty,
                paragraph_count: persisted.snapshot.paragraph_count,
                updated_at_ms: persisted.updated_at_ms,
            });
        }

        recoverable.sort_by(|left, right| right.updated_at_ms.cmp(&left.updated_at_ms));
        Ok(recoverable)
    }

    fn prune_missing_windows(&mut self, app: &AppHandle) -> Result<(), String> {
        let session_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        for session_id in session_ids {
            let Some(record) = self.sessions.get_mut(&session_id) else {
                continue;
            };

            record
                .attached_windows
                .retain(|label| app.get_webview_window(label).is_some());
            if record
                .owner_window_label
                .as_ref()
                .is_some_and(|label| app.get_webview_window(label).is_none())
            {
                record.owner_window_label = None;
            }
            if record
                .pending_popout_target
                .as_ref()
                .is_some_and(|label| app.get_webview_window(label).is_none())
            {
                record.pending_popout_target = None;
            }
        }

        let orphaned = self
            .sessions
            .iter()
            .filter(|(_, record)| record.attached_windows.is_empty())
            .map(|(session_id, record)| {
                (
                    session_id.clone(),
                    record.snapshot.file_path.clone(),
                    record.snapshot.dirty,
                    record.persistence_path.clone(),
                )
            })
            .collect::<Vec<_>>();

        for (session_id, file_path, dirty, persistence_path) in orphaned {
            if dirty {
                if let Some(record) = self.sessions.remove(&session_id) {
                    self.persist_record(&record)?;
                }
            } else {
                self.sessions.remove(&session_id);
                Self::delete_persisted_path(&persistence_path)?;
            }

            self.clear_active_speech_target_if_matches(&session_id);

            if self.file_to_session.get(&file_path) == Some(&session_id) {
                self.file_to_session.remove(&file_path);
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct SendInsertion {
    index: usize,
    blocks: Vec<TexBlock>,
}

fn route_headings(blocks: &[TexBlock]) -> Vec<TexRouteHeading> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let level = block.level?;
            if block.kind != "heading" || !(1..=4).contains(&level) {
                return None;
            }

            Some(TexRouteHeading {
                block_index: i64::try_from(index).unwrap_or(i64::MAX),
                level,
                text: if block.text.trim().is_empty() {
                    "(empty)".to_string()
                } else {
                    block.text.clone()
                },
            })
        })
        .collect()
}

fn subtree_end_index(blocks: &[TexBlock], start_index: usize) -> usize {
    let start_level = blocks
        .get(start_index)
        .and_then(|block| (block.kind == "heading").then_some(block.level).flatten())
        .unwrap_or(0);

    let mut end_index = blocks.len();
    for (index, candidate) in blocks.iter().enumerate().skip(start_index + 1) {
        let Some(level) = candidate.level else {
            continue;
        };
        if candidate.kind == "heading" && level <= start_level {
            end_index = index;
            break;
        }
    }
    end_index
}

fn rebase_source_blocks(
    source_blocks: &[TexBlock],
    source_root_level: i64,
    target_root_level: Option<i64>,
    source_max_relative_depth: i64,
) -> Result<Vec<TexBlock>, String> {
    if source_blocks.is_empty() {
        return Err("Cannot send an empty block selection.".to_string());
    }
    if !(1..=4).contains(&source_root_level) {
        return Err("Source root level must be between 1 and 4.".to_string());
    }

    let Some(first_level) = source_blocks.first().and_then(|block| block.level) else {
        return Err("Source selection must begin on a heading block.".to_string());
    };
    if first_level != source_root_level {
        return Err("Source selection root level did not match the active heading.".to_string());
    }

    let Some(target_level) = target_root_level else {
        return Ok(source_blocks.to_vec());
    };
    if !(1..=4).contains(&target_level) {
        return Err("Target level must be between 1 and 4.".to_string());
    }
    if target_level + source_max_relative_depth > 4 {
        return Err("This send would exceed Tag depth.".to_string());
    }

    let mut blocks = source_blocks.to_vec();
    for block in &mut blocks {
        if block.kind != "heading" {
            continue;
        }
        let Some(level) = block.level else {
            continue;
        };
        let relative = level - source_root_level;
        let rebased = target_level + relative;
        if !(1..=4).contains(&rebased) {
            return Err("This send would exceed Tag depth.".to_string());
        }
        block.level = Some(rebased);
    }
    Ok(blocks)
}

fn build_send_insertion(
    destination_blocks: &[TexBlock],
    target_block_index: Option<i64>,
    insert_mode: TexSendInsertMode,
    source_blocks: &[TexBlock],
    source_root_level: i64,
    source_max_relative_depth: i64,
) -> Result<SendInsertion, String> {
    if target_block_index.is_none() {
        if insert_mode != TexSendInsertMode::Below {
            return Err("Document Root only supports Below.".to_string());
        }

        return Ok(SendInsertion {
            index: destination_blocks.len(),
            blocks: rebase_source_blocks(
                source_blocks,
                source_root_level,
                None,
                source_max_relative_depth,
            )?,
        });
    }

    let target_index = usize::try_from(target_block_index.unwrap_or_default())
        .map_err(|_| "Target block index was invalid.".to_string())?;
    let target_block = destination_blocks
        .get(target_index)
        .ok_or_else(|| "Target heading was not found.".to_string())?;
    if target_block.kind != "heading" {
        return Err("Target block must be a heading.".to_string());
    }
    let target_level = target_block
        .level
        .ok_or_else(|| "Target heading level was missing.".to_string())?;
    if !(1..=4).contains(&target_level) {
        return Err("Target heading level must be between 1 and 4.".to_string());
    }

    let rebased_root_level = match insert_mode {
        TexSendInsertMode::Below => target_level,
        TexSendInsertMode::Under => {
            if target_level >= 4 {
                return Err(
                    "Under is unavailable here because this send would exceed Tag depth."
                        .to_string(),
                );
            }
            target_level + 1
        }
    };

    let insertion_index = subtree_end_index(destination_blocks, target_index);
    let blocks = rebase_source_blocks(
        source_blocks,
        source_root_level,
        Some(rebased_root_level),
        source_max_relative_depth,
    )?;

    Ok(SendInsertion {
        index: insertion_index,
        blocks,
    })
}

fn is_detached_window_label(label: &str) -> bool {
    label.starts_with(DETACHED_WINDOW_PREFIX)
}

fn snapshot_from_document(
    session_id: String,
    version: u64,
    dirty: bool,
    document: TexDocumentPayload,
) -> TexSessionSnapshot {
    TexSessionSnapshot {
        session_id,
        version,
        dirty,
        file_path: document.file_path,
        file_name: document.file_name,
        paragraph_count: document.paragraph_count,
        blocks: document.blocks,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn generate_session_id(file_path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    file_path.hash(&mut hasher);
    SESSION_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .hash(&mut hasher);
    now_ms().hash(&mut hasher);
    format!("tex-{:x}", hasher.finish())
}

fn normalize_docx_path(file_path: &str) -> Result<String, String> {
    let trimmed = file_path.trim();
    if trimmed.is_empty() {
        return Err("Tex file path cannot be empty.".to_string());
    }

    let mut normalized = PathBuf::from(trimmed);
    if normalized
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("docx"))
        != Some(true)
    {
        normalized.set_extension("docx");
    }

    Ok(normalized.to_string_lossy().into_owned())
}

fn lock_store<'a>(
    state: &'a State<'_, SharedTexSessionStore>,
) -> Result<std::sync::MutexGuard<'a, TexSessionStore>, String> {
    state
        .lock()
        .map_err(|_| "Could not lock Tex session store.".to_string())
}

fn emit_detached_windows_changed(app: &AppHandle, detached_windows: &[DetachedWindowEntry]) {
    let _ = app.emit(DETACHED_WINDOWS_CHANGED_EVENT, detached_windows);
}

fn emit_recoverable_sessions_changed(
    app: &AppHandle,
    recoverable_sessions: &[TexRecoverableSession],
) {
    let _ = app.emit_to(
        "main",
        RECOVERABLE_SESSIONS_CHANGED_EVENT,
        recoverable_sessions,
    );
}

fn emit_session_updated(app: &AppHandle, snapshot: &TexSessionSnapshot) {
    let _ = app.emit(
        SESSION_UPDATED_EVENT,
        TexSessionUpdatedEvent {
            snapshot: snapshot.clone(),
        },
    );
}

fn emit_speech_target_changed(app: &AppHandle, state: &TexSpeechTargetState) {
    let _ = app.emit(SPEECH_TARGET_CHANGED_EVENT, state);
}

pub fn create_store(app_data_dir: PathBuf) -> Result<SharedTexSessionStore, String> {
    Ok(Arc::new(Mutex::new(TexSessionStore::new(app_data_dir)?)))
}

#[tauri::command]
pub fn tex_open_session_from_file(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: OpenTexSessionArgs,
) -> Result<TexSessionOpenResult, String> {
    let (result, detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let result = store.open_session_from_file(args.file_path, args.window_label)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            result,
            detached_windows,
            recoverable_sessions,
            speech_target_state,
        )
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(result)
}

#[tauri::command]
pub fn tex_create_session_at_path(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: CreateTexSessionArgs,
) -> Result<TexSessionOpenResult, String> {
    let (result, detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let result = store.create_session_at_path(args.file_path, args.window_label)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            result,
            detached_windows,
            recoverable_sessions,
            speech_target_state,
        )
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(result)
}

#[tauri::command]
pub fn tex_attach_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: AttachTexSessionArgs,
) -> Result<TexSessionAttachResult, String> {
    let (result, detached_windows, recoverable_sessions, popout_event, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let (result, popout_event) =
            store.attach_session(args.session_id, args.window_label, args.request_role)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            result,
            detached_windows,
            recoverable_sessions,
            popout_event,
            speech_target_state,
        )
    };

    if let Some(event) = popout_event.as_ref() {
        let _ = app.emit_to(
            &event.from_window_label,
            SESSION_POPOUT_ATTACHED_EVENT,
            event,
        );
    }
    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(result)
}

#[tauri::command]
pub fn tex_update_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: UpdateTexSessionArgs,
) -> Result<TexSessionSnapshot, String> {
    let (snapshot, detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let snapshot = store.update_session(
            args.session_id,
            args.window_label,
            args.base_version,
            args.document,
        )?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            snapshot,
            detached_windows,
            recoverable_sessions,
            speech_target_state,
        )
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(snapshot)
}

#[tauri::command]
pub fn tex_save_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: SaveTexSessionArgs,
) -> Result<TexSessionSnapshot, String> {
    let (snapshot, detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let snapshot = store.save_session(args.session_id, args.window_label)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            snapshot,
            detached_windows,
            recoverable_sessions,
            speech_target_state,
        )
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(snapshot)
}

#[tauri::command]
pub fn tex_prepare_popout(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: PrepareTexPopoutArgs,
) -> Result<TexPopoutTarget, String> {
    let target = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        store.prepare_popout(
            args.session_id,
            args.from_window_label,
            args.to_window_label,
        )?
    };

    Ok(target)
}

#[tauri::command]
pub fn tex_release_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: ReleaseTexSessionArgs,
) -> Result<(), String> {
    let (detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        store.release_session(args.session_id, args.window_label, args.discard_unsaved)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (detached_windows, recoverable_sessions, speech_target_state)
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(())
}

#[tauri::command]
pub fn tex_list_recoverable_sessions(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
) -> Result<Vec<TexRecoverableSession>, String> {
    let (recoverable, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let recoverable = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (recoverable, speech_target_state)
    };
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(recoverable)
}

#[tauri::command]
pub fn tex_discard_recoverable_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: SessionIdArgs,
) -> Result<(), String> {
    let (recoverable, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.discard_recoverable_session(args.session_id)?;
        let recoverable = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (recoverable, speech_target_state)
    };

    emit_recoverable_sessions_changed(&app, &recoverable);
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(())
}

#[tauri::command]
pub fn tex_list_detached_windows(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
) -> Result<Vec<DetachedWindowEntry>, String> {
    let (detached, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let detached = store.detached_windows();
        let speech_target_state = store.active_speech_target_state();
        (detached, speech_target_state)
    };
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(detached)
}

#[tauri::command]
pub fn tex_list_open_sessions(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
) -> Result<Vec<TexSessionRouteTarget>, String> {
    let (sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let sessions = store.list_open_sessions();
        let speech_target_state = store.active_speech_target_state();
        (sessions, speech_target_state)
    };
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(sessions)
}

#[tauri::command]
pub fn tex_get_active_speech_target(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
) -> Result<TexSpeechTargetState, String> {
    let speech_target_state = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        store.active_speech_target_state()
    };
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(speech_target_state)
}

#[tauri::command]
pub fn tex_set_active_speech_target(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: SetSpeechTargetArgs,
) -> Result<TexSpeechTargetState, String> {
    let speech_target_state = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        store.set_active_speech_target(args.target_session_id)?
    };
    emit_speech_target_changed(&app, &speech_target_state);
    Ok(speech_target_state)
}

#[tauri::command]
pub fn tex_send_to_session(
    app: AppHandle,
    state: State<'_, SharedTexSessionStore>,
    args: SendToSessionArgs,
) -> Result<TexSendResult, String> {
    let (result, detached_windows, recoverable_sessions, speech_target_state) = {
        let mut store = lock_store(&state)?;
        store.prune_missing_windows(&app)?;
        let result = store.send_to_session(args)?;
        let detached_windows = store.detached_windows();
        let recoverable_sessions = store.recoverable_sessions()?;
        let speech_target_state = store.active_speech_target_state();
        (
            result,
            detached_windows,
            recoverable_sessions,
            speech_target_state,
        )
    };

    emit_detached_windows_changed(&app, &detached_windows);
    emit_recoverable_sessions_changed(&app, &recoverable_sessions);
    emit_speech_target_changed(&app, &speech_target_state);
    emit_session_updated(&app, &result.snapshot);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_document(path: &str, name: &str) -> TexDocumentPayload {
        TexDocumentPayload {
            file_path: path.to_string(),
            file_name: name.to_string(),
            paragraph_count: 1,
            blocks: vec![TexBlock {
                id: "block-1".to_string(),
                kind: "paragraph".to_string(),
                text: "hello".to_string(),
                runs: vec![],
                level: None,
                style_id: None,
                style_name: None,
                is_f8_cite: false,
            }],
        }
    }

    fn sample_record(temp_dir: &Path, session_id: &str, dirty: bool) -> TexSessionRecord {
        TexSessionRecord {
            snapshot: snapshot_from_document(
                session_id.to_string(),
                0,
                dirty,
                sample_document("/tmp/example.docx", "example.docx"),
            ),
            owner_window_label: Some("main".to_string()),
            attached_windows: HashSet::from(["main".to_string()]),
            updated_at_ms: 123,
            persistence_path: temp_dir.join(format!("{session_id}.json")),
            pending_popout_target: None,
        }
    }

    #[test]
    fn persist_and_load_round_trip() {
        let temp_dir =
            std::env::temp_dir().join(format!("tex-session-test-{}", generate_session_id("one")));
        fs::create_dir_all(&temp_dir).unwrap();
        let store = TexSessionStore {
            sessions_dir: temp_dir.clone(),
            sessions: HashMap::new(),
            file_to_session: HashMap::new(),
            active_speech_target_session_id: None,
        };
        let record = sample_record(&temp_dir, "session-1", true);
        store.persist_record(&record).unwrap();

        let loaded = store.load_persisted("session-1").unwrap().unwrap();
        assert!(loaded.snapshot.dirty);
        assert_eq!(loaded.snapshot.file_name, "example.docx");

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn release_dirty_session_keeps_recovery_file() {
        let temp_root =
            std::env::temp_dir().join(format!("tex-session-test-{}", generate_session_id("two")));
        let mut store = TexSessionStore::new(temp_root.clone()).unwrap();
        let session_id = "session-2".to_string();
        let file_path = "/tmp/example.docx".to_string();
        let record = TexSessionRecord {
            snapshot: snapshot_from_document(
                session_id.clone(),
                1,
                true,
                sample_document(&file_path, "example.docx"),
            ),
            owner_window_label: Some("main".to_string()),
            attached_windows: HashSet::from(["main".to_string()]),
            updated_at_ms: now_ms(),
            persistence_path: store.session_path(&session_id),
            pending_popout_target: None,
        };
        store
            .file_to_session
            .insert(file_path.clone(), session_id.clone());
        store.sessions.insert(session_id.clone(), record);

        store
            .release_session(session_id.clone(), "main".to_string(), false)
            .unwrap();

        assert!(!store.sessions.contains_key(&session_id));
        assert!(store.session_path(&session_id).exists());

        fs::remove_dir_all(temp_root).unwrap();
    }

    #[test]
    fn discard_recoverable_removes_persisted_file() {
        let temp_root =
            std::env::temp_dir().join(format!("tex-session-test-{}", generate_session_id("three")));
        let store = TexSessionStore::new(temp_root.clone()).unwrap();
        let record = sample_record(&temp_root.join(SESSIONS_DIR_NAME), "session-3", true);
        store.persist_record(&record).unwrap();

        let mut store = TexSessionStore::new(temp_root.clone()).unwrap();
        store
            .discard_recoverable_session("session-3".to_string())
            .unwrap();

        assert!(!store.session_path("session-3").exists());

        fs::remove_dir_all(temp_root).unwrap();
    }

    #[test]
    fn create_session_at_path_creates_blank_docx() {
        let temp_root =
            std::env::temp_dir().join(format!("tex-session-test-{}", generate_session_id("four")));
        fs::create_dir_all(&temp_root).unwrap();
        let mut store = TexSessionStore::new(temp_root.clone()).unwrap();
        let file_path = temp_root.join("new-brief");

        let result = store
            .create_session_at_path(file_path.to_string_lossy().into_owned(), "main".to_string())
            .unwrap();

        let TexSessionOpenResult::Opened { snapshot } = result else {
            panic!("expected opened result");
        };

        assert_eq!(snapshot.file_name, "new-brief.docx");
        assert_eq!(
            Path::new(&snapshot.file_path)
                .file_name()
                .and_then(|value| value.to_str()),
            Some("new-brief.docx")
        );
        assert!(Path::new(&snapshot.file_path).exists());

        fs::remove_dir_all(temp_root).unwrap();
    }

    #[test]
    fn create_session_at_path_rejects_existing_file() {
        let temp_root =
            std::env::temp_dir().join(format!("tex-session-test-{}", generate_session_id("five")));
        fs::create_dir_all(&temp_root).unwrap();
        let mut store = TexSessionStore::new(temp_root.clone()).unwrap();
        let file_path = temp_root.join("existing.docx");

        save_tex_document(&file_path, &[]).unwrap();
        let error = store
            .create_session_at_path(file_path.to_string_lossy().into_owned(), "main".to_string())
            .unwrap_err();

        assert!(error.contains("already exists"));

        fs::remove_dir_all(temp_root).unwrap();
    }

    fn sample_heading(level: i64, text: &str) -> TexBlock {
        TexBlock {
            id: format!("h-{level}-{text}"),
            kind: "heading".to_string(),
            text: text.to_string(),
            runs: vec![],
            level: Some(level),
            style_id: Some(format!("Heading{level}")),
            style_name: Some(format!("Heading {level}")),
            is_f8_cite: false,
        }
    }

    fn sample_paragraph(text: &str) -> TexBlock {
        TexBlock {
            id: format!("p-{text}"),
            kind: "paragraph".to_string(),
            text: text.to_string(),
            runs: vec![],
            level: None,
            style_id: None,
            style_name: None,
            is_f8_cite: false,
        }
    }

    #[test]
    fn send_to_session_below_rebases_to_target_level() {
        let mut store = TexSessionStore::new(std::env::temp_dir()).unwrap();
        let target_session_id = "target".to_string();
        store.sessions.insert(
            target_session_id.clone(),
            TexSessionRecord {
                snapshot: TexSessionSnapshot {
                    session_id: target_session_id.clone(),
                    version: 1,
                    dirty: false,
                    file_path: "/tmp/target.docx".to_string(),
                    file_name: "target.docx".to_string(),
                    paragraph_count: 2,
                    blocks: vec![
                        sample_heading(3, "Existing block"),
                        sample_paragraph("body"),
                    ],
                },
                owner_window_label: Some("main".to_string()),
                attached_windows: HashSet::from(["main".to_string()]),
                updated_at_ms: now_ms(),
                persistence_path: store.session_path(&target_session_id),
                pending_popout_target: None,
            },
        );

        let result = store
            .send_to_session(SendToSessionArgs {
                target_session_id: target_session_id.clone(),
                target_block_index: Some(0),
                insert_mode: TexSendInsertMode::Below,
                source_blocks: vec![
                    sample_heading(2, "Source hat"),
                    sample_heading(3, "Source block"),
                ],
                source_root_level: 2,
                source_max_relative_depth: 1,
            })
            .unwrap();

        assert_eq!(result.snapshot.blocks[2].level, Some(3));
        assert_eq!(result.snapshot.blocks[3].level, Some(4));
        assert!(result.snapshot.dirty);
    }

    #[test]
    fn send_to_session_under_tag_rejects() {
        let mut store = TexSessionStore::new(std::env::temp_dir()).unwrap();
        let target_session_id = "target-tag".to_string();
        store.sessions.insert(
            target_session_id.clone(),
            TexSessionRecord {
                snapshot: TexSessionSnapshot {
                    session_id: target_session_id.clone(),
                    version: 1,
                    dirty: false,
                    file_path: "/tmp/target-tag.docx".to_string(),
                    file_name: "target-tag.docx".to_string(),
                    paragraph_count: 1,
                    blocks: vec![sample_heading(4, "Target tag")],
                },
                owner_window_label: Some("main".to_string()),
                attached_windows: HashSet::from(["main".to_string()]),
                updated_at_ms: now_ms(),
                persistence_path: store.session_path(&target_session_id),
                pending_popout_target: None,
            },
        );

        let error = store
            .send_to_session(SendToSessionArgs {
                target_session_id,
                target_block_index: Some(0),
                insert_mode: TexSendInsertMode::Under,
                source_blocks: vec![sample_heading(4, "Source tag")],
                source_root_level: 4,
                source_max_relative_depth: 0,
            })
            .unwrap_err();

        assert!(error.contains("Tag depth"));
    }
}
