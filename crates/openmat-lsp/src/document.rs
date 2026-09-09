//! Versioned source documents and byte-to-LSP position conversion.

use crate::protocol::{Position, Range};
use openmat_compiler::compile;
use openmat_hir::{LowerResult, Stmt, StmtKind, lower};
use openmat_parser::{ParseResult, parse};
use openmat_source::{Diagnostic, Severity, SourceId, TextRange};
use std::collections::BTreeMap;
use std::fmt;

/// A parsed, lowered source from an editor buffer or the host's disk snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    uri: String,
    version: i32,
    source_id: SourceId,
    text: String,
    line_index: LineIndex,
    parsed: ParseResult,
    lowered: LowerResult,
    compiler_diagnostics: Vec<Diagnostic>,
}

impl Document {
    fn new(uri: String, version: i32, source_id: SourceId, text: String) -> Self {
        let line_index = LineIndex::new(&text);
        let parsed = parse(source_id, &text);
        let lowered = lower(&parsed.syntax);
        let has_frontend_errors = parsed
            .diagnostics
            .iter()
            .chain(&lowered.diagnostics)
            .any(|diagnostic| diagnostic.severity == Severity::Error);
        let has_frontend_only_declarations =
            contains_frontend_only_declaration(&lowered.file.statements);
        let compiler_diagnostics = if has_frontend_errors || has_frontend_only_declarations {
            Vec::new()
        } else {
            compile(&lowered.file).map_or_else(
                |error| {
                    error
                        .into_diagnostics()
                        .into_iter()
                        .map(|diagnostic| diagnostic.to_source_diagnostic())
                        .collect()
                },
                |_| Vec::new(),
            )
        };
        Self {
            uri,
            version,
            source_id,
            text,
            line_index,
            parsed,
            lowered,
            compiler_diagnostics,
        }
    }

    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }

    #[must_use]
    pub const fn version(&self) -> i32 {
        self.version
    }

    #[must_use]
    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn parsed(&self) -> &ParseResult {
        &self.parsed
    }

    #[must_use]
    pub const fn lowered(&self) -> &LowerResult {
        &self.lowered
    }

    /// Iterates frontend diagnostics and compiler diagnostics when every HIR
    /// statement has executable lowering support.
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.parsed
            .diagnostics
            .iter()
            .chain(self.lowered.diagnostics.iter())
            .chain(self.compiler_diagnostics.iter())
    }

    #[must_use]
    pub fn byte_to_position(&self, offset: u32) -> Position {
        self.line_index.byte_to_position(&self.text, offset)
    }

    #[must_use]
    pub fn byte_range_to_range(&self, range: TextRange) -> Range {
        Range::new(
            self.byte_to_position(range.start()),
            self.byte_to_position(range.end()),
        )
    }

    #[must_use]
    pub fn position_to_byte(&self, position: Position) -> Option<usize> {
        self.line_index.position_to_byte(&self.text, position)
    }
}

fn contains_frontend_only_declaration(statements: &[Stmt]) -> bool {
    statements.iter().any(|statement| match &statement.kind {
        StmtKind::Declaration(_) => true,
        StmtKind::If {
            branches,
            else_body,
        } => {
            branches
                .iter()
                .any(|branch| contains_frontend_only_declaration(&branch.body))
                || contains_frontend_only_declaration(else_body)
        }
        StmtKind::For { body, .. } | StmtKind::While { body, .. } => {
            contains_frontend_only_declaration(body)
        }
        StmtKind::Try(try_statement) => {
            contains_frontend_only_declaration(&try_statement.body)
                || try_statement
                    .catch
                    .as_ref()
                    .is_some_and(|catch| contains_frontend_only_declaration(&catch.body))
        }
        StmtKind::Switch {
            cases, otherwise, ..
        } => {
            cases
                .iter()
                .any(|case| contains_frontend_only_declaration(&case.body))
                || otherwise
                    .as_ref()
                    .is_some_and(|otherwise| contains_frontend_only_declaration(&otherwise.body))
        }
        StmtKind::Function(function) => contains_frontend_only_declaration(&function.body),
        StmtKind::Class(class) => class.method_blocks.iter().any(|block| {
            block
                .methods
                .iter()
                .any(|method| contains_frontend_only_declaration(&method.body))
        }),
        _ => false,
    })
}

/// Typed lifecycle and version failures from [`DocumentStore`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentError {
    DuplicateOpen {
        uri: String,
    },
    NotOpen {
        uri: String,
    },
    DuplicateVersion {
        uri: String,
        version: i32,
    },
    StaleVersion {
        uri: String,
        current: i32,
        received: i32,
    },
    SourceIdExhausted,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOpen { uri } => write!(formatter, "document is already open: {uri}"),
            Self::NotOpen { uri } => write!(formatter, "document is not open: {uri}"),
            Self::DuplicateVersion { uri, version } => {
                write!(formatter, "duplicate version {version} for document: {uri}")
            }
            Self::StaleVersion {
                uri,
                current,
                received,
            } => write!(
                formatter,
                "stale version {received} for document {uri}; current version is {current}"
            ),
            Self::SourceIdExhausted => formatter.write_str("source identifier space exhausted"),
        }
    }
}

impl std::error::Error for DocumentError {}

/// Transport-neutral storage for full-synchronization editor documents.
#[derive(Debug)]
pub struct DocumentStore {
    open: BTreeMap<String, Document>,
    disk: BTreeMap<String, WorkspaceSource>,
    workspace: Option<WorkspaceContext>,
    source_ids: BTreeMap<String, SourceId>,
    next_source_id: u32,
}

/// One UTF-8 source supplied by the native workspace host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceDocument {
    pub uri: String,
    pub relative_path: String,
    pub text: String,
}

/// Resolution context for sources beneath one canonical workspace URI prefix.
/// Directory paths are workspace-relative; an empty string denotes its root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceContext {
    pub root_uri: String,
    pub current_directory: String,
    pub search_paths: Vec<String>,
    pub complete: bool,
}

/// Complete source snapshot. Replacing it removes files absent from the new
/// snapshot without discarding unsaved editor buffers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSnapshot {
    pub root_uri: String,
    pub current_directory: String,
    pub search_paths: Vec<String>,
    /// Whether every eligible source was successfully read. Partial snapshots
    /// may serve navigation but must not promise complete project refactoring.
    pub complete: bool,
    pub documents: Vec<WorkspaceDocument>,
}

#[derive(Debug)]
struct WorkspaceSource {
    relative_path: String,
    document: Document,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: BTreeMap::new(),
            disk: BTreeMap::new(),
            workspace: None,
            source_ids: BTreeMap::new(),
            next_source_id: 1,
        }
    }

    /// Replaces host-owned sources, parsing only new or changed text. Editor
    /// buffers remain authoritative until `didClose`, including deleted files.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::SourceIdExhausted`] if a new URI cannot receive
    /// a stable source identifier.
    pub fn replace_workspace_documents(
        &mut self,
        snapshot: WorkspaceSnapshot,
    ) -> Result<bool, DocumentError> {
        let context = WorkspaceContext {
            root_uri: snapshot.root_uri,
            current_directory: snapshot.current_directory,
            search_paths: snapshot.search_paths,
            complete: snapshot.complete,
        };
        let mut changed = self.workspace.as_ref() != Some(&context);
        self.workspace = Some(context);
        let incoming = snapshot
            .documents
            .into_iter()
            .map(|document| (document.uri.clone(), document))
            .collect::<BTreeMap<_, _>>();
        self.disk.retain(|uri, _| {
            let keep = incoming.contains_key(uri);
            changed |= !keep;
            keep
        });
        for (uri, incoming) in incoming {
            if let Some(existing) = self.disk.get_mut(&uri) {
                if existing.relative_path != incoming.relative_path {
                    existing.relative_path.clone_from(&incoming.relative_path);
                    changed = true;
                }
                if existing.document.text == incoming.text {
                    continue;
                }
            }
            let source_id = self.source_id_for(&uri)?;
            let document = self
                .open
                .get(&uri)
                .filter(|document| document.text == incoming.text)
                .map_or_else(
                    || Document::new(uri.clone(), 0, source_id, incoming.text),
                    |document| {
                        let mut document = document.clone();
                        document.version = 0;
                        document
                    },
                );
            self.disk.insert(
                uri,
                WorkspaceSource {
                    relative_path: incoming.relative_path,
                    document,
                },
            );
            changed = true;
        }
        Ok(changed)
    }

    #[must_use]
    pub const fn workspace_context(&self) -> Option<&WorkspaceContext> {
        self.workspace.as_ref()
    }

    #[must_use]
    pub fn workspace_index_complete(&self) -> bool {
        self.workspace
            .as_ref()
            .is_none_or(|context| context.complete)
    }

    /// Returns a path only for a document belonging to the active root. URI
    /// fallback covers newly created or deleted files still open in the editor.
    #[must_use]
    pub fn workspace_relative_path(&self, uri: &str) -> Option<String> {
        let context = self.workspace.as_ref()?;
        let suffix = uri.strip_prefix(&context.root_uri)?;
        let path = self.disk.get(uri).map_or_else(
            || decode_uri_path(suffix),
            |source| Some(source.relative_path.clone()),
        )?;
        normalized_relative_path(&path)
    }

    /// Returns the editor version, or `None` for a closed, host-indexed source.
    #[must_use]
    pub fn edit_version(&self, uri: &str) -> Option<i32> {
        self.open.get(uri).map(Document::version)
    }

    /// Returns an unsaved buffer in preference to its disk source.
    #[must_use]
    pub fn get_effective(&self, uri: &str) -> Option<&Document> {
        self.open
            .get(uri)
            .or_else(|| self.disk.get(uri).map(|source| &source.document))
    }

    /// Iterates effective sources in deterministic URI order, without duplicate
    /// entries for files that are also open in an editor.
    pub fn iter_effective(&self) -> impl Iterator<Item = (&str, &Document)> {
        let mut effective = self
            .disk
            .iter()
            .map(|(uri, source)| (uri.as_str(), &source.document))
            .collect::<BTreeMap<_, _>>();
        effective.extend(self.iter());
        effective.into_iter()
    }

    /// Opens and parses a document.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::DuplicateOpen`] for an already-open URI or
    /// [`DocumentError::SourceIdExhausted`] if no stable identifier remains.
    pub fn did_open(
        &mut self,
        uri: impl Into<String>,
        version: i32,
        text: impl Into<String>,
    ) -> Result<&Document, DocumentError> {
        let uri = uri.into();
        if self.open.contains_key(&uri) {
            return Err(DocumentError::DuplicateOpen { uri });
        }
        let source_id = self.source_id_for(&uri)?;
        let text = text.into();
        let document = self
            .disk
            .get(&uri)
            .filter(|source| source.document.text == text)
            .map_or_else(
                || Document::new(uri.clone(), version, source_id, text),
                |source| {
                    let mut document = source.document.clone();
                    document.version = version;
                    document
                },
            );
        self.open.insert(uri.clone(), document);
        self.open.get(&uri).ok_or(DocumentError::NotOpen { uri })
    }

    /// Replaces the complete document text and reparses it.
    ///
    /// # Errors
    ///
    /// Returns a typed error for a missing document, duplicate version, or
    /// stale version.
    pub fn did_change(
        &mut self,
        uri: &str,
        version: i32,
        text: impl Into<String>,
    ) -> Result<&Document, DocumentError> {
        let document = self.open.get(uri).ok_or_else(|| DocumentError::NotOpen {
            uri: uri.to_owned(),
        })?;
        if version == document.version {
            return Err(DocumentError::DuplicateVersion {
                uri: uri.to_owned(),
                version,
            });
        }
        if version < document.version {
            return Err(DocumentError::StaleVersion {
                uri: uri.to_owned(),
                current: document.version,
                received: version,
            });
        }
        let source_id = document.source_id;
        self.open.insert(
            uri.to_owned(),
            Document::new(uri.to_owned(), version, source_id, text.into()),
        );
        self.open.get(uri).ok_or_else(|| DocumentError::NotOpen {
            uri: uri.to_owned(),
        })
    }

    /// Closes a document while retaining its URI-to-`SourceId` assignment.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::NotOpen`] when the URI is not open.
    pub fn did_close(&mut self, uri: &str) -> Result<Document, DocumentError> {
        self.open.remove(uri).ok_or_else(|| DocumentError::NotOpen {
            uri: uri.to_owned(),
        })
    }

    #[must_use]
    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.open.get(uri)
    }

    /// Iterates open documents in deterministic URI order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Document)> {
        self.open
            .iter()
            .map(|(uri, document)| (uri.as_str(), document))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    fn allocate_source_id(&mut self) -> Result<SourceId, DocumentError> {
        let next = self.next_source_id;
        self.next_source_id = next
            .checked_add(1)
            .ok_or(DocumentError::SourceIdExhausted)?;
        Ok(SourceId::new(next))
    }

    fn source_id_for(&mut self, uri: &str) -> Result<SourceId, DocumentError> {
        if let Some(source_id) = self.source_ids.get(uri) {
            return Ok(*source_id);
        }
        let source_id = self.allocate_source_id()?;
        self.source_ids.insert(uri.to_owned(), source_id);
        Ok(source_id)
    }
}

fn normalized_relative_path(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            component => components.push(component),
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn decode_uri_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Line {
    start: usize,
    content_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LineIndex {
    lines: Vec<Line>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut cursor = 0;
        while cursor < bytes.len() {
            let break_width = match bytes[cursor] {
                b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => 2,
                b'\r' | b'\n' => 1,
                _ => {
                    cursor += 1;
                    continue;
                }
            };
            lines.push(Line {
                start,
                content_end: cursor,
            });
            cursor += break_width;
            start = cursor;
        }
        lines.push(Line {
            start,
            content_end: text.len(),
        });
        Self { lines }
    }

    fn byte_to_position(&self, text: &str, offset: u32) -> Position {
        let offset = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(text.len());
        let line_index = self
            .lines
            .partition_point(|line| line.start <= offset)
            .saturating_sub(1);
        let line = self.lines.get(line_index).copied().unwrap_or(Line {
            start: 0,
            content_end: 0,
        });
        let mut character_end = offset.min(line.content_end);
        while character_end > line.start && !text.is_char_boundary(character_end) {
            character_end -= 1;
        }
        let utf16_units = text
            .get(line.start..character_end)
            .map_or(0, |slice| slice.encode_utf16().count());
        Position::new(
            u32::try_from(line_index).unwrap_or(u32::MAX),
            u32::try_from(utf16_units).unwrap_or(u32::MAX),
        )
    }

    fn position_to_byte(&self, text: &str, position: Position) -> Option<usize> {
        let line_index = usize::try_from(position.line).ok()?;
        let line = *self.lines.get(line_index)?;
        let line_text = text.get(line.start..line.content_end)?;
        let requested = usize::try_from(position.character).unwrap_or(usize::MAX);
        let mut utf16_units = 0;
        for (relative, character) in line_text.char_indices() {
            if utf16_units >= requested {
                return Some(line.start + relative);
            }
            let next = utf16_units + character.len_utf16();
            if requested < next {
                return Some(line.start + relative);
            }
            utf16_units = next;
        }
        Some(line.content_end)
    }
}

#[cfg(test)]
mod tests {
    use super::{DocumentError, DocumentStore, WorkspaceDocument, WorkspaceSnapshot};
    use crate::protocol::{Position, Range};
    use openmat_source::TextRange;

    const URI: &str = "file:///workspace/demo.m";

    fn snapshot(root: &str, files: &[(&str, &str)]) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            root_uri: root.to_owned(),
            current_directory: String::new(),
            search_paths: Vec::new(),
            complete: true,
            documents: files
                .iter()
                .map(|(path, text)| WorkspaceDocument {
                    uri: format!("{root}{path}"),
                    relative_path: (*path).to_owned(),
                    text: (*text).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn disk_snapshot_overlays_preserve_buffers_and_close_to_latest_disk() {
        let mut store = DocumentStore::new();
        let root = "file:///workspace/";
        let initial = snapshot(root, &[("demo.m", "x = 1;"), ("helper.m", "y = 2;")]);
        assert!(store.replace_workspace_documents(initial.clone()).unwrap());
        let source_id = store.get_effective(URI).unwrap().source_id();
        assert!(!store.replace_workspace_documents(initial).unwrap());
        assert_eq!(store.edit_version(URI), None);
        assert_eq!(store.len(), 0);
        assert_eq!(store.iter_effective().count(), 2);

        store.did_open(URI, 7, "x = 3;").unwrap();
        assert_eq!(store.edit_version(URI), Some(7));
        assert_eq!(store.iter_effective().count(), 2);
        store
            .replace_workspace_documents(snapshot(root, &[("demo.m", "x = 4;")]))
            .unwrap();
        assert_eq!(store.get_effective(URI).unwrap().text(), "x = 3;");
        assert_eq!(store.iter_effective().count(), 1);
        store.did_close(URI).unwrap();
        assert_eq!(store.get_effective(URI).unwrap().text(), "x = 4;");
        assert_eq!(store.get_effective(URI).unwrap().source_id(), source_id);
        assert_eq!(store.edit_version(URI), None);

        store.did_open(URI, 8, "x = 5;").unwrap();
        store
            .replace_workspace_documents(snapshot(root, &[]))
            .unwrap();
        assert_eq!(store.get_effective(URI).unwrap().text(), "x = 5;");
        store.did_close(URI).unwrap();
        assert!(store.get_effective(URI).is_none());
    }

    #[test]
    fn workspace_context_excludes_retained_old_root_buffers() {
        let mut store = DocumentStore::new();
        store
            .replace_workspace_documents(snapshot("file:///workspace/", &[("demo.m", "x = 1;")]))
            .unwrap();
        store.did_open(URI, 1, "x = 2;").unwrap();
        store
            .replace_workspace_documents(snapshot("file:///other/", &[("demo.m", "x = 3;")]))
            .unwrap();
        assert!(store.workspace_relative_path(URI).is_none());
        assert_eq!(store.get(URI).unwrap().text(), "x = 2;");
        assert_eq!(
            store.workspace_relative_path("file:///other/demo.m"),
            Some("demo.m".to_owned())
        );
        assert_eq!(
            store.workspace_relative_path("file:///other/new%20folder/%E4%B8%AD.m"),
            Some("new folder/中.m".to_owned())
        );
        assert!(
            store
                .workspace_relative_path("file:///other/%2e%2e/escape.m")
                .is_none()
        );
    }

    #[test]
    fn lifecycle_reparses_and_preserves_source_identity() {
        let mut store = DocumentStore::new();
        let first = store
            .did_open(URI, 1, "x = @;")
            .expect("document should open");
        let source_id = first.source_id();
        assert!(!first.diagnostics().collect::<Vec<_>>().is_empty());

        let changed = store
            .did_change(URI, 2, "x = 1;")
            .expect("newer version should replace text");
        assert_eq!(changed.source_id(), source_id);
        assert!(changed.diagnostics().collect::<Vec<_>>().is_empty());

        store.did_close(URI).expect("document should close");
        let reopened = store
            .did_open(URI, 7, "y = 2;")
            .expect("closed URI should reopen");
        assert_eq!(reopened.source_id(), source_id);
    }

    #[test]
    fn frontend_only_declarations_do_not_publish_unknown_compiler_statement_noise() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                "file:///workspace/declarations.m",
                1,
                "function worker()\n global shared\n persistent cache\nend\n",
            )
            .expect("declaration document");

        assert!(document.diagnostics().collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn rejects_duplicate_and_stale_updates_with_typed_errors() {
        let mut store = DocumentStore::new();
        store.did_open(URI, 4, "x = 1;").expect("open");
        assert!(matches!(
            store.did_open(URI, 5, "x = 2;"),
            Err(DocumentError::DuplicateOpen { .. })
        ));
        assert!(matches!(
            store.did_change(URI, 4, "x = 2;"),
            Err(DocumentError::DuplicateVersion { version: 4, .. })
        ));
        assert!(matches!(
            store.did_change(URI, 3, "x = 3;"),
            Err(DocumentError::StaleVersion {
                current: 4,
                received: 3,
                ..
            })
        ));
        assert_eq!(store.get(URI).map(super::Document::text), Some("x = 1;"));
    }

    #[test]
    fn maps_utf8_bytes_to_utf16_positions() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "a😀中z")
            .expect("document should open");
        assert_eq!(document.byte_to_position(0), Position::new(0, 0));
        assert_eq!(document.byte_to_position(1), Position::new(0, 1));
        assert_eq!(document.byte_to_position(5), Position::new(0, 3));
        assert_eq!(document.byte_to_position(8), Position::new(0, 4));
        assert_eq!(document.byte_to_position(9), Position::new(0, 5));
        assert_eq!(document.position_to_byte(Position::new(0, 3)), Some(5));
        assert_eq!(document.position_to_byte(Position::new(0, 2)), Some(1));
    }

    #[test]
    fn maps_crlf_line_ends_and_empty_ranges() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "ab\r\n中\r\n")
            .expect("document should open");
        assert_eq!(document.byte_to_position(2), Position::new(0, 2));
        assert_eq!(document.byte_to_position(3), Position::new(0, 2));
        assert_eq!(document.byte_to_position(4), Position::new(1, 0));
        assert_eq!(document.byte_to_position(7), Position::new(1, 1));
        assert_eq!(document.byte_to_position(9), Position::new(2, 0));
        assert_eq!(
            document.byte_range_to_range(TextRange::empty(9)),
            Range::new(Position::new(2, 0), Position::new(2, 0))
        );
    }
}
