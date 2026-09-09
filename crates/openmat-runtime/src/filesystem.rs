use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs, io,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::SystemTime,
};

/// Placement used when directories are added to the MATLAB search path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchPathPosition {
    /// Give newly added directories precedence over existing search-path entries.
    Begin,
    /// Append newly added directories after existing search-path entries.
    End,
}

/// One atomic view of the session-owned MATLAB search path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPathSnapshot {
    /// Canonical directories in lookup order.
    pub paths: Vec<PathBuf>,
    /// Monotonic version changed by every effective mutation.
    pub generation: u64,
}

/// Shared, dynamically mutable MATLAB search-path state.
#[derive(Clone, Debug)]
pub struct SearchPath {
    state: Arc<RwLock<SearchPathState>>,
}

#[derive(Debug)]
struct SearchPathState {
    paths: Vec<PathBuf>,
    generation: u64,
}

impl SearchPath {
    /// Creates an empty search path.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            state: Arc::new(RwLock::new(SearchPathState {
                paths: Vec::new(),
                generation: 0,
            })),
        }
    }

    /// Creates a search path from canonicalized, existing directories.
    /// MATLAB-special `private` and package (`+name`) directories are omitted
    /// from effective path state.
    ///
    /// Relative inputs are resolved against the process working directory. Hosts
    /// should use [`Self::replace_from`] when they own a session working directory.
    ///
    /// # Errors
    ///
    /// Returns a structured filesystem error when any supplied directory is invalid.
    pub fn new<I, P>(paths: I) -> Result<Self, FileSystemError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let state = Self::empty();
        state.replace_from(paths, None)?;
        Ok(state)
    }

    /// Returns one consistent path list and its generation.
    ///
    /// # Errors
    ///
    /// Returns an I/O-category error if the shared state lock is unavailable.
    pub fn snapshot(&self) -> Result<SearchPathSnapshot, FileSystemError> {
        let state = self.state.read().map_err(|_| search_path_lock_error())?;
        Ok(SearchPathSnapshot {
            paths: state.paths.clone(),
            generation: state.generation,
        })
    }

    /// Replaces all search-path entries atomically, omitting MATLAB-special directories.
    ///
    /// # Errors
    ///
    /// Returns without mutation if any directory cannot be resolved.
    pub fn replace_from<I, P>(
        &self,
        paths: I,
        relative_to: Option<&Path>,
    ) -> Result<SearchPathSnapshot, FileSystemError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let resolved = resolve_search_directories(paths, relative_to)?;
        self.update(|paths| *paths = resolved)
    }

    /// Adds canonical non-special directories at the beginning or end of the lookup order.
    ///
    /// Existing occurrences are removed first, matching MATLAB's deterministic
    /// path precedence when a directory is added more than once.
    ///
    /// # Errors
    ///
    /// Returns without mutation if any directory cannot be resolved.
    pub fn add_from<I, P>(
        &self,
        paths: I,
        relative_to: Option<&Path>,
        position: SearchPathPosition,
    ) -> Result<SearchPathSnapshot, FileSystemError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let additions = resolve_search_directories(paths, relative_to)?;
        self.update(|paths| {
            paths.retain(|path| !additions.contains(path));
            match position {
                SearchPathPosition::Begin => {
                    let mut combined = additions;
                    combined.append(paths);
                    *paths = combined;
                }
                SearchPathPosition::End => paths.extend(additions),
            }
        })
    }

    /// Removes canonical directories from the lookup order.
    ///
    /// Supplied directories are normalized relative to the session directory;
    /// each must still exist so aliases can be removed by canonical identity.
    ///
    /// # Errors
    ///
    /// Returns without mutation if a supplied path cannot be resolved.
    pub fn remove_from<I, P>(
        &self,
        paths: I,
        relative_to: Option<&Path>,
    ) -> Result<SearchPathSnapshot, FileSystemError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let removals = resolve_search_directories(paths, relative_to)?;
        self.update(|paths| paths.retain(|path| !removals.contains(path)))
    }

    /// Recursively expands one root using MATLAB `genpath` exclusions.
    ///
    /// Directories named `private` and directories beginning with `@` or `+`
    /// are not emitted or traversed.
    ///
    /// # Errors
    ///
    /// Returns a structured filesystem error when the root or a descendant
    /// cannot be resolved or enumerated.
    pub fn generate_from(
        root: impl AsRef<Path>,
        relative_to: Option<&Path>,
    ) -> Result<Vec<PathBuf>, FileSystemError> {
        generate_search_directories(root.as_ref(), relative_to)
    }

    fn update(
        &self,
        mutate: impl FnOnce(&mut Vec<PathBuf>),
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        let mut state = self.state.write().map_err(|_| search_path_lock_error())?;
        let previous = state.paths.clone();
        mutate(&mut state.paths);
        if state.paths != previous {
            state.generation = state.generation.checked_add(1).ok_or_else(|| {
                FileSystemError::new(
                    FileSystemErrorCategory::Io,
                    "",
                    "the session search-path generation was exhausted",
                )
            })?;
        }
        Ok(SearchPathSnapshot {
            paths: state.paths.clone(),
            generation: state.generation,
        })
    }
}

impl Default for SearchPath {
    fn default() -> Self {
        Self::empty()
    }
}

/// A UTF-8 MATLAB source unit resolved through session path precedence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMatlabSource {
    /// Canonical source-file path used for diagnostics and caller-relative lookup.
    pub path: PathBuf,
    /// Complete UTF-8 source text.
    pub source: String,
    /// Search-path generation that selected this file.
    pub search_path_generation: u64,
}

/// Shared MATLAB source-name resolver for ordinary, caller-private, and package functions.
///
/// This type owns no process-global state. Every lookup snapshots the supplied
/// session search path and accepts the session current directory explicitly.
#[derive(Clone, Debug, Default)]
pub struct MatlabSourceResolver {
    search_path: SearchPath,
}

impl MatlabSourceResolver {
    /// Creates a resolver over shared session search-path state.
    #[must_use]
    pub const fn from_search_path(search_path: SearchPath) -> Self {
        Self { search_path }
    }

    /// Returns the shared search-path handle used by this resolver.
    #[must_use]
    pub fn search_path_handle(&self) -> SearchPath {
        self.search_path.clone()
    }

    /// Resolves one ordinary or package-qualified `.m` source using MATLAB
    /// caller visibility and path order.
    ///
    /// When `caller` is present, `<caller-dir>/private/name.m` has priority over
    /// an ordinary sibling. A caller already inside `private` can see sibling
    /// private functions. The current folder and configured path never expose a
    /// private directory without an eligible caller.
    ///
    /// # Errors
    ///
    /// Returns a structured filesystem error for an invalid unit name, invalid
    /// directory, escaping candidate, non-file candidate, unreadable source, or
    /// non-UTF-8 contents.
    pub fn resolve(
        &self,
        working_directory: Option<&Path>,
        caller: Option<&Path>,
        name: &str,
    ) -> Result<Option<ResolvedMatlabSource>, FileSystemError> {
        let components = matlab_unit_components(name)?;
        let snapshot = self.search_path.snapshot()?;
        let current = working_directory
            .map(|path| resolve_directory(path, None))
            .transpose()?;
        let source_path = matlab_source_relative_path(&components);
        let class_source_path = matlab_class_source_relative_path(&components);
        let mut candidates =
            Vec::with_capacity(snapshot.paths.len().saturating_mul(2).saturating_add(8));

        if components.len() > 1 {
            if let Some(caller_root) = caller
                .filter(|path| path.is_absolute())
                .and_then(package_lookup_root_for_caller)
            {
                push_source_candidate(&mut candidates, &caller_root, &source_path);
                push_source_candidate(&mut candidates, &caller_root, &class_source_path);
            }
        } else if let Some(caller) = caller.filter(|path| path.is_absolute())
            && let Some(parent) = caller.parent()
        {
            if is_private_directory(parent) {
                push_source_candidate(&mut candidates, parent, &source_path);
            } else {
                push_source_candidate(&mut candidates, &parent.join("private"), &source_path);
                if !is_package_directory(parent) {
                    push_source_candidate(&mut candidates, parent, &source_path);
                    push_source_candidate(&mut candidates, parent, &class_source_path);
                }
            }
        }
        if let Some(current) = &current {
            push_source_candidate(&mut candidates, current, &source_path);
            push_source_candidate(&mut candidates, current, &class_source_path);
        }
        for root in &snapshot.paths {
            if is_matlab_special_search_directory(root) {
                continue;
            }
            push_source_candidate(&mut candidates, root, &source_path);
            push_source_candidate(&mut candidates, root, &class_source_path);
        }

        for (candidate, root) in candidates {
            match fs::symlink_metadata(&candidate) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(map_io_error(&candidate, "inspect MATLAB source", &error));
                }
            }
            let canonical = fs::canonicalize(&candidate)
                .map(displayable_canonical_path)
                .map_err(|error| map_io_error(&candidate, "resolve MATLAB source", &error))?;
            let containment_root = fs::canonicalize(&root)
                .map(displayable_canonical_path)
                .map_err(|error| map_io_error(&root, "resolve MATLAB source root", &error))?;
            if !canonical.starts_with(&containment_root) {
                return Err(FileSystemError::new(
                    FileSystemErrorCategory::InvalidPath,
                    canonical.to_string_lossy(),
                    format!(
                        "MATLAB source `{}` escapes search directory `{}`",
                        canonical.to_string_lossy(),
                        containment_root.to_string_lossy()
                    ),
                ));
            }
            let metadata = fs::metadata(&canonical)
                .map_err(|error| map_io_error(&canonical, "inspect MATLAB source", &error))?;
            if !metadata.is_file() {
                return Err(FileSystemError::new(
                    FileSystemErrorCategory::NotFile,
                    canonical.to_string_lossy(),
                    format!(
                        "MATLAB source candidate `{}` is not a regular file",
                        canonical.to_string_lossy()
                    ),
                ));
            }
            let bytes = fs::read(&canonical)
                .map_err(|error| map_io_error(&canonical, "read MATLAB source", &error))?;
            let source = String::from_utf8(bytes).map_err(|_| {
                FileSystemError::new(
                    FileSystemErrorCategory::InvalidEncoding,
                    canonical.to_string_lossy(),
                    format!(
                        "MATLAB source `{}` is not valid UTF-8",
                        canonical.to_string_lossy()
                    ),
                )
            })?;
            return Ok(Some(ResolvedMatlabSource {
                path: canonical,
                source,
                search_path_generation: snapshot.generation,
            }));
        }
        Ok(crate::ui_sources::resolve(name, snapshot.generation))
    }
}

/// Default upper bound for one language-level file transfer.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Stable category for a host filesystem failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSystemErrorCategory {
    /// The supplied path was empty or otherwise invalid.
    InvalidPath,
    /// A text file does not use the required encoding.
    InvalidEncoding,
    /// The selected path does not exist.
    NotFound,
    /// The selected path is not a regular file.
    NotFile,
    /// The selected path is not a directory.
    NotDirectory,
    /// The host denied the requested operation.
    PermissionDenied,
    /// The file exceeds the configured transfer boundary.
    TooLarge,
    /// Another host I/O failure occurred.
    Io,
}

/// Structured host filesystem failure exposed to language built-ins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSystemError {
    /// Stable failure category.
    pub category: FileSystemErrorCategory,
    /// User-supplied or resolved path associated with the failure.
    pub path: String,
    /// OpenMat-owned diagnostic text.
    pub message: String,
}

/// Host metadata normalized for language-level filesystem built-ins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSystemMetadata {
    /// Absolute display path without a Windows verbatim-path prefix.
    pub path: PathBuf,
    /// Final path component as displayed by the host.
    pub name: String,
    /// File length in bytes, or zero for directories.
    pub bytes: u64,
    /// Whether the path identifies a regular file.
    pub is_file: bool,
    /// Whether the path identifies a directory.
    pub is_directory: bool,
    /// Last modification time when supplied by the host.
    pub modified: Option<SystemTime>,
}

/// Access and creation semantics for one session-owned stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileOpenAccess {
    /// Read an existing file.
    Read,
    /// Create or truncate a file for writing.
    Write,
    /// Create a file or append each write to its end.
    Append,
    /// Read and write an existing file.
    ReadUpdate,
    /// Create or truncate a file for reading and writing.
    WriteUpdate,
    /// Read a file while appending each write to its end.
    AppendUpdate,
}

/// Complete host-independent mode for one session-owned stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileOpenMode {
    /// Access and creation semantics.
    pub access: FileOpenAccess,
    /// Whether the language-level mode explicitly selected text translation.
    pub text: bool,
}

impl FileOpenMode {
    /// Returns the MATLAB-compatible canonical permission string.
    #[must_use]
    pub const fn permission(self) -> &'static str {
        match (self.access, self.text) {
            (FileOpenAccess::Read, false) => "rb",
            (FileOpenAccess::Write, false) => "wb",
            (FileOpenAccess::Append, false) => "ab",
            (FileOpenAccess::ReadUpdate, false) => "rb+",
            (FileOpenAccess::WriteUpdate, false) => "wb+",
            (FileOpenAccess::AppendUpdate, false) => "ab+",
            (FileOpenAccess::Read, true) => "rt",
            (FileOpenAccess::Write, true) => "wt",
            (FileOpenAccess::Append, true) => "at",
            (FileOpenAccess::ReadUpdate, true) => "rt+",
            (FileOpenAccess::WriteUpdate, true) => "wt+",
            (FileOpenAccess::AppendUpdate, true) => "at+",
        }
    }
}

/// Origin used by a session stream seek operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSeekOrigin {
    /// Beginning of the file.
    Start,
    /// Current stream position.
    Current,
    /// End of the file.
    End,
}

/// Language-visible metadata retained for one open stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenFileInfo {
    /// Session-local numeric identifier.
    pub identifier: u32,
    /// Filename exactly as supplied by the language caller.
    pub filename: String,
    /// Resolved absolute file path.
    pub path: PathBuf,
    /// Canonical permission string.
    pub permission: String,
    /// Canonical machine-format name used by binary conversions.
    pub machine_format: String,
    /// Encoding label retained for text consumers.
    pub encoding: String,
}

impl FileSystemError {
    fn new(
        category: FileSystemErrorCategory,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for FileSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for FileSystemError {}

/// Narrow, replaceable host boundary used by language-level file functions.
pub trait FileSystemService: Send {
    /// Returns the session working directory used for relative paths.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the shared session state cannot be read.
    fn working_directory(&self) -> Result<PathBuf, FileSystemError>;

    /// Changes the session working directory without changing process-global state.
    ///
    /// # Errors
    ///
    /// Returns a structured path, directory, permission, or host I/O error.
    fn change_working_directory(&mut self, path: &str) -> Result<PathBuf, FileSystemError>;

    /// Returns the session MATLAB search path.
    ///
    /// The default keeps custom test and embedding filesystems source compatible
    /// while indicating that no host search path is configured.
    ///
    /// # Errors
    ///
    /// Implementations may report host state or synchronization failures.
    fn search_path(&self) -> Result<SearchPathSnapshot, FileSystemError> {
        Ok(SearchPathSnapshot {
            paths: Vec::new(),
            generation: 0,
        })
    }

    /// Replaces the complete session MATLAB search path.
    ///
    /// # Errors
    ///
    /// The default reports that this optional host service is unavailable.
    fn replace_search_path(
        &mut self,
        _paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        Err(search_path_unavailable())
    }

    /// Adds directories to the session MATLAB search path.
    ///
    /// # Errors
    ///
    /// The default reports that this optional host service is unavailable.
    fn add_search_path(
        &mut self,
        _paths: &[String],
        _position: SearchPathPosition,
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        Err(search_path_unavailable())
    }

    /// Removes directories from the session MATLAB search path.
    ///
    /// # Errors
    ///
    /// The default reports that this optional host service is unavailable.
    fn remove_search_path(
        &mut self,
        _paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        Err(search_path_unavailable())
    }

    /// Enumerates the recursive `genpath` expansion of one directory.
    ///
    /// # Errors
    ///
    /// The default reports that this optional host service is unavailable.
    fn generate_search_path(&mut self, _root: &str) -> Result<Vec<PathBuf>, FileSystemError> {
        Err(search_path_unavailable())
    }

    /// Resolves one `name.m` file through caller, current-folder, and search-path precedence.
    ///
    /// # Errors
    ///
    /// Returns a structured filesystem error for invalid candidates or unreadable UTF-8.
    fn resolve_matlab_source(
        &mut self,
        _caller: Option<&str>,
        _name: &str,
    ) -> Result<Option<ResolvedMatlabSource>, FileSystemError> {
        Ok(None)
    }

    /// Reads one regular file without exceeding `maximum_bytes`.
    ///
    /// # Errors
    ///
    /// Returns a structured path, host I/O, or transfer-limit failure.
    fn read_file(&mut self, path: &str, maximum_bytes: u64) -> Result<Vec<u8>, FileSystemError>;

    /// Creates or replaces one regular file.
    ///
    /// # Errors
    ///
    /// Returns a structured path or host I/O failure.
    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), FileSystemError>;

    /// Inspects one exact file or directory.
    ///
    /// # Errors
    ///
    /// Returns a structured path, permission, not-found, or host I/O error.
    fn metadata(&mut self, path: &str) -> Result<FileSystemMetadata, FileSystemError>;

    /// Expands one exact path or a final-component `*`/`?` pattern.
    ///
    /// An exact directory returns its children including synthetic `.` and
    /// `..` entries. An exact file returns one entry. A missing wildcard
    /// returns an empty list.
    ///
    /// # Errors
    ///
    /// Returns a structured error for invalid paths, inaccessible parents, or
    /// wildcards before the final path component.
    fn directory_entries(&mut self, path: &str)
    -> Result<Vec<FileSystemMetadata>, FileSystemError>;

    /// Creates a directory and any missing parents.
    ///
    /// # Errors
    ///
    /// Returns a structured path or host I/O error.
    fn create_directory(&mut self, path: &str) -> Result<(), FileSystemError>;

    /// Removes one directory, optionally including its descendants.
    ///
    /// # Errors
    ///
    /// Returns a structured path or host I/O error.
    fn remove_directory(&mut self, path: &str, recursive: bool) -> Result<(), FileSystemError>;

    /// Removes one regular file.
    ///
    /// # Errors
    ///
    /// Returns a structured path, file-kind, or host I/O error.
    fn remove_file(&mut self, path: &str) -> Result<(), FileSystemError>;

    /// Copies one regular file or directory tree.
    ///
    /// # Errors
    ///
    /// Returns a structured source, destination, or host I/O error.
    fn copy_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError>;

    /// Moves one regular file or directory tree.
    ///
    /// # Errors
    ///
    /// Returns a structured source, destination, or host I/O error.
    fn move_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError>;

    /// Opens one session-owned stream and returns its numeric identifier.
    ///
    /// # Errors
    ///
    /// Returns a structured path, permission, capacity, or host I/O failure.
    fn open_file(
        &mut self,
        path: &str,
        mode: FileOpenMode,
        machine_format: &str,
        encoding: &str,
    ) -> Result<u32, FileSystemError>;

    /// Closes one stream, returning `false` when the identifier is not open.
    fn close_open_file(&mut self, identifier: u32) -> bool;

    /// Closes every stream owned by this service.
    fn close_all_open_files(&mut self);

    /// Returns all currently open numeric identifiers in ascending order.
    fn open_file_identifiers(&self) -> Vec<u32>;

    /// Returns retained metadata for one stream, or `None` for an invalid identifier.
    fn open_file_info(&self, identifier: u32) -> Option<OpenFileInfo>;

    /// Reads at most `maximum_bytes` at the current position.
    ///
    /// # Errors
    ///
    /// Returns a structured allocation or host I/O failure.
    fn read_open_file(
        &mut self,
        identifier: u32,
        maximum_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FileSystemError>;

    /// Writes bytes at the current position.
    ///
    /// # Errors
    ///
    /// Returns a structured host I/O failure.
    fn write_open_file(
        &mut self,
        identifier: u32,
        contents: &[u8],
    ) -> Result<Option<usize>, FileSystemError>;

    /// Repositions one stream and returns the resulting byte position.
    ///
    /// # Errors
    ///
    /// Returns a structured range or host I/O failure.
    fn seek_open_file(
        &mut self,
        identifier: u32,
        offset: i64,
        origin: FileSeekOrigin,
    ) -> Result<Option<u64>, FileSystemError>;

    /// Returns the current byte position of one stream.
    ///
    /// # Errors
    ///
    /// Returns a structured host I/O failure.
    fn tell_open_file(&mut self, identifier: u32) -> Result<Option<u64>, FileSystemError>;
}

/// Shared, session-local current-directory state.
#[derive(Clone, Debug)]
pub struct WorkingDirectory {
    state: Arc<RwLock<WorkingDirectoryState>>,
}

#[derive(Clone, Debug)]
struct WorkingDirectoryState {
    path: PathBuf,
    generation: u64,
}

/// Immutable view of one current-directory generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingDirectorySnapshot {
    /// Canonical absolute path used for host filesystem operations.
    pub path: PathBuf,
    /// Monotonic identity changed after every successful directory switch.
    pub generation: u64,
}

impl WorkingDirectory {
    /// Creates shared session state after validating the initial directory.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the path is empty, missing, inaccessible,
    /// or not a directory.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, FileSystemError> {
        let path = resolve_directory(path.as_ref(), None)?;
        Ok(Self {
            state: Arc::new(RwLock::new(WorkingDirectoryState {
                path,
                generation: 1,
            })),
        })
    }

    /// Returns an owned, internally consistent state snapshot.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the shared state lock is unavailable.
    pub fn snapshot(&self) -> Result<WorkingDirectorySnapshot, FileSystemError> {
        let state = self.state.read().map_err(|_| lock_error())?;
        Ok(WorkingDirectorySnapshot {
            path: state.path.clone(),
            generation: state.generation,
        })
    }

    /// Resolves and atomically selects a new absolute or session-relative directory.
    ///
    /// # Errors
    ///
    /// Returns a structured path, directory, permission, state, or host I/O error
    /// without changing the previous successful snapshot.
    pub fn change(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<WorkingDirectorySnapshot, FileSystemError> {
        let supplied = path.as_ref();
        let mut state = self.state.write().map_err(|_| lock_error())?;
        let resolved = resolve_directory(supplied, Some(&state.path))?;
        if state.path != resolved {
            let generation = state.generation.checked_add(1).ok_or_else(|| {
                FileSystemError::new(
                    FileSystemErrorCategory::Io,
                    state.path.to_string_lossy(),
                    "the session working-directory generation was exhausted",
                )
            })?;
            state.path = resolved;
            state.generation = generation;
        }
        Ok(WorkingDirectorySnapshot {
            path: state.path.clone(),
            generation: state.generation,
        })
    }
}

/// Native filesystem service rooted at a shared session working directory.
#[derive(Debug)]
pub struct LocalFileSystem {
    working_directory: WorkingDirectory,
    search_path: SearchPath,
    open_files: BTreeMap<u32, OpenFile>,
}

#[derive(Debug)]
struct OpenFile {
    file: fs::File,
    info: OpenFileInfo,
}

impl Clone for LocalFileSystem {
    fn clone(&self) -> Self {
        Self {
            working_directory: self.working_directory.clone(),
            search_path: self.search_path.clone(),
            open_files: BTreeMap::new(),
        }
    }
}

impl LocalFileSystem {
    /// Creates a service after canonicalizing and validating its working directory.
    ///
    /// # Errors
    ///
    /// Returns a structured filesystem error when the directory cannot be
    /// resolved or is not a directory.
    pub fn new(working_directory: impl AsRef<Path>) -> Result<Self, FileSystemError> {
        WorkingDirectory::new(working_directory).map(Self::from_working_directory)
    }

    /// Creates a filesystem view over host-owned shared directory state.
    #[must_use]
    pub fn from_working_directory(working_directory: WorkingDirectory) -> Self {
        Self {
            working_directory,
            search_path: SearchPath::empty(),
            open_files: BTreeMap::new(),
        }
    }

    /// Creates a filesystem view over shared working-directory and search-path state.
    #[must_use]
    pub fn from_session_paths(
        working_directory: WorkingDirectory,
        search_path: SearchPath,
    ) -> Self {
        Self {
            working_directory,
            search_path,
            open_files: BTreeMap::new(),
        }
    }

    /// Returns the shared directory handle used by this filesystem service.
    #[must_use]
    pub fn working_directory_handle(&self) -> WorkingDirectory {
        self.working_directory.clone()
    }

    /// Returns the shared MATLAB search-path handle used by this service.
    #[must_use]
    pub fn search_path_handle(&self) -> SearchPath {
        self.search_path.clone()
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, FileSystemError> {
        if path.is_empty() || path.contains('\0') {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::InvalidPath,
                path,
                "file path cannot be empty or contain a null character",
            ));
        }
        let path = Path::new(path);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.working_directory.snapshot()?.path.join(path)
        })
    }

    fn next_file_identifier(&self) -> Result<u32, FileSystemError> {
        (3..=u32::MAX)
            .find(|identifier| !self.open_files.contains_key(identifier))
            .ok_or_else(|| {
                FileSystemError::new(
                    FileSystemErrorCategory::Io,
                    "",
                    "the session file-identifier space is exhausted",
                )
            })
    }
}

impl FileSystemService for LocalFileSystem {
    fn working_directory(&self) -> Result<PathBuf, FileSystemError> {
        Ok(self.working_directory.snapshot()?.path)
    }

    fn change_working_directory(&mut self, path: &str) -> Result<PathBuf, FileSystemError> {
        Ok(self.working_directory.change(path)?.path)
    }

    fn search_path(&self) -> Result<SearchPathSnapshot, FileSystemError> {
        self.search_path.snapshot()
    }

    fn replace_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        let current = self.working_directory.snapshot()?.path;
        self.search_path.replace_from(paths, Some(&current))
    }

    fn add_search_path(
        &mut self,
        paths: &[String],
        position: SearchPathPosition,
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        let current = self.working_directory.snapshot()?.path;
        self.search_path.add_from(paths, Some(&current), position)
    }

    fn remove_search_path(
        &mut self,
        paths: &[String],
    ) -> Result<SearchPathSnapshot, FileSystemError> {
        let current = self.working_directory.snapshot()?.path;
        self.search_path.remove_from(paths, Some(&current))
    }

    fn generate_search_path(&mut self, root: &str) -> Result<Vec<PathBuf>, FileSystemError> {
        let current = self.working_directory.snapshot()?.path;
        generate_search_directories(Path::new(root), Some(&current))
    }

    fn resolve_matlab_source(
        &mut self,
        caller: Option<&str>,
        name: &str,
    ) -> Result<Option<ResolvedMatlabSource>, FileSystemError> {
        let current = self.working_directory.snapshot()?.path;
        let caller = caller.map(Path::new).filter(|path| path.is_absolute());
        MatlabSourceResolver::from_search_path(self.search_path.clone()).resolve(
            Some(&current),
            caller,
            name,
        )
    }

    fn read_file(&mut self, path: &str, maximum_bytes: u64) -> Result<Vec<u8>, FileSystemError> {
        let resolved = self.resolve(path)?;
        let metadata = fs::metadata(&resolved)
            .map_err(|error| map_io_error(&resolved, "inspect file", &error))?;
        if !metadata.is_file() {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotFile,
                resolved.to_string_lossy(),
                format!("`{}` is not a regular file", resolved.to_string_lossy()),
            ));
        }
        if metadata.len() > maximum_bytes {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::TooLarge,
                resolved.to_string_lossy(),
                format!(
                    "file `{}` is {} bytes, exceeding the {} byte transfer limit",
                    resolved.to_string_lossy(),
                    metadata.len(),
                    maximum_bytes
                ),
            ));
        }
        let bytes =
            fs::read(&resolved).map_err(|error| map_io_error(&resolved, "read file", &error))?;
        let actual = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if actual > maximum_bytes {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::TooLarge,
                resolved.to_string_lossy(),
                format!(
                    "file `{}` grew to {actual} bytes while reading, exceeding the {maximum_bytes} byte transfer limit",
                    resolved.to_string_lossy()
                ),
            ));
        }
        Ok(bytes)
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), FileSystemError> {
        let resolved = self.resolve(path)?;
        if let Ok(metadata) = fs::metadata(&resolved)
            && !metadata.is_file()
        {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotFile,
                resolved.to_string_lossy(),
                format!("`{}` is not a regular file", resolved.to_string_lossy()),
            ));
        }
        fs::write(&resolved, contents)
            .map_err(|error| map_io_error(&resolved, "write file", &error))
    }

    fn metadata(&mut self, path: &str) -> Result<FileSystemMetadata, FileSystemError> {
        let resolved = self.resolve(path)?;
        metadata_for_path(&resolved)
    }

    fn directory_entries(
        &mut self,
        path: &str,
    ) -> Result<Vec<FileSystemMetadata>, FileSystemError> {
        let resolved = self.resolve(path)?;
        directory_entries(&resolved)
    }

    fn create_directory(&mut self, path: &str) -> Result<(), FileSystemError> {
        let resolved = self.resolve(path)?;
        fs::create_dir_all(&resolved)
            .map_err(|error| map_io_error(&resolved, "create directory", &error))
    }

    fn remove_directory(&mut self, path: &str, recursive: bool) -> Result<(), FileSystemError> {
        let resolved = self.resolve(path)?;
        let metadata = fs::metadata(&resolved)
            .map_err(|error| map_io_error(&resolved, "inspect directory", &error))?;
        if !metadata.is_dir() {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotDirectory,
                resolved.to_string_lossy(),
                format!("`{}` is not a directory", resolved.to_string_lossy()),
            ));
        }
        let result = if recursive {
            fs::remove_dir_all(&resolved)
        } else {
            fs::remove_dir(&resolved)
        };
        result.map_err(|error| map_io_error(&resolved, "remove directory", &error))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), FileSystemError> {
        let resolved = self.resolve(path)?;
        let metadata = fs::metadata(&resolved)
            .map_err(|error| map_io_error(&resolved, "inspect file", &error))?;
        if !metadata.is_file() {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotFile,
                resolved.to_string_lossy(),
                format!("`{}` is not a regular file", resolved.to_string_lossy()),
            ));
        }
        fs::remove_file(&resolved).map_err(|error| map_io_error(&resolved, "remove file", &error))
    }

    fn copy_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError> {
        let source = self.resolve(source)?;
        let destination = self.resolve(destination)?;
        copy_resolved_path(&source, &destination, force)
    }

    fn move_path(
        &mut self,
        source: &str,
        destination: &str,
        force: bool,
    ) -> Result<(), FileSystemError> {
        let source = self.resolve(source)?;
        let destination = self.resolve(destination)?;
        move_resolved_path(&source, &destination, force)
    }

    fn open_file(
        &mut self,
        path: &str,
        mode: FileOpenMode,
        machine_format: &str,
        encoding: &str,
    ) -> Result<u32, FileSystemError> {
        let resolved = self.resolve(path)?;
        let identifier = self.next_file_identifier()?;
        if let Ok(metadata) = fs::metadata(&resolved)
            && !metadata.is_file()
        {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotFile,
                resolved.to_string_lossy(),
                format!("`{}` is not a regular file", resolved.to_string_lossy()),
            ));
        }

        let mut options = fs::OpenOptions::new();
        match mode.access {
            FileOpenAccess::Read => {
                options.read(true);
            }
            FileOpenAccess::Write => {
                options.write(true).create(true).truncate(true);
            }
            FileOpenAccess::Append => {
                options.append(true).create(true);
            }
            FileOpenAccess::ReadUpdate => {
                options.read(true).write(true);
            }
            FileOpenAccess::WriteUpdate => {
                options.read(true).write(true).create(true).truncate(true);
            }
            FileOpenAccess::AppendUpdate => {
                options.read(true).append(true).create(true);
            }
        }
        let file = options
            .open(&resolved)
            .map_err(|error| map_io_error(&resolved, "open file", &error))?;
        let canonical = fs::canonicalize(&resolved)
            .map(displayable_canonical_path)
            .unwrap_or(resolved);
        let info = OpenFileInfo {
            identifier,
            filename: path.to_owned(),
            path: canonical,
            permission: mode.permission().to_owned(),
            machine_format: machine_format.to_owned(),
            encoding: encoding.to_owned(),
        };
        self.open_files.insert(identifier, OpenFile { file, info });
        Ok(identifier)
    }

    fn close_open_file(&mut self, identifier: u32) -> bool {
        self.open_files.remove(&identifier).is_some()
    }

    fn close_all_open_files(&mut self) {
        self.open_files.clear();
    }

    fn open_file_identifiers(&self) -> Vec<u32> {
        self.open_files.keys().copied().collect()
    }

    fn open_file_info(&self, identifier: u32) -> Option<OpenFileInfo> {
        self.open_files
            .get(&identifier)
            .map(|open| open.info.clone())
    }

    fn read_open_file(
        &mut self,
        identifier: u32,
        maximum_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FileSystemError> {
        let Some(open) = self.open_files.get_mut(&identifier) else {
            return Ok(None);
        };
        let path = open.info.path.clone();
        let mut contents = Vec::new();
        contents
            .try_reserve(maximum_bytes.min(64 * 1024))
            .map_err(|_| {
                FileSystemError::new(
                    FileSystemErrorCategory::TooLarge,
                    path.to_string_lossy(),
                    format!("cannot reserve a {maximum_bytes} byte stream read buffer"),
                )
            })?;
        Read::by_ref(&mut open.file)
            .take(u64::try_from(maximum_bytes).unwrap_or(u64::MAX))
            .read_to_end(&mut contents)
            .map_err(|error| map_io_error(&path, "read open file", &error))?;
        Ok(Some(contents))
    }

    fn write_open_file(
        &mut self,
        identifier: u32,
        contents: &[u8],
    ) -> Result<Option<usize>, FileSystemError> {
        let Some(open) = self.open_files.get_mut(&identifier) else {
            return Ok(None);
        };
        let path = open.info.path.clone();
        open.file
            .write_all(contents)
            .map_err(|error| map_io_error(&path, "write open file", &error))?;
        Ok(Some(contents.len()))
    }

    fn seek_open_file(
        &mut self,
        identifier: u32,
        offset: i64,
        origin: FileSeekOrigin,
    ) -> Result<Option<u64>, FileSystemError> {
        let Some(open) = self.open_files.get_mut(&identifier) else {
            return Ok(None);
        };
        let path = open.info.path.clone();
        let position = match origin {
            FileSeekOrigin::Start => u64::try_from(offset).map(SeekFrom::Start).map_err(|_| {
                FileSystemError::new(
                    FileSystemErrorCategory::Io,
                    path.to_string_lossy(),
                    "cannot seek before the beginning of a file",
                )
            })?,
            FileSeekOrigin::Current => SeekFrom::Current(offset),
            FileSeekOrigin::End => SeekFrom::End(offset),
        };
        open.file
            .seek(position)
            .map(Some)
            .map_err(|error| map_io_error(&path, "seek open file", &error))
    }

    fn tell_open_file(&mut self, identifier: u32) -> Result<Option<u64>, FileSystemError> {
        let Some(open) = self.open_files.get_mut(&identifier) else {
            return Ok(None);
        };
        let path = open.info.path.clone();
        open.file
            .stream_position()
            .map(Some)
            .map_err(|error| map_io_error(&path, "query open file position", &error))
    }
}

fn metadata_for_path(path: &Path) -> Result<FileSystemMetadata, FileSystemError> {
    let metadata =
        fs::metadata(path).map_err(|error| map_io_error(path, "inspect path", &error))?;
    Ok(metadata_value(path, &metadata))
}

fn metadata_value(path: &Path, metadata: &fs::Metadata) -> FileSystemMetadata {
    FileSystemMetadata {
        path: displayable_canonical_path(path.to_path_buf()),
        name: path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        bytes: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        is_file: metadata.is_file(),
        is_directory: metadata.is_dir(),
        modified: metadata.modified().ok(),
    }
}

fn directory_entries(path: &Path) -> Result<Vec<FileSystemMetadata>, FileSystemError> {
    let wildcard = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| contains_wildcard(name));
    if let Some(pattern) = wildcard {
        let supplied_parent = path.parent().unwrap_or_else(|| Path::new("."));
        if supplied_parent
            .components()
            .any(|component| contains_wildcard(&component.as_os_str().to_string_lossy()))
        {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::InvalidPath,
                path.to_string_lossy(),
                "wildcards before the final path component are not supported",
            ));
        }
        let parent = fs::canonicalize(supplied_parent)
            .map_err(|error| map_io_error(supplied_parent, "resolve wildcard directory", &error))?;
        let mut entries = Vec::new();
        if wildcard_matches(pattern, ".") {
            entries.push(synthetic_directory_entry(&parent, &parent.join("."), ".")?);
        }
        if wildcard_matches(pattern, "..")
            && let Some(grandparent) = parent.parent()
        {
            entries.push(synthetic_directory_entry(
                grandparent,
                &parent.join(".."),
                "..",
            )?);
        }
        for entry in fs::read_dir(&parent)
            .map_err(|error| map_io_error(&parent, "read directory", &error))?
        {
            let entry = entry.map_err(|error| map_io_error(&parent, "read directory", &error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if wildcard_matches(pattern, &name) {
                let metadata = entry
                    .metadata()
                    .map_err(|error| map_io_error(&entry.path(), "inspect path", &error))?;
                entries.push(metadata_value(&entry.path(), &metadata));
            }
        }
        sort_entries(&mut entries);
        return Ok(entries);
    }

    let metadata =
        fs::metadata(path).map_err(|error| map_io_error(path, "inspect path", &error))?;
    let path =
        fs::canonicalize(path).map_err(|error| map_io_error(path, "resolve path", &error))?;
    if metadata.is_file() {
        return Ok(vec![metadata_value(&path, &metadata)]);
    }
    if !metadata.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    entries.push(synthetic_directory_entry(&path, &path.join("."), ".")?);
    if let Some(parent) = path.parent() {
        entries.push(synthetic_directory_entry(parent, &path.join(".."), "..")?);
    }
    for entry in
        fs::read_dir(&path).map_err(|error| map_io_error(&path, "read directory", &error))?
    {
        let entry = entry.map_err(|error| map_io_error(&path, "read directory", &error))?;
        let metadata = entry
            .metadata()
            .map_err(|error| map_io_error(&entry.path(), "inspect path", &error))?;
        entries.push(metadata_value(&entry.path(), &metadata));
    }
    let synthetic = entries.drain(..entries.len().min(2)).collect::<Vec<_>>();
    sort_entries(&mut entries);
    entries.splice(0..0, synthetic);
    Ok(entries)
}

fn synthetic_directory_entry(
    target: &Path,
    display_path: &Path,
    name: &str,
) -> Result<FileSystemMetadata, FileSystemError> {
    let metadata =
        fs::metadata(target).map_err(|error| map_io_error(target, "inspect directory", &error))?;
    let mut value = metadata_value(display_path, &metadata);
    name.clone_into(&mut value.name);
    Ok(value)
}

fn sort_entries(entries: &mut [FileSystemMetadata]) {
    entries
        .sort_by(|left, right| platform_name_key(&left.name).cmp(&platform_name_key(&right.name)));
}

#[cfg(windows)]
fn platform_name_key(name: &str) -> String {
    name.to_lowercase()
}

#[cfg(not(windows))]
fn platform_name_key(name: &str) -> String {
    name.to_owned()
}

fn contains_wildcard(value: &str) -> bool {
    value.contains(['*', '?'])
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    #[cfg(windows)]
    let (pattern, value) = (
        if pattern == "*.*" {
            "*".to_owned()
        } else {
            pattern.to_lowercase()
        },
        value.to_lowercase(),
    );
    #[cfg(not(windows))]
    let (pattern, value) = (pattern.to_owned(), value.to_owned());
    wildcard_chars(
        &pattern.chars().collect::<Vec<_>>(),
        &value.chars().collect::<Vec<_>>(),
    )
}

fn wildcard_chars(pattern: &[char], value: &[char]) -> bool {
    match pattern.split_first() {
        None => value.is_empty(),
        Some((&'*', rest)) => {
            wildcard_chars(rest, value)
                || value
                    .split_first()
                    .is_some_and(|(_, tail)| wildcard_chars(pattern, tail))
        }
        Some((&'?', rest)) => value
            .split_first()
            .is_some_and(|(_, tail)| wildcard_chars(rest, tail)),
        Some((&head, rest)) => value
            .split_first()
            .is_some_and(|(&candidate, tail)| head == candidate && wildcard_chars(rest, tail)),
    }
}

fn copy_resolved_path(
    source: &Path,
    destination: &Path,
    force: bool,
) -> Result<(), FileSystemError> {
    let metadata = fs::metadata(source)
        .map_err(|error| map_io_error(source, "inspect copy source", &error))?;
    let destination = effective_destination(source, destination);
    if metadata.is_file() {
        prepare_file_destination(&destination, force)?;
        fs::copy(source, &destination)
            .map(|_| ())
            .map_err(|error| map_io_error(&destination, "copy file", &error))
    } else if metadata.is_dir() {
        reject_copy_into_source(source, &destination)?;
        copy_directory(source, &destination, force)
    } else {
        Err(FileSystemError::new(
            FileSystemErrorCategory::NotFile,
            source.to_string_lossy(),
            format!("`{}` is not a file or directory", source.to_string_lossy()),
        ))
    }
}

fn reject_copy_into_source(source: &Path, destination: &Path) -> Result<(), FileSystemError> {
    let source = fs::canonicalize(source)
        .map_err(|error| map_io_error(source, "resolve copy source", &error))?;
    let destination = if destination.exists() {
        fs::canonicalize(destination)
            .map_err(|error| map_io_error(destination, "resolve copy destination", &error))?
    } else {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent)
            .map_err(|error| map_io_error(parent, "resolve copy destination", &error))?;
        parent.join(destination.file_name().unwrap_or_default())
    };
    if destination.starts_with(&source) {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::InvalidPath,
            destination.to_string_lossy(),
            format!(
                "copy destination `{}` is inside source directory `{}`",
                destination.to_string_lossy(),
                source.to_string_lossy()
            ),
        ));
    }
    Ok(())
}

fn effective_destination(source: &Path, destination: &Path) -> PathBuf {
    if destination.is_dir() {
        source
            .file_name()
            .map_or_else(|| destination.to_path_buf(), |name| destination.join(name))
    } else {
        destination.to_path_buf()
    }
}

fn prepare_file_destination(path: &Path, force: bool) -> Result<(), FileSystemError> {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.is_dir() {
            return Err(FileSystemError::new(
                FileSystemErrorCategory::NotFile,
                path.to_string_lossy(),
                format!(
                    "copy destination `{}` is a directory",
                    path.to_string_lossy()
                ),
            ));
        }
        if force && metadata.permissions().readonly() {
            set_writable(path, metadata.permissions())?;
        }
    }
    if let Some(parent) = path.parent()
        && !parent.is_dir()
    {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::NotFound,
            parent.to_string_lossy(),
            format!(
                "destination directory `{}` does not exist",
                parent.to_string_lossy()
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)] // Windows toggles one file attribute, not Unix mode bits.
fn set_writable(path: &Path, mut permissions: fs::Permissions) -> Result<(), FileSystemError> {
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
        .map_err(|error| map_io_error(path, "make destination writable", &error))
}

#[cfg(not(windows))]
fn set_writable(_path: &Path, _permissions: fs::Permissions) -> Result<(), FileSystemError> {
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path, force: bool) -> Result<(), FileSystemError> {
    fs::create_dir_all(destination)
        .map_err(|error| map_io_error(destination, "create copy destination", &error))?;
    for entry in
        fs::read_dir(source).map_err(|error| map_io_error(source, "read copy source", &error))?
    {
        let entry = entry.map_err(|error| map_io_error(source, "read copy source", &error))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| map_io_error(&source_path, "inspect copy source", &error))?;
        if metadata.is_dir() {
            copy_directory(&source_path, &destination_path, force)?;
        } else if metadata.is_file() {
            prepare_file_destination(&destination_path, force)?;
            fs::copy(&source_path, &destination_path)
                .map_err(|error| map_io_error(&destination_path, "copy file", &error))?;
        }
    }
    Ok(())
}

fn move_resolved_path(
    source: &Path,
    destination: &Path,
    force: bool,
) -> Result<(), FileSystemError> {
    let metadata = fs::metadata(source)
        .map_err(|error| map_io_error(source, "inspect move source", &error))?;
    let destination = effective_destination(source, destination);
    if metadata.is_file() {
        prepare_file_destination(&destination, force)?;
        if destination.exists() {
            fs::remove_file(&destination)
                .map_err(|error| map_io_error(&destination, "replace move destination", &error))?;
        }
    } else if destination.exists() {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::Io,
            destination.to_string_lossy(),
            format!(
                "move destination `{}` already exists",
                destination.to_string_lossy()
            ),
        ));
    }
    match fs::rename(source, &destination) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            copy_resolved_path(source, &destination, force)?;
            let removal = if metadata.is_dir() {
                fs::remove_dir_all(source)
            } else {
                fs::remove_file(source)
            };
            removal.map_err(|error| {
                FileSystemError::new(
                    FileSystemErrorCategory::Io,
                    source.to_string_lossy(),
                    format!(
                        "copied move source but could not remove `{}` after rename failed ({rename_error}): {error}",
                        source.to_string_lossy()
                    ),
                )
            })
        }
    }
}

fn resolve_directory(path: &Path, relative_to: Option<&Path>) -> Result<PathBuf, FileSystemError> {
    if path.as_os_str().is_empty() {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::InvalidPath,
            "",
            "the session working directory cannot be empty",
        ));
    }
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = relative_to {
        base.join(path)
    } else {
        std::env::current_dir()
            .map_err(|error| map_io_error(path, "resolve process working directory", &error))?
            .join(path)
    };
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| map_io_error(&candidate, "resolve working directory", &error))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| map_io_error(&canonical, "inspect working directory", &error))?;
    if !metadata.is_dir() {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::NotDirectory,
            canonical.to_string_lossy(),
            format!(
                "session working directory `{}` is not a directory",
                canonical.to_string_lossy()
            ),
        ));
    }
    Ok(displayable_canonical_path(canonical))
}

fn resolve_search_directories<I, P>(
    paths: I,
    relative_to: Option<&Path>,
) -> Result<Vec<PathBuf>, FileSystemError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut resolved = Vec::new();
    for path in paths {
        let path = resolve_directory(path.as_ref(), relative_to)?;
        if is_matlab_special_search_directory(&path) {
            continue;
        }
        if !resolved.contains(&path) {
            resolved.push(path);
        }
    }
    Ok(resolved)
}

fn generate_search_directories(
    root: &Path,
    relative_to: Option<&Path>,
) -> Result<Vec<PathBuf>, FileSystemError> {
    let root = resolve_directory(root, relative_to)?;
    let mut generated = Vec::new();
    let mut visited = BTreeSet::new();
    collect_search_directories(&root, &mut generated, &mut visited)?;
    Ok(generated)
}

fn push_source_candidate(candidates: &mut Vec<(PathBuf, PathBuf)>, root: &Path, source: &Path) {
    let candidate = root.join(source);
    if !candidates
        .iter()
        .any(|(existing, _)| existing == &candidate)
    {
        candidates.push((candidate, root.to_path_buf()));
    }
}

fn is_private_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case("private")
            } else {
                name == "private"
            }
        })
}

fn is_package_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| name.starts_with('+'))
}

fn is_matlab_special_search_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| {
            (if cfg!(windows) {
                name.eq_ignore_ascii_case("private")
            } else {
                name == "private"
            }) || name.starts_with(['@', '+'])
        })
}

fn package_lookup_root_for_caller(caller: &Path) -> Option<PathBuf> {
    let mut directory = caller.parent()?;
    let mut found_package = false;
    while is_package_directory(directory) {
        found_package = true;
        directory = directory.parent()?;
    }
    found_package.then(|| directory.to_path_buf()).or_else(|| {
        caller
            .parent()
            .filter(|parent| !is_private_directory(parent))
            .map(Path::to_path_buf)
    })
}

fn collect_search_directories(
    directory: &Path,
    generated: &mut Vec<PathBuf>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<(), FileSystemError> {
    let canonical = fs::canonicalize(directory)
        .map(displayable_canonical_path)
        .map_err(|error| map_io_error(directory, "resolve search-path directory", &error))?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    generated.push(canonical.clone());
    let mut children = Vec::new();
    for entry in fs::read_dir(&canonical)
        .map_err(|error| map_io_error(&canonical, "enumerate search-path directory", &error))?
    {
        let entry = entry
            .map_err(|error| map_io_error(&canonical, "enumerate search-path directory", &error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| map_io_error(&entry.path(), "inspect search-path entry", &error))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "private" || name.starts_with(['@', '+']) {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        collect_search_directories(&child, generated, visited)?;
    }
    Ok(())
}

fn matlab_unit_components(name: &str) -> Result<Vec<&str>, FileSystemError> {
    let components = name.split('.').collect::<Vec<_>>();
    let valid = !name.is_empty()
        && !name.contains(['/', '\\', ':', '\0'])
        && components.iter().all(|component| {
            !component.is_empty()
                && *component != "."
                && *component != ".."
                && matches!(
                    Path::new(component).components().next(),
                    Some(std::path::Component::Normal(_))
                )
        });
    if !valid {
        return Err(FileSystemError::new(
            FileSystemErrorCategory::InvalidPath,
            name,
            format!("`{name}` is not a valid MATLAB source-unit name"),
        ));
    }
    Ok(components)
}

fn matlab_source_relative_path(components: &[&str]) -> PathBuf {
    let mut path = PathBuf::new();
    for package in &components[..components.len().saturating_sub(1)] {
        path.push(format!("+{package}"));
    }
    path.push(format!(
        "{}.m",
        components.last().copied().unwrap_or_default()
    ));
    path
}

fn matlab_class_source_relative_path(components: &[&str]) -> PathBuf {
    let mut path = PathBuf::new();
    for package in &components[..components.len().saturating_sub(1)] {
        path.push(format!("+{package}"));
    }
    let class = components.last().copied().unwrap_or_default();
    path.push(format!("@{class}"));
    path.push(format!("{class}.m"));
    path
}

#[cfg(windows)]
fn displayable_canonical_path(path: PathBuf) -> PathBuf {
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
const fn displayable_canonical_path(path: PathBuf) -> PathBuf {
    path
}

fn lock_error() -> FileSystemError {
    FileSystemError::new(
        FileSystemErrorCategory::Io,
        "",
        "the session working-directory state is unavailable",
    )
}

fn search_path_lock_error() -> FileSystemError {
    FileSystemError::new(
        FileSystemErrorCategory::Io,
        "",
        "the session search-path state is unavailable",
    )
}

fn search_path_unavailable() -> FileSystemError {
    FileSystemError::new(
        FileSystemErrorCategory::Io,
        "",
        "the session search-path service is unavailable",
    )
}

fn map_io_error(path: &Path, operation: &str, error: &io::Error) -> FileSystemError {
    let category = match error.kind() {
        io::ErrorKind::NotFound => FileSystemErrorCategory::NotFound,
        io::ErrorKind::PermissionDenied => FileSystemErrorCategory::PermissionDenied,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidFilename => {
            FileSystemErrorCategory::InvalidPath
        }
        _ => FileSystemErrorCategory::Io,
    };
    FileSystemError::new(
        category,
        path.to_string_lossy(),
        format!("cannot {operation} `{}`: {error}", path.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Component,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            loop {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
                let path = parent.join(format!(
                    "openmat-filesystem-{}-{nonce}-{counter}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create unique temporary directory: {error}"),
                }
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            let relative = Path::new(relative);
            assert!(!relative.is_absolute());
            assert!(relative.components().all(|component| {
                matches!(component, Component::Normal(_) | Component::CurDir)
            }));
            let target = self.path.join(relative);
            assert!(target.starts_with(&self.path));
            target
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            assert!(self.path.starts_with(parent));
            if self.path.exists() {
                fs::remove_dir_all(&self.path).unwrap();
            }
        }
    }

    #[test]
    fn local_service_resolves_relative_paths_and_enforces_read_limit() {
        let directory = TemporaryDirectory::new();
        let mut service = LocalFileSystem::new(&directory.path).unwrap();
        service.write_file("sample.txt", b"abcd").unwrap();
        assert_eq!(fs::read(directory.path("sample.txt")).unwrap(), b"abcd");
        assert_eq!(service.read_file("sample.txt", 4).unwrap(), b"abcd");
        assert_eq!(
            service.read_file("sample.txt", 3).unwrap_err().category,
            FileSystemErrorCategory::TooLarge
        );
    }

    #[test]
    fn shared_working_directory_changes_atomically_for_all_filesystem_views() {
        let directory = TemporaryDirectory::new();
        let child = directory.path("child");
        fs::create_dir(&child).unwrap();
        fs::write(directory.path("not-a-directory.txt"), b"text").unwrap();

        let working_directory = WorkingDirectory::new(&directory.path).unwrap();
        let mut first = LocalFileSystem::from_working_directory(working_directory.clone());
        let mut second = LocalFileSystem::from_working_directory(working_directory.clone());
        let initial = working_directory.snapshot().unwrap();

        second.change_working_directory("child").unwrap();
        let changed = working_directory.snapshot().unwrap();
        assert_eq!(changed.generation, initial.generation + 1);
        assert_eq!(first.working_directory().unwrap(), changed.path);
        first.write_file("shared.txt", b"shared").unwrap();
        assert_eq!(fs::read(child.join("shared.txt")).unwrap(), b"shared");

        let error = second
            .change_working_directory("../not-a-directory.txt")
            .unwrap_err();
        assert_eq!(error.category, FileSystemErrorCategory::NotDirectory);
        assert_eq!(working_directory.snapshot().unwrap(), changed);
    }

    #[test]
    fn matlab_source_resolution_enforces_caller_private_visibility_and_priority() {
        let directory = TemporaryDirectory::new();
        fs::create_dir_all(directory.path("parent/private")).unwrap();
        fs::create_dir_all(directory.path("parent/sub")).unwrap();
        fs::write(directory.path("parent/main.m"), b"value = choice();\n").unwrap();
        fs::write(directory.path("parent/sub/main.m"), b"value = choice();\n").unwrap();
        fs::write(
            directory.path("parent/private/private_main.m"),
            b"value = sibling();\n",
        )
        .unwrap();
        fs::write(directory.path("parent/choice.m"), b"ordinary = 22;\n").unwrap();
        fs::write(
            directory.path("parent/private/choice.m"),
            b"private = 11;\n",
        )
        .unwrap();
        fs::write(
            directory.path("parent/private/sibling.m"),
            b"sibling = 44;\n",
        )
        .unwrap();

        let parent = fs::canonicalize(directory.path("parent")).unwrap();
        let caller = fs::canonicalize(directory.path("parent/main.m")).unwrap();
        let sub_caller = fs::canonicalize(directory.path("parent/sub/main.m")).unwrap();
        let private_caller =
            fs::canonicalize(directory.path("parent/private/private_main.m")).unwrap();
        let resolver = MatlabSourceResolver::default();

        let private = resolver
            .resolve(Some(&parent), Some(&caller), "choice")
            .unwrap()
            .expect("eligible caller-private function");
        assert_eq!(private.source, "private = 11;\n");
        let command_window = resolver
            .resolve(Some(&parent), None, "choice")
            .unwrap()
            .expect("current-folder ordinary function");
        assert_eq!(command_window.source, "ordinary = 22;\n");
        let subfolder = resolver
            .resolve(Some(&parent), Some(&sub_caller), "choice")
            .unwrap()
            .expect("private visibility does not leak to a child folder");
        assert_eq!(subfolder.path, command_window.path);
        let private_path = SearchPath::new([directory.path("parent/private")]).unwrap();
        assert!(private_path.snapshot().unwrap().paths.is_empty());
        let explicit_private_path = MatlabSourceResolver::from_search_path(private_path);
        assert!(
            explicit_private_path
                .resolve(Some(&directory.path), None, "choice")
                .unwrap()
                .is_none(),
            "an explicitly supplied private directory must not become globally visible"
        );
        let sibling = resolver
            .resolve(Some(&parent), Some(&private_caller), "sibling")
            .unwrap()
            .expect("private functions see private siblings");
        assert_eq!(sibling.source, "sibling = 44;\n");
    }

    #[test]
    fn matlab_source_resolution_maps_qualified_package_names_without_exposing_package_folders() {
        let directory = TemporaryDirectory::new();
        fs::create_dir_all(directory.path("project/+alpha/+nested")).unwrap();
        fs::write(
            directory.path("project/+alpha/twice.m"),
            b"function y = twice(x)\ny = 2*x;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path("project/+alpha/+nested/inc.m"),
            b"function y = inc(x)\ny = x+1;\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path("project/+alpha/caller.m"),
            b"function y = caller()\ny = alpha.twice(3);\nend\n",
        )
        .unwrap();

        let project = fs::canonicalize(directory.path("project")).unwrap();
        let caller = fs::canonicalize(directory.path("project/+alpha/caller.m")).unwrap();
        let resolver = MatlabSourceResolver::default();
        let function = resolver
            .resolve(Some(&project), None, "alpha.twice")
            .unwrap()
            .expect("package function from current folder");
        assert_eq!(function.path.file_name().unwrap(), "twice.m");
        let nested = resolver
            .resolve(Some(&project), None, "alpha.nested.inc")
            .unwrap()
            .expect("nested package function");
        assert_eq!(nested.path.file_name().unwrap(), "inc.m");
        let caller_qualified = resolver
            .resolve(None, Some(&caller), "alpha.twice")
            .unwrap()
            .expect("package caller can use its qualified package name");
        assert_eq!(caller_qualified.path, function.path);
        assert!(
            resolver
                .resolve(None, Some(&caller), "twice")
                .unwrap()
                .is_none(),
            "package siblings are not ordinary caller-local functions"
        );

        let direct_package_path = SearchPath::new([directory.path("project/+alpha")]).unwrap();
        assert!(direct_package_path.snapshot().unwrap().paths.is_empty());
    }

    #[test]
    fn matlab_source_resolution_finds_class_folders_without_exposing_them_as_paths() {
        let directory = TemporaryDirectory::new();
        fs::create_dir_all(directory.path("project/@FolderBox")).unwrap();
        fs::create_dir_all(directory.path("project/+alpha/@PackagedBox")).unwrap();
        fs::write(
            directory.path("project/@FolderBox/FolderBox.m"),
            b"classdef FolderBox\nend\n",
        )
        .unwrap();
        fs::write(
            directory.path("project/+alpha/@PackagedBox/PackagedBox.m"),
            b"classdef PackagedBox\nend\n",
        )
        .unwrap();

        let project = fs::canonicalize(directory.path("project")).unwrap();
        let resolver = MatlabSourceResolver::default();
        let ordinary = resolver
            .resolve(Some(&project), None, "FolderBox")
            .unwrap()
            .expect("current-folder @Class constructor");
        assert_eq!(
            ordinary.path.parent().unwrap().file_name().unwrap(),
            "@FolderBox"
        );
        let packaged = resolver
            .resolve(Some(&project), None, "alpha.PackagedBox")
            .unwrap()
            .expect("package @Class constructor");
        assert_eq!(
            packaged.path.parent().unwrap().file_name().unwrap(),
            "@PackagedBox"
        );

        let direct_class_path = SearchPath::new([directory.path("project/@FolderBox")]).unwrap();
        assert!(direct_class_path.snapshot().unwrap().paths.is_empty());
    }

    #[test]
    fn metadata_and_final_component_wildcards_distinguish_files_and_directories() {
        let directory = TemporaryDirectory::new();
        fs::write(directory.path("alpha.txt"), b"alpha").unwrap();
        fs::write(directory.path("BETA.TXT"), b"beta").unwrap();
        fs::create_dir(directory.path("folder.txt")).unwrap();
        let mut service = LocalFileSystem::new(&directory.path).unwrap();

        let file = service.metadata("alpha.txt").unwrap();
        assert!(file.is_file);
        assert!(!file.is_directory);
        assert_eq!(file.bytes, 5);
        let folder = service.metadata("folder.txt").unwrap();
        assert!(!folder.is_file);
        assert!(folder.is_directory);

        let entries = service.directory_entries("*.txt").unwrap();
        #[cfg(windows)]
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha.txt", "BETA.TXT", "folder.txt"]
        );
        #[cfg(not(windows))]
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha.txt", "folder.txt"]
        );
        let all_entries = service.directory_entries("*").unwrap();
        assert_eq!(all_entries[0].name, ".");
        assert_eq!(all_entries[1].name, "..");
    }

    #[test]
    fn mutations_copy_move_and_remove_only_validated_temporary_targets() {
        let directory = TemporaryDirectory::new();
        let mut service = LocalFileSystem::new(&directory.path).unwrap();
        service.create_directory("source/nested").unwrap();
        service.write_file("source/root.txt", b"root").unwrap();
        service
            .write_file("source/nested/child.txt", b"child")
            .unwrap();

        let recursive_destination = service
            .copy_path("source", "source/recursive-copy", false)
            .unwrap_err();
        assert_eq!(
            recursive_destination.category,
            FileSystemErrorCategory::InvalidPath
        );
        assert!(!directory.path("source/recursive-copy").exists());

        service.copy_path("source", "copied", false).unwrap();
        assert_eq!(
            fs::read(directory.path("copied/nested/child.txt")).unwrap(),
            b"child"
        );
        service
            .write_file("replacement.txt", b"replacement")
            .unwrap();
        service
            .move_path("replacement.txt", "copied/root.txt", false)
            .unwrap();
        assert_eq!(
            fs::read(directory.path("copied/root.txt")).unwrap(),
            b"replacement"
        );
        assert!(!directory.path("replacement.txt").exists());

        service.remove_file("copied/root.txt").unwrap();
        service.remove_directory("copied", true).unwrap();
        assert!(!directory.path("copied").exists());
    }

    #[test]
    fn session_streams_are_positioned_reused_and_not_shared_by_clone() {
        let directory = TemporaryDirectory::new();
        fs::write(directory.path("stream.bin"), [1, 2, 3, 4]).unwrap();
        let mut service = LocalFileSystem::new(&directory.path).unwrap();
        let update = FileOpenMode {
            access: FileOpenAccess::ReadUpdate,
            text: false,
        };
        let identifier = service
            .open_file("stream.bin", update, "ieee-le", "UTF-8")
            .unwrap();
        assert_eq!(identifier, 3);
        assert_eq!(service.open_file_identifiers(), [3]);
        assert!(service.clone().open_file_identifiers().is_empty());
        assert_eq!(service.tell_open_file(identifier).unwrap(), Some(0));
        assert_eq!(
            service.read_open_file(identifier, 2).unwrap(),
            Some(vec![1, 2])
        );
        assert_eq!(service.tell_open_file(identifier).unwrap(), Some(2));
        service
            .seek_open_file(identifier, -1, FileSeekOrigin::Current)
            .unwrap();
        service.write_open_file(identifier, &[9]).unwrap();
        service.close_all_open_files();
        assert!(service.open_file_identifiers().is_empty());
        assert_eq!(
            fs::read(directory.path("stream.bin")).unwrap(),
            [1, 9, 3, 4]
        );

        let reused = service
            .open_file("stream.bin", update, "ieee-le", "UTF-8")
            .unwrap();
        assert_eq!(reused, 3);
        assert!(service.close_open_file(reused));
        assert!(!service.close_open_file(reused));

        let append = FileOpenMode {
            access: FileOpenAccess::AppendUpdate,
            text: false,
        };
        let appended = service
            .open_file("stream.bin", append, "ieee-le", "UTF-8")
            .unwrap();
        assert_eq!(service.tell_open_file(appended).unwrap(), Some(0));
        service.write_open_file(appended, &[7]).unwrap();
        assert!(service.close_open_file(appended));
        assert_eq!(
            fs::read(directory.path("stream.bin")).unwrap(),
            [1, 9, 3, 4, 7]
        );
    }
}
