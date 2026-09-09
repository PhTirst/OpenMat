use std::fs;
use std::path::{Component, Path, PathBuf};

use openmat_runtime::{FileSystemError, FileSystemErrorCategory, MatlabSourceResolver, SearchPath};

use crate::EngineError;

/// A UTF-8 MATLAB source file read from a canonical filesystem path.
///
/// This protocol-independent value deliberately exposes neither parser nor
/// runtime types, so frontends can configure file execution without depending
/// on implementation crates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSourceFile {
    path: PathBuf,
    source_name: String,
    text: String,
}

impl ResolvedSourceFile {
    /// Returns the canonical path used to read this source.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the UTF-8 source name used by diagnostics.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the decoded UTF-8 source text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Deterministic filesystem boundary for MATLAB source loading.
///
/// Dependency lookup checks an eligible caller-private directory, the calling
/// file's directory, the session current folder, and configured directories in
/// order. Canonical duplicate directories are removed while retaining the first
/// occurrence.
#[derive(Clone, Debug, Default)]
pub struct FileSourceResolver {
    resolver: MatlabSourceResolver,
}

impl FileSourceResolver {
    /// Creates a resolver without configured search directories.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            resolver: MatlabSourceResolver::default(),
        }
    }

    /// Creates a resolver from validated, canonical search directories.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error for a non-UTF-8, missing,
    /// unreadable, or non-directory path.
    pub fn new<I, P>(paths: I) -> Result<Self, EngineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let paths = paths
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect::<Vec<_>>();
        for path in &paths {
            validate_utf8_path(path)?;
        }
        let search_path = SearchPath::new(paths)
            .map_err(|error| EngineError::new("source.searchPathRead", error.message))?;
        Ok(Self {
            resolver: MatlabSourceResolver::from_search_path(search_path),
        })
    }

    /// Creates a resolver over host-owned dynamic search-path state.
    #[must_use]
    pub const fn from_search_path(search_path: SearchPath) -> Self {
        Self {
            resolver: MatlabSourceResolver::from_search_path(search_path),
        }
    }

    /// Returns canonical search directories in deterministic lookup order.
    ///
    /// # Errors
    ///
    /// Returns a stable source-state error if the shared path snapshot is unavailable.
    pub fn search_paths(&self) -> Result<Vec<PathBuf>, EngineError> {
        self.resolver
            .search_path_handle()
            .snapshot()
            .map(|snapshot| snapshot.paths)
            .map_err(|error| EngineError::new("source.searchPathRead", error.message))
    }

    /// Returns the shared dynamic search-path handle.
    #[must_use]
    pub fn search_path_handle(&self) -> SearchPath {
        self.resolver.search_path_handle()
    }

    /// Reads an explicitly selected entry file.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error when the path cannot be resolved to a
    /// regular UTF-8 file.
    pub fn read_entry(&self, path: impl AsRef<Path>) -> Result<ResolvedSourceFile, EngineError> {
        let path = path.as_ref();
        validate_utf8_path(path)?;
        let canonical = canonicalize(path, "source.read")?;
        read_canonical_file(canonical)
    }

    /// Reads an entry path relative to the session current directory.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error when the path cannot be resolved to a
    /// regular UTF-8 file.
    pub fn read_entry_from(
        &self,
        working_directory: &Path,
        path: impl AsRef<Path>,
    ) -> Result<ResolvedSourceFile, EngineError> {
        let path = path.as_ref();
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_directory.join(path)
        };
        self.read_entry(resolved)
    }

    /// Resolves `name.m` relative to a caller and then configured search paths.
    ///
    /// `caller` must be a canonical source file returned by this resolver.
    /// A missing symbol returns `Ok(None)` so ordinary runtime undefined-name
    /// behavior is preserved. Existing but invalid candidates return a stable
    /// error and do not silently fall through to a lower-priority directory.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error for invalid names, path escapes,
    /// non-files, read failures, or invalid UTF-8.
    pub fn resolve(
        &self,
        caller: Option<&Path>,
        name: &str,
    ) -> Result<Option<ResolvedSourceFile>, EngineError> {
        self.resolve_from(None, caller, name)
    }

    /// Resolves a source unit with caller-private and caller-directory lookup
    /// ahead of the session current directory and configured search paths.
    ///
    /// # Errors
    ///
    /// Returns a stable `source.*` error for invalid names, paths, source files,
    /// or UTF-8 contents.
    pub fn resolve_from(
        &self,
        working_directory: Option<&Path>,
        caller: Option<&Path>,
        name: &str,
    ) -> Result<Option<ResolvedSourceFile>, EngineError> {
        validate_unit_name(name)?;
        if let Some(caller) = caller {
            validate_utf8_path(caller)?;
        }
        if let Some(working_directory) = working_directory {
            validate_utf8_path(working_directory)?;
        }
        self.resolver
            .resolve(working_directory, caller, name)
            .map_err(map_runtime_source_error)?
            .map(|source| {
                let path = if openmat_runtime::is_builtin_source(&source.path) {
                    source.path.clone()
                } else {
                    canonicalize(&source.path, "source.read")?
                };
                let source_name = display_path(&path)?.to_owned();
                Ok(ResolvedSourceFile {
                    path,
                    source_name,
                    text: source.source,
                })
            })
            .transpose()
    }

    /// Resolves one body declared by a classdef in an `@Class` directory.
    ///
    /// The lookup is deliberately restricted to a sibling `method.m` file. It
    /// never falls through to the current directory or search path, so an
    /// unrelated ordinary function cannot satisfy a class method declaration.
    ///
    /// # Errors
    ///
    /// Returns a stable source error when the class source is not the matching
    /// `@Class/Class.m`, the method name is unsafe, or the candidate escapes the
    /// class directory or cannot be read as UTF-8.
    pub fn resolve_class_method(
        &self,
        class_source: &Path,
        class_name: &str,
        method_name: &str,
    ) -> Result<Option<ResolvedSourceFile>, EngineError> {
        validate_unit_name(class_name)?;
        validate_unit_name(method_name)?;
        if method_name.contains('.') {
            return Err(EngineError::new(
                "source.invalidUnitName",
                format!("`{method_name}` is not a valid external method file name"),
            ));
        }
        validate_utf8_path(class_source)?;
        let class_source = canonicalize(class_source, "source.read")?;
        let class = class_name.rsplit('.').next().unwrap_or(class_name);
        validate_file_stem(&class_source, class)?;
        let class_directory = class_source.parent().ok_or_else(|| {
            EngineError::new(
                "source.classFolderRequired",
                format!(
                    "external method `{class_name}.{method_name}` requires an `@{class}` folder"
                ),
            )
        })?;
        let expected_directory = format!("@{class}");
        let actual_directory = class_directory
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        let directory_matches = if cfg!(windows) {
            actual_directory.eq_ignore_ascii_case(&expected_directory)
        } else {
            actual_directory == expected_directory
        };
        if !directory_matches {
            return Err(EngineError::new(
                "source.classFolderRequired",
                format!(
                    "external method `{class_name}.{method_name}` requires an `@{class}` folder"
                ),
            ));
        }

        let candidate = class_directory.join(format!("{method_name}.m"));
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(path_error(
                    "source.read",
                    &candidate,
                    "cannot inspect external method source",
                    &error,
                ));
            }
        }
        let candidate = canonicalize(&candidate, "source.read")?;
        let class_directory = canonicalize(class_directory, "source.read")?;
        if !candidate.starts_with(&class_directory) {
            return Err(EngineError::new(
                "source.invalidPath",
                format!(
                    "external method source `{}` escapes class directory `{}`",
                    candidate.to_string_lossy(),
                    class_directory.to_string_lossy()
                ),
            ));
        }
        read_canonical_file(candidate).map(Some)
    }
}

fn map_runtime_source_error(error: FileSystemError) -> EngineError {
    let category = match error.category {
        FileSystemErrorCategory::NotFound => "source.notFound",
        FileSystemErrorCategory::NotFile => "source.notFile",
        FileSystemErrorCategory::NotDirectory => "source.searchPathNotDirectory",
        FileSystemErrorCategory::InvalidPath => "source.pathEscape",
        FileSystemErrorCategory::InvalidEncoding => "source.invalidUtf8",
        FileSystemErrorCategory::PermissionDenied
        | FileSystemErrorCategory::TooLarge
        | FileSystemErrorCategory::Io => "source.read",
    };
    EngineError::new(category, error.message)
}

fn validate_unit_name(name: &str) -> Result<(), EngineError> {
    let components = name.split('.').collect::<Vec<_>>();
    let valid = !name.is_empty()
        && !name.contains(['/', '\\', ':', '\0'])
        && components.iter().all(|component| {
            !component.is_empty()
                && *component != "."
                && *component != ".."
                && matches!(
                    Path::new(component).components().next(),
                    Some(Component::Normal(_))
                )
        });
    if !valid {
        return Err(EngineError::new(
            "source.invalidUnitName",
            format!("`{name}` is not a safe MATLAB source unit name"),
        ));
    }
    Ok(())
}

fn validate_file_stem(path: &Path, expected: &str) -> Result<(), EngineError> {
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            EngineError::new(
                "source.nonUtf8Path",
                "MATLAB function and class file names must be valid UTF-8",
            )
        })?;
    let matches = if cfg!(windows) {
        stem.eq_ignore_ascii_case(expected)
    } else {
        stem == expected
    };
    if matches {
        Ok(())
    } else {
        Err(EngineError::new(
            "source.nameMismatch",
            format!(
                "source file `{}` must be named `{expected}.m`",
                path.to_string_lossy()
            ),
        ))
    }
}

fn read_canonical_file(path: PathBuf) -> Result<ResolvedSourceFile, EngineError> {
    let metadata = fs::metadata(&path)
        .map_err(|error| path_error("source.read", &path, "cannot inspect source file", &error))?;
    if !metadata.is_file() {
        return Err(EngineError::new(
            "source.notFile",
            format!(
                "source path `{}` is not a regular file",
                display_path(&path)?
            ),
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| path_error("source.read", &path, "cannot read source file", &error))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        EngineError::new(
            "source.invalidUtf8",
            format!(
                "source file `{}` is not valid UTF-8",
                path.to_string_lossy()
            ),
        )
    })?;
    let source_name = display_path(&path)?.to_owned();
    Ok(ResolvedSourceFile {
        path,
        source_name,
        text,
    })
}

fn canonicalize(path: &Path, category: &'static str) -> Result<PathBuf, EngineError> {
    fs::canonicalize(path).map_err(|error| {
        let category = if error.kind() == std::io::ErrorKind::NotFound {
            "source.notFound"
        } else {
            category
        };
        path_error(category, path, "cannot resolve source path", &error)
    })
}

fn validate_utf8_path(path: &Path) -> Result<(), EngineError> {
    if path.to_str().is_none() {
        return Err(EngineError::new(
            "source.nonUtf8Path",
            "source paths must be representable as UTF-8",
        ));
    }
    Ok(())
}

fn display_path(path: &Path) -> Result<&str, EngineError> {
    path.to_str().ok_or_else(|| {
        EngineError::new(
            "source.nonUtf8Path",
            "source paths must be representable as UTF-8",
        )
    })
}

fn path_error(
    category: &'static str,
    path: &Path,
    operation: &'static str,
    error: &std::io::Error,
) -> EngineError {
    EngineError::new(
        category,
        format!("{operation} `{}`: {error}", path.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMPORARY_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "openmat-kernel-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("temporary test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn caller_directory_precedes_configured_paths_and_duplicates_are_removed() {
        let caller_dir = TestDirectory::new("caller-priority");
        let search_dir = TestDirectory::new("search-priority");
        let caller = caller_dir.path().join("main.m");
        fs::write(&caller, "answer = helper();\n").unwrap();
        fs::write(caller_dir.path().join("helper.m"), "caller = 1;\n").unwrap();
        fs::write(search_dir.path().join("helper.m"), "search = 1;\n").unwrap();

        let resolver = FileSourceResolver::new([search_dir.path(), search_dir.path()]).unwrap();
        assert_eq!(resolver.search_paths().unwrap().len(), 1);
        let caller = resolver.read_entry(&caller).unwrap();
        let source_file = resolver
            .resolve(Some(caller.path()), "helper")
            .unwrap()
            .expect("helper source");

        assert_eq!(source_file.text(), "caller = 1;\n");
        assert_eq!(source_file.path().parent(), caller.path().parent());
    }

    #[test]
    fn caller_private_directory_precedes_ordinary_sibling_and_is_not_global() {
        let directory = TestDirectory::new("caller-private");
        fs::create_dir(directory.path().join("private")).unwrap();
        let caller_path = directory.path().join("main.m");
        fs::write(&caller_path, "answer = helper();\n").unwrap();
        fs::write(directory.path().join("private/helper.m"), "private = 11;\n").unwrap();
        fs::write(directory.path().join("helper.m"), "ordinary = 22;\n").unwrap();
        let resolver = FileSourceResolver::empty();
        let caller = resolver.read_entry(&caller_path).unwrap();

        let private = resolver
            .resolve_from(Some(directory.path()), Some(caller.path()), "helper")
            .unwrap()
            .expect("caller-private helper");
        assert!(private.path().ends_with("private/helper.m"));
        let ordinary = resolver
            .resolve_from(Some(directory.path()), None, "helper")
            .unwrap()
            .expect("current-folder helper");
        assert_eq!(ordinary.text(), "ordinary = 22;\n");
    }

    #[test]
    fn invalid_candidates_do_not_fall_through() {
        let first = TestDirectory::new("invalid-first");
        let second = TestDirectory::new("invalid-second");
        fs::create_dir(first.path().join("helper.m")).unwrap();
        fs::write(second.path().join("helper.m"), "value = 1;\n").unwrap();
        let resolver = FileSourceResolver::new([first.path(), second.path()]).unwrap();

        let error = resolver
            .resolve(None, "helper")
            .expect_err("non-file first candidate must be reported");
        assert_eq!(error.category(), "source.notFile");
        assert_eq!(
            resolver.resolve(None, "../helper").unwrap_err().category(),
            "source.invalidUnitName"
        );
    }

    #[test]
    fn entry_reads_utf8_and_missing_symbol_is_not_an_io_error() {
        let directory = TestDirectory::new("utf8-entry");
        let entry = directory.path().join("入口.m");
        fs::write(&entry, "变量 = 1;\n").unwrap();
        let resolver = FileSourceResolver::empty();

        let source = resolver.read_entry(&entry).unwrap();
        assert_eq!(source.text(), "变量 = 1;\n");
        assert!(
            resolver
                .resolve(Some(source.path()), "missing")
                .unwrap()
                .is_none()
        );
    }
}
