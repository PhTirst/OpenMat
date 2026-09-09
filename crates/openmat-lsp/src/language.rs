//! Language features computed from the current parsed document.

use crate::document::{Document, DocumentStore};
use crate::protocol::{
    CodeAction, CompletionData, CompletionItem, CompletionKind, CompletionResolution, Diagnostic,
    DiagnosticSeverity, DocumentSymbol, Hover, Location, Position, Range, SemanticTokenType,
    SymbolKind, TextEdit, WorkspaceEdit, WorkspaceSymbol,
};
use openmat_hir::{
    Attribute, ClassDef, DeclarationContext, DeclarationForm, DeclarationKind,
    DeclarationStatement, Expr, ExprKind, FunctionDef, Name, Stmt, StmtKind,
};
use openmat_source::{Severity, TextRange};
use openmat_syntax::{SyntaxKind, Token, TokenKind};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn builtin_names() -> &'static BTreeSet<String> {
    static NAMES: OnceLock<BTreeSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        openmat_builtins::minimal_registry()
            .expect("core built-in registration must succeed")
            .names()
            .map(str::to_owned)
            .collect()
    })
}

const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "enumeration",
    "events",
    "for",
    "function",
    "global",
    "if",
    "methods",
    "otherwise",
    "parfor",
    "persistent",
    "properties",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
];

/// Converts compiler diagnostics from byte ranges to LSP UTF-16 ranges.
#[must_use]
pub fn diagnostics(document: &Document) -> Vec<Diagnostic> {
    document
        .diagnostics()
        .filter(|diagnostic| diagnostic.source_id == document.source_id())
        .map(|diagnostic| Diagnostic {
            range: document.byte_range_to_range(diagnostic.range),
            severity: match diagnostic.severity {
                Severity::Error => DiagnosticSeverity::Error,
                Severity::Warning => DiagnosticSeverity::Warning,
                Severity::Note => DiagnosticSeverity::Information,
            },
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
        })
        .collect()
}

/// Returns hierarchical function, class, property, and method symbols.
#[must_use]
pub fn document_symbols(document: &Document) -> Vec<DocumentSymbol> {
    symbols_from_statements(document, &document.lowered().file.statements, false)
}

/// Returns keyword/operator/symbol-category hover information at a position.
#[must_use]
pub fn hover(document: &Document, position: Position) -> Option<Hover> {
    let byte = document.position_to_byte(position)?;
    let byte = u32::try_from(byte).unwrap_or(u32::MAX);
    let token = token_at(document, byte)?;
    let description = if token.kind.is_keyword() {
        format!("`{}` — OpenMat/MATLAB keyword", token.text)
    } else if let Some(description) = operator_description(token.kind) {
        format!("`{}` — {description}", token.text)
    } else if token.kind == TokenKind::Identifier {
        let definitions = definitions_at(document, byte);
        definitions.get(&token.text).map_or_else(
            || {
                if builtin_names().contains(&token.text) {
                    format!("`{}` — OpenMat built-in function", token.text)
                } else {
                    format!("`{}` — identifier", token.text)
                }
            },
            |definition| format!("`{}` — {}", token.text, definition.detail),
        )
    } else {
        return None;
    };
    Some(Hover {
        markdown: description,
        range: document.byte_range_to_range(token.range),
    })
}

/// Returns registered core built-ins, keywords, and current-document names.
///
/// Every item uses an explicit text edit so its replacement range is defined.
#[must_use]
pub fn completion(document: &Document, position: Position) -> Vec<CompletionItem> {
    let Some(byte) = document.position_to_byte(position) else {
        return Vec::new();
    };
    let byte_u32 = u32::try_from(byte).unwrap_or(u32::MAX);
    let (prefix, edit_start) = completion_prefix(document, byte, byte_u32);
    let edit_range = document.byte_range_to_range(
        TextRange::from_usize(edit_start, byte).unwrap_or_else(|_| TextRange::empty(byte_u32)),
    );

    let mut candidates = BTreeMap::new();
    for name in builtin_names() {
        candidates.insert(
            name.clone(),
            Definition {
                kind: CompletionKind::Function,
                detail: "built-in function",
            },
        );
    }
    for keyword in KEYWORDS {
        candidates.insert(
            (*keyword).to_owned(),
            Definition {
                kind: CompletionKind::Keyword,
                detail: "keyword",
            },
        );
    }
    for (name, definition) in definitions_at(document, byte_u32) {
        candidates.insert(name, definition);
    }
    candidates
        .into_iter()
        .filter(|(label, _)| prefix.is_empty() || label.starts_with(prefix))
        .map(|(label, definition)| CompletionItem {
            text_edit: TextEdit {
                range: edit_range,
                new_text: label.clone(),
            },
            label,
            kind: definition.kind,
            detail: definition.detail.to_owned(),
            data: CompletionData {
                uri: document.uri().to_owned(),
                version: document.version(),
            },
        })
        .collect()
}

/// Enriches a version-validated completion item without changing its edit.
#[must_use]
pub fn resolve_completion(
    document: &Document,
    label: &str,
    position: Position,
) -> Option<CompletionResolution> {
    let byte = u32::try_from(document.position_to_byte(position)?).ok()?;
    if KEYWORDS.contains(&label) {
        return Some(CompletionResolution {
            detail: format!("OpenMat keyword `{label}`"),
            markdown: format!("`{label}` is an OpenMat/MATLAB keyword."),
        });
    }
    if let Some(definition) = definitions_at(document, byte).get(label) {
        return Some(CompletionResolution {
            detail: format!("OpenMat {} `{label}`", definition.detail),
            markdown: format!(
                "`{label}` is a {} defined in the current document.",
                definition.detail
            ),
        });
    }
    builtin_names()
        .contains(label)
        .then(|| CompletionResolution {
            detail: format!("OpenMat built-in function `{label}`"),
            markdown: format!(
                "`{label}` is a built-in function registered in the OpenMat core runtime."
            ),
        })
}

/// Returns full-document semantic tokens using the server's advertised legend.
///
/// Identifier categories are derived conservatively from open-document HIR.
/// Unresolved MATLAB call-versus-index syntax remains a variable unless a
/// unique open-document declaration establishes a more specific category.
#[must_use]
pub fn semantic_tokens(documents: &DocumentStore, document: &Document) -> Vec<u32> {
    let environment = SemanticEnvironment::from_documents(documents);
    let classifications = semantic_classifications(document, &environment);
    let mut tokens = Vec::new();
    for token in document.parsed().syntax.tokens() {
        let kind = if token.kind == TokenKind::Identifier {
            classifications
                .get(&(token.range.start(), token.range.end()))
                .copied()
                .unwrap_or(SemanticTokenType::Variable)
                .into()
        } else if token.kind.is_keyword() {
            Some(SemanticTokenType::Keyword)
        } else {
            match token.kind {
                TokenKind::Number => Some(SemanticTokenType::Number),
                TokenKind::CharLiteral | TokenKind::StringLiteral => {
                    Some(SemanticTokenType::String)
                }
                TokenKind::Comment => Some(SemanticTokenType::Comment),
                kind if operator_description(kind).is_some() => Some(SemanticTokenType::Operator),
                _ => None,
            }
        };
        if let Some(kind) = kind {
            push_absolute_tokens(document, token.range, kind, &mut tokens);
        }
    }
    tokens.sort_by_key(|token| (token.line, token.start, token.length, token.kind));
    tokens.dedup();
    encode_semantic_tokens(&tokens)
}

/// Searches symbols across open buffers and host-indexed workspace sources.
///
/// Results use case-insensitive substring filtering, deterministic ordering,
/// and preserve duplicate declarations at distinct locations.
#[must_use]
pub fn workspace_symbols(documents: &DocumentStore, query: &str) -> Vec<WorkspaceSymbol> {
    let query = query.to_lowercase();
    let mut symbols = Vec::new();
    for (_, document) in documents.iter_effective() {
        if documents.workspace_context().is_some()
            && documents.workspace_relative_path(document.uri()).is_none()
        {
            continue;
        }
        for symbol in document_symbols(document) {
            flatten_workspace_symbol(document, &symbol, None, &mut symbols);
        }
    }
    symbols.retain(|symbol| query.is_empty() || symbol.name.to_lowercase().contains(&query));
    symbols.sort_by_key(|symbol| {
        (
            symbol.name.to_lowercase(),
            symbol.name.clone(),
            symbol.kind.lsp_value(),
            symbol.container_name.clone(),
            symbol.location.uri.clone(),
            symbol.location.range,
        )
    });
    symbols
}

/// Returns a conservative whole-file formatting edit.
///
/// Formatting only removes lexer-level trailing whitespace outside comments
/// and strings, normalizes standalone syntax newline tokens to `\n`, and adds
/// a final newline. Comment and string token contents are copied byte-for-byte.
#[must_use]
pub fn formatting(document: &Document) -> Vec<TextEdit> {
    let tokens = document.parsed().syntax.tokens();
    let mut formatted = String::with_capacity(document.text().len().saturating_add(1));
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Eof => {}
            TokenKind::Whitespace
                if tokens.get(index + 1).is_some_and(|next| {
                    matches!(next.kind, TokenKind::Newline | TokenKind::Eof)
                }) => {}
            TokenKind::Newline => formatted.push('\n'),
            _ => formatted.push_str(&token.text),
        }
    }
    if !formatted.ends_with('\n') {
        formatted.push('\n');
    }
    if formatted == document.text() {
        return Vec::new();
    }
    let end = document.byte_to_position(u32::try_from(document.text().len()).unwrap_or(u32::MAX));
    vec![TextEdit {
        range: Range::new(Position::new(0, 0), end),
        new_text: formatted,
    }]
}

/// Builds safe quick fixes for diagnostics confirmed against the current
/// document version and the request's diagnostic context.
#[must_use]
pub fn code_actions(
    document: &Document,
    requested_range: Range,
    context_diagnostics: &[Diagnostic],
) -> Vec<CodeAction> {
    let context = context_diagnostics
        .iter()
        .filter_map(|diagnostic| {
            diagnostic
                .code
                .as_deref()
                .map(|code| (code.to_owned(), diagnostic.range))
        })
        .collect::<BTreeSet<_>>();
    let mut fixed_ranges = BTreeSet::new();
    diagnostics(document)
        .into_iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some("OMP0002"))
        .filter(|diagnostic| range_intersects(diagnostic.range, requested_range))
        .filter(|diagnostic| context.contains(&(String::from("OMP0002"), diagnostic.range)))
        .filter(|diagnostic| fixed_ranges.insert(diagnostic.range))
        .map(|diagnostic| {
            let mut changes = BTreeMap::new();
            changes.insert(
                (document.uri().to_owned(), Some(document.version())),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: String::new(),
                }],
            );
            CodeAction {
                title: String::from("Remove unmatched `end`"),
                kind: String::from("quickfix"),
                diagnostics: vec![diagnostic],
                edit: WorkspaceEdit::from_changes(changes),
                is_preferred: true,
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct SemanticEnvironment {
    globals: BTreeMap<String, Option<SemanticTokenType>>,
    members: BTreeMap<String, Option<SemanticTokenType>>,
}

impl SemanticEnvironment {
    fn from_documents(documents: &DocumentStore) -> Self {
        let mut environment = Self::default();
        for (_, document) in documents.iter() {
            for statement in &document.lowered().file.statements {
                match &statement.kind {
                    StmtKind::Function(function) => {
                        if let Some(name) = &function.name {
                            insert_conservative(
                                &mut environment.globals,
                                &name.text,
                                SemanticTokenType::Function,
                            );
                        }
                    }
                    StmtKind::Class(class) => {
                        if let Some(name) = &class.name {
                            insert_conservative(
                                &mut environment.globals,
                                &name.text,
                                SemanticTokenType::Class,
                            );
                        }
                        for block in &class.property_blocks {
                            for property in &block.properties {
                                if let Some(name) = &property.name {
                                    insert_conservative(
                                        &mut environment.members,
                                        &name.text,
                                        SemanticTokenType::Property,
                                    );
                                }
                            }
                        }
                        for block in &class.method_blocks {
                            for method in &block.methods {
                                if let Some(name) = &method.name {
                                    insert_conservative(
                                        &mut environment.members,
                                        &name.text,
                                        SemanticTokenType::Method,
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        environment
    }
}

fn insert_conservative(
    definitions: &mut BTreeMap<String, Option<SemanticTokenType>>,
    name: &str,
    kind: SemanticTokenType,
) {
    definitions
        .entry(name.to_owned())
        .and_modify(|existing| *existing = None)
        .or_insert(Some(kind));
}

fn semantic_classifications(
    document: &Document,
    environment: &SemanticEnvironment,
) -> BTreeMap<(u32, u32), SemanticTokenType> {
    let mut classifications = BTreeMap::new();
    let scope = scope_for_statements(&document.lowered().file.statements, &[]);
    classify_statements(
        &document.lowered().file.statements,
        &scope,
        environment,
        &mut classifications,
    );
    classifications
}

fn scope_for_statements(
    statements: &[Stmt],
    parameters: &[&Name],
) -> BTreeMap<String, SemanticTokenType> {
    let mut scope = BTreeMap::new();
    for parameter in parameters {
        scope.insert(parameter.text.clone(), SemanticTokenType::Parameter);
    }
    collect_scope_declarations(statements, &mut scope);
    scope
}

fn collect_scope_declarations(
    statements: &[Stmt],
    scope: &mut BTreeMap<String, SemanticTokenType>,
) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Assignment { target, .. } => collect_assignment_declarations(target, scope),
            StmtKind::Declaration(declaration) if declaration_is_scope_binding(declaration) => {
                for name in &declaration.names {
                    scope
                        .entry(name.text.clone())
                        .or_insert(SemanticTokenType::Variable);
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_scope_declarations(&branch.body, scope);
                }
                collect_scope_declarations(else_body, scope);
            }
            StmtKind::For { variable, body, .. } => {
                collect_assignment_declarations(variable, scope);
                collect_scope_declarations(body, scope);
            }
            StmtKind::While { body, .. } => collect_scope_declarations(body, scope),
            StmtKind::Try(try_statement) => {
                collect_scope_declarations(&try_statement.body, scope);
                if let Some(catch) = &try_statement.catch {
                    if let Some(variable) = &catch.variable {
                        scope
                            .entry(variable.text.clone())
                            .or_insert(SemanticTokenType::Variable);
                    }
                    collect_scope_declarations(&catch.body, scope);
                }
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => {
                for case in cases {
                    collect_scope_declarations(&case.body, scope);
                }
                if let Some(otherwise) = otherwise {
                    collect_scope_declarations(&otherwise.body, scope);
                }
            }
            StmtKind::Function(function) => {
                if let Some(name) = &function.name {
                    scope
                        .entry(name.text.clone())
                        .or_insert(SemanticTokenType::Function);
                }
            }
            StmtKind::Class(class) => {
                if let Some(name) = &class.name {
                    scope
                        .entry(name.text.clone())
                        .or_insert(SemanticTokenType::Class);
                }
            }
            _ => {}
        }
    }
}

fn collect_assignment_declarations(target: &Expr, scope: &mut BTreeMap<String, SemanticTokenType>) {
    match &target.kind {
        ExprKind::Name(name) => {
            scope
                .entry(name.clone())
                .or_insert(SemanticTokenType::Variable);
        }
        ExprKind::Matrix(rows) if rows.len() == 1 => {
            for expression in &rows[0] {
                if let ExprKind::Name(name) = &expression.kind {
                    scope
                        .entry(name.clone())
                        .or_insert(SemanticTokenType::Variable);
                }
            }
        }
        _ => {}
    }
}

fn classify_statements(
    statements: &[Stmt],
    scope: &BTreeMap<String, SemanticTokenType>,
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Assignment { target, value } => {
                classify_expr(target, scope, environment, classifications);
                classify_expr(value, scope, environment, classifications);
            }
            StmtKind::Expr(expression) => {
                classify_expr(expression, scope, environment, classifications);
            }
            StmtKind::Declaration(declaration) => {
                for name in &declaration.names {
                    mark_name(classifications, name, SemanticTokenType::Variable);
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    classify_expr(&branch.condition, scope, environment, classifications);
                    classify_statements(&branch.body, scope, environment, classifications);
                }
                classify_statements(else_body, scope, environment, classifications);
            }
            StmtKind::For {
                variable,
                iterable,
                body,
            } => {
                classify_expr(variable, scope, environment, classifications);
                classify_expr(iterable, scope, environment, classifications);
                classify_statements(body, scope, environment, classifications);
            }
            StmtKind::While { condition, body } => {
                classify_expr(condition, scope, environment, classifications);
                classify_statements(body, scope, environment, classifications);
            }
            StmtKind::Try(try_statement) => {
                classify_statements(&try_statement.body, scope, environment, classifications);
                if let Some(catch) = &try_statement.catch {
                    if let Some(variable) = &catch.variable {
                        mark_name(classifications, variable, SemanticTokenType::Variable);
                    }
                    classify_statements(&catch.body, scope, environment, classifications);
                }
            }
            StmtKind::Switch {
                selector,
                cases,
                otherwise,
            } => {
                classify_expr(selector, scope, environment, classifications);
                for case in cases {
                    classify_expr(&case.expression, scope, environment, classifications);
                    classify_statements(&case.body, scope, environment, classifications);
                }
                if let Some(otherwise) = otherwise {
                    classify_statements(&otherwise.body, scope, environment, classifications);
                }
            }
            StmtKind::Function(function) => classify_function(
                function,
                SemanticTokenType::Function,
                environment,
                classifications,
            ),
            StmtKind::Class(class) => classify_class(class, environment, classifications),
            _ => {}
        }
    }
}

fn classify_function(
    function: &FunctionDef,
    kind: SemanticTokenType,
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    if let Some(name) = &function.name {
        mark_name(classifications, name, kind);
    }
    for parameter in function.inputs.iter().chain(&function.outputs) {
        mark_name(classifications, parameter, SemanticTokenType::Parameter);
    }
    let parameters = function
        .inputs
        .iter()
        .chain(&function.outputs)
        .collect::<Vec<_>>();
    let scope = scope_for_statements(&function.body, &parameters);
    classify_statements(&function.body, &scope, environment, classifications);
}

fn classify_class(
    class: &ClassDef,
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    if let Some(name) = &class.name {
        mark_name(classifications, name, SemanticTokenType::Class);
    }
    if let Some(superclass) = &class.superclass {
        mark_name(classifications, superclass, SemanticTokenType::Class);
    }
    classify_attributes(&class.attributes, environment, classifications);
    for block in &class.property_blocks {
        classify_attributes(&block.attributes, environment, classifications);
        for property in &block.properties {
            if let Some(name) = &property.name {
                mark_name(classifications, name, SemanticTokenType::Property);
            }
            if let Some(default) = &property.default {
                classify_expr(default, &BTreeMap::new(), environment, classifications);
            }
        }
    }
    for block in &class.method_blocks {
        classify_attributes(&block.attributes, environment, classifications);
        for method in &block.methods {
            classify_function(
                method,
                SemanticTokenType::Method,
                environment,
                classifications,
            );
        }
    }
}

fn classify_attributes(
    attributes: &[Attribute],
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    for attribute in attributes {
        if let Some(value) = &attribute.value {
            classify_expr(value, &BTreeMap::new(), environment, classifications);
        }
    }
}

fn classify_expr(
    expression: &Expr,
    scope: &BTreeMap<String, SemanticTokenType>,
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    match &expression.kind {
        ExprKind::Name(name) => mark_range(
            classifications,
            expression.span,
            classify_name(name, scope, environment),
        ),
        ExprKind::FunctionHandle(Some(name)) => {
            mark_name(classifications, name, SemanticTokenType::Function);
        }
        ExprKind::Paren(inner) => classify_expr(inner, scope, environment, classifications),
        ExprKind::ParenApply { target, arguments } => {
            classify_apply_target(target, scope, environment, classifications);
            for argument in arguments {
                classify_expr(argument, scope, environment, classifications);
            }
        }
        ExprKind::Field { target, name } => {
            classify_expr(target, scope, environment, classifications);
            if let Some(name) = name {
                let kind = environment
                    .members
                    .get(&name.text)
                    .and_then(|kind| *kind)
                    .unwrap_or(SemanticTokenType::Property);
                mark_name(classifications, name, kind);
            }
        }
        ExprKind::Unary { operand, .. } | ExprKind::Transpose { operand, .. } => {
            classify_expr(operand, scope, environment, classifications);
        }
        ExprKind::Binary { left, right, .. } => {
            classify_expr(left, scope, environment, classifications);
            classify_expr(right, scope, environment, classifications);
        }
        ExprKind::Range { start, step, end } => {
            classify_expr(start, scope, environment, classifications);
            if let Some(step) = step {
                classify_expr(step, scope, environment, classifications);
            }
            classify_expr(end, scope, environment, classifications);
        }
        ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
            for element in rows.iter().flatten() {
                classify_expr(element, scope, environment, classifications);
            }
        }
        _ => {}
    }
}

fn classify_apply_target(
    target: &Expr,
    scope: &BTreeMap<String, SemanticTokenType>,
    environment: &SemanticEnvironment,
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
) {
    match &target.kind {
        ExprKind::Name(name) => mark_range(
            classifications,
            target.span,
            classify_name(name, scope, environment),
        ),
        ExprKind::Field {
            target: receiver,
            name,
        } => {
            classify_expr(receiver, scope, environment, classifications);
            if let Some(name) = name {
                let kind = environment
                    .members
                    .get(&name.text)
                    .and_then(|kind| *kind)
                    .unwrap_or(SemanticTokenType::Property);
                mark_name(classifications, name, kind);
            }
        }
        _ => classify_expr(target, scope, environment, classifications),
    }
}

fn classify_name(
    name: &str,
    scope: &BTreeMap<String, SemanticTokenType>,
    environment: &SemanticEnvironment,
) -> SemanticTokenType {
    scope
        .get(name)
        .copied()
        .or_else(|| environment.globals.get(name).and_then(|kind| *kind))
        .or_else(|| environment.members.get(name).and_then(|kind| *kind))
        .unwrap_or(SemanticTokenType::Variable)
}

fn declaration_is_scope_binding(declaration: &DeclarationStatement) -> bool {
    declaration.form == DeclarationForm::IdentifierList
        && match declaration.kind {
            DeclarationKind::Global => matches!(
                declaration.context,
                DeclarationContext::Script | DeclarationContext::Function
            ),
            DeclarationKind::Persistent => declaration.context == DeclarationContext::Function,
        }
}

fn mark_name(
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
    name: &Name,
    kind: SemanticTokenType,
) {
    mark_range(classifications, name.span, kind);
}

fn mark_range(
    classifications: &mut BTreeMap<(u32, u32), SemanticTokenType>,
    range: TextRange,
    kind: SemanticTokenType,
) {
    classifications
        .entry((range.start(), range.end()))
        .and_modify(|current| {
            if semantic_priority(kind) > semantic_priority(*current) {
                *current = kind;
            }
        })
        .or_insert(kind);
}

const fn semantic_priority(kind: SemanticTokenType) -> u8 {
    match kind {
        SemanticTokenType::Variable
        | SemanticTokenType::Keyword
        | SemanticTokenType::Number
        | SemanticTokenType::String
        | SemanticTokenType::Comment
        | SemanticTokenType::Operator => 0,
        SemanticTokenType::Parameter => 1,
        SemanticTokenType::Property => 2,
        SemanticTokenType::Function | SemanticTokenType::Method | SemanticTokenType::Class => 3,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AbsoluteSemanticToken {
    line: u32,
    start: u32,
    length: u32,
    kind: SemanticTokenType,
}

fn push_absolute_tokens(
    document: &Document,
    range: TextRange,
    kind: SemanticTokenType,
    tokens: &mut Vec<AbsoluteSemanticToken>,
) {
    let start = usize::try_from(range.start()).unwrap_or(usize::MAX);
    let end = usize::try_from(range.end())
        .unwrap_or(usize::MAX)
        .min(document.text().len());
    let Some(text) = document.text().get(start..end) else {
        return;
    };
    let bytes = text.as_bytes();
    let mut segment_start = 0;
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
        push_absolute_segment(
            document,
            start + segment_start,
            start + cursor,
            kind,
            tokens,
        );
        cursor += break_width;
        segment_start = cursor;
    }
    push_absolute_segment(document, start + segment_start, end, kind, tokens);
}

fn push_absolute_segment(
    document: &Document,
    start: usize,
    end: usize,
    kind: SemanticTokenType,
    tokens: &mut Vec<AbsoluteSemanticToken>,
) {
    if start >= end {
        return;
    }
    let start = document.byte_to_position(u32::try_from(start).unwrap_or(u32::MAX));
    let end = document.byte_to_position(u32::try_from(end).unwrap_or(u32::MAX));
    if start.line == end.line && start.character < end.character {
        tokens.push(AbsoluteSemanticToken {
            line: start.line,
            start: start.character,
            length: end.character - start.character,
            kind,
        });
    }
}

fn encode_semantic_tokens(tokens: &[AbsoluteSemanticToken]) -> Vec<u32> {
    let mut encoded = Vec::with_capacity(tokens.len().saturating_mul(5));
    let mut previous_line = 0;
    let mut previous_start = 0;
    for token in tokens {
        let delta_line = token.line - previous_line;
        let delta_start = if delta_line == 0 {
            token.start - previous_start
        } else {
            token.start
        };
        encoded.extend_from_slice(&[
            delta_line,
            delta_start,
            token.length,
            token.kind.lsp_index(),
            0,
        ]);
        previous_line = token.line;
        previous_start = token.start;
    }
    encoded
}

fn flatten_workspace_symbol(
    document: &Document,
    symbol: &DocumentSymbol,
    container_name: Option<&str>,
    symbols: &mut Vec<WorkspaceSymbol>,
) {
    symbols.push(WorkspaceSymbol {
        name: symbol.name.clone(),
        kind: symbol.kind,
        location: Location {
            uri: document.uri().to_owned(),
            range: symbol.selection_range,
        },
        container_name: container_name.map(str::to_owned),
    });
    for child in &symbol.children {
        flatten_workspace_symbol(document, child, Some(&symbol.name), symbols);
    }
}

fn range_intersects(left: Range, right: Range) -> bool {
    if right.start == right.end {
        left.start <= right.start && right.start <= left.end
    } else if left.start == left.end {
        right.start <= left.start && left.start <= right.end
    } else {
        left.start < right.end && right.start < left.end
    }
}

fn symbols_from_statements(
    document: &Document,
    statements: &[Stmt],
    methods: bool,
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    for statement in statements {
        match &statement.kind {
            StmtKind::Function(function) => {
                let children = symbols_from_statements(document, &function.body, false);
                if let Some(name) = &function.name {
                    symbols.push(DocumentSymbol {
                        name: name.text.clone(),
                        detail: Some(if methods { "method" } else { "function" }.to_owned()),
                        kind: if methods {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        },
                        range: document.byte_range_to_range(function.span),
                        selection_range: document.byte_range_to_range(name.span),
                        children,
                    });
                } else {
                    symbols.extend(children);
                }
            }
            StmtKind::Class(class) => {
                if let Some(symbol) = class_symbol(document, class) {
                    symbols.push(symbol);
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    symbols.extend(symbols_from_statements(document, &branch.body, false));
                }
                symbols.extend(symbols_from_statements(document, else_body, false));
            }
            StmtKind::For { body, .. } | StmtKind::While { body, .. } => {
                symbols.extend(symbols_from_statements(document, body, false));
            }
            StmtKind::Try(try_statement) => {
                symbols.extend(symbols_from_statements(
                    document,
                    &try_statement.body,
                    false,
                ));
                if let Some(catch) = &try_statement.catch {
                    symbols.extend(symbols_from_statements(document, &catch.body, false));
                }
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => {
                for case in cases {
                    symbols.extend(symbols_from_statements(document, &case.body, false));
                }
                if let Some(otherwise) = otherwise {
                    symbols.extend(symbols_from_statements(document, &otherwise.body, false));
                }
            }
            _ => {}
        }
    }
    symbols
}

fn class_symbol(document: &Document, class: &ClassDef) -> Option<DocumentSymbol> {
    let name = class.name.as_ref()?;
    let mut children = Vec::new();
    for block in &class.property_blocks {
        for property in &block.properties {
            let Some(property_name) = &property.name else {
                continue;
            };
            children.push(DocumentSymbol {
                name: property_name.text.clone(),
                detail: Some("property".to_owned()),
                kind: SymbolKind::Property,
                range: document.byte_range_to_range(property.span),
                selection_range: document.byte_range_to_range(property_name.span),
                children: Vec::new(),
            });
        }
    }
    for block in &class.method_blocks {
        children.extend(method_symbols(document, &block.methods));
    }
    Some(DocumentSymbol {
        name: name.text.clone(),
        detail: Some("class".to_owned()),
        kind: SymbolKind::Class,
        range: document.byte_range_to_range(class.span),
        selection_range: document.byte_range_to_range(name.span),
        children,
    })
}

fn method_symbols(document: &Document, methods: &[FunctionDef]) -> Vec<DocumentSymbol> {
    methods
        .iter()
        .filter_map(|method| {
            let name = method.name.as_ref()?;
            Some(DocumentSymbol {
                name: name.text.clone(),
                detail: Some("method".to_owned()),
                kind: SymbolKind::Method,
                range: document.byte_range_to_range(method.span),
                selection_range: document.byte_range_to_range(name.span),
                children: symbols_from_statements(document, &method.body, false),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Definition {
    kind: CompletionKind,
    detail: &'static str,
}

fn definitions_at(document: &Document, byte: u32) -> BTreeMap<String, Definition> {
    let cursor = DefinitionCursor { document, byte };
    let statements = &document.lowered().file.statements;
    // Local functions can call their siblings, but cannot capture script
    // variables. Only nested functions inherit a parent function's bindings.
    let mut file_definitions = BTreeMap::new();
    for statement in statements {
        collect_named_definition(statement, &mut file_definitions);
    }
    if let Some(scope) = definitions_in_child_scope(statements, &cursor, &file_definitions) {
        return scope;
    }
    let mut definitions = BTreeMap::new();
    collect_definitions(statements, &mut definitions);
    definitions
}

struct DefinitionCursor<'a> {
    document: &'a Document,
    byte: u32,
}

impl DefinitionCursor<'_> {
    fn within(&self, span: TextRange) -> bool {
        if span.start() <= self.byte && self.byte < span.end() {
            return true;
        }
        if self.byte != span.end()
            || usize::try_from(self.byte).ok() != Some(self.document.text().len())
        {
            return false;
        }
        // An unfinished scope reaches EOF without its own closing `end`.
        // Inspect only tokens after its children, so a nested child's `end`
        // cannot accidentally close the parent for completion purposes.
        let syntax = &self.document.parsed().syntax;
        syntax.nodes().iter().any(|node| {
            node.range == span
                && matches!(node.kind, SyntaxKind::FunctionDef | SyntaxKind::ClassDef)
                && {
                    let tail_start = node
                        .children
                        .iter()
                        .filter_map(|child| syntax.node(*child))
                        .map(|child| child.token_range.end)
                        .max()
                        .unwrap_or(node.token_range.start);
                    !syntax.tokens()[tail_start..node.token_range.end]
                        .iter()
                        .any(|token| token.kind == TokenKind::KwEnd)
                }
        })
    }
}

fn collect_named_definition(statement: &Stmt, definitions: &mut BTreeMap<String, Definition>) {
    match &statement.kind {
        StmtKind::Function(function) => collect_function_name(function, definitions, false),
        StmtKind::Class(class) => {
            if let Some(name) = &class.name {
                definitions.insert(
                    name.text.clone(),
                    Definition {
                        kind: CompletionKind::Class,
                        detail: "class",
                    },
                );
            }
        }
        _ => {}
    }
}

fn definitions_in_child_scope(
    statements: &[Stmt],
    cursor: &DefinitionCursor<'_>,
    inherited: &BTreeMap<String, Definition>,
) -> Option<BTreeMap<String, Definition>> {
    for statement in statements {
        let found = match &statement.kind {
            StmtKind::Function(function) if cursor.within(function.span) => {
                Some(function_definitions_at(function, cursor, inherited, false))
            }
            StmtKind::Class(class) if cursor.within(class.span) => {
                Some(class_definitions_at(class, cursor, inherited))
            }
            StmtKind::If {
                branches,
                else_body,
            } => branches
                .iter()
                .find_map(|branch| definitions_in_child_scope(&branch.body, cursor, inherited))
                .or_else(|| definitions_in_child_scope(else_body, cursor, inherited)),
            StmtKind::For { body, .. } | StmtKind::While { body, .. } => {
                definitions_in_child_scope(body, cursor, inherited)
            }
            StmtKind::Try(try_statement) => {
                definitions_in_child_scope(&try_statement.body, cursor, inherited).or_else(|| {
                    try_statement.catch.as_ref().and_then(|catch| {
                        definitions_in_child_scope(&catch.body, cursor, inherited)
                    })
                })
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => cases
                .iter()
                .find_map(|case| definitions_in_child_scope(&case.body, cursor, inherited))
                .or_else(|| {
                    otherwise.as_ref().and_then(|otherwise| {
                        definitions_in_child_scope(&otherwise.body, cursor, inherited)
                    })
                }),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn function_definitions_at(
    function: &FunctionDef,
    cursor: &DefinitionCursor<'_>,
    inherited: &BTreeMap<String, Definition>,
    method: bool,
) -> BTreeMap<String, Definition> {
    let mut local = BTreeMap::new();
    collect_function_name(function, &mut local, method);
    for name in function.inputs.iter().chain(&function.outputs) {
        if name.text != "~" {
            local.insert(
                name.text.clone(),
                Definition {
                    kind: CompletionKind::Variable,
                    detail: "function parameter",
                },
            );
        }
    }
    collect_definitions(&function.body, &mut local);
    let mut scope = inherited.clone();
    for (name, definition) in local {
        // Assigning a parent's variable in a nested function writes the shared
        // binding. Only a child parameter or explicit declaration introduces
        // a different binding with that name.
        let captures_parent = matches!(definition.detail, "local variable" | "loop variable")
            && inherited
                .get(&name)
                .is_some_and(|definition| definition.kind == CompletionKind::Variable);
        if !captures_parent {
            scope.insert(name, definition);
        }
    }
    definitions_in_child_scope(&function.body, cursor, &scope).unwrap_or(scope)
}

fn class_definitions_at(
    class: &ClassDef,
    cursor: &DefinitionCursor<'_>,
    inherited: &BTreeMap<String, Definition>,
) -> BTreeMap<String, Definition> {
    for block in &class.method_blocks {
        for method in &block.methods {
            if cursor.within(method.span) {
                // Properties require a receiver; method bodies also cannot
                // see parameters or locals belonging to another method.
                return function_definitions_at(method, cursor, inherited, true);
            }
        }
    }
    let mut scope = inherited.clone();
    for block in &class.property_blocks {
        for property in &block.properties {
            if let Some(name) = &property.name {
                scope.insert(
                    name.text.clone(),
                    Definition {
                        kind: CompletionKind::Property,
                        detail: "property",
                    },
                );
            }
        }
    }
    for block in &class.method_blocks {
        for method in &block.methods {
            collect_function_name(method, &mut scope, true);
        }
    }
    scope
}

fn collect_definitions(statements: &[Stmt], definitions: &mut BTreeMap<String, Definition>) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Assignment { target, .. } => {
                collect_assignment_definitions(target, definitions, "local variable");
            }
            StmtKind::Declaration(declaration) if declaration_is_scope_binding(declaration) => {
                collect_declaration_definitions(declaration, definitions);
            }
            StmtKind::Function(_) | StmtKind::Class(_) => {
                collect_named_definition(statement, definitions);
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_definitions(&branch.body, definitions);
                }
                collect_definitions(else_body, definitions);
            }
            StmtKind::For { variable, body, .. } => {
                collect_assignment_definitions(variable, definitions, "loop variable");
                collect_definitions(body, definitions);
            }
            StmtKind::While { body, .. } => collect_definitions(body, definitions),
            StmtKind::Try(try_statement) => {
                collect_definitions(&try_statement.body, definitions);
                if let Some(catch) = &try_statement.catch {
                    if let Some(variable) = &catch.variable {
                        definitions
                            .entry(variable.text.clone())
                            .or_insert(Definition {
                                kind: CompletionKind::Variable,
                                detail: "local variable",
                            });
                    }
                    collect_definitions(&catch.body, definitions);
                }
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => {
                for case in cases {
                    collect_definitions(&case.body, definitions);
                }
                if let Some(otherwise) = otherwise {
                    collect_definitions(&otherwise.body, definitions);
                }
            }
            _ => {}
        }
    }
}

fn collect_assignment_definitions(
    target: &Expr,
    definitions: &mut BTreeMap<String, Definition>,
    detail: &'static str,
) {
    match &target.kind {
        ExprKind::Name(name) => {
            definitions.entry(name.clone()).or_insert(Definition {
                kind: CompletionKind::Variable,
                detail,
            });
        }
        ExprKind::Matrix(rows) if rows.len() == 1 => {
            for element in &rows[0] {
                collect_assignment_definitions(element, definitions, detail);
            }
        }
        _ => {}
    }
}

fn collect_declaration_definitions(
    declaration: &DeclarationStatement,
    definitions: &mut BTreeMap<String, Definition>,
) {
    let detail = match declaration.kind {
        DeclarationKind::Global => "global variable declaration",
        DeclarationKind::Persistent => "persistent variable declaration",
    };
    for name in &declaration.names {
        definitions.insert(
            name.text.clone(),
            Definition {
                kind: CompletionKind::Variable,
                detail,
            },
        );
    }
}

fn collect_function_name(
    function: &FunctionDef,
    definitions: &mut BTreeMap<String, Definition>,
    method: bool,
) {
    if let Some(name) = &function.name {
        definitions.insert(
            name.text.clone(),
            Definition {
                kind: if method {
                    CompletionKind::Method
                } else {
                    CompletionKind::Function
                },
                detail: if method { "method" } else { "function" },
            },
        );
    }
}

fn completion_prefix(document: &Document, byte: usize, byte_u32: u32) -> (&str, usize) {
    let Some(token) = document.parsed().syntax.tokens().iter().find(|token| {
        (token.kind == TokenKind::Identifier || token.kind.is_keyword())
            && token.range.start() <= byte_u32
            && byte_u32 <= token.range.end()
    }) else {
        return ("", byte);
    };
    let start = usize::try_from(token.range.start()).unwrap_or(byte);
    document
        .text()
        .get(start..byte)
        .map_or(("", byte), |prefix| (prefix, start))
}

fn token_at(document: &Document, byte: u32) -> Option<&Token> {
    let tokens = document.parsed().syntax.tokens();
    tokens
        .iter()
        .find(|token| !token.kind.is_trivia() && token.range.contains(byte))
        .or_else(|| {
            tokens.iter().rev().find(|token| {
                !token.kind.is_trivia()
                    && token.kind != TokenKind::Eof
                    && !token.range.is_empty()
                    && token.range.end() == byte
            })
        })
}

fn operator_description(kind: TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::Plus => Some("addition or unary plus operator"),
        TokenKind::Minus => Some("subtraction or unary minus operator"),
        TokenKind::Star => Some("matrix multiplication operator"),
        TokenKind::Slash => Some("matrix right-division operator"),
        TokenKind::Backslash => Some("matrix left-division operator"),
        TokenKind::Caret => Some("matrix power operator"),
        TokenKind::DotStar => Some("element-wise multiplication operator"),
        TokenKind::DotSlash => Some("element-wise right-division operator"),
        TokenKind::DotBackslash => Some("element-wise left-division operator"),
        TokenKind::DotCaret => Some("element-wise power operator"),
        TokenKind::Equal => Some("assignment operator"),
        TokenKind::EqualEqual => Some("equality operator"),
        TokenKind::NotEqual => Some("inequality operator"),
        TokenKind::Less | TokenKind::LessEqual | TokenKind::Greater | TokenKind::GreaterEqual => {
            Some("relational operator")
        }
        TokenKind::And | TokenKind::AndAnd => Some("logical AND operator"),
        TokenKind::Or | TokenKind::OrOr => Some("logical OR operator"),
        TokenKind::Tilde => Some("logical NOT or ignored-output operator"),
        TokenKind::Colon => Some("range or full-index operator"),
        TokenKind::ConjugateTranspose => Some("complex-conjugate transpose operator"),
        TokenKind::DotTranspose => Some("nonconjugate transpose operator"),
        TokenKind::Dot => Some("field access operator"),
        TokenKind::At => Some("function-handle operator"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SemanticEnvironment, code_actions, completion, diagnostics, document_symbols, formatting,
        hover, resolve_completion, semantic_classifications, semantic_tokens, workspace_symbols,
    };
    use crate::document::{Document, DocumentStore};
    use crate::protocol::{CompletionKind, Position, SemanticTokenType, SymbolKind};

    const URI: &str = "file:///workspace/demo.m";

    #[test]
    fn diagnostic_ranges_use_utf16_after_crlf() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "x = 1;\r\n😀")
            .expect("document should open");
        let diagnostic = diagnostics(document)
            .into_iter()
            .find(|item| item.code.as_deref() == Some("OML0002"))
            .expect("emoji should be an invalid token");
        assert_eq!(diagnostic.range.start, Position::new(1, 0));
        assert_eq!(diagnostic.range.end, Position::new(1, 2));
    }

    #[test]
    fn valid_frontend_constructs_do_not_publish_diagnostics() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                "matrix = [1, 2];\r\nrange = 1:3;\r\ncell_value = {1};\r\n",
            )
            .expect("document should open");
        assert!(diagnostics(document).is_empty());
    }

    #[test]
    fn frontend_errors_suppress_secondary_compiler_noise() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "value = @;\r\n")
            .expect("document should open");
        let diagnostics = diagnostics(document);
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.iter().all(|item| {
            !item
                .code
                .as_deref()
                .is_some_and(|code| code.starts_with("OMC"))
        }));
    }

    #[test]
    fn symbols_include_class_properties_methods_and_functions() {
        let text = "function y = top(x)\n y = x;\nend\nclassdef Counter\nproperties\nvalue = 0\nend\nmethods\nfunction y = add(x)\ny = x;\nend\nend\nend\n";
        let mut store = DocumentStore::new();
        let document = store.did_open(URI, 1, text).expect("document should open");
        let symbols = document_symbols(document);
        assert_eq!(symbols[0].name, "top");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[1].name, "Counter");
        assert_eq!(symbols[1].kind, SymbolKind::Class);
        assert!(
            symbols[1]
                .children
                .iter()
                .any(|symbol| { symbol.name == "value" && symbol.kind == SymbolKind::Property })
        );
        assert!(
            symbols[1]
                .children
                .iter()
                .any(|symbol| symbol.name == "add" && symbol.kind == SymbolKind::Method)
        );
        assert!(
            symbols
                .iter()
                .flat_map(|symbol| std::iter::once(symbol).chain(symbol.children.iter()))
                .all(|symbol| symbol.range.start <= symbol.range.end)
        );
    }

    #[test]
    fn document_symbols_visit_try_and_catch_bodies() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "try\n",
                    "function value = protected_helper(input)\n",
                    "value = input;\n",
                    "end\n",
                    "catch caught\n",
                    "function value = recovery_helper(input)\n",
                    "value = input;\n",
                    "end\n",
                    "end\n",
                ),
            )
            .expect("try/catch symbols document");

        let symbols = document_symbols(document);
        assert_eq!(
            symbols
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["protected_helper", "recovery_helper"]
        );
    }

    #[test]
    fn try_catch_definitions_feed_hover_and_completion() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "try\n",
                    "  try_value = 1;\n",
                    "  try\n",
                    "  catch nested_error\n",
                    "    nested_value = nested_error;\n",
                    "  end\n",
                    "catch handled\n",
                    "  catch_value = handled;\n",
                    "end\n",
                    "after = handled;\n",
                ),
            )
            .expect("try/catch definitions document");

        for name in [
            "try_value",
            "nested_error",
            "nested_value",
            "handled",
            "catch_value",
        ] {
            let resolution = resolve_completion(document, name, Position::new(9, 0))
                .unwrap_or_else(|| panic!("missing completion definition for {name}"));
            assert!(resolution.detail.contains("variable"));
        }
        let handled = hover(document, Position::new(9, 10)).expect("post-catch binding hover");
        assert!(handled.markdown.contains("local variable"));
    }

    #[test]
    fn catch_binding_semantics_override_same_named_global() {
        const MAIN_URI: &str = "file:///workspace/catch-semantic.m";
        let text = concat!(
            "try\n",
            "  protected = 1;\n",
            "catch helper\n",
            "  inside = helper;\n",
            "end\n",
            "after = helper;\n",
        );
        let mut store = DocumentStore::new();
        store
            .did_open(
                "file:///workspace/helper.m",
                1,
                "function out = helper(input)\nout = input;\nend\n",
            )
            .expect("global helper document");
        store
            .did_open(MAIN_URI, 1, text)
            .expect("catch semantic document");
        let document = store.get(MAIN_URI).expect("open catch semantic document");
        let environment = SemanticEnvironment::from_documents(&store);
        let classifications = semantic_classifications(document, &environment);

        for (start, _) in text.match_indices("helper") {
            let start = u32::try_from(start).expect("small test offset");
            assert_eq!(
                classifications.get(&(start, start + 6)),
                Some(&SemanticTokenType::Variable)
            );
        }
    }

    #[test]
    fn declarations_feed_diagnostics_completion_and_semantic_classification() {
        const MAIN_URI: &str = "file:///workspace/declarations.m";
        let text = concat!(
            "global helper shared\n",
            "shared = helper;\n",
            "persistent invalid_script\n",
            "function y = worker(input)\n",
            "  persistent cache state\n",
            "  global helper\n",
            "  switch input\n",
            "  case 1\n",
            "    persistent branch_cache\n",
            "  otherwise\n",
            "    global branch_global\n",
            "  end\n",
            "  cache = input;\n",
            "  y = helper + cache + branch_cache + branch_global;\n",
            "end\n",
        );
        let mut store = DocumentStore::new();
        store
            .did_open(
                "file:///workspace/helper.m",
                1,
                "function out = helper(input)\nout = input;\nend\n",
            )
            .expect("global helper document");
        store
            .did_open(MAIN_URI, 1, text)
            .expect("declaration semantic document");
        let document = store.get(MAIN_URI).expect("open declaration document");
        let environment = SemanticEnvironment::from_documents(&store);
        let classifications = semantic_classifications(document, &environment);

        assert_eq!(
            diagnostics(document)
                .iter()
                .filter(|diagnostic| diagnostic.code.as_deref() == Some("OMH0004"))
                .count(),
            1
        );
        assert!(
            resolve_completion(document, "cache", Position::new(13, 0))
                .expect("persistent completion")
                .detail
                .contains("persistent variable declaration")
        );
        assert!(
            resolve_completion(document, "helper", Position::new(13, 0))
                .expect("global completion")
                .detail
                .contains("global variable declaration")
        );
        assert!(
            resolve_completion(document, "branch_cache", Position::new(13, 0))
                .expect("nested persistent completion")
                .detail
                .contains("persistent variable declaration")
        );
        assert!(
            resolve_completion(document, "branch_global", Position::new(13, 0))
                .expect("nested global completion")
                .detail
                .contains("global variable declaration")
        );
        for (start, _) in text.match_indices("helper") {
            let start = u32::try_from(start).expect("small test offset");
            assert_eq!(
                classifications.get(&(start, start + 6)),
                Some(&SemanticTokenType::Variable)
            );
        }
        assert!(
            semantic_tokens(&store, document)
                .chunks_exact(5)
                .any(|token| token[3] == SemanticTokenType::Variable.lsp_index())
        );
    }

    #[test]
    fn malformed_and_context_invalid_declarations_publish_stable_frontend_diagnostics() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                "file:///workspace/invalid-declarations.m",
                1,
                concat!(
                    "global\n",
                    "persistent script_cache\n",
                    "function worker()\n",
                    "  persistent cache(1) recovered\n",
                    "end\n",
                ),
            )
            .expect("invalid declaration document");
        let published = diagnostics(document);
        let codes = published
            .iter()
            .filter_map(|diagnostic| diagnostic.code.as_deref())
            .collect::<Vec<_>>();

        assert!(codes.contains(&"OMP0007"));
        assert!(codes.contains(&"OMP0008"));
        assert!(codes.contains(&"OMH0004"));
    }

    #[test]
    fn empty_and_nested_try_catch_preserve_frontend_diagnostics_without_panics() {
        let valid = concat!(
            "try\n",
            "end\n",
            "try\n",
            "catch empty_error\n",
            "end\n",
            "try\n",
            "  try\n",
            "  catch nested_error\n",
            "  end\n",
            "catch outer_error\n",
            "end\n",
        );
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, valid)
            .expect("valid empty and nested try/catch document");
        assert!(document.parsed().diagnostics.is_empty());
        assert!(document.lowered().diagnostics.is_empty());

        let invalid = store
            .did_open(
                "file:///workspace/invalid-catch.m",
                1,
                "try\ncatch caught\n\u{1f600}\nend\n",
            )
            .expect("diagnostic try/catch document");
        assert!(
            diagnostics(invalid)
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OML0002"))
        );
    }

    #[test]
    fn hover_classifies_keyword_operator_and_defined_symbol() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "function y = calc(x)\ny = x + 1;\nend\n")
            .expect("document should open");
        let keyword = hover(document, Position::new(0, 2)).expect("keyword hover");
        let function = hover(document, Position::new(0, 14)).expect("function hover");
        let operator = hover(document, Position::new(1, 6)).expect("operator hover");
        assert!(keyword.markdown.contains("keyword"));
        assert!(function.markdown.contains("function"));
        assert!(operator.markdown.contains("addition"));
    }

    #[test]
    fn completion_includes_keywords_and_document_definitions_with_edit_ranges() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 1, "function y = calculate(x)\ny = x;\nend\ncal")
            .expect("document should open");
        let items = completion(document, Position::new(3, 3));
        let calculate = items
            .iter()
            .find(|item| item.label == "calculate")
            .expect("defined function completion");
        assert_eq!(calculate.kind, CompletionKind::Function);
        assert_eq!(calculate.data.uri, URI);
        assert_eq!(calculate.data.version, 1);
        assert_eq!(calculate.text_edit.range.start, Position::new(3, 0));
        assert_eq!(calculate.text_edit.range.end, Position::new(3, 3));
        let resolved = resolve_completion(document, "calculate", calculate.text_edit.range.end)
            .expect("completion resolution");
        assert!(resolved.detail.contains("function"));
        assert!(resolved.markdown.contains("current document"));

        let keyword = resolve_completion(document, "function", Position::new(3, 0))
            .expect("keyword resolution");
        assert!(keyword.detail.contains("keyword"));
        assert!(
            resolve_completion(document, "unknown_completion_name", Position::new(3, 0)).is_none()
        );

        let keywords = completion(document, Position::new(3, 0));
        assert!(keywords.iter().any(|item| item.label == "function"));
        assert!(keywords.iter().all(|item| {
            item.text_edit.range.start == Position::new(3, 0)
                && item.text_edit.range.end == Position::new(3, 0)
        }));
    }

    #[test]
    fn completion_discovers_the_registered_core_and_resolves_fft() {
        let mut store = DocumentStore::new();
        let document = store.did_open(URI, 7, "f").expect("open prefix");
        let items = completion(document, Position::new(0, 1));
        for name in ["fft", "fft2", "fftn", "fftshift", "function"] {
            assert!(
                items.iter().any(|item| item.label == name),
                "missing {name}"
            );
        }
        assert!(items.iter().all(|item| item.label.starts_with('f')));
        let fft = items.iter().find(|item| item.label == "fft").unwrap();
        assert_eq!(fft.kind, CompletionKind::Function);
        assert_eq!(fft.detail, "built-in function");
        assert_eq!(fft.data.version, 7);
        assert_eq!(fft.text_edit.new_text, "fft");
        assert_eq!(fft.text_edit.range.start, Position::new(0, 0));
        assert_eq!(fft.text_edit.range.end, Position::new(0, 1));
        let resolved =
            resolve_completion(document, "fft", fft.text_edit.range.end).expect("resolve built-in");
        assert!(resolved.markdown.contains("core runtime"));

        let all = completion(document, Position::new(0, 0));
        let registry = openmat_builtins::minimal_registry().expect("core registry");
        for name in registry.names() {
            assert!(all.iter().any(|item| item.label == name), "missing {name}");
        }

        let document = store.did_change(URI, 8, "fft([1 2]);").expect("edit call");
        assert!(
            hover(document, Position::new(0, 1))
                .unwrap()
                .markdown
                .contains("built-in function")
        );
    }

    #[test]
    fn local_definitions_override_builtin_completion_and_hover() {
        for (text, line, kind) in [
            ("fft = [1 2];\nf", 1, CompletionKind::Variable),
            (
                "function y = fft(x)\ny = x;\nend\nf",
                3,
                CompletionKind::Function,
            ),
        ] {
            let mut store = DocumentStore::new();
            let document = store.did_open(URI, 1, text).expect("open shadowing source");
            let items = completion(document, Position::new(line, 1));
            let matching: Vec<_> = items.iter().filter(|item| item.label == "fft").collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0].kind, kind);
            assert!(!matching[0].detail.contains("built-in"));
            assert!(
                resolve_completion(document, "fft", matching[0].text_edit.range.end)
                    .unwrap()
                    .markdown
                    .contains("current document")
            );
        }
        let mut store = DocumentStore::new();
        let document = store.did_open(URI, 1, "fft = [1 2];\nfft(1)").unwrap();
        assert!(
            hover(document, Position::new(1, 1))
                .unwrap()
                .markdown
                .contains("variable")
        );
    }

    fn assert_binding_at(
        document: &Document,
        context: &str,
        name: &str,
        kind: CompletionKind,
        detail: &str,
    ) {
        assert!(context.ends_with(name));
        let byte =
            u32::try_from(document.text().find(context).expect("test context") + context.len())
                .expect("small fixture");
        let position = document.byte_to_position(byte);
        let item = completion(document, position)
            .into_iter()
            .find(|item| item.label == name)
            .unwrap_or_else(|| panic!("missing {name} at {context}"));
        assert_eq!(item.kind, kind, "completion kind at {context}");
        assert_eq!(item.detail, detail, "completion detail at {context}");
        assert!(
            hover(document, document.byte_to_position(byte - 1))
                .expect("identifier hover")
                .markdown
                .contains(detail),
            "hover at {context}"
        );
        assert!(
            resolve_completion(document, name, item.text_edit.range.end)
                .expect("resolved completion")
                .detail
                .contains(detail),
            "resolve at {context}"
        );
    }

    #[test]
    fn sibling_function_parameters_do_not_shadow_builtins() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "function y = first(input)\n",
                    "  first_value = fft(input);\n",
                    "  y = first_value;\n",
                    "end\n",
                    "function y = second(fft)\n",
                    "  second_value = fft(1);\n",
                    "  y = second_value;\n",
                    "end\n",
                ),
            )
            .unwrap();
        assert_binding_at(
            document,
            "first_value = fft",
            "fft",
            CompletionKind::Function,
            "built-in function",
        );
        assert_binding_at(
            document,
            "second_value = fft",
            "fft",
            CompletionKind::Variable,
            "function parameter",
        );
        let first_names: Vec<_> = completion(document, Position::new(1, 0))
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert!(first_names.iter().any(|name| name == "second"));
        assert!(!first_names.iter().any(|name| name == "second_value"));
        let second_names = completion(document, Position::new(5, 0));
        assert!(
            !second_names
                .iter()
                .any(|item| item.label == "input" || item.label == "first_value")
        );
    }

    #[test]
    fn nested_functions_inherit_lexical_bindings_without_leaking_sibling_locals() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "function y = outer(fft)\n",
                    "  shared = 1;\n",
                    "  parent_value = fft(1);\n",
                    "  function z = child(input, shared)\n",
                    "    fft = input + shared;\n",
                    "    child_value = fft(1);\n",
                    "    captured = parent_value;\n",
                    "    z = captured;\n",
                    "  end\n",
                    "  function z = sibling(other)\n",
                    "    sibling_value = fft(1);\n",
                    "    z = sibling_value;\n",
                    "  end\n",
                    "  y = parent_value;\n",
                    "end\n",
                    "function y = unrelated(x)\n",
                    "  outside_value = fft(x);\n",
                    "  y = outside_value;\n",
                    "end\n",
                ),
            )
            .unwrap();
        assert_binding_at(
            document,
            "parent_value = fft",
            "fft",
            CompletionKind::Variable,
            "function parameter",
        );
        assert_binding_at(
            document,
            "child_value = fft",
            "fft",
            CompletionKind::Variable,
            "function parameter",
        );
        assert_binding_at(
            document,
            "captured = parent_value",
            "parent_value",
            CompletionKind::Variable,
            "local variable",
        );
        assert_binding_at(
            document,
            "fft = input + shared",
            "shared",
            CompletionKind::Variable,
            "function parameter",
        );
        assert_binding_at(
            document,
            "sibling_value = fft",
            "fft",
            CompletionKind::Variable,
            "function parameter",
        );
        assert_binding_at(
            document,
            "outside_value = fft",
            "fft",
            CompletionKind::Function,
            "built-in function",
        );
        let sibling_names = completion(document, Position::new(10, 0));
        for visible in ["outer", "shared", "child", "sibling", "other"] {
            assert!(
                sibling_names.iter().any(|item| item.label == visible),
                "missing {visible}"
            );
        }
        for hidden in ["input", "child_value", "captured"] {
            assert!(
                !sibling_names.iter().any(|item| item.label == hidden),
                "leaked {hidden}"
            );
        }
        let unrelated_names = completion(document, Position::new(16, 0));
        for hidden in ["child", "sibling", "shared", "other"] {
            assert!(
                !unrelated_names.iter().any(|item| item.label == hidden),
                "leaked {hidden}"
            );
        }
    }

    #[test]
    fn scripts_and_local_functions_have_separate_variable_scopes() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "fft = [1 2];\n",
                    "script_value = fft(1);\n",
                    "function y = helper(local_input)\n",
                    "  local_value = fft(local_input);\n",
                    "  y = local_value;\n",
                    "end\n",
                ),
            )
            .unwrap();
        assert_binding_at(
            document,
            "script_value = fft",
            "fft",
            CompletionKind::Variable,
            "local variable",
        );
        assert_binding_at(
            document,
            "local_value = fft",
            "fft",
            CompletionKind::Function,
            "built-in function",
        );
        let script_names = completion(document, Position::new(1, 0));
        assert!(script_names.iter().any(|item| item.label == "helper"));
        assert!(
            !script_names
                .iter()
                .any(|item| item.label == "local_input" || item.label == "local_value")
        );
        assert!(
            !completion(document, Position::new(3, 0))
                .iter()
                .any(|item| item.label == "script_value")
        );
    }

    #[test]
    fn methods_isolate_locals_and_do_not_treat_properties_as_bare_variables() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                1,
                concat!(
                    "classdef Example\n",
                    "properties\n",
                    "  fft = 1\n",
                    "end\n",
                    "methods\n",
                    "  function y = first(obj, input)\n",
                    "    first_value = fft(input);\n",
                    "    y = first_value;\n",
                    "  end\n",
                    "  function y = second(obj, fft)\n",
                    "    second_value = fft(1);\n",
                    "    y = second_value;\n",
                    "  end\n",
                    "end\n",
                    "end\n",
                ),
            )
            .unwrap();
        assert_binding_at(
            document,
            "properties\n  fft",
            "fft",
            CompletionKind::Property,
            "property",
        );
        assert_binding_at(
            document,
            "first_value = fft",
            "fft",
            CompletionKind::Function,
            "built-in function",
        );
        assert_binding_at(
            document,
            "second_value = fft",
            "fft",
            CompletionKind::Variable,
            "function parameter",
        );
        assert!(
            !completion(document, Position::new(6, 0))
                .iter()
                .any(|item| item.label == "second_value")
        );
        assert!(
            !completion(document, Position::new(10, 0))
                .iter()
                .any(|item| item.label == "input")
        );
    }

    #[test]
    fn incomplete_function_at_eof_keeps_its_parameters_and_multiple_outputs() {
        let mut store = DocumentStore::new();
        let text = "function y = unfinished(fft)\n[left, right] = size(fft);\n  ";
        let document = store.did_open(URI, 1, text).unwrap();
        let items = completion(document, Position::new(2, 2));
        for name in ["fft", "y", "left", "right"] {
            assert!(
                items
                    .iter()
                    .any(|item| item.label == name && item.kind == CompletionKind::Variable),
                "missing {name}"
            );
        }
        assert!(
            resolve_completion(document, "fft", Position::new(2, 2))
                .unwrap()
                .detail
                .contains("function parameter")
        );
    }

    #[test]
    fn eof_belongs_only_to_scopes_missing_their_own_end() {
        for (text, expected_detail) in [
            (
                "function y = helper(fft)\ny = fft;\nend;",
                "built-in function",
            ),
            (
                "classdef Example\nproperties\nfft = 1\nend\nend;",
                "built-in function",
            ),
            (
                "function y = parent()\nfft = 1;\nfunction z = child(fft)\nz = fft;\nend;",
                "local variable",
            ),
        ] {
            let mut store = DocumentStore::new();
            let document = store.did_open(URI, 1, text).unwrap();
            let position = document.byte_to_position(u32::try_from(text.len()).unwrap());
            let item = completion(document, position)
                .into_iter()
                .find(|item| item.label == "fft")
                .unwrap();
            assert_eq!(item.detail, expected_detail, "{text}");
            assert!(
                resolve_completion(document, "fft", position)
                    .unwrap()
                    .detail
                    .contains(expected_detail),
                "{text}"
            );
        }
    }

    #[test]
    fn invalid_positions_return_no_language_result() {
        let mut store = DocumentStore::new();
        let document = store.did_open(URI, 1, "x = 1;").expect("open");
        assert!(hover(document, Position::new(99, 0)).is_none());
        assert!(completion(document, Position::new(99, 0)).is_empty());
    }

    #[test]
    fn semantic_tokens_cover_legend_and_use_valid_utf16_deltas() {
        let text = "classdef Counter\r\nproperties\r\nvalue = 1\r\nend\r\nmethods\r\nfunction out = add(obj, input)\r\n% 😀\r\nlocal = \"keep\";\r\nout = obj.value + input + local;\r\nend\r\nend\r\nend\r\nfunction y = helper(x)\r\ny = x;\r\nend\r\n";
        let mut store = DocumentStore::new();
        store.did_open(URI, 1, text).expect("open semantic source");
        let document = store.get(URI).expect("open document");
        let data = semantic_tokens(&store, document);
        assert_eq!(data.len() % 5, 0);

        let mut line = 0;
        let mut start = 0;
        let mut previous = None;
        let mut kinds = std::collections::BTreeSet::new();
        let mut comment_length = None;
        for token in data.chunks_exact(5) {
            line += token[0];
            start = if token[0] == 0 {
                start + token[1]
            } else {
                token[1]
            };
            assert!(token[2] > 0);
            kinds.insert(token[3]);
            if token[3] == SemanticTokenType::Comment.lsp_index() {
                comment_length = Some(token[2]);
            }
            let absolute = (line, start);
            assert!(previous.is_none_or(|previous| previous <= absolute));
            previous = Some(absolute);
        }
        assert_eq!(kinds, (0..=10).collect());
        assert_eq!(comment_length, Some(4));
    }

    #[test]
    fn workspace_symbols_filter_sort_and_preserve_duplicate_locations() {
        let mut store = DocumentStore::new();
        store
            .did_open("file:///z.m", 1, "function y = duplicate(x)\ny = x;\nend\n")
            .expect("z document");
        store
            .did_open(
                "file:///a.m",
                1,
                "function y = duplicate(x)\ny = x;\nend\nclassdef Holder\nproperties\nvalue = 1\nend\nend\n",
            )
            .expect("a document");

        let duplicate = workspace_symbols(&store, "PLIC");
        assert_eq!(duplicate.len(), 2);
        assert_eq!(duplicate[0].location.uri, "file:///a.m");
        assert_eq!(duplicate[1].location.uri, "file:///z.m");

        let value = workspace_symbols(&store, "value");
        assert_eq!(value.len(), 1);
        assert_eq!(value[0].container_name.as_deref(), Some("Holder"));
        assert_eq!(value[0].kind, SymbolKind::Property);
    }

    #[test]
    fn formatting_preserves_comment_and_string_contents() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(
                URI,
                4,
                "value = \"keep  \";   \r\n% keep comment   \r\nnext = 1;  ",
            )
            .expect("format document");
        let edits = formatting(document);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(0, 0));
        assert_eq!(
            edits[0].new_text,
            "value = \"keep  \";\n% keep comment   \nnext = 1;\n"
        );

        let unchanged = store
            .did_change(URI, 5, &edits[0].new_text)
            .expect("formatted update");
        assert!(formatting(unchanged).is_empty());
    }

    #[test]
    fn code_action_only_fixes_confirmed_current_unmatched_end() {
        let mut store = DocumentStore::new();
        let document = store
            .did_open(URI, 8, "end\r\nvalue = 1;\r\n")
            .expect("quick-fix document");
        let diagnostic = diagnostics(document)
            .into_iter()
            .find(|diagnostic| diagnostic.code.as_deref() == Some("OMP0002"))
            .expect("unmatched end diagnostic");
        let actions = code_actions(
            document,
            diagnostic.range,
            std::slice::from_ref(&diagnostic),
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "quickfix");
        assert!(actions[0].is_preferred);
        assert_eq!(actions[0].edit.document_changes[0].version, Some(8));
        assert_eq!(actions[0].edit.document_changes[0].edits[0].new_text, "");
        assert!(code_actions(document, diagnostic.range, &[]).is_empty());
    }
}
