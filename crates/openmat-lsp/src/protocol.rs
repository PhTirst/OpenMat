//! Small, transport-independent types used by the implemented LSP subset.

use std::collections::BTreeMap;

/// A zero-based UTF-16 position, as required by LSP.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    #[must_use]
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// A half-open LSP range.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    #[must_use]
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
}

impl DiagnosticSeverity {
    #[must_use]
    pub const fn lsp_value(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warning => 2,
            Self::Information => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SymbolKind {
    Class,
    Method,
    Property,
    Function,
}

impl SymbolKind {
    #[must_use]
    pub const fn lsp_value(self) -> u8 {
        match self {
            Self::Class => 5,
            Self::Method => 6,
            Self::Property => 7,
            Self::Function => 12,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub range: Range,
    pub selection_range: Range,
    pub children: Vec<Self>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hover {
    pub markdown: String,
    pub range: Range,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CompletionKind {
    Method,
    Function,
    Property,
    Class,
    Variable,
    Keyword,
}

impl CompletionKind {
    #[must_use]
    pub const fn lsp_value(self) -> u8 {
        match self {
            Self::Method => 2,
            Self::Function => 3,
            Self::Variable => 6,
            Self::Class => 7,
            Self::Property => 10,
            Self::Keyword => 14,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// A source location in an open buffer or a host-indexed workspace source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// The range and current spelling returned by `textDocument/prepareRename`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRename {
    pub range: Range,
    pub placeholder: String,
}

/// Edits for one source. A closed disk source uses the LSP null version;
/// an open buffer carries its exact document version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentEdit {
    pub uri: String,
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

/// A filename change coupled to a primary function's declaration and uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRename {
    pub old_uri: String,
    pub new_uri: String,
}

/// A deterministic edit over open and host-indexed workspace documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceEdit {
    pub document_changes: Vec<DocumentEdit>,
    pub file_renames: Vec<FileRename>,
}

impl WorkspaceEdit {
    #[must_use]
    pub fn from_changes(changes: BTreeMap<(String, Option<i32>), Vec<TextEdit>>) -> Self {
        Self {
            document_changes: changes
                .into_iter()
                .map(|((uri, version), edits)| DocumentEdit {
                    uri,
                    version,
                    edits,
                })
                .collect(),
            file_renames: Vec::new(),
        }
    }
}

/// Versioned identity carried through `completionItem/resolve`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionData {
    pub uri: String,
    pub version: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionResolution {
    pub detail: String,
    pub markdown: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: String,
    pub text_edit: TextEdit,
    pub data: CompletionData,
}

/// One flat workspace symbol result. Duplicate declarations at distinct
/// locations remain distinct rather than being guessed into one symbol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub location: Location,
    pub container_name: Option<String>,
}

/// Token types in the exact order advertised by the server legend.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SemanticTokenType {
    Keyword,
    Function,
    Method,
    Class,
    Property,
    Parameter,
    Variable,
    Number,
    String,
    Comment,
    Operator,
}

impl SemanticTokenType {
    #[must_use]
    pub const fn lsp_index(self) -> u32 {
        match self {
            Self::Keyword => 0,
            Self::Function => 1,
            Self::Method => 2,
            Self::Class => 3,
            Self::Property => 4,
            Self::Parameter => 5,
            Self::Variable => 6,
            Self::Number => 7,
            Self::String => 8,
            Self::Comment => 9,
            Self::Operator => 10,
        }
    }
}

pub const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "keyword",
    "function",
    "method",
    "class",
    "property",
    "parameter",
    "variable",
    "number",
    "string",
    "comment",
    "operator",
];

/// A directly applicable, versioned quick fix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeAction {
    pub title: String,
    pub kind: String,
    pub diagnostics: Vec<Diagnostic>,
    pub edit: WorkspaceEdit,
    pub is_preferred: bool,
}

#[cfg(test)]
mod tests {
    use super::{SEMANTIC_TOKEN_TYPES, SemanticTokenType};

    #[test]
    fn semantic_token_indices_match_advertised_legend() {
        let kinds = [
            SemanticTokenType::Keyword,
            SemanticTokenType::Function,
            SemanticTokenType::Method,
            SemanticTokenType::Class,
            SemanticTokenType::Property,
            SemanticTokenType::Parameter,
            SemanticTokenType::Variable,
            SemanticTokenType::Number,
            SemanticTokenType::String,
            SemanticTokenType::Comment,
            SemanticTokenType::Operator,
        ];
        assert_eq!(kinds.len(), SEMANTIC_TOKEN_TYPES.len());
        for (index, kind) in kinds.into_iter().enumerate() {
            assert_eq!(
                kind.lsp_index(),
                u32::try_from(index).expect("small legend")
            );
        }
    }
}
