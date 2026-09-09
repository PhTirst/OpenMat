use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use openmat_runtime::{
    SearchPath, SearchPathPosition, SearchPathSnapshot, WorkingDirectory, WorkingDirectorySnapshot,
};

pub(crate) const PROTOCOL_V1: &str = "openmat-workspace-v1";
pub(crate) const PROTOCOL_V2: &str = "openmat-workspace-v2";
pub(crate) const PROTOCOL_V3: &str = "openmat-workspace-v3";
const MAX_TEXT_FILE_BYTES: u64 = 128 * 1024;
const TICKET_BYTES: usize = 32;
const DOWNLOAD_TICKET_TTL: Duration = Duration::from_secs(60);
const MAX_DOWNLOAD_TICKETS: usize = 256;
const MAX_UPLOAD_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const UPLOAD_TICKET_TTL: Duration = Duration::from_secs(60);
const MAX_UPLOAD_TICKETS: usize = 256;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceProtocol {
    V1,
    V2,
    V3,
}

impl WorkspaceProtocol {
    const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => PROTOCOL_V1,
            Self::V2 => PROTOCOL_V2,
            Self::V3 => PROTOCOL_V3,
        }
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceService {
    working_directory: WorkingDirectory,
    search_path: SearchPath,
    roots: Arc<RwLock<BTreeMap<u64, PathBuf>>>,
    downloads: Arc<Mutex<HashMap<String, DownloadGrant>>>,
    uploads: Arc<Mutex<HashMap<String, UploadGrant>>>,
}

#[derive(Debug)]
struct DownloadGrant {
    root_generation: u64,
    path: String,
    expires_at: Instant,
}

#[derive(Debug)]
struct UploadGrant {
    root_generation: u64,
    path: String,
    size: u64,
    overwrite: bool,
    expires_at: Instant,
}

#[derive(Debug)]
pub(crate) struct WorkspaceDownloadFile {
    pub(crate) file: File,
    pub(crate) name: String,
    pub(crate) size: u64,
}

#[derive(Debug)]
pub(crate) struct WorkspaceUploadFile {
    file: Option<File>,
    temporary: PathBuf,
    target: PathBuf,
    path: String,
    size: u64,
    overwrite: bool,
    permissions: Option<fs::Permissions>,
    committed: bool,
}

#[derive(Debug)]
pub(crate) struct WorkspaceRootError(String);

impl fmt::Display for WorkspaceRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: WorkspaceEntryKind,
    pub(crate) size: Option<u64>,
    pub(crate) revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryBrowserEntry {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceEntryKind {
    File,
    Directory,
    Other,
}

pub(crate) struct LspSourceSnapshot {
    pub(crate) documents: BTreeMap<String, String>,
    pub(crate) incomplete_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceErrorBody {
    code: &'static str,
    message: String,
    field: Option<&'static str>,
    details: Option<serde_json::Value>,
}

impl WorkspaceErrorBody {
    fn new(code: &'static str, message: impl Into<String>, field: Option<&'static str>) -> Self {
        Self {
            code,
            message: message.into(),
            field,
            details: None,
        }
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub(crate) fn http_status(&self) -> &'static str {
        match self.code {
            "workspace.conflict" => "409 Conflict",
            "workspace.fileTooLarge" => "413 Content Too Large",
            "workspace.notFound" | "workspace.notDirectory" => "404 Not Found",
            "workspace.ioFailure" | "workspace.stateUnavailable" => "500 Internal Server Error",
            _ => "400 Bad Request",
        }
    }

    pub(crate) fn value(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "message": self.message,
            "field": self.field,
            "details": self.details,
        })
    }
}

impl WorkspaceUploadFile {
    pub(crate) const fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn writer(&mut self) -> &mut File {
        self.file.as_mut().expect("upload file must remain open")
    }

    pub(crate) fn commit(mut self) -> Result<WorkspaceEntry, WorkspaceErrorBody> {
        let file = self.file.take().expect("upload file must remain open");
        file.sync_all()
            .map_err(|error| map_write_error(&error, &self.path))?;
        drop(file);
        if let Some(permissions) = self.permissions.take() {
            fs::set_permissions(&self.temporary, permissions)
                .map_err(|error| map_write_error(&error, &self.path))?;
        }
        match fs::symlink_metadata(&self.target) {
            Ok(metadata) => {
                if !self.overwrite {
                    return Err(path_conflict(&self.path, "already exists"));
                }
                if platform::is_reparse_or_symlink(&metadata) || !metadata.is_file() {
                    return Err(path_conflict(
                        &self.path,
                        "is not a replaceable regular file",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(&error, &self.path)),
        }
        if self.overwrite {
            platform::replace_file(&self.temporary, &self.target)
                .map_err(|error| map_write_error(&error, &self.path))?;
        } else {
            fs::hard_link(&self.temporary, &self.target)
                .map_err(|error| map_create_error(&error, &self.path))?;
            fs::remove_file(&self.temporary)
                .map_err(|error| map_write_error(&error, &self.path))?;
        }
        self.committed = true;
        Ok(WorkspaceEntry {
            name: file_name_from_path(&self.path)?.to_owned(),
            path: self.path.clone(),
            kind: WorkspaceEntryKind::File,
            size: Some(self.size),
            revision: None,
        })
    }
}

impl Drop for WorkspaceUploadFile {
    fn drop(&mut self) {
        if !self.committed {
            self.file.take();
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CreatableKind {
    File,
    Directory,
}

impl WorkspaceService {
    pub(crate) fn new(root: &Path) -> Result<Self, WorkspaceRootError> {
        let configured_root = absolute_path(root).map_err(|error| {
            WorkspaceRootError(format!(
                "invalid workspace root '{}': {error}",
                root.display()
            ))
        })?;
        let metadata = fs::symlink_metadata(&configured_root).map_err(|error| {
            WorkspaceRootError(format!(
                "could not inspect workspace root '{}': {error}",
                configured_root.display()
            ))
        })?;
        if !metadata.is_dir() {
            return Err(WorkspaceRootError(format!(
                "workspace root is not a directory: {}",
                configured_root.display()
            )));
        }
        if platform::is_reparse_or_symlink(&metadata) {
            return Err(WorkspaceRootError(format!(
                "workspace root must not itself be a symbolic link or reparse point: {}",
                configured_root.display()
            )));
        }
        let canonical_root = fs::canonicalize(&configured_root).map_err(|error| {
            WorkspaceRootError(format!(
                "could not canonicalize workspace root '{}': {error}",
                configured_root.display()
            ))
        })?;
        let working_directory = WorkingDirectory::new(&canonical_root).map_err(|error| {
            WorkspaceRootError(format!(
                "could not initialize workspace root '{}': {error}",
                configured_root.display()
            ))
        })?;
        let snapshot = working_directory
            .snapshot()
            .map_err(|error| WorkspaceRootError(error.to_string()))?;
        let roots = BTreeMap::from([(snapshot.generation, snapshot.path)]);
        Ok(Self {
            working_directory,
            search_path: SearchPath::empty(),
            roots: Arc::new(RwLock::new(roots)),
            downloads: Arc::new(Mutex::new(HashMap::new())),
            uploads: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) fn working_directory(&self) -> WorkingDirectory {
        self.working_directory.clone()
    }

    pub(crate) fn search_path(&self) -> SearchPath {
        self.search_path.clone()
    }

    pub(crate) fn search_path_event(
        &self,
        protocol: WorkspaceProtocol,
    ) -> Result<(u64, String), WorkspaceRootError> {
        let snapshot = self
            .search_path
            .snapshot()
            .map_err(|error| WorkspaceRootError(error.to_string()))?;
        let generation = snapshot.generation;
        let data = self
            .search_path_value(&snapshot)
            .map_err(|error| WorkspaceRootError(error.message))?;
        Ok((
            generation,
            serde_json::json!({
                "protocol": protocol.as_str(),
                "event": { "type": "searchPathChanged", "data": data },
            })
            .to_string(),
        ))
    }

    pub(crate) fn current_directory_event(
        &self,
        protocol: WorkspaceProtocol,
    ) -> Result<(u64, String), WorkspaceRootError> {
        let snapshot = self
            .working_directory
            .snapshot()
            .map_err(|error| WorkspaceRootError(error.to_string()))?;
        let generation = snapshot.generation;
        Ok((
            generation,
            current_directory_event(&snapshot, protocol).to_string(),
        ))
    }

    pub(crate) fn current_directory_watch_target(
        &self,
    ) -> Result<(u64, PathBuf), WorkspaceRootError> {
        let snapshot = self
            .working_directory
            .snapshot()
            .map_err(|error| WorkspaceRootError(error.to_string()))?;
        Ok((snapshot.generation, snapshot.path))
    }

    /// Reads a bounded language-service snapshot through the same path and text
    /// checks as workspace reads. A private service freezes the root while the
    /// live kernel may change its working directory concurrently.
    #[allow(clippy::too_many_lines)] // Keep the bounded traversal and its completeness accounting together.
    pub(crate) fn lsp_source_snapshot(
        &self,
        generation: u64,
    ) -> Result<LspSourceSnapshot, WorkspaceRootError> {
        const MAX_ENTRIES: usize = 20_000;
        const MAX_DOCUMENTS: usize = 2_048;
        const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
        const MAX_DEPTH: usize = 32;
        let (current_generation, root) = self.current_directory_watch_target()?;
        if current_generation != generation {
            return Err(WorkspaceRootError(
                "workspace changed during indexing".into(),
            ));
        }
        let source = Self::new(&root)?;
        let mut directories = std::collections::VecDeque::from([(String::new(), 0)]);
        let mut snapshot = LspSourceSnapshot {
            documents: BTreeMap::new(),
            incomplete_reason: None,
        };
        let mut visited = 0;
        let mut total_bytes = 0;
        while let Some((relative, depth)) = directories.pop_front() {
            let Ok((directory, metadata)) = source.resolve_existing(&relative, true) else {
                snapshot
                    .incomplete_reason
                    .get_or_insert_with(|| format!("Could not inspect directory '{relative}'."));
                continue;
            };
            if !metadata.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                snapshot
                    .incomplete_reason
                    .get_or_insert_with(|| format!("Could not read directory '{relative}'."));
                continue;
            };
            // Bound enumeration too, including directories with no MATLAB files.
            let mut entries = entries
                .take(MAX_ENTRIES - visited)
                .filter_map(|entry| {
                    visited += 1;
                    if entry.is_err() {
                        snapshot.incomplete_reason.get_or_insert_with(|| {
                            format!("Could not enumerate directory '{relative}'.")
                        });
                    }
                    entry.ok()
                })
                .collect::<Vec<_>>();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let Ok(name) = entry.file_name().into_string() else {
                    snapshot
                        .incomplete_reason
                        .get_or_insert_with(|| "A workspace filename is not UTF-8.".into());
                    continue;
                };
                let path = join_relative(&relative, &name);
                let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                    snapshot
                        .incomplete_reason
                        .get_or_insert_with(|| format!("Could not inspect '{path}'."));
                    continue;
                };
                if platform::is_reparse_or_symlink(&metadata) {
                    continue;
                }
                if metadata.is_dir() && name != ".git" {
                    if depth < MAX_DEPTH {
                        directories.push_back((path, depth + 1));
                    } else {
                        snapshot.incomplete_reason.get_or_insert_with(|| {
                            "Workspace indexing exceeded the directory depth limit (32).".into()
                        });
                    }
                } else if metadata.is_file() && platform::is_matlab_source_name(&name) {
                    let file = match source.read_file(&path) {
                        Ok(file) => file,
                        Err(error) => {
                            snapshot.incomplete_reason.get_or_insert(error.message);
                            continue;
                        }
                    };
                    if total_bytes + file.content.len() > MAX_TOTAL_BYTES {
                        snapshot.incomplete_reason.get_or_insert_with(|| {
                            "Workspace indexing exceeded the total text limit (16 MiB).".into()
                        });
                        return Ok(snapshot);
                    }
                    total_bytes += file.content.len();
                    snapshot.documents.insert(path, file.content);
                }
                if snapshot.documents.len() == MAX_DOCUMENTS {
                    snapshot.incomplete_reason.get_or_insert_with(|| {
                        "Workspace indexing reached the document limit (2,048).".into()
                    });
                    return Ok(snapshot);
                }
            }
            if visited == MAX_ENTRIES {
                snapshot.incomplete_reason.get_or_insert_with(|| {
                    "Workspace indexing reached the entry limit (20,000).".into()
                });
                return Ok(snapshot);
            }
        }
        Ok(snapshot)
    }

    pub(crate) fn handle_text_for_protocol(
        &self,
        text: &str,
        expected_protocol: WorkspaceProtocol,
    ) -> String {
        let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
        let request_id = parsed
            .as_ref()
            .and_then(|value| value.get("requestId"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "invalid-request".to_owned());
        let Some(request) = parsed.as_ref().and_then(serde_json::Value::as_object) else {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace request must be a JSON object.",
            );
        };
        if request.len() != 3 {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace envelope must contain only protocol, requestId, and request.",
            );
        }
        let Some(protocol) = request.get("protocol").and_then(serde_json::Value::as_str) else {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace protocol must be a string.",
            );
        };
        if protocol != expected_protocol.as_str() {
            return failure_frame(
                expected_protocol,
                &request_id,
                &WorkspaceErrorBody::new(
                    "workspace.unsupportedProtocol",
                    format!(
                        "Endpoint requires workspace protocol '{}', received '{protocol}'.",
                        expected_protocol.as_str()
                    ),
                    Some("protocol"),
                ),
            );
        }
        if request.get("requestId").and_then(serde_json::Value::as_str) != Some(request_id.as_str())
        {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace requestId must be a non-empty string.",
            );
        }
        let Some(payload) = request
            .get("request")
            .and_then(serde_json::Value::as_object)
        else {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace request must be an object.",
            );
        };
        if payload.len() != 2 {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace request must contain only type and params.",
            );
        }
        let Some(request_type) = payload.get("type").and_then(serde_json::Value::as_str) else {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace request type must be a string.",
            );
        };
        let Some(params) = payload.get("params").and_then(serde_json::Value::as_object) else {
            return invalid_request_frame(
                expected_protocol,
                &request_id,
                "Workspace request params must be an object.",
            );
        };

        match expected_protocol {
            WorkspaceProtocol::V1 => self.handle_v1_payload(&request_id, request_type, params),
            WorkspaceProtocol::V2 | WorkspaceProtocol::V3 => {
                self.handle_modern_payload(expected_protocol, &request_id, request_type, params)
            }
        }
    }

    fn handle_v1_payload(
        &self,
        request_id: &str,
        request_type: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> String {
        match request_type {
            "list" if params.is_empty() => match self.list_entries("", false) {
                Ok(entries) => success_frame(
                    WorkspaceProtocol::V1,
                    request_id,
                    &serde_json::json!({
                        "type": "list",
                        "data": {
                            "rootName": self.root_name().unwrap_or_else(|_| "workspace".to_owned()),
                            "entries": entries.iter().map(entry_value_v1).collect::<Vec<_>>(),
                        }
                    }),
                ),
                Err(error) => failure_frame(WorkspaceProtocol::V1, request_id, &error),
            },
            "create" if params.len() == 2 => {
                let Some(name) = params.get("name").and_then(serde_json::Value::as_str) else {
                    return invalid_request_frame(
                        WorkspaceProtocol::V1,
                        request_id,
                        "Create name must be a string.",
                    );
                };
                let kind = match parse_creatable_kind(params.get("kind")) {
                    Ok(kind) => kind,
                    Err(message) => {
                        return invalid_request_frame(WorkspaceProtocol::V1, request_id, message);
                    }
                };
                if let Err(error) = validate_name(name, "name") {
                    return failure_frame(WorkspaceProtocol::V1, request_id, &error);
                }
                match self.create_entry(name, kind) {
                    Ok(entry) => success_frame(
                        WorkspaceProtocol::V1,
                        request_id,
                        &serde_json::json!({
                            "type": "create",
                            "data": { "entry": entry_value_v1(&entry) }
                        }),
                    ),
                    Err(error) => failure_frame(WorkspaceProtocol::V1, request_id, &error),
                }
            }
            "list" | "create" => invalid_request_frame(
                WorkspaceProtocol::V1,
                request_id,
                "Workspace request params contain missing or unexpected fields.",
            ),
            _ => unsupported_request_frame(WorkspaceProtocol::V1, request_id, request_type),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_modern_payload(
        &self,
        protocol: WorkspaceProtocol,
        request_id: &str,
        request_type: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> String {
        let result = match request_type {
            "currentDirectory" if params.is_empty() => self
                .current_directory_value()
                .map(|data| serde_json::json!({ "type": "currentDirectory", "data": data })),
            "changeDirectory" if params.len() == 1 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.change_directory(path)
                    .map(|data| serde_json::json!({ "type": "changeDirectory", "data": data }))
            }
            "browseDirectories" if params.len() == 1 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.browse_directories(path)
                    .map(|data| serde_json::json!({ "type": "browseDirectories", "data": data }))
            }
            "searchPath" if params.is_empty() => self
                .search_path_snapshot()
                .map(|snapshot| serde_json::json!({ "type": "searchPath", "data": snapshot })),
            "addSearchPath" if params.len() == 3 => {
                let (Some(path), Some(recursive), Some(position)) = (
                    string_param(params, "path"),
                    bool_param(params, "recursive"),
                    string_param(params, "position"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let position = match position {
                    "begin" => SearchPathPosition::Begin,
                    "end" => SearchPathPosition::End,
                    _ => return invalid_workspace_params(protocol, request_id),
                };
                self.add_search_path(path, recursive, position).map(
                    |snapshot| serde_json::json!({ "type": "addSearchPath", "data": snapshot }),
                )
            }
            "removeSearchPath" if params.len() == 2 => {
                let (Some(path), Some(recursive)) = (
                    string_param(params, "path"),
                    bool_param(params, "recursive"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.remove_search_path(path, recursive).map(
                    |snapshot| serde_json::json!({ "type": "removeSearchPath", "data": snapshot }),
                )
            }
            "list" if params.len() == 2 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let Some(recursive) = bool_param(params, "recursive") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.list_entries(path, recursive).map(|entries| {
                    serde_json::json!({
                        "type": "list",
                        "data": {
                            "rootName": self.root_name().unwrap_or_else(|_| "workspace".to_owned()),
                            "rootPath": self.root_path().unwrap_or_default(),
                            "rootGeneration": self.root_generation().unwrap_or(0),
                            "path": path,
                            "recursive": recursive,
                            "entries": entries.iter().map(entry_value_v2).collect::<Vec<_>>(),
                        }
                    })
                })
            }
            "read" if params.len() == 1 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let root = self.root_snapshot();
                let generation = root.as_ref().map_or(0, |root| root.generation);
                let root_path = root
                    .as_ref()
                    .ok()
                    .and_then(|root| path_text(&root.path).ok())
                    .unwrap_or_default();
                self.read_file(path).map(|read| {
                    serde_json::json!({
                        "type": "read",
                        "data": {
                            "path": path,
                            "content": read.content,
                            "revision": read.revision,
                            "size": read.size,
                            "rootGeneration": generation,
                            "rootPath": root_path,
                        }
                    })
                })
            }
            "prepareDownload" if protocol == WorkspaceProtocol::V3 && params.len() == 2 => {
                let (Some(path), Some(root_generation)) = (
                    string_param(params, "path"),
                    params
                        .get("rootGeneration")
                        .and_then(serde_json::Value::as_u64),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.prepare_download(path, root_generation)
                    .map(|download| {
                        serde_json::json!({
                            "type": "prepareDownload",
                            "data": download,
                        })
                    })
            }
            "prepareUpload" if protocol == WorkspaceProtocol::V3 && params.len() == 4 => {
                let (Some(path), Some(size), Some(root_generation), Some(overwrite)) = (
                    string_param(params, "path"),
                    params.get("size").and_then(serde_json::Value::as_u64),
                    params
                        .get("rootGeneration")
                        .and_then(serde_json::Value::as_u64),
                    bool_param(params, "overwrite"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.prepare_upload(path, size, root_generation, overwrite)
                    .map(|upload| {
                        serde_json::json!({
                            "type": "prepareUpload",
                            "data": upload,
                        })
                    })
            }
            "write" if params.len() == 3 || params.len() == 4 => {
                let (Some(path), Some(content), Some(expected_revision)) = (
                    string_param(params, "path"),
                    string_param(params, "content"),
                    string_param(params, "expectedRevision"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                if expected_revision.is_empty() {
                    return invalid_request_frame(
                        protocol,
                        request_id,
                        "Write expectedRevision must be a non-empty string.",
                    );
                }
                let root_generation = match params.get("rootGeneration") {
                    None => None,
                    Some(value) => value.as_u64(),
                };
                if params.len() == 4 && root_generation.is_none() {
                    return invalid_workspace_params(protocol, request_id);
                }
                self.service_for_generation(root_generation)
                    .and_then(|service| service.write_file(path, content, expected_revision))
                    .map(|entry| {
                        serde_json::json!({
                            "type": "write",
                            "data": { "entry": entry_value_v2(&entry) }
                        })
                    })
            }
            "create" if params.len() == 2 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let kind = match parse_creatable_kind(params.get("kind")) {
                    Ok(kind) => kind,
                    Err(message) => {
                        return invalid_request_frame(protocol, request_id, message);
                    }
                };
                self.create_entry(path, kind).map(|entry| {
                    serde_json::json!({
                        "type": "create",
                        "data": { "entry": entry_value_v2(&entry) }
                    })
                })
            }
            "rename" if params.len() == 2 || params.len() == 3 => {
                let (Some(path), Some(new_name)) = (
                    string_param(params, "path"),
                    string_param(params, "newName"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let root_generation = params
                    .get("rootGeneration")
                    .and_then(serde_json::Value::as_u64);
                if params.len() == 3 && root_generation.is_none() {
                    return invalid_workspace_params(protocol, request_id);
                }
                self.service_for_generation(root_generation)
                    .and_then(|service| service.rename_entry(path, new_name))
                    .map(|entry| {
                        serde_json::json!({
                            "type": "rename",
                            "data": { "previousPath": path, "entry": entry_value_v2(&entry) }
                        })
                    })
            }
            "move" if params.len() == 2 => {
                let (Some(path), Some(target_path)) = (
                    string_param(params, "path"),
                    string_param(params, "targetPath"),
                ) else {
                    return invalid_workspace_params(protocol, request_id);
                };
                self.move_entry(path, target_path).map(|entry| {
                    serde_json::json!({
                        "type": "move",
                        "data": { "previousPath": path, "entry": entry_value_v2(&entry) }
                    })
                })
            }
            "delete" if params.len() == 3 => {
                let Some(path) = string_param(params, "path") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let Some(recursive) = bool_param(params, "recursive") else {
                    return invalid_workspace_params(protocol, request_id);
                };
                let confirm_path = match params.get("confirmPath") {
                    Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(value)) => Some(value.as_str()),
                    _ => return invalid_workspace_params(protocol, request_id),
                };
                self.delete_entry(path, recursive, confirm_path)
                    .map(|kind| {
                        serde_json::json!({
                            "type": "delete",
                            "data": {
                                "path": path,
                                "kind": kind_name(kind),
                                "recursive": recursive,
                            }
                        })
                    })
            }
            "prepareDownload" | "prepareUpload" if protocol == WorkspaceProtocol::V3 => {
                return invalid_workspace_params(protocol, request_id);
            }
            "currentDirectory" | "changeDirectory" | "browseDirectories" | "searchPath"
            | "addSearchPath" | "removeSearchPath" | "list" | "read" | "write" | "create"
            | "rename" | "move" | "delete" => {
                return invalid_workspace_params(protocol, request_id);
            }
            _ => return unsupported_request_frame(protocol, request_id, request_type),
        };

        match result {
            Ok(value) => success_frame(protocol, request_id, &value),
            Err(error) => failure_frame(protocol, request_id, &error),
        }
    }

    fn root_snapshot(&self) -> Result<WorkingDirectorySnapshot, WorkspaceErrorBody> {
        let snapshot = self
            .working_directory
            .snapshot()
            .map_err(map_working_directory_error)?;
        self.remember_root(&snapshot)?;
        Ok(snapshot)
    }

    fn root_name(&self) -> Result<String, WorkspaceErrorBody> {
        let root = self.root_snapshot()?.path;
        Ok(root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("workspace")
            .to_owned())
    }

    fn root_path(&self) -> Result<String, WorkspaceErrorBody> {
        path_text(&self.root_snapshot()?.path)
    }

    fn root_generation(&self) -> Result<u64, WorkspaceErrorBody> {
        Ok(self.root_snapshot()?.generation)
    }

    fn current_directory_value(&self) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let snapshot = self.root_snapshot()?;
        current_directory_value(&snapshot)
    }

    fn search_path_snapshot(&self) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let snapshot = self
            .search_path
            .snapshot()
            .map_err(map_working_directory_error)?;
        self.search_path_value(&snapshot)
    }

    fn search_path_value(
        &self,
        snapshot: &SearchPathSnapshot,
    ) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let root = self.root_snapshot()?.path;
        let directories = snapshot
            .paths
            .iter()
            .map(|path| {
                Ok(serde_json::json!({
                    "path": path_text(path)?,
                    "workspacePath": workspace_relative_path(&root, path)?,
                }))
            })
            .collect::<Result<Vec<_>, WorkspaceErrorBody>>()?;
        Ok(serde_json::json!({
            "generation": snapshot.generation,
            "directories": directories,
        }))
    }

    fn add_search_path(
        &self,
        path: &str,
        recursive: bool,
        position: SearchPathPosition,
    ) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let (directory, metadata) = self.resolve_existing(path, true)?;
        if !metadata.is_dir() {
            return Err(WorkspaceErrorBody::new(
                "workspace.notDirectory",
                format!("'{path}' is not a directory."),
                Some("path"),
            ));
        }
        let paths = if recursive {
            SearchPath::generate_from(&directory, None).map_err(map_working_directory_error)?
        } else {
            vec![directory]
        };
        let snapshot = self
            .search_path
            .add_from(paths, None, position)
            .map_err(map_working_directory_error)?;
        self.search_path_value(&snapshot)
    }

    fn remove_search_path(
        &self,
        path: &str,
        recursive: bool,
    ) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let (directory, metadata) = self.resolve_existing(path, true)?;
        if !metadata.is_dir() {
            return Err(WorkspaceErrorBody::new(
                "workspace.notDirectory",
                format!("'{path}' is not a directory."),
                Some("path"),
            ));
        }
        let current = self
            .search_path
            .snapshot()
            .map_err(map_working_directory_error)?;
        let removals = if recursive {
            current
                .paths
                .iter()
                .filter(|candidate| *candidate == &directory || candidate.starts_with(&directory))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            vec![directory]
        };
        let snapshot = self
            .search_path
            .remove_from(removals, None)
            .map_err(map_working_directory_error)?;
        self.search_path_value(&snapshot)
    }

    fn change_directory(&self, path: &str) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let candidate = self.resolve_directory_path(path)?;
        let snapshot = self
            .working_directory
            .change(candidate)
            .map_err(map_working_directory_error)?;
        self.remember_root(&snapshot)?;
        current_directory_value(&snapshot)
    }

    fn resolve_directory_path(&self, path: &str) -> Result<PathBuf, WorkspaceErrorBody> {
        if path.trim().is_empty() || path.contains('\0') {
            return Err(WorkspaceErrorBody::new(
                "workspace.invalidDirectory",
                "Current Folder requires a non-empty path.",
                Some("path"),
            ));
        }
        let supplied = Path::new(path);
        let current = self.root_snapshot()?.path;
        let candidate = if supplied.is_absolute() {
            supplied.to_path_buf()
        } else {
            current.join(supplied)
        };
        let metadata =
            fs::symlink_metadata(&candidate).map_err(|error| map_root_io_error(&error))?;
        if platform::is_reparse_or_symlink(&metadata) {
            return Err(WorkspaceErrorBody::new(
                "workspace.reparsePoint",
                "Current Folder cannot itself be a symbolic link or reparse point.",
                Some("path"),
            ));
        }
        if !metadata.is_dir() {
            return Err(WorkspaceErrorBody::new(
                "workspace.notDirectory",
                format!("'{}' is not a directory.", candidate.display()),
                Some("path"),
            ));
        }
        fs::canonicalize(&candidate)
            .map(platform::displayable_canonical_path)
            .map_err(|error| map_root_io_error(&error))
    }

    fn remember_root(&self, snapshot: &WorkingDirectorySnapshot) -> Result<(), WorkspaceErrorBody> {
        self.roots
            .write()
            .map_err(|_| workspace_state_error())?
            .insert(snapshot.generation, snapshot.path.clone());
        Ok(())
    }

    fn service_for_generation(&self, generation: Option<u64>) -> Result<Self, WorkspaceErrorBody> {
        let Some(generation) = generation else {
            return Ok(self.clone());
        };
        let current = self.root_snapshot()?;
        // Explicit generation operations must hold a fixed root even when it is
        // currently selected: the kernel may change Current Folder after this
        // check, including between resolving a move's source and destination.
        let path = if current.generation == generation {
            current.path
        } else {
            self.roots
                .read()
                .map_err(|_| workspace_state_error())?
                .get(&generation)
                .cloned()
                .ok_or_else(|| {
                    WorkspaceErrorBody::new(
                        "workspace.unknownRootGeneration",
                        "The document's original Current Folder is no longer available.",
                        Some("rootGeneration"),
                    )
                })?
        };
        Self::new(&path).map_err(|error| {
            WorkspaceErrorBody::new(
                "workspace.rootUnavailable",
                error.to_string(),
                Some("rootGeneration"),
            )
        })
    }

    fn browse_directories(&self, path: &str) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let directory = self.resolve_directory_path(path)?;
        let directory_text = path_text(&directory)?;
        let parent_path = directory.parent().map(path_text).transpose()?;
        let mut entries = Vec::new();
        for item in fs::read_dir(&directory).map_err(|error| map_root_io_error(&error))? {
            let item = item.map_err(|error| map_root_io_error(&error))?;
            let file_type = item
                .file_type()
                .map_err(|error| map_root_io_error(&error))?;
            if !file_type.is_dir() {
                continue;
            }
            let name = item.file_name().into_string().map_err(|_| {
                WorkspaceErrorBody::new(
                    "workspace.unsupportedName",
                    "The directory contains a name that is not valid UTF-8.",
                    None,
                )
            })?;
            entries.push(DirectoryBrowserEntry {
                name,
                path: path_text(&item.path())?,
            });
        }
        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.name.cmp(&right.name))
        });

        let mut root_paths = platform::directory_roots();
        if let Some(root) = directory.ancestors().last()
            && !root_paths.iter().any(|candidate| candidate == root)
        {
            root_paths.push(root.to_path_buf());
        }
        let mut roots = root_paths
            .into_iter()
            .map(|root| {
                Ok(DirectoryBrowserEntry {
                    name: directory_root_name(&root),
                    path: path_text(&root)?,
                })
            })
            .collect::<Result<Vec<_>, WorkspaceErrorBody>>()?;
        roots.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(serde_json::json!({
            "path": directory_text,
            "parentPath": parent_path,
            "roots": roots.iter().map(directory_browser_entry_value).collect::<Vec<_>>(),
            "entries": entries.iter().map(directory_browser_entry_value).collect::<Vec<_>>(),
        }))
    }

    fn list_entries(
        &self,
        path: &str,
        recursive: bool,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceErrorBody> {
        let (directory, metadata) = self.resolve_existing(path, true)?;
        if !metadata.is_dir() {
            return Err(WorkspaceErrorBody::new(
                "workspace.notDirectory",
                format!("'{path}' is not a directory."),
                Some("path"),
            ));
        }
        let mut entries = Vec::new();
        Self::collect_entries(path, &directory, recursive, &mut entries)?;
        entries.sort_by(compare_entries);
        Ok(entries)
    }

    fn collect_entries(
        relative_directory: &str,
        directory: &Path,
        recursive: bool,
        entries: &mut Vec<WorkspaceEntry>,
    ) -> Result<(), WorkspaceErrorBody> {
        for item in
            fs::read_dir(directory).map_err(|error| map_io_error(&error, relative_directory))?
        {
            let item = item.map_err(|error| map_io_error(&error, relative_directory))?;
            let name = item.file_name().into_string().map_err(|_| {
                WorkspaceErrorBody::new(
                    "workspace.unsupportedName",
                    "The workspace contains a name that is not valid UTF-8.",
                    None,
                )
            })?;
            let path = join_relative(relative_directory, &name);
            let metadata =
                fs::symlink_metadata(item.path()).map_err(|error| map_io_error(&error, &path))?;
            let entry = entry_from_metadata(&name, &path, &metadata);
            let recurse = recursive && entry.kind == WorkspaceEntryKind::Directory;
            entries.push(entry);
            if recurse {
                Self::collect_entries(&path, &item.path(), true, entries)?;
            }
        }
        Ok(())
    }

    fn read_file(&self, path: &str) -> Result<ReadFile, WorkspaceErrorBody> {
        let (target, metadata) = self.resolve_existing(path, false)?;
        require_file(path, &metadata)?;
        let bytes = read_bounded_file(&target, &metadata, path)?;
        let content = String::from_utf8(bytes).map_err(|_| {
            WorkspaceErrorBody::new(
                "workspace.invalidUtf8",
                format!("'{path}' is not valid UTF-8 text."),
                Some("path"),
            )
        })?;
        let size = u64::try_from(content.len()).unwrap_or(u64::MAX);
        let revision = content_revision(content.as_bytes());
        Ok(ReadFile {
            content,
            revision,
            size,
        })
    }

    fn prepare_download(
        &self,
        path: &str,
        root_generation: u64,
    ) -> Result<serde_json::Value, WorkspaceErrorBody> {
        let service = self.service_for_generation(Some(root_generation))?;
        let (_, metadata) = service.resolve_existing(path, false)?;
        require_file(path, &metadata)?;
        let name = file_name_from_path(path)?.to_owned();
        let mut random = [0_u8; TICKET_BYTES];
        getrandom::fill(&mut random).map_err(|_| {
            WorkspaceErrorBody::new(
                "workspace.downloadUnavailable",
                "Could not create a secure download ticket.",
                None,
            )
        })?;
        let ticket = encode_ticket(&random);
        let now = Instant::now();
        let mut downloads = self.downloads.lock().map_err(|_| workspace_state_error())?;
        downloads.retain(|_, grant| grant.expires_at > now);
        if downloads.len() >= MAX_DOWNLOAD_TICKETS {
            return Err(WorkspaceErrorBody::new(
                "workspace.tooManyDownloads",
                "Too many workspace downloads are waiting to start. Try again shortly.",
                None,
            ));
        }
        if downloads.contains_key(&ticket) {
            return Err(WorkspaceErrorBody::new(
                "workspace.downloadUnavailable",
                "Could not create a unique download ticket.",
                None,
            ));
        }
        downloads.insert(
            ticket.clone(),
            DownloadGrant {
                root_generation,
                path: path.to_owned(),
                expires_at: now + DOWNLOAD_TICKET_TTL,
            },
        );
        Ok(serde_json::json!({
            "ticket": ticket,
            "name": name,
            "size": metadata.len(),
            "expiresInSeconds": DOWNLOAD_TICKET_TTL.as_secs(),
        }))
    }

    pub(crate) fn consume_download(&self, ticket: &str) -> Option<WorkspaceDownloadFile> {
        if !valid_ticket(ticket) {
            return None;
        }
        let now = Instant::now();
        let grant = {
            let mut downloads = self.downloads.lock().ok()?;
            downloads.retain(|_, grant| grant.expires_at > now);
            downloads.remove(ticket)?
        };
        if grant.expires_at <= now {
            return None;
        }
        let service = self
            .service_for_generation(Some(grant.root_generation))
            .ok()?;
        let (target, metadata) = service.resolve_existing(&grant.path, false).ok()?;
        require_file(&grant.path, &metadata).ok()?;
        let file = File::open(target).ok()?;
        let opened_metadata = file.metadata().ok()?;
        require_file(&grant.path, &opened_metadata).ok()?;
        Some(WorkspaceDownloadFile {
            file,
            name: file_name_from_path(&grant.path).ok()?.to_owned(),
            size: opened_metadata.len(),
        })
    }

    fn prepare_upload(
        &self,
        path: &str,
        size: u64,
        root_generation: u64,
        overwrite: bool,
    ) -> Result<serde_json::Value, WorkspaceErrorBody> {
        if size > MAX_UPLOAD_FILE_BYTES {
            return Err(upload_too_large(path, size));
        }
        let service = self.service_for_generation(Some(root_generation))?;
        service.validate_upload_target(path, overwrite)?;
        let name = file_name_from_path(path)?.to_owned();
        let mut random = [0_u8; TICKET_BYTES];
        getrandom::fill(&mut random).map_err(|_| {
            WorkspaceErrorBody::new(
                "workspace.uploadUnavailable",
                "Could not create a secure upload ticket.",
                None,
            )
        })?;
        let ticket = encode_ticket(&random);
        let now = Instant::now();
        let mut uploads = self.uploads.lock().map_err(|_| workspace_state_error())?;
        uploads.retain(|_, grant| grant.expires_at > now);
        if uploads.len() >= MAX_UPLOAD_TICKETS {
            return Err(WorkspaceErrorBody::new(
                "workspace.tooManyUploads",
                "Too many workspace uploads are waiting to start. Try again shortly.",
                None,
            ));
        }
        if uploads.contains_key(&ticket) {
            return Err(WorkspaceErrorBody::new(
                "workspace.uploadUnavailable",
                "Could not create a unique upload ticket.",
                None,
            ));
        }
        uploads.insert(
            ticket.clone(),
            UploadGrant {
                root_generation,
                path: path.to_owned(),
                size,
                overwrite,
                expires_at: now + UPLOAD_TICKET_TTL,
            },
        );
        Ok(serde_json::json!({
            "ticket": ticket,
            "name": name,
            "size": size,
            "expiresInSeconds": UPLOAD_TICKET_TTL.as_secs(),
        }))
    }

    pub(crate) fn consume_upload(
        &self,
        ticket: &str,
        content_length: u64,
    ) -> Result<Option<WorkspaceUploadFile>, WorkspaceErrorBody> {
        if !valid_ticket(ticket) {
            return Ok(None);
        }
        let now = Instant::now();
        let grant = {
            let mut uploads = self.uploads.lock().map_err(|_| workspace_state_error())?;
            uploads.retain(|_, grant| grant.expires_at > now);
            uploads.remove(ticket)
        };
        let Some(grant) = grant else {
            return Ok(None);
        };
        if grant.expires_at <= now {
            return Ok(None);
        }
        if content_length != grant.size {
            return Err(WorkspaceErrorBody::new(
                "workspace.uploadSizeMismatch",
                format!(
                    "Upload for '{}' declared {content_length} bytes; the ticket expects {} bytes.",
                    grant.path, grant.size
                ),
                Some("size"),
            ));
        }
        let service = self.service_for_generation(Some(grant.root_generation))?;
        let (parent, name) = service.resolve_parent_for_new(&grant.path)?;
        let target = parent.join(name);
        let permissions = service.validate_upload_target(&grant.path, grant.overwrite)?;
        let (temporary, file) = create_temporary_file(&parent)?;
        Ok(Some(WorkspaceUploadFile {
            file: Some(file),
            temporary,
            target,
            path: grant.path,
            size: grant.size,
            overwrite: grant.overwrite,
            permissions,
            committed: false,
        }))
    }

    fn validate_upload_target(
        &self,
        path: &str,
        overwrite: bool,
    ) -> Result<Option<fs::Permissions>, WorkspaceErrorBody> {
        let (parent, name) = self.resolve_parent_for_new(path)?;
        let target = parent.join(name);
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if platform::is_reparse_or_symlink(&metadata) || !metadata.is_file() {
                    return Err(path_conflict(path, "is not a replaceable regular file"));
                }
                if !overwrite {
                    return Err(path_conflict(path, "already exists"));
                }
                Ok(Some(metadata.permissions()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(map_io_error(&error, path)),
        }
    }

    fn write_file(
        &self,
        path: &str,
        content: &str,
        expected_revision: &str,
    ) -> Result<WorkspaceEntry, WorkspaceErrorBody> {
        let content_size = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if content_size > MAX_TEXT_FILE_BYTES {
            return Err(file_too_large(path, content_size));
        }
        let (target, metadata) = self.resolve_existing(path, false)?;
        require_file(path, &metadata)?;
        let current = read_bounded_file(&target, &metadata, path)?;
        let current_revision = content_revision(&current);
        if current_revision != expected_revision {
            return Err(revision_conflict(path, &current_revision));
        }
        let parent = target.parent().ok_or_else(|| {
            WorkspaceErrorBody::new(
                "workspace.pathEscape",
                "Workspace file has no jailed parent.",
                Some("path"),
            )
        })?;
        let (temporary, mut file) = create_temporary_file(parent)?;
        let write_result = (|| -> Result<(), WorkspaceErrorBody> {
            file.write_all(content.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| map_write_error(&error, path))?;
            fs::set_permissions(&temporary, metadata.permissions())
                .map_err(|error| map_write_error(&error, path))?;
            drop(file);

            let latest_metadata =
                fs::symlink_metadata(&target).map_err(|error| map_io_error(&error, path))?;
            require_file(path, &latest_metadata)?;
            let latest = read_bounded_file(&target, &latest_metadata, path)?;
            let latest_revision = content_revision(&latest);
            if latest_revision != expected_revision {
                return Err(revision_conflict(path, &latest_revision));
            }
            platform::replace_file(&temporary, &target)
                .map_err(|error| map_write_error(&error, path))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;

        let name = file_name_from_path(path)?;
        Ok(WorkspaceEntry {
            name: name.to_owned(),
            path: path.to_owned(),
            kind: WorkspaceEntryKind::File,
            size: Some(content_size),
            revision: Some(content_revision(content.as_bytes())),
        })
    }

    fn create_entry(
        &self,
        path: &str,
        kind: CreatableKind,
    ) -> Result<WorkspaceEntry, WorkspaceErrorBody> {
        let (parent, name) = self.resolve_parent_for_new(path)?;
        let target = parent.join(name);
        match fs::symlink_metadata(&target) {
            Ok(_) => return Err(path_conflict(path, "already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(&error, path)),
        }
        match kind {
            CreatableKind::File => OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .and_then(|file| file.sync_all())
                .map_err(|error| map_create_error(&error, path))?,
            CreatableKind::Directory => {
                fs::create_dir(&target).map_err(|error| map_create_error(&error, path))?;
            }
        }
        Ok(WorkspaceEntry {
            name: name.to_owned(),
            path: path.to_owned(),
            kind: match kind {
                CreatableKind::File => WorkspaceEntryKind::File,
                CreatableKind::Directory => WorkspaceEntryKind::Directory,
            },
            size: matches!(kind, CreatableKind::File).then_some(0),
            revision: matches!(kind, CreatableKind::File).then(|| content_revision(&[])),
        })
    }

    fn rename_entry(
        &self,
        path: &str,
        new_name: &str,
    ) -> Result<WorkspaceEntry, WorkspaceErrorBody> {
        validate_name(new_name, "newName")?;
        let parent_path = parent_relative(path)?;
        let target_path = join_relative(parent_path, new_name);
        self.move_entry(path, &target_path)
    }

    fn move_entry(
        &self,
        path: &str,
        target_path: &str,
    ) -> Result<WorkspaceEntry, WorkspaceErrorBody> {
        let source_segments = validate_relative_path(path, false)?;
        let target_segments = validate_relative_path(target_path, false)?;
        let (source, metadata) = self.resolve_existing(path, false)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(reparse_or_other_error(path));
        }
        if metadata.is_dir()
            && target_segments.len() > source_segments.len()
            && target_segments[..source_segments.len()] == source_segments
        {
            return Err(WorkspaceErrorBody::new(
                "workspace.invalidMove",
                "A directory cannot be moved inside itself.",
                Some("targetPath"),
            ));
        }
        if metadata.is_dir() {
            ensure_tree_has_no_reparse(&source, path)?;
        }
        let (target_parent, target_name) = self.resolve_parent_for_new(target_path)?;
        let target = target_parent.join(target_name);
        match fs::symlink_metadata(&target) {
            Ok(_) => return Err(path_conflict(target_path, "already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(&error, target_path)),
        }
        fs::rename(&source, &target).map_err(|error| map_move_error(&error, path, target_path))?;
        let moved_metadata =
            fs::symlink_metadata(&target).map_err(|error| map_io_error(&error, target_path))?;
        let mut entry = entry_from_metadata(target_name, target_path, &moved_metadata);
        if entry.kind == WorkspaceEntryKind::File {
            let bytes = read_bounded_file(&target, &moved_metadata, target_path)?;
            entry.revision = Some(content_revision(&bytes));
        }
        Ok(entry)
    }

    fn delete_entry(
        &self,
        path: &str,
        recursive: bool,
        confirm_path: Option<&str>,
    ) -> Result<WorkspaceEntryKind, WorkspaceErrorBody> {
        let (target, metadata) = self.resolve_existing(path, false)?;
        let kind = if metadata.is_file() {
            if recursive || confirm_path.is_some() {
                return Err(WorkspaceErrorBody::new(
                    "workspace.invalidDeleteConfirmation",
                    "File deletion requires recursive=false and confirmPath=null.",
                    Some("recursive"),
                ));
            }
            fs::remove_file(&target).map_err(|error| map_delete_error(&error, path))?;
            WorkspaceEntryKind::File
        } else if metadata.is_dir() {
            if recursive {
                if confirm_path != Some(path) {
                    return Err(WorkspaceErrorBody::new(
                        "workspace.deleteConfirmationRequired",
                        "Recursive directory deletion requires confirmPath to exactly match path.",
                        Some("confirmPath"),
                    ));
                }
                ensure_tree_has_no_reparse(&target, path)?;
                fs::remove_dir_all(&target).map_err(|error| map_delete_error(&error, path))?;
            } else {
                if confirm_path.is_some() {
                    return Err(WorkspaceErrorBody::new(
                        "workspace.invalidDeleteConfirmation",
                        "Non-recursive deletion requires confirmPath=null.",
                        Some("confirmPath"),
                    ));
                }
                fs::remove_dir(&target).map_err(|error| match error.kind() {
                    std::io::ErrorKind::DirectoryNotEmpty => WorkspaceErrorBody::new(
                        "workspace.directoryNotEmpty",
                        format!(
                            "Directory '{path}' is not empty; recursive confirmation is required."
                        ),
                        Some("recursive"),
                    ),
                    _ => map_delete_error(&error, path),
                })?;
            }
            WorkspaceEntryKind::Directory
        } else {
            return Err(reparse_or_other_error(path));
        };
        Ok(kind)
    }

    fn resolve_existing(
        &self,
        path: &str,
        allow_root: bool,
    ) -> Result<(PathBuf, Metadata), WorkspaceErrorBody> {
        let segments = validate_relative_path(path, allow_root)?;
        let root = self.root_snapshot()?.path;
        Self::ensure_root_stable(&root)?;
        let canonical_root = fs::canonicalize(&root).map_err(|error| map_root_io_error(&error))?;
        let mut current = root;
        let mut metadata =
            fs::symlink_metadata(&current).map_err(|error| map_root_io_error(&error))?;
        for (index, segment) in segments.iter().enumerate() {
            current.push(segment);
            metadata = fs::symlink_metadata(&current).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => WorkspaceErrorBody::new(
                    "workspace.notFound",
                    format!("Workspace path '{path}' does not exist."),
                    Some("path"),
                ),
                _ => map_io_error(&error, path),
            })?;
            if platform::is_reparse_or_symlink(&metadata) {
                return Err(reparse_or_other_error(path));
            }
            if index + 1 < segments.len() && !metadata.is_dir() {
                return Err(WorkspaceErrorBody::new(
                    "workspace.notDirectory",
                    format!("A parent component of '{path}' is not a directory."),
                    Some("path"),
                ));
            }
            let canonical =
                fs::canonicalize(&current).map_err(|error| map_io_error(&error, path))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(WorkspaceErrorBody::new(
                    "workspace.pathEscape",
                    format!("Workspace path '{path}' resolves outside the configured root."),
                    Some("path"),
                ));
            }
        }
        Ok((current, metadata))
    }

    fn resolve_parent_for_new<'a>(
        &self,
        path: &'a str,
    ) -> Result<(PathBuf, &'a str), WorkspaceErrorBody> {
        let segments = validate_relative_path(path, false)?;
        let name = segments.last().copied().ok_or_else(|| {
            WorkspaceErrorBody::new(
                "workspace.invalidPath",
                "A non-empty relative workspace path is required.",
                Some("path"),
            )
        })?;
        let parent_path = if segments.len() == 1 {
            String::new()
        } else {
            segments[..segments.len() - 1].join("/")
        };
        let (parent, metadata) = self.resolve_existing(&parent_path, true)?;
        if !metadata.is_dir() {
            return Err(WorkspaceErrorBody::new(
                "workspace.notDirectory",
                format!("Parent of '{path}' is not a directory."),
                Some("path"),
            ));
        }
        Ok((parent, name))
    }

    fn ensure_root_stable(root: &Path) -> Result<(), WorkspaceErrorBody> {
        let metadata = fs::symlink_metadata(root).map_err(|error| map_root_io_error(&error))?;
        if !metadata.is_dir() || platform::is_reparse_or_symlink(&metadata) {
            return Err(WorkspaceErrorBody::new(
                "workspace.rootChanged",
                "The configured workspace root changed or became a reparse point.",
                None,
            ));
        }
        Ok(())
    }
}

struct ReadFile {
    content: String,
    revision: String,
    size: u64,
}

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn string_param<'a>(
    params: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Option<&'a str> {
    params.get(name).and_then(serde_json::Value::as_str)
}

fn bool_param(params: &serde_json::Map<String, serde_json::Value>, name: &str) -> Option<bool> {
    params.get(name).and_then(serde_json::Value::as_bool)
}

fn parse_creatable_kind(value: Option<&serde_json::Value>) -> Result<CreatableKind, &'static str> {
    match value.and_then(serde_json::Value::as_str) {
        Some("file") => Ok(CreatableKind::File),
        Some("directory") => Ok(CreatableKind::Directory),
        _ => Err("Create kind must be file or directory."),
    }
}

fn validate_relative_path(path: &str, allow_empty: bool) -> Result<Vec<&str>, WorkspaceErrorBody> {
    if path.contains('\0') {
        return Err(WorkspaceErrorBody::new(
            "workspace.invalidPath",
            "Workspace paths must not contain NUL.",
            Some("path"),
        ));
    }
    if path.is_empty() {
        return if allow_empty {
            Ok(Vec::new())
        } else {
            Err(WorkspaceErrorBody::new(
                "workspace.invalidPath",
                "A non-empty relative workspace path is required.",
                Some("path"),
            ))
        };
    }
    if Path::new(path).is_absolute() || path.contains('\\') {
        return Err(WorkspaceErrorBody::new(
            "workspace.pathEscape",
            "Workspace paths must be relative and use forward slashes.",
            Some("path"),
        ));
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(WorkspaceErrorBody::new(
            "workspace.invalidPath",
            "Workspace paths must not contain empty segments.",
            Some("path"),
        ));
    }
    for segment in &segments {
        if *segment == "." || *segment == ".." {
            return Err(WorkspaceErrorBody::new(
                "workspace.pathEscape",
                "Workspace paths must not contain '.' or '..' segments.",
                Some("path"),
            ));
        }
        validate_name(segment, "path")?;
    }
    Ok(segments)
}

fn validate_name(name: &str, field: &'static str) -> Result<(), WorkspaceErrorBody> {
    if name.is_empty() || name.trim().is_empty() {
        return Err(WorkspaceErrorBody::new(
            "workspace.invalidName",
            "A file or folder name is required.",
            Some(field),
        ));
    }
    if name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(WorkspaceErrorBody::new(
            "workspace.pathEscape",
            "Name must be one Windows-compatible path component.",
            Some(field),
        ));
    }
    platform::validate_windows_compatible_name(name, field)
}

fn require_file(path: &str, metadata: &Metadata) -> Result<(), WorkspaceErrorBody> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(WorkspaceErrorBody::new(
            "workspace.notTextFile",
            format!("'{path}' is not a regular text file."),
            Some("path"),
        ))
    }
}

fn read_bounded_file(
    target: &Path,
    metadata: &Metadata,
    relative_path: &str,
) -> Result<Vec<u8>, WorkspaceErrorBody> {
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(file_too_large(relative_path, metadata.len()));
    }
    let bytes = fs::read(target).map_err(|error| map_io_error(&error, relative_path))?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_TEXT_FILE_BYTES {
        return Err(file_too_large(relative_path, size));
    }
    Ok(bytes)
}

fn content_revision(content: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in content {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64-{:016x}-{}", hash, content.len())
}

fn encode_ticket(bytes: &[u8; TICKET_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(TICKET_BYTES * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn valid_ticket(ticket: &str) -> bool {
    ticket.len() == TICKET_BYTES * 2
        && ticket
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn create_temporary_file(parent: &Path) -> Result<(PathBuf, fs::File), WorkspaceErrorBody> {
    for _ in 0..32 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".openmat-save-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(map_write_error(&error, "temporary save file")),
        }
    }
    Err(WorkspaceErrorBody::new(
        "workspace.ioFailure",
        "Could not allocate a unique temporary save file.",
        None,
    ))
}

fn ensure_tree_has_no_reparse(path: &Path, relative_path: &str) -> Result<(), WorkspaceErrorBody> {
    for item in fs::read_dir(path).map_err(|error| map_io_error(&error, relative_path))? {
        let item = item.map_err(|error| map_io_error(&error, relative_path))?;
        let metadata = fs::symlink_metadata(item.path())
            .map_err(|error| map_io_error(&error, relative_path))?;
        if platform::is_reparse_or_symlink(&metadata) {
            return Err(reparse_or_other_error(relative_path));
        }
        if metadata.is_dir() {
            ensure_tree_has_no_reparse(&item.path(), relative_path)?;
        }
    }
    Ok(())
}

fn entry_from_metadata(name: &str, path: &str, metadata: &Metadata) -> WorkspaceEntry {
    let kind = if platform::is_reparse_or_symlink(metadata) {
        WorkspaceEntryKind::Other
    } else if metadata.is_dir() {
        WorkspaceEntryKind::Directory
    } else if metadata.is_file() {
        WorkspaceEntryKind::File
    } else {
        WorkspaceEntryKind::Other
    };
    WorkspaceEntry {
        name: name.to_owned(),
        path: path.to_owned(),
        kind,
        size: (kind == WorkspaceEntryKind::File).then_some(metadata.len()),
        revision: None,
    }
}

fn compare_entries(left: &WorkspaceEntry, right: &WorkspaceEntry) -> std::cmp::Ordering {
    parent_relative(&left.path)
        .unwrap_or("")
        .cmp(parent_relative(&right.path).unwrap_or(""))
        .then_with(|| entry_kind_order(left.kind).cmp(&entry_kind_order(right.kind)))
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        .then_with(|| left.name.cmp(&right.name))
}

fn entry_kind_order(kind: WorkspaceEntryKind) -> u8 {
    match kind {
        WorkspaceEntryKind::Directory => 0,
        WorkspaceEntryKind::File => 1,
        WorkspaceEntryKind::Other => 2,
    }
}

fn kind_name(kind: WorkspaceEntryKind) -> &'static str {
    match kind {
        WorkspaceEntryKind::File => "file",
        WorkspaceEntryKind::Directory => "directory",
        WorkspaceEntryKind::Other => "other",
    }
}

fn entry_value_v1(entry: &WorkspaceEntry) -> serde_json::Value {
    serde_json::json!({ "name": entry.name, "kind": kind_name(entry.kind) })
}

pub(crate) fn entry_value_v2(entry: &WorkspaceEntry) -> serde_json::Value {
    serde_json::json!({
        "name": entry.name,
        "path": entry.path,
        "kind": kind_name(entry.kind),
        "size": entry.size,
        "revision": entry.revision,
    })
}

fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_relative(path: &str) -> Result<&str, WorkspaceErrorBody> {
    validate_relative_path(path, false)?;
    Ok(path.rsplit_once('/').map_or("", |(parent, _)| parent))
}

fn file_name_from_path(path: &str) -> Result<&str, WorkspaceErrorBody> {
    validate_relative_path(path, false)?;
    Ok(path.rsplit('/').next().unwrap_or(path))
}

fn file_too_large(path: &str, size: u64) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.fileTooLarge",
        format!("'{path}' is {size} bytes; text files are limited to {MAX_TEXT_FILE_BYTES} bytes."),
        Some("path"),
    )
    .with_details(serde_json::json!({
        "size": size,
        "maxSize": MAX_TEXT_FILE_BYTES,
    }))
}

fn upload_too_large(path: &str, size: u64) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.fileTooLarge",
        format!("'{path}' is {size} bytes; uploads are limited to {MAX_UPLOAD_FILE_BYTES} bytes."),
        Some("size"),
    )
    .with_details(serde_json::json!({
        "size": size,
        "maxSize": MAX_UPLOAD_FILE_BYTES,
    }))
}

fn revision_conflict(path: &str, current_revision: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.revisionConflict",
        format!("'{path}' changed outside OpenMat. The editor content was not overwritten."),
        Some("expectedRevision"),
    )
    .with_details(serde_json::json!({ "currentRevision": current_revision }))
}

fn path_conflict(path: &str, reason: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.conflict",
        format!("Workspace path '{path}' {reason}. Nothing was overwritten."),
        Some("path"),
    )
}

fn reparse_or_other_error(path: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.reparsePoint",
        format!(
            "Workspace path '{path}' is or contains a symbolic link, reparse point, or unsupported item."
        ),
        Some("path"),
    )
}

fn map_create_error(error: &std::io::Error, path: &str) -> WorkspaceErrorBody {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists => path_conflict(path, "already exists"),
        std::io::ErrorKind::PermissionDenied => WorkspaceErrorBody::new(
            "workspace.permissionDenied",
            format!("Permission was denied while creating '{path}'."),
            Some("path"),
        ),
        _ => WorkspaceErrorBody::new(
            "workspace.ioFailure",
            format!("Could not create '{path}': {error}"),
            Some("path"),
        ),
    }
}

fn map_root_io_error(error: &std::io::Error) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.rootUnavailable",
        format!("The configured workspace root is unavailable: {error}"),
        None,
    )
}

fn map_io_error(error: &std::io::Error, path: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.ioFailure",
        format!("Could not access workspace path '{path}': {error}"),
        Some("path"),
    )
}

fn map_write_error(error: &std::io::Error, path: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            "workspace.permissionDenied"
        } else {
            "workspace.ioFailure"
        },
        format!("Could not atomically save '{path}': {error}"),
        Some("path"),
    )
}

fn map_move_error(error: &std::io::Error, path: &str, target_path: &str) -> WorkspaceErrorBody {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists => path_conflict(target_path, "already exists"),
        std::io::ErrorKind::PermissionDenied => WorkspaceErrorBody::new(
            "workspace.permissionDenied",
            format!("Permission was denied while moving '{path}' to '{target_path}'."),
            Some("targetPath"),
        ),
        _ => WorkspaceErrorBody::new(
            "workspace.ioFailure",
            format!("Could not move '{path}' to '{target_path}': {error}"),
            Some("targetPath"),
        ),
    }
}

fn map_delete_error(error: &std::io::Error, path: &str) -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            "workspace.permissionDenied"
        } else {
            "workspace.ioFailure"
        },
        format!("Could not delete '{path}': {error}"),
        Some("path"),
    )
}

fn invalid_workspace_params(protocol: WorkspaceProtocol, request_id: &str) -> String {
    invalid_request_frame(
        protocol,
        request_id,
        "Workspace request params contain missing, mistyped, or unexpected fields.",
    )
}

fn unsupported_request_frame(
    protocol: WorkspaceProtocol,
    request_id: &str,
    request_type: &str,
) -> String {
    failure_frame(
        protocol,
        request_id,
        &WorkspaceErrorBody::new(
            "workspace.unsupportedRequest",
            format!("Unsupported workspace request type '{request_type}'."),
            Some("request.type"),
        ),
    )
}

fn invalid_request_frame(protocol: WorkspaceProtocol, request_id: &str, message: &str) -> String {
    failure_frame(
        protocol,
        request_id,
        &WorkspaceErrorBody::new("workspace.invalidRequest", message, None),
    )
}

fn success_frame(
    protocol: WorkspaceProtocol,
    request_id: &str,
    result: &serde_json::Value,
) -> String {
    serde_json::to_string(&serde_json::json!({
        "protocol": protocol.as_str(),
        "requestId": request_id,
        "ok": true,
        "result": result,
    }))
    .expect("workspace success response contains only serializable values")
}

fn failure_frame(
    protocol: WorkspaceProtocol,
    request_id: &str,
    error: &WorkspaceErrorBody,
) -> String {
    let mut error_value = serde_json::json!({
        "code": error.code,
        "message": error.message,
    });
    if let Some(field) = error.field {
        error_value["field"] = serde_json::Value::String(field.to_owned());
    }
    if let Some(details) = &error.details {
        error_value["details"] = details.clone();
    }
    serde_json::to_string(&serde_json::json!({
        "protocol": protocol.as_str(),
        "requestId": request_id,
        "ok": false,
        "error": error_value,
    }))
    .expect("workspace failure response contains only serializable values")
}

fn path_text(path: &Path) -> Result<String, WorkspaceErrorBody> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        WorkspaceErrorBody::new(
            "workspace.unsupportedPath",
            "Current Folder is not valid UTF-8 and cannot be represented in the Web IDE.",
            None,
        )
    })
}

fn workspace_relative_path(root: &Path, path: &Path) -> Result<Option<String>, WorkspaceErrorBody> {
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    let mut components = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            continue;
        };
        let component = component.to_str().ok_or_else(|| {
            WorkspaceErrorBody::new(
                "workspace.unsupportedPath",
                "A search-path directory is not valid UTF-8.",
                None,
            )
        })?;
        components.push(component);
    }
    Ok(Some(components.join("/")))
}

fn directory_root_name(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        text.into_owned()
    } else {
        trimmed.to_owned()
    }
}

fn directory_browser_entry_value(entry: &DirectoryBrowserEntry) -> serde_json::Value {
    serde_json::json!({ "name": entry.name, "path": entry.path })
}

fn current_directory_value(
    snapshot: &WorkingDirectorySnapshot,
) -> Result<serde_json::Value, WorkspaceErrorBody> {
    let path = path_text(&snapshot.path)?;
    let name = snapshot
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    Ok(serde_json::json!({
        "path": path,
        "rootName": name,
        "generation": snapshot.generation,
    }))
}

fn current_directory_event(
    snapshot: &WorkingDirectorySnapshot,
    protocol: WorkspaceProtocol,
) -> serde_json::Value {
    let data = current_directory_value(snapshot).unwrap_or_else(|error| {
        serde_json::json!({
            "path": "",
            "rootName": "workspace",
            "generation": snapshot.generation,
            "error": { "code": error.code, "message": error.message },
        })
    });
    serde_json::json!({
        "protocol": protocol.as_str(),
        "event": { "type": "currentDirectoryChanged", "data": data },
    })
}

pub(crate) fn workspace_changed_event(root_generation: u64, protocol: WorkspaceProtocol) -> String {
    serde_json::json!({
        "protocol": protocol.as_str(),
        "event": {
            "type": "workspaceChanged",
            "data": { "rootGeneration": root_generation },
        },
    })
    .to_string()
}

fn map_working_directory_error(error: openmat_runtime::FileSystemError) -> WorkspaceErrorBody {
    use openmat_runtime::FileSystemErrorCategory;

    let code = match error.category {
        FileSystemErrorCategory::InvalidPath => "workspace.invalidDirectory",
        FileSystemErrorCategory::NotFound => "workspace.directoryNotFound",
        FileSystemErrorCategory::NotDirectory | FileSystemErrorCategory::NotFile => {
            "workspace.notDirectory"
        }
        FileSystemErrorCategory::PermissionDenied => "workspace.permissionDenied",
        FileSystemErrorCategory::InvalidEncoding
        | FileSystemErrorCategory::TooLarge
        | FileSystemErrorCategory::Io => "workspace.ioFailure",
    };
    WorkspaceErrorBody::new(code, error.message, Some("path"))
}

fn workspace_state_error() -> WorkspaceErrorBody {
    WorkspaceErrorBody::new(
        "workspace.stateUnavailable",
        "The Current Folder state is temporarily unavailable.",
        None,
    )
}

mod platform {
    use std::fs::Metadata;
    use std::path::{Path, PathBuf};

    use super::WorkspaceErrorBody;

    const WINDOWS_RESERVED_STEMS: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
        "CONOUT$", "CLOCK$",
    ];

    pub(super) fn validate_windows_compatible_name(
        name: &str,
        field: &'static str,
    ) -> Result<(), WorkspaceErrorBody> {
        if name.ends_with([' ', '.'])
            || name
                .chars()
                .any(|character| character <= '\u{1f}' || "<>:\"|?*".contains(character))
        {
            return Err(WorkspaceErrorBody::new(
                "workspace.invalidWindowsName",
                "Name contains characters or a trailing suffix that Windows does not allow.",
                Some(field),
            ));
        }
        if name.encode_utf16().count() > 255 {
            return Err(WorkspaceErrorBody::new(
                "workspace.invalidWindowsName",
                "Name is longer than the Windows component limit.",
                Some(field),
            ));
        }
        let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
        if WINDOWS_RESERVED_STEMS.contains(&stem.as_str()) {
            return Err(WorkspaceErrorBody::new(
                "workspace.reservedWindowsName",
                format!("'{name}' is a reserved Windows device name."),
                Some(field),
            ));
        }
        Ok(())
    }

    pub(super) fn is_matlab_source_name(name: &str) -> bool {
        Path::new(name)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| {
                if cfg!(windows) {
                    extension.eq_ignore_ascii_case("m")
                } else {
                    extension == "m"
                }
            })
    }

    #[cfg(windows)]
    pub(super) fn is_reparse_or_symlink(metadata: &Metadata) -> bool {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    pub(super) fn is_reparse_or_symlink(metadata: &Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    #[cfg(windows)]
    pub(super) fn displayable_canonical_path(path: PathBuf) -> PathBuf {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{rest}"))
        } else if let Some(rest) = text.strip_prefix(r"\\?\") {
            PathBuf::from(rest)
        } else {
            path
        }
    }

    #[cfg(not(windows))]
    pub(super) const fn displayable_canonical_path(path: PathBuf) -> PathBuf {
        path
    }

    #[cfg(windows)]
    pub(super) fn directory_roots() -> Vec<PathBuf> {
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn GetLogicalDrives() -> u32;
        }

        // SAFETY: GetLogicalDrives has no parameters and returns a bit mask.
        let mask = unsafe { GetLogicalDrives() };
        (0_u32..26)
            .filter(|index| mask & (1_u32 << index) != 0)
            .map(|index| {
                let letter = char::from_u32(u32::from(b'A') + index).unwrap_or('A');
                PathBuf::from(format!("{letter}:\\"))
            })
            .collect()
    }

    #[cfg(not(windows))]
    pub(super) fn directory_roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/")]
    }

    #[cfg(windows)]
    pub(super) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;

        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
        }

        let source = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: Both paths are owned, NUL-terminated UTF-16 buffers that live
        // for the duration of the call. Flags request a same-volume replacement.
        let replaced = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(windows))]
    pub(super) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::fs::rename(source, destination)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "openmat-workspace-test-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[allow(clippy::needless_pass_by_value)] // Inline JSON keeps protocol cases readable.
    fn exchange(
        service: &WorkspaceService,
        protocol: WorkspaceProtocol,
        request_id: &str,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let frame = serde_json::json!({
            "protocol": protocol.as_str(),
            "requestId": request_id,
            "request": request,
        })
        .to_string();
        serde_json::from_str(&service.handle_text_for_protocol(&frame, protocol)).unwrap()
    }

    fn assert_current_directory_event(service: &WorkspaceService, expected_generation: u64) {
        let (_, event) = service
            .current_directory_event(WorkspaceProtocol::V2)
            .unwrap();
        let event: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["event"]["type"], "currentDirectoryChanged");
        assert_eq!(event["event"]["data"]["generation"], expected_generation);
    }

    #[test]
    fn v1_list_and_create_remain_compatible() {
        let root = TestRoot::new();
        let service = WorkspaceService::new(&root.0).unwrap();
        let file = exchange(
            &service,
            WorkspaceProtocol::V1,
            "create-file",
            serde_json::json!({
                "type": "create",
                "params": { "name": "analysis.m", "kind": "file" }
            }),
        );
        assert_eq!(file["ok"], true);
        assert_eq!(
            file["result"]["data"]["entry"],
            serde_json::json!({ "name": "analysis.m", "kind": "file" })
        );
        let listed = exchange(
            &service,
            WorkspaceProtocol::V1,
            "list",
            serde_json::json!({ "type": "list", "params": {} }),
        );
        assert_eq!(
            listed["result"]["data"]["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let nested = exchange(
            &service,
            WorkspaceProtocol::V1,
            "nested-v1-create",
            serde_json::json!({
                "type": "create",
                "params": { "name": "folder/nested.m", "kind": "file" }
            }),
        );
        assert_eq!(nested["error"]["code"], "workspace.pathEscape");
    }

    #[test]
    fn v2_round_trips_nested_files_and_detects_external_write_conflicts() {
        let root = TestRoot::new();
        let service = WorkspaceService::new(&root.0).unwrap();
        for (path, kind) in [("src", "directory"), ("src/main.m", "file")] {
            assert_eq!(
                exchange(
                    &service,
                    WorkspaceProtocol::V2,
                    "create",
                    serde_json::json!({
                        "type": "create", "params": { "path": path, "kind": kind }
                    }),
                )["ok"],
                true
            );
        }
        let opened = exchange(
            &service,
            WorkspaceProtocol::V2,
            "read",
            serde_json::json!({ "type": "read", "params": { "path": "src/main.m" } }),
        );
        let revision = opened["result"]["data"]["revision"].as_str().unwrap();
        let saved = exchange(
            &service,
            WorkspaceProtocol::V2,
            "write",
            serde_json::json!({
                "type": "write",
                "params": {
                    "path": "src/main.m",
                    "content": "answer = 42;\n",
                    "expectedRevision": revision
                }
            }),
        );
        assert_eq!(saved["ok"], true);
        let saved_revision = saved["result"]["data"]["entry"]["revision"]
            .as_str()
            .unwrap();
        fs::write(root.0.join("src/main.m"), "external = true;\n").unwrap();
        let conflict = exchange(
            &service,
            WorkspaceProtocol::V2,
            "write-conflict",
            serde_json::json!({
                "type": "write",
                "params": {
                    "path": "src/main.m",
                    "content": "lost = true;\n",
                    "expectedRevision": saved_revision
                }
            }),
        );
        assert_eq!(conflict["error"]["code"], "workspace.revisionConflict");
        assert!(conflict["error"]["details"]["currentRevision"].is_string());
        assert_eq!(
            fs::read_to_string(root.0.join("src/main.m")).unwrap(),
            "external = true;\n"
        );
    }

    #[test]
    fn v2_manages_recursive_search_paths_for_the_shared_kernel_state() {
        let root = TestRoot::new();
        fs::create_dir_all(root.0.join("toolbox/solver")).unwrap();
        fs::create_dir_all(root.0.join("toolbox/private")).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let shared = service.search_path();

        let added = exchange(
            &service,
            WorkspaceProtocol::V2,
            "add-path",
            serde_json::json!({
                "type": "addSearchPath",
                "params": { "path": "toolbox", "recursive": true, "position": "begin" }
            }),
        );
        assert_eq!(added["ok"], true);
        let directories = added["result"]["data"]["directories"].as_array().unwrap();
        assert_eq!(directories.len(), 2);
        assert_eq!(directories[0]["workspacePath"], "toolbox");
        assert_eq!(directories[1]["workspacePath"], "toolbox/solver");
        assert_eq!(shared.snapshot().unwrap().paths.len(), 2);

        let listed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "list-path",
            serde_json::json!({ "type": "searchPath", "params": {} }),
        );
        assert_eq!(listed["result"]["data"], added["result"]["data"]);

        let removed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "remove-path",
            serde_json::json!({
                "type": "removeSearchPath",
                "params": { "path": "toolbox", "recursive": true }
            }),
        );
        assert_eq!(removed["ok"], true);
        assert!(
            removed["result"]["data"]["directories"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(shared.snapshot().unwrap().paths.is_empty());
    }

    #[test]
    fn v2_browses_directories_without_changing_current_directory() {
        let root = TestRoot::new();
        fs::create_dir_all(root.0.join("alpha/nested")).unwrap();
        fs::create_dir(root.0.join("beta")).unwrap();
        fs::write(root.0.join("not-a-folder.m"), "value = 1;\n").unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let initial = service.working_directory().snapshot().unwrap();
        let root_path = path_text(&initial.path).unwrap();

        let browsed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "browse-root",
            serde_json::json!({
                "type": "browseDirectories", "params": { "path": root_path }
            }),
        );
        assert_eq!(browsed["ok"], true);
        assert_eq!(browsed["result"]["data"]["path"], root_path);
        let names = browsed["result"]["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, ["alpha", "beta"]);
        assert!(
            !browsed["result"]["data"]["roots"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let alpha_path = path_text(&fs::canonicalize(root.0.join("alpha")).unwrap()).unwrap();
        let nested = exchange(
            &service,
            WorkspaceProtocol::V2,
            "browse-alpha",
            serde_json::json!({
                "type": "browseDirectories", "params": { "path": alpha_path }
            }),
        );
        assert_eq!(nested["result"]["data"]["parentPath"], root_path);
        assert_eq!(nested["result"]["data"]["entries"][0]["name"], "nested");
        assert_eq!(service.working_directory().snapshot().unwrap(), initial);

        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "browse-file",
            serde_json::json!({
                "type": "browseDirectories",
                "params": { "path": root.0.join("not-a-folder.m").to_string_lossy() }
            }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.notDirectory");
    }

    #[test]
    fn v2_changes_current_directory_and_keeps_open_document_roots_stable() {
        let root = TestRoot::new();
        let first = root.0.join("first");
        let second = root.0.join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("opened.m"), "value = 1;\n").unwrap();
        fs::write(second.join("second.m"), "value = 2;\n").unwrap();
        let service = WorkspaceService::new(&first).unwrap();

        let initial = exchange(
            &service,
            WorkspaceProtocol::V2,
            "current",
            serde_json::json!({ "type": "currentDirectory", "params": {} }),
        );
        let initial_generation = initial["result"]["data"]["generation"].as_u64().unwrap();
        assert_eq!(initial["result"]["data"]["rootName"], "first");
        let opened = exchange(
            &service,
            WorkspaceProtocol::V2,
            "read-before-switch",
            serde_json::json!({ "type": "read", "params": { "path": "opened.m" } }),
        );
        assert_eq!(
            opened["result"]["data"]["rootGeneration"],
            initial_generation
        );
        let revision = opened["result"]["data"]["revision"]
            .as_str()
            .unwrap()
            .to_owned();

        let changed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "change",
            serde_json::json!({
                "type": "changeDirectory",
                "params": { "path": second.to_string_lossy() }
            }),
        );
        assert_eq!(changed["ok"], true);
        let changed_generation = changed["result"]["data"]["generation"].as_u64().unwrap();
        assert_eq!(changed_generation, initial_generation + 1);
        assert_eq!(changed["result"]["data"]["rootName"], "second");
        assert_eq!(
            service.working_directory().snapshot().unwrap().path,
            WorkingDirectory::new(&second)
                .unwrap()
                .snapshot()
                .unwrap()
                .path
        );

        let listed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "list-after-switch",
            serde_json::json!({
                "type": "list", "params": { "path": "", "recursive": false }
            }),
        );
        assert_eq!(
            listed["result"]["data"]["rootGeneration"],
            changed_generation
        );
        assert_eq!(listed["result"]["data"]["entries"][0]["name"], "second.m");

        let saved = exchange(
            &service,
            WorkspaceProtocol::V2,
            "save-old-root",
            serde_json::json!({
                "type": "write",
                "params": {
                    "path": "opened.m",
                    "content": "value = 3;\n",
                    "expectedRevision": revision,
                    "rootGeneration": initial_generation
                }
            }),
        );
        assert_eq!(saved["ok"], true);
        assert_eq!(
            fs::read_to_string(first.join("opened.m")).unwrap(),
            "value = 3;\n"
        );

        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "reject-file-as-directory",
            serde_json::json!({
                "type": "changeDirectory", "params": { "path": "second.m" }
            }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.notDirectory");
        assert_eq!(
            service.working_directory().snapshot().unwrap().generation,
            changed_generation
        );

        assert_current_directory_event(&service, changed_generation);
    }

    #[test]
    fn v2_workspace_changed_event_carries_the_current_root_generation() {
        let changed: serde_json::Value =
            serde_json::from_str(&workspace_changed_event(17, WorkspaceProtocol::V2)).unwrap();
        assert_eq!(changed["protocol"], PROTOCOL_V2);
        assert_eq!(changed["event"]["type"], "workspaceChanged");
        assert_eq!(changed["event"]["data"]["rootGeneration"], 17);
    }

    #[test]
    fn v2_recursively_lists_moves_and_requires_exact_recursive_delete_confirmation() {
        let root = TestRoot::new();
        fs::create_dir_all(root.0.join("src/nested")).unwrap();
        fs::write(root.0.join("src/nested/one.m"), "one = 1;\n").unwrap();
        fs::create_dir(root.0.join("dest")).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let listed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "list",
            serde_json::json!({
                "type": "list", "params": { "path": "", "recursive": true }
            }),
        );
        let paths = listed["result"]["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"src/nested/one.m"));

        let renamed = exchange(
            &service,
            WorkspaceProtocol::V2,
            "rename",
            serde_json::json!({
                "type": "rename",
                "params": { "path": "src/nested/one.m", "newName": "two.m" }
            }),
        );
        assert_eq!(
            renamed["result"]["data"]["entry"]["path"],
            "src/nested/two.m"
        );
        let moved = exchange(
            &service,
            WorkspaceProtocol::V2,
            "move",
            serde_json::json!({
                "type": "move",
                "params": { "path": "src/nested/two.m", "targetPath": "dest/two.m" }
            }),
        );
        assert_eq!(moved["result"]["data"]["entry"]["path"], "dest/two.m");

        let refused = exchange(
            &service,
            WorkspaceProtocol::V2,
            "delete-refused",
            serde_json::json!({
                "type": "delete",
                "params": { "path": "dest", "recursive": true, "confirmPath": "src" }
            }),
        );
        assert_eq!(
            refused["error"]["code"],
            "workspace.deleteConfirmationRequired"
        );
        assert!(root.0.join("dest/two.m").exists());
        let deleted = exchange(
            &service,
            WorkspaceProtocol::V2,
            "delete",
            serde_json::json!({
                "type": "delete",
                "params": { "path": "dest", "recursive": true, "confirmPath": "dest" }
            }),
        );
        assert_eq!(deleted["ok"], true);
        assert!(!root.0.join("dest").exists());
    }

    #[test]
    fn rejects_traversal_empty_segments_absolute_and_windows_reserved_names() {
        let root = TestRoot::new();
        let service = WorkspaceService::new(&root.0).unwrap();
        for (path, code) in [
            ("..", "workspace.pathEscape"),
            ("../escape", "workspace.pathEscape"),
            ("folder//escape", "workspace.invalidPath"),
            ("folder\\escape", "workspace.pathEscape"),
            ("C:\\escape.m", "workspace.pathEscape"),
            ("CON", "workspace.reservedWindowsName"),
            ("nul.txt", "workspace.reservedWindowsName"),
            ("bad?.m", "workspace.invalidWindowsName"),
            ("trailing. ", "workspace.invalidWindowsName"),
        ] {
            let rejected = exchange(
                &service,
                WorkspaceProtocol::V2,
                "rejected",
                serde_json::json!({
                    "type": "create", "params": { "path": path, "kind": "file" }
                }),
            );
            assert_eq!(rejected["error"]["code"], code, "path {path}");
        }
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    }

    #[test]
    fn read_rejects_invalid_utf8_and_oversized_files_with_structured_errors() {
        let root = TestRoot::new();
        fs::write(root.0.join("binary.dat"), [0xff, 0xfe]).unwrap();
        let too_large_length = usize::try_from(MAX_TEXT_FILE_BYTES + 1).unwrap();
        fs::write(root.0.join("large.m"), vec![b'x'; too_large_length]).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        for (path, code) in [
            ("binary.dat", "workspace.invalidUtf8"),
            ("large.m", "workspace.fileTooLarge"),
        ] {
            let rejected = exchange(
                &service,
                WorkspaceProtocol::V2,
                "read-rejected",
                serde_json::json!({ "type": "read", "params": { "path": path } }),
            );
            assert_eq!(rejected["error"]["code"], code);
        }
    }

    #[test]
    fn download_ticket_streams_binary_files_once_without_the_text_limit() {
        let root = TestRoot::new();
        let mut expected = vec![0xff, 0xfe, 0x00, 0x01];
        expected.extend(vec![b'x'; usize::try_from(MAX_TEXT_FILE_BYTES).unwrap()]);
        fs::write(root.0.join("result.mat"), &expected).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let root_generation = service.root_generation().unwrap();
        let prepared = exchange(
            &service,
            WorkspaceProtocol::V3,
            "prepare-download",
            serde_json::json!({
                "type": "prepareDownload",
                "params": { "path": "result.mat", "rootGeneration": root_generation }
            }),
        );
        assert_eq!(prepared["ok"], true);
        assert_eq!(prepared["result"]["data"]["name"], "result.mat");
        assert_eq!(
            prepared["result"]["data"]["size"],
            u64::try_from(expected.len()).unwrap()
        );
        let ticket = prepared["result"]["data"]["ticket"].as_str().unwrap();
        assert!(valid_ticket(ticket));

        let mut download = service.consume_download(ticket).unwrap();
        let mut actual = Vec::new();
        download.file.read_to_end(&mut actual).unwrap();
        assert_eq!(download.name, "result.mat");
        assert_eq!(actual, expected);
        assert!(service.consume_download(ticket).is_none());
    }

    #[test]
    fn upload_ticket_commits_binary_bytes_once_and_requires_explicit_overwrite() {
        let root = TestRoot::new();
        fs::write(root.0.join("existing.mat"), b"old").unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let root_generation = service.root_generation().unwrap();
        let bytes = [0x00, 0xff, 0x80, b'M', b'A', b'T'];
        let prepared = exchange(
            &service,
            WorkspaceProtocol::V3,
            "prepare-upload",
            serde_json::json!({
                "type": "prepareUpload",
                "params": {
                    "path": "nested.mat",
                    "size": bytes.len(),
                    "rootGeneration": root_generation,
                    "overwrite": false
                }
            }),
        );
        assert_eq!(prepared["ok"], true);
        let ticket = prepared["result"]["data"]["ticket"].as_str().unwrap();
        assert!(valid_ticket(ticket));
        let mut upload = service
            .consume_upload(ticket, u64::try_from(bytes.len()).unwrap())
            .unwrap()
            .unwrap();
        upload.writer().write_all(&bytes).unwrap();
        let entry = upload.commit().unwrap();
        assert_eq!(entry.path, "nested.mat");
        assert_eq!(entry.size, Some(u64::try_from(bytes.len()).unwrap()));
        assert_eq!(fs::read(root.0.join("nested.mat")).unwrap(), bytes);
        assert!(service.consume_upload(ticket, 0).unwrap().is_none());

        let rejected = exchange(
            &service,
            WorkspaceProtocol::V3,
            "upload-conflict",
            serde_json::json!({
                "type": "prepareUpload",
                "params": {
                    "path": "existing.mat",
                    "size": bytes.len(),
                    "rootGeneration": root_generation,
                    "overwrite": false
                }
            }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.conflict");

        let replacement = exchange(
            &service,
            WorkspaceProtocol::V3,
            "upload-replacement",
            serde_json::json!({
                "type": "prepareUpload",
                "params": {
                    "path": "existing.mat",
                    "size": bytes.len(),
                    "rootGeneration": root_generation,
                    "overwrite": true
                }
            }),
        );
        let ticket = replacement["result"]["data"]["ticket"].as_str().unwrap();
        let mut upload = service
            .consume_upload(ticket, u64::try_from(bytes.len()).unwrap())
            .unwrap()
            .unwrap();
        upload.writer().write_all(&bytes).unwrap();
        upload.commit().unwrap();
        assert_eq!(fs::read(root.0.join("existing.mat")).unwrap(), bytes);
    }

    #[test]
    fn v2_rejects_the_v3_prepare_download_request() {
        let root = TestRoot::new();
        fs::write(root.0.join("result.mat"), b"data").unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "prepare-download-v2",
            serde_json::json!({
                "type": "prepareDownload",
                "params": { "path": "result.mat", "rootGeneration": 0 }
            }),
        );
        assert_eq!(rejected["protocol"], PROTOCOL_V2);
        assert_eq!(rejected["error"]["code"], "workspace.unsupportedRequest");
        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "prepare-upload-v2",
            serde_json::json!({
                "type": "prepareUpload",
                "params": {
                    "path": "result.mat",
                    "size": 4,
                    "rootGeneration": 0,
                    "overwrite": false
                }
            }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.unsupportedRequest");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_traversal() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new();
        let outside = TestRoot::new();
        fs::write(outside.0.join("outside.m"), "outside = true;\n").unwrap();
        symlink(&outside.0, root.0.join("linked")).unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "symlink",
            serde_json::json!({ "type": "read", "params": { "path": "linked/outside.m" } }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.reparsePoint");
        let (generation, _) = service.current_directory_watch_target().unwrap();
        assert!(
            service
                .lsp_source_snapshot(generation)
                .unwrap()
                .documents
                .is_empty()
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_reparse_traversal_when_symlink_creation_is_available() {
        use std::os::windows::fs::symlink_dir;

        let root = TestRoot::new();
        let outside = TestRoot::new();
        fs::write(outside.0.join("outside.m"), "outside = true;\n").unwrap();
        if symlink_dir(&outside.0, root.0.join("linked")).is_err() {
            return;
        }
        let service = WorkspaceService::new(&root.0).unwrap();
        let rejected = exchange(
            &service,
            WorkspaceProtocol::V2,
            "reparse",
            serde_json::json!({ "type": "read", "params": { "path": "linked/outside.m" } }),
        );
        assert_eq!(rejected["error"]["code"], "workspace.reparsePoint");
        let (generation, _) = service.current_directory_watch_target().unwrap();
        assert!(
            service
                .lsp_source_snapshot(generation)
                .unwrap()
                .documents
                .is_empty()
        );
    }

    #[test]
    fn lsp_snapshot_reads_only_matlab_text_and_reports_partial_sources() {
        let root = TestRoot::new();
        fs::create_dir(root.0.join("+pkg")).unwrap();
        fs::create_dir(root.0.join(".git")).unwrap();
        fs::write(root.0.join("main.m"), "result = pkg.helper(1);\n").unwrap();
        fs::write(
            root.0.join("+pkg/helper.m"),
            "function y = helper(x)\ny=x;\nend\n",
        )
        .unwrap();
        fs::write(root.0.join("data.bin"), [0xff, 0xfe]).unwrap();
        fs::write(root.0.join(".git/hidden.m"), "internal=1;\n").unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let (generation, _) = service.current_directory_watch_target().unwrap();
        let complete = service.lsp_source_snapshot(generation).unwrap();
        assert!(complete.incomplete_reason.is_none());
        assert_eq!(
            complete
                .documents
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["+pkg/helper.m", "main.m"]
        );
        fs::write(root.0.join("broken.m"), [0xff, 0xfe]).unwrap();
        let partial = service.lsp_source_snapshot(generation).unwrap();
        assert!(
            partial
                .incomplete_reason
                .as_ref()
                .unwrap()
                .contains("UTF-8")
        );
        assert_eq!(partial.documents.len(), 2);
        let other = TestRoot::new();
        service.working_directory().change(&other.0).unwrap();
        assert!(service.lsp_source_snapshot(generation).is_err());
    }

    #[test]
    fn lsp_snapshot_uses_platform_source_extension_rules() {
        let root = TestRoot::new();
        fs::write(root.0.join("uppercase.M"), "x=1;\n").unwrap();
        let service = WorkspaceService::new(&root.0).unwrap();
        let (generation, _) = service.current_directory_watch_target().unwrap();
        let snapshot = service.lsp_source_snapshot(generation).unwrap();
        assert_eq!(
            snapshot.documents.contains_key("uppercase.M"),
            cfg!(windows)
        );
        assert!(snapshot.incomplete_reason.is_none());
    }

    #[test]
    fn generation_bound_rename_preserves_original_root_and_rejects_unknown_or_occupied_targets() {
        for protocol in [WorkspaceProtocol::V2, WorkspaceProtocol::V3] {
            let first = TestRoot::new();
            let second = TestRoot::new();
            fs::write(first.0.join("helper.m"), "original=1;\n").unwrap();
            fs::write(first.0.join("occupied.m"), "keep=1;\n").unwrap();
            fs::write(second.0.join("helper.m"), "unrelated=2;\n").unwrap();
            let service = WorkspaceService::new(&first.0).unwrap();
            let (generation, _) = service.current_directory_watch_target().unwrap();
            service.working_directory().change(&second.0).unwrap();
            let renamed = exchange(
                &service,
                protocol,
                "rename-original",
                serde_json::json!({
                    "type": "rename", "params": { "path": "helper.m", "newName": "calculate.m", "rootGeneration": generation }
                }),
            );
            assert_eq!(renamed["ok"], true, "{renamed}");
            assert_eq!(renamed["result"]["data"]["previousPath"], "helper.m");
            assert!(!first.0.join("helper.m").exists());
            assert_eq!(
                fs::read_to_string(first.0.join("calculate.m")).unwrap(),
                "original=1;\n"
            );
            assert_eq!(
                fs::read_to_string(second.0.join("helper.m")).unwrap(),
                "unrelated=2;\n"
            );
            assert!(!second.0.join("calculate.m").exists());
            let conflict = exchange(
                &service,
                protocol,
                "occupied",
                serde_json::json!({
                    "type": "rename", "params": { "path": "calculate.m", "newName": "occupied.m", "rootGeneration": generation }
                }),
            );
            assert_eq!(
                conflict["error"]["code"], "workspace.conflict",
                "{conflict}"
            );
            assert_eq!(
                fs::read_to_string(first.0.join("occupied.m")).unwrap(),
                "keep=1;\n"
            );
            assert!(first.0.join("calculate.m").exists());
            let unknown = exchange(
                &service,
                protocol,
                "unknown-root",
                serde_json::json!({
                    "type": "rename", "params": { "path": "helper.m", "newName": "unknown.m", "rootGeneration": u64::MAX }
                }),
            );
            assert_eq!(unknown["error"]["code"], "workspace.unknownRootGeneration");
            assert!(second.0.join("helper.m").exists());
            for invalid_generation in [
                serde_json::Value::Null,
                serde_json::json!(-1),
                serde_json::json!("1"),
            ] {
                let invalid = exchange(
                    &service,
                    protocol,
                    "invalid-root",
                    serde_json::json!({
                        "type": "rename", "params": { "path": "helper.m", "newName": "invalid.m", "rootGeneration": invalid_generation }
                    }),
                );
                assert_eq!(invalid["error"]["code"], "workspace.invalidRequest");
                assert!(second.0.join("helper.m").exists());
            }
            let current = exchange(
                &service,
                protocol,
                "rename-current",
                serde_json::json!({
                    "type": "rename", "params": { "path": "helper.m", "newName": "current.m" }
                }),
            );
            assert_eq!(current["ok"], true);
            assert_eq!(
                fs::read_to_string(second.0.join("current.m")).unwrap(),
                "unrelated=2;\n"
            );
        }
    }

    #[test]
    fn explicit_current_generation_remains_frozen_if_the_kernel_changes_directory_mid_operation() {
        let first = TestRoot::new();
        let second = TestRoot::new();
        fs::write(first.0.join("helper.m"), "original=1;\n").unwrap();
        fs::write(second.0.join("helper.m"), "unrelated=2;\n").unwrap();
        let service = WorkspaceService::new(&first.0).unwrap();
        let (generation, _) = service.current_directory_watch_target().unwrap();
        let operation = service.service_for_generation(Some(generation)).unwrap();
        service.working_directory().change(&second.0).unwrap();
        operation.rename_entry("helper.m", "calculate.m").unwrap();
        assert_eq!(
            fs::read_to_string(first.0.join("calculate.m")).unwrap(),
            "original=1;\n"
        );
        assert_eq!(
            fs::read_to_string(second.0.join("helper.m")).unwrap(),
            "unrelated=2;\n"
        );
        assert!(!second.0.join("calculate.m").exists());
    }

    #[test]
    fn lsp_snapshot_bounds_document_count_and_marks_the_result_incomplete() {
        let root = TestRoot::new();
        for index in 0..2_049 {
            fs::write(root.0.join(format!("source{index:04}.m")), "x=1;\n").unwrap();
        }
        let service = WorkspaceService::new(&root.0).unwrap();
        let (generation, _) = service.current_directory_watch_target().unwrap();
        let snapshot = service.lsp_source_snapshot(generation).unwrap();
        assert_eq!(snapshot.documents.len(), 2_048);
        assert!(
            snapshot
                .incomplete_reason
                .as_ref()
                .unwrap()
                .contains("2,048")
        );
    }
}
