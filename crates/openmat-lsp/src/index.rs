//! Deterministic symbol navigation over editor buffers and workspace sources.
//!
//! File visibility follows the runtime's caller-private, current-directory,
//! package and search-path ordering. Object member types and nested captures
//! remain deliberately unresolved.

use crate::document::{Document, DocumentStore};
use crate::protocol::{FileRename, Location, Position, PrepareRename, TextEdit, WorkspaceEdit};
use openmat_hir::{
    ClassDef, DeclarationContext, DeclarationForm, DeclarationKind, DeclarationStatement, Expr,
    ExprKind, FunctionDef, Name, Stmt, StmtKind,
};
use openmat_source::TextRange;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlobalKind {
    Function,
    Class,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Namespace {
    Global,
    File,
    Local { scope: usize },
    Declared { scope: usize },
    Isolated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeBindingKind {
    Ordinary,
    Global,
    Persistent,
    ConflictingDeclarations,
}

#[derive(Clone, Debug)]
struct IndexedOccurrence {
    location: Location,
    is_definition: bool,
    scope: Option<usize>,
}

#[derive(Clone, Debug)]
struct IndexedSymbol {
    name: String,
    definition: Location,
    occurrences: Vec<IndexedOccurrence>,
    namespace: Namespace,
    global_kind: Option<GlobalKind>,
    renameable: bool,
    ambiguous: bool,
}

#[derive(Clone, Copy, Debug)]
struct OccurrenceLookup {
    symbol: usize,
    occurrence: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SymbolIndex {
    symbols: Vec<IndexedSymbol>,
    occurrences: Vec<OccurrenceLookup>,
    globals: BTreeMap<String, Vec<usize>>,
    file_symbols: BTreeMap<String, BTreeMap<String, Vec<usize>>>,
    workspace: Option<WorkspaceResolution>,
    scope_names: BTreeMap<usize, BTreeSet<String>>,
    scope_documents: BTreeMap<usize, String>,
    next_scope: usize,
}

impl SymbolIndex {
    pub(crate) fn rebuild(&mut self, documents: &DocumentStore) {
        *self = Self::build(documents);
    }

    fn build(documents: &DocumentStore) -> Self {
        let workspace = documents
            .workspace_context()
            .map(|context| WorkspaceResolution {
                current_directory: normalize_path(&context.current_directory),
                search_paths: context
                    .search_paths
                    .iter()
                    .map(|path| normalize_path(path))
                    .collect(),
                paths: documents
                    .iter_effective()
                    .filter_map(|(uri, _)| {
                        documents
                            .workspace_relative_path(uri)
                            .map(|path| (uri.to_owned(), normalize_path(&path)))
                    })
                    .collect(),
                files: BTreeMap::new(),
            });
        let mut index = Self {
            workspace,
            ..Self::default()
        };
        for (_, document) in documents.iter_effective() {
            index.collect_globals(document);
        }
        if index.workspace.is_none() {
            for symbols in index.globals.values() {
                let ambiguous = symbols.len() != 1;
                for symbol in symbols {
                    index.symbols[*symbol].ambiguous = ambiguous;
                }
            }
        }
        for (_, document) in documents.iter_effective() {
            index.index_document(document);
        }
        index.finish();
        index
    }

    pub(crate) fn definition(&self, uri: &str, position: Position) -> Option<Location> {
        let symbol = self.symbol_at(uri, position)?;
        let symbol = self.symbols.get(symbol)?;
        (!symbol.ambiguous).then(|| symbol.definition.clone())
    }

    pub(crate) fn references(
        &self,
        uri: &str,
        position: Position,
        include_definition: bool,
    ) -> Option<Vec<Location>> {
        let symbol = self.symbols.get(self.symbol_at(uri, position)?)?;
        if symbol.ambiguous {
            return None;
        }
        Some(
            symbol
                .occurrences
                .iter()
                .filter(|occurrence| include_definition || !occurrence.is_definition)
                .map(|occurrence| occurrence.location.clone())
                .collect(),
        )
    }

    pub(crate) fn prepare_rename(&self, uri: &str, position: Position) -> Option<PrepareRename> {
        let lookup = self.lookup_at(uri, position)?;
        let symbol = self.symbols.get(lookup.symbol)?;
        let occurrence = symbol.occurrences.get(lookup.occurrence)?;
        (symbol.renameable
            && !symbol.ambiguous
            && self.rename_block_reason(uri, position).is_none())
        .then(|| PrepareRename {
            range: occurrence.location.range,
            placeholder: symbol.name.clone(),
        })
    }

    pub(crate) fn rename_block_reason(
        &self,
        uri: &str,
        position: Position,
    ) -> Option<&'static str> {
        let symbol = self.symbols.get(self.symbol_at(uri, position)?)?;
        let workspace = self.workspace.as_ref()?;
        let path = workspace.paths.get(&symbol.definition.uri)?;
        let class_constructor = last_component(parent_path(path))
            .strip_prefix('@')
            .is_some_and(|class| class == symbol.name);
        (symbol.namespace == Namespace::Global && (symbol.global_kind == Some(GlobalKind::Class) || class_constructor)).then_some(
            "Renaming workspace classes requires coordinated class files and App Designer metadata changes, which are not supported yet.",
        )
    }

    pub(crate) fn rename(
        &self,
        documents: &DocumentStore,
        uri: &str,
        position: Position,
        new_name: &str,
    ) -> Option<WorkspaceEdit> {
        let symbol_id = self.symbol_at(uri, position)?;
        let symbol = self.symbols.get(symbol_id)?;
        if symbol.ambiguous
            || !symbol.renameable
            || self
                .rename_target_block_reason(uri, position, new_name)
                .is_some()
            || !is_valid_identifier(new_name)
            || !self.rename_is_collision_free(symbol_id, new_name)
        {
            return None;
        }

        let mut changes = BTreeMap::<(String, Option<i32>), Vec<TextEdit>>::new();
        for occurrence in &symbol.occurrences {
            documents.get_effective(&occurrence.location.uri)?;
            let version = documents.edit_version(&occurrence.location.uri);
            changes
                .entry((occurrence.location.uri.clone(), version))
                .or_default()
                .push(TextEdit {
                    range: occurrence.location.range,
                    new_text: new_name.to_owned(),
                });
        }
        let mut edit = WorkspaceEdit::from_changes(changes);
        if symbol.namespace == Namespace::Global
            && symbol.global_kind == Some(GlobalKind::Function)
            && symbol.name != new_name
            && let Some(workspace) = &self.workspace
        {
            let path = workspace.paths.get(&symbol.definition.uri)?;
            let new_path = join_path(parent_path(path), &format!("{new_name}.m"));
            if path_key(path) != path_key(&new_path)
                && workspace.files.contains_key(&path_key(&new_path))
            {
                return None;
            }
            if self.introduced_file_captures_name(workspace, &new_path, new_name) {
                return None;
            }
            let (parent_uri, _) = symbol.definition.uri.rsplit_once('/')?;
            edit.file_renames.push(FileRename {
                old_uri: symbol.definition.uri.clone(),
                new_uri: format!("{parent_uri}/{}.m", encode_uri_segment(new_name)),
            });
        }
        Some(edit)
    }

    pub(crate) fn rename_target_block_reason(
        &self,
        uri: &str,
        position: Position,
        new_name: &str,
    ) -> Option<&'static str> {
        if let Some(reason) = self.rename_block_reason(uri, position) {
            return Some(reason);
        }
        let symbol = self.symbols.get(self.symbol_at(uri, position)?)?;
        (self.workspace.is_some()
            && symbol.namespace == Namespace::Global
            && symbol.global_kind == Some(GlobalKind::Function)
            && symbol.name != new_name
            && symbol.name.to_lowercase() == new_name.to_lowercase())
        .then_some("Renaming a primary function by letter case alone requires a case-preserving file move, which is not supported yet.")
    }

    fn collect_globals(&mut self, document: &Document) {
        if self.workspace.is_some() {
            self.collect_workspace_declarations(document);
            return;
        }
        for statement in &document.lowered().file.statements {
            let declaration = match &statement.kind {
                StmtKind::Function(function) => function
                    .name
                    .as_ref()
                    .map(|name| (name, GlobalKind::Function)),
                StmtKind::Class(class) => class.name.as_ref().map(|name| (name, GlobalKind::Class)),
                _ => None,
            };
            let Some((name, kind)) = declaration else {
                continue;
            };
            let symbol = self.add_symbol(document, name, Namespace::Global, Some(kind), true, None);
            self.globals
                .entry(name.text.clone())
                .or_default()
                .push(symbol);
        }
    }

    fn collect_workspace_declarations(&mut self, document: &Document) {
        let path = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.paths.get(document.uri()))
            .cloned();
        let primary_name = path
            .as_deref()
            .and_then(|path| path.rsplit('/').next())
            .and_then(matlab_file_stem);
        let mut primary = None;
        for (position, statement) in document.lowered().file.statements.iter().enumerate() {
            let declaration = match &statement.kind {
                StmtKind::Function(function) => function
                    .name
                    .as_ref()
                    .map(|name| (name, GlobalKind::Function)),
                StmtKind::Class(class) => class.name.as_ref().map(|name| (name, GlobalKind::Class)),
                _ => None,
            };
            let Some((name, kind)) = declaration else {
                continue;
            };
            // A function in a script, a later local function, or a declaration
            // whose name differs from its file cannot be a workspace export.
            let exported = position == 0
                && primary_name.is_some_and(|stem| path_key(stem) == path_key(&name.text));
            let namespace = if exported {
                Namespace::Global
            } else {
                Namespace::File
            };
            let symbol = self.add_symbol(document, name, namespace, Some(kind), true, None);
            self.file_symbols
                .entry(document.uri().to_owned())
                .or_default()
                .entry(name.text.clone())
                .or_default()
                .push(symbol);
            if exported {
                primary = Some(symbol);
                self.globals
                    .entry(name.text.clone())
                    .or_default()
                    .push(symbol);
            }
        }
        if let Some(path) = path {
            // An indexed script or invalid primary declaration shadows a later
            // search-path candidate too, even though it has no callable symbol.
            self.workspace
                .as_mut()
                .expect("workspace resolution")
                .files
                .entry(path_key(&path))
                .and_modify(|existing| *existing = None)
                .or_insert(primary);
        }
        if let Some(names) = self.file_symbols.get(document.uri()) {
            for symbols in names.values().filter(|symbols| symbols.len() > 1) {
                for symbol in symbols {
                    self.symbols[*symbol].ambiguous = true;
                }
            }
        }
    }

    fn index_document(&mut self, document: &Document) {
        let mut scope = self.make_scope(document, &[], &document.lowered().file.statements, true);
        // Direct root declarations are the globals this scope is allowed to resolve.
        // Only declarations nested inside functions need to block global fallback.
        scope.blocked_globals.clear();
        self.index_statements(
            document,
            &document.lowered().file.statements,
            &scope,
            StatementContext::Root,
        );
    }

    fn make_scope(
        &mut self,
        document: &Document,
        header_names: &[&Name],
        statements: &[Stmt],
        allow_globals: bool,
    ) -> Scope {
        let scope_id = self.next_scope;
        self.next_scope = self.next_scope.saturating_add(1);
        self.scope_documents
            .insert(scope_id, document.uri().to_owned());
        let mut declarations = BTreeMap::<String, Vec<TextRange>>::new();
        let mut binding_kinds = BTreeMap::<String, ScopeBindingKind>::new();
        let mut binding_definitions = BTreeMap::<String, TextRange>::new();
        for name in header_names {
            if is_indexable_name(&name.text) {
                declarations
                    .entry(name.text.clone())
                    .or_default()
                    .push(name.span);
                binding_kinds
                    .entry(name.text.clone())
                    .or_insert(ScopeBindingKind::Ordinary);
            }
        }
        collect_local_declarations(
            statements,
            &mut declarations,
            &mut binding_kinds,
            &mut binding_definitions,
        );
        self.scope_names
            .insert(scope_id, declarations.keys().cloned().collect());

        let mut locals = BTreeMap::new();
        for (name, mut ranges) in declarations {
            ranges.sort_by_key(|range| (range.start(), range.end()));
            ranges.dedup();
            let Some(definition) = binding_definitions
                .get(&name)
                .copied()
                .or_else(|| ranges.first().copied())
            else {
                continue;
            };
            let binding = binding_kinds
                .get(&name)
                .copied()
                .unwrap_or(ScopeBindingKind::Ordinary);
            let namespace = match binding {
                ScopeBindingKind::Ordinary => Namespace::Local { scope: scope_id },
                ScopeBindingKind::Global
                | ScopeBindingKind::Persistent
                | ScopeBindingKind::ConflictingDeclarations => {
                    Namespace::Declared { scope: scope_id }
                }
            };
            let symbol = self.add_symbol_at_range(
                document,
                &name,
                definition,
                namespace,
                None,
                true,
                Some(scope_id),
            );
            for range in ranges {
                if range != definition {
                    self.add_occurrence(symbol, document, range, false, Some(scope_id));
                }
            }
            locals.insert(name, symbol);
        }

        let mut blocked_globals = BTreeSet::new();
        collect_nested_declaration_names(statements, &mut blocked_globals);
        self.scope_names
            .entry(scope_id)
            .or_default()
            .extend(blocked_globals.iter().cloned());
        Scope {
            id: scope_id,
            locals,
            binding_kinds,
            blocked_globals,
            allow_globals,
        }
    }

    fn index_statements(
        &mut self,
        document: &Document,
        statements: &[Stmt],
        scope: &Scope,
        context: StatementContext,
    ) {
        for statement in statements {
            match &statement.kind {
                StmtKind::Assignment { target, value } => {
                    self.index_assignment_target(document, target, scope);
                    self.index_expr(document, value, scope);
                }
                StmtKind::Expr(expression) => self.index_expr(document, expression, scope),
                StmtKind::Declaration(declaration) => {
                    self.index_declaration(document, declaration, scope);
                }
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    for branch in branches {
                        self.index_expr(document, &branch.condition, scope);
                        self.index_statements(
                            document,
                            &branch.body,
                            scope,
                            StatementContext::Nested,
                        );
                    }
                    self.index_statements(document, else_body, scope, StatementContext::Nested);
                }
                StmtKind::For {
                    variable,
                    iterable,
                    body,
                } => {
                    self.index_assignment_target(document, variable, scope);
                    self.index_expr(document, iterable, scope);
                    self.index_statements(document, body, scope, StatementContext::Nested);
                }
                StmtKind::While { condition, body } => {
                    self.index_expr(document, condition, scope);
                    self.index_statements(document, body, scope, StatementContext::Nested);
                }
                StmtKind::Try(try_statement) => {
                    self.index_statements(
                        document,
                        &try_statement.body,
                        scope,
                        StatementContext::Nested,
                    );
                    if let Some(catch) = &try_statement.catch {
                        if let Some(variable) = &catch.variable {
                            self.index_name(document, &variable.text, variable.span, scope, false);
                        }
                        self.index_statements(
                            document,
                            &catch.body,
                            scope,
                            StatementContext::Nested,
                        );
                    }
                }
                StmtKind::Switch {
                    selector,
                    cases,
                    otherwise,
                } => {
                    self.index_expr(document, selector, scope);
                    for case in cases {
                        self.index_expr(document, &case.expression, scope);
                        self.index_statements(
                            document,
                            &case.body,
                            scope,
                            StatementContext::Nested,
                        );
                    }
                    if let Some(otherwise) = otherwise {
                        self.index_statements(
                            document,
                            &otherwise.body,
                            scope,
                            StatementContext::Nested,
                        );
                    }
                }
                StmtKind::Function(function) => {
                    let function_context = if context == StatementContext::Root {
                        FunctionContext::TopLevel
                    } else {
                        FunctionContext::Nested
                    };
                    self.index_function(document, function, function_context);
                }
                StmtKind::Class(class) => {
                    self.index_class(document, class, context == StatementContext::Root);
                }
                _ => {}
            }
        }
    }

    fn index_function(
        &mut self,
        document: &Document,
        function: &FunctionDef,
        context: FunctionContext,
    ) {
        if context == FunctionContext::Nested
            && let Some(name) = &function.name
        {
            self.add_symbol(document, name, Namespace::Isolated, None, false, None);
        }
        let header = function
            .inputs
            .iter()
            .chain(&function.outputs)
            .collect::<Vec<_>>();
        let scope = self.make_scope(
            document,
            &header,
            &function.body,
            context != FunctionContext::Nested,
        );
        self.index_statements(document, &function.body, &scope, StatementContext::Nested);
    }

    fn index_class(&mut self, document: &Document, class: &ClassDef, top_level: bool) {
        if !top_level && let Some(name) = &class.name {
            self.add_symbol(document, name, Namespace::Isolated, None, false, None);
        }
        if let Some(superclass) = &class.superclass
            && let Some(symbol) =
                self.resolve_global(document, &superclass.text, Some(GlobalKind::Class))
        {
            self.add_occurrence(symbol, document, leaf_name_range(superclass), false, None);
        }
        for block in &class.property_blocks {
            for property in &block.properties {
                if let Some(name) = &property.name {
                    self.add_symbol(document, name, Namespace::Isolated, None, false, None);
                }
            }
        }
        for block in &class.method_blocks {
            for method in &block.methods {
                if let Some(name) = &method.name {
                    self.add_symbol(document, name, Namespace::Isolated, None, false, None);
                }
                self.index_function(document, method, FunctionContext::Method);
            }
        }
    }

    fn index_assignment_target(&mut self, document: &Document, target: &Expr, scope: &Scope) {
        match &target.kind {
            ExprKind::Name(name) => self.index_name(document, name, target.span, scope, false),
            ExprKind::Matrix(rows) if rows.len() == 1 => {
                for element in &rows[0] {
                    if let ExprKind::Name(name) = &element.kind {
                        self.index_name(document, name, element.span, scope, false);
                    }
                }
            }
            ExprKind::ParenApply { target, arguments } => {
                // The target is an assignment receiver, not a known function call.
                // Keep it unresolved while still indexing subscript expressions.
                self.record_receiver_names(target, scope.id);
                for argument in arguments {
                    self.index_expr(document, argument, scope);
                }
            }
            ExprKind::Field { target, .. } => self.index_receiver(document, target, scope),
            _ => {}
        }
    }

    fn index_declaration(
        &mut self,
        document: &Document,
        declaration: &DeclarationStatement,
        scope: &Scope,
    ) {
        if !declaration_is_scope_binding(declaration) {
            return;
        }
        for name in &declaration.names {
            if scope.binding_kinds.contains_key(&name.text) {
                self.index_name(document, &name.text, name.span, scope, false);
            }
        }
    }

    fn index_expr(&mut self, document: &Document, expression: &Expr, scope: &Scope) {
        match &expression.kind {
            ExprKind::Name(name) => {
                self.index_name(document, name, expression.span, scope, false);
            }
            ExprKind::FunctionHandle(Some(name)) => {
                self.record_name(scope.id, &name.text);
                if !scope.locals.contains_key(&name.text)
                    && !scope.blocked_globals.contains(&name.text)
                    && scope.allow_globals
                    && let Some(symbol) =
                        self.resolve_global(document, &name.text, Some(GlobalKind::Function))
                {
                    self.add_occurrence(
                        symbol,
                        document,
                        leaf_name_range(name),
                        false,
                        Some(scope.id),
                    );
                }
            }
            ExprKind::Paren(inner) => self.index_expr(document, inner, scope),
            ExprKind::ParenApply { target, arguments } => {
                self.index_apply_target(document, target, scope);
                for argument in arguments {
                    self.index_expr(document, argument, scope);
                }
            }
            ExprKind::Field { target, .. } => self.index_receiver(document, target, scope),
            ExprKind::Unary { operand, .. } | ExprKind::Transpose { operand, .. } => {
                self.index_expr(document, operand, scope);
            }
            ExprKind::Binary { left, right, .. } => {
                self.index_expr(document, left, scope);
                self.index_expr(document, right, scope);
            }
            ExprKind::Range { start, step, end } => {
                self.index_expr(document, start, scope);
                if let Some(step) = step {
                    self.index_expr(document, step, scope);
                }
                self.index_expr(document, end, scope);
            }
            ExprKind::Matrix(rows) | ExprKind::Cell(rows) => {
                for element in rows.iter().flatten() {
                    self.index_expr(document, element, scope);
                }
            }
            _ => {}
        }
    }

    fn index_apply_target(&mut self, document: &Document, target: &Expr, scope: &Scope) {
        match &target.kind {
            ExprKind::Name(name) => self.index_name(document, name, target.span, scope, true),
            ExprKind::Field {
                target: receiver, ..
            } => {
                self.index_receiver(document, receiver, scope);
                if self.workspace.is_some()
                    && scope.allow_globals
                    && let Some((qualified, root, leaf)) = qualified_name(target)
                    && !scope.locals.contains_key(root)
                    && !scope.blocked_globals.contains(root)
                {
                    self.record_name(scope.id, &qualified);
                    if let Some(symbol) = self.resolve_global(document, &qualified, None) {
                        self.add_occurrence(symbol, document, leaf.span, false, Some(scope.id));
                    }
                }
            }
            _ => self.index_expr(document, target, scope),
        }
    }

    fn index_receiver(&mut self, document: &Document, expression: &Expr, scope: &Scope) {
        match &expression.kind {
            ExprKind::Name(name) => {
                self.record_name(scope.id, name);
                if let Some(symbol) = scope.locals.get(name).copied() {
                    self.add_occurrence(symbol, document, expression.span, false, Some(scope.id));
                }
            }
            ExprKind::Paren(inner) | ExprKind::Transpose { operand: inner, .. } => {
                self.index_receiver(document, inner, scope);
            }
            ExprKind::ParenApply { target, arguments } => {
                self.index_receiver(document, target, scope);
                for argument in arguments {
                    self.index_expr(document, argument, scope);
                }
            }
            ExprKind::Field { target, .. } => self.index_receiver(document, target, scope),
            _ => {}
        }
    }

    fn record_receiver_names(&mut self, expression: &Expr, scope: usize) {
        match &expression.kind {
            ExprKind::Name(name) => self.record_name(scope, name),
            ExprKind::Paren(inner) | ExprKind::Transpose { operand: inner, .. } => {
                self.record_receiver_names(inner, scope);
            }
            ExprKind::ParenApply { target, .. } | ExprKind::Field { target, .. } => {
                self.record_receiver_names(target, scope);
            }
            _ => {}
        }
    }

    fn record_name(&mut self, scope: usize, name: &str) {
        if is_indexable_name(name) {
            self.scope_names
                .entry(scope)
                .or_default()
                .insert(name.to_owned());
        }
    }

    fn index_name(
        &mut self,
        document: &Document,
        name: &str,
        range: TextRange,
        scope: &Scope,
        allow_global: bool,
    ) {
        self.record_name(scope.id, name);
        let symbol = scope.locals.get(name).copied().or_else(|| {
            (allow_global && scope.allow_globals && !scope.blocked_globals.contains(name))
                .then(|| self.resolve_global(document, name, None))
                .flatten()
        });
        if let Some(symbol) = symbol {
            self.add_occurrence(symbol, document, range, false, Some(scope.id));
        }
    }

    fn resolve_global(
        &self,
        document: &Document,
        name: &str,
        kind: Option<GlobalKind>,
    ) -> Option<usize> {
        if let Some(workspace) = &self.workspace {
            if let Some(candidates) = self
                .file_symbols
                .get(document.uri())
                .and_then(|names| names.get(name))
            {
                return self.unique_candidate(candidates, kind);
            }
            let caller = workspace.paths.get(document.uri())?;
            for candidate in workspace.candidates(caller, name) {
                if let Some(symbol) = workspace.files.get(&path_key(&candidate)) {
                    // The first existing source wins, even if its primary
                    // declaration is invalid or is of an incompatible kind.
                    return symbol.filter(|symbol| {
                        self.symbols[*symbol].name == name.rsplit('.').next().unwrap_or(name)
                            && kind
                                .is_none_or(|kind| self.symbols[*symbol].global_kind == Some(kind))
                    });
                }
            }
            return None;
        }
        let candidates = self.globals.get(name)?;
        self.unique_candidate(candidates, kind)
    }

    fn unique_candidate(&self, candidates: &[usize], kind: Option<GlobalKind>) -> Option<usize> {
        let mut candidates = candidates.iter().copied().filter(|candidate| {
            kind.is_none_or(|kind| self.symbols[*candidate].global_kind == Some(kind))
        });
        let candidate = candidates.next()?;
        candidates.next().is_none().then_some(candidate)
    }

    fn add_symbol(
        &mut self,
        document: &Document,
        name: &Name,
        namespace: Namespace,
        global_kind: Option<GlobalKind>,
        renameable: bool,
        scope: Option<usize>,
    ) -> usize {
        self.add_symbol_at_range(
            document,
            &name.text,
            name.span,
            namespace,
            global_kind,
            renameable,
            scope,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_symbol_at_range(
        &mut self,
        document: &Document,
        name: &str,
        range: TextRange,
        namespace: Namespace,
        global_kind: Option<GlobalKind>,
        renameable: bool,
        scope: Option<usize>,
    ) -> usize {
        let location = Location {
            uri: document.uri().to_owned(),
            range: document.byte_range_to_range(range),
        };
        let symbol = self.symbols.len();
        self.symbols.push(IndexedSymbol {
            name: name.to_owned(),
            definition: location.clone(),
            occurrences: vec![IndexedOccurrence {
                location,
                is_definition: true,
                scope,
            }],
            namespace,
            global_kind,
            renameable,
            ambiguous: false,
        });
        symbol
    }

    fn add_occurrence(
        &mut self,
        symbol: usize,
        document: &Document,
        range: TextRange,
        is_definition: bool,
        scope: Option<usize>,
    ) {
        if let Some(symbol) = self.symbols.get_mut(symbol) {
            symbol.occurrences.push(IndexedOccurrence {
                location: Location {
                    uri: document.uri().to_owned(),
                    range: document.byte_range_to_range(range),
                },
                is_definition,
                scope,
            });
        }
    }

    fn finish(&mut self) {
        for symbol in &mut self.symbols {
            symbol.occurrences.sort_by(compare_occurrences);
            let mut deduplicated = Vec::<IndexedOccurrence>::new();
            for occurrence in symbol.occurrences.drain(..) {
                if let Some(previous) = deduplicated.last_mut()
                    && same_location(&previous.location, &occurrence.location)
                {
                    previous.is_definition |= occurrence.is_definition;
                    previous.scope = previous.scope.or(occurrence.scope);
                    continue;
                }
                deduplicated.push(occurrence);
            }
            symbol.occurrences = deduplicated;
        }
        for (symbol_id, symbol) in self.symbols.iter().enumerate() {
            for occurrence in 0..symbol.occurrences.len() {
                self.occurrences.push(OccurrenceLookup {
                    symbol: symbol_id,
                    occurrence,
                });
            }
        }
        self.occurrences.sort_by(|left, right| {
            compare_occurrences(
                &self.symbols[left.symbol].occurrences[left.occurrence],
                &self.symbols[right.symbol].occurrences[right.occurrence],
            )
        });
    }

    fn lookup_at(&self, uri: &str, position: Position) -> Option<OccurrenceLookup> {
        let mut found = self.occurrences.iter().copied().filter(|lookup| {
            let occurrence = &self.symbols[lookup.symbol].occurrences[lookup.occurrence];
            occurrence.location.uri == uri && range_contains(occurrence.location.range, position)
        });
        let first = found.next()?;
        found
            .all(|candidate| candidate.symbol == first.symbol)
            .then_some(first)
    }

    fn symbol_at(&self, uri: &str, position: Position) -> Option<usize> {
        self.lookup_at(uri, position).map(|lookup| lookup.symbol)
    }

    fn rename_is_collision_free(&self, symbol_id: usize, new_name: &str) -> bool {
        let symbol = &self.symbols[symbol_id];
        if symbol.name == new_name {
            return true;
        }
        match symbol.namespace {
            Namespace::Global => {
                if self.globals.contains_key(new_name)
                    || self
                        .file_symbols
                        .get(&symbol.definition.uri)
                        .is_some_and(|names| names.contains_key(new_name))
                {
                    return false;
                }
                !symbol.occurrences.iter().any(|occurrence| {
                    occurrence.scope.is_some_and(|scope| {
                        self.scope_names
                            .get(&scope)
                            .is_some_and(|names| names.contains(new_name))
                    })
                })
            }
            Namespace::File => {
                !self
                    .file_symbols
                    .get(&symbol.definition.uri)
                    .is_some_and(|names| names.contains_key(new_name))
                    && !self.scope_documents.iter().any(|(scope, uri)| {
                        *uri == symbol.definition.uri
                            && self
                                .scope_names
                                .get(scope)
                                .is_some_and(|names| names.contains(new_name))
                    })
            }
            Namespace::Local { scope } => !self
                .scope_names
                .get(&scope)
                .is_some_and(|names| names.contains(new_name)),
            Namespace::Declared { scope } => !self
                .scope_names
                .get(&scope)
                .is_some_and(|names| names.contains(new_name)),
            Namespace::Isolated => false,
        }
    }

    fn introduced_file_captures_name(
        &self,
        workspace: &WorkspaceResolution,
        new_path: &str,
        new_name: &str,
    ) -> bool {
        self.scope_documents.iter().any(|(scope, uri)| {
            let Some(caller) = workspace.paths.get(uri) else {
                return false;
            };
            self.scope_names.get(scope).is_some_and(|names| {
                names
                    .iter()
                    .filter(|name| name.rsplit('.').next() == Some(new_name))
                    .any(|name| {
                        for candidate in workspace.candidates(caller, name) {
                            let candidate = path_key(&candidate);
                            if candidate == path_key(new_path) {
                                return true;
                            }
                            if workspace.files.contains_key(&candidate) {
                                return false;
                            }
                        }
                        false
                    })
            })
        })
    }
}

#[derive(Clone, Debug)]
struct WorkspaceResolution {
    current_directory: String,
    search_paths: Vec<String>,
    paths: BTreeMap<String, String>,
    files: BTreeMap<String, Option<usize>>,
}

impl WorkspaceResolution {
    fn candidates(&self, caller: &str, name: &str) -> Vec<String> {
        let components = name.split('.').collect::<Vec<_>>();
        if components.is_empty()
            || components
                .iter()
                .any(|component| !is_valid_identifier(component))
        {
            return Vec::new();
        }
        let leaf = components.last().copied().unwrap_or_default();
        let packages = components[..components.len() - 1]
            .iter()
            .map(|component| format!("+{component}"))
            .collect::<Vec<_>>()
            .join("/");
        let source = join_path(&packages, &format!("{leaf}.m"));
        let class_source = join_path(&packages, &format!("@{leaf}/{leaf}.m"));
        let caller_directory = parent_path(caller);
        let mut candidates = Vec::new();
        if components.len() > 1 {
            let mut root = caller_directory;
            let mut found_package = false;
            while last_component(root).starts_with('+') {
                found_package = true;
                root = parent_path(root);
            }
            if found_package || !is_private_path(root) {
                candidates.push(join_path(root, &source));
                candidates.push(join_path(root, &class_source));
            }
        } else if is_private_path(caller_directory) {
            candidates.push(join_path(caller_directory, &source));
        } else {
            candidates.push(join_path(&join_path(caller_directory, "private"), &source));
            if !last_component(caller_directory).starts_with('+') {
                candidates.push(join_path(caller_directory, &source));
                candidates.push(join_path(caller_directory, &class_source));
            }
        }
        candidates.push(join_path(&self.current_directory, &source));
        candidates.push(join_path(&self.current_directory, &class_source));
        for directory in &self.search_paths {
            if is_private_path(directory) || last_component(directory).starts_with(['+', '@']) {
                continue;
            }
            candidates.push(join_path(directory, &source));
            candidates.push(join_path(directory, &class_source));
        }
        candidates
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_owned()
}

fn path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

fn matlab_file_stem(filename: &str) -> Option<&str> {
    let (stem, extension) = filename.rsplit_once('.')?;
    (path_key(extension) == "m").then_some(stem)
}

fn join_path(directory: &str, path: &str) -> String {
    if directory.is_empty() {
        path.to_owned()
    } else {
        format!("{directory}/{path}")
    }
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn last_component(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or_default()
}

fn is_private_path(path: &str) -> bool {
    if cfg!(windows) {
        last_component(path).eq_ignore_ascii_case("private")
    } else {
        last_component(path) == "private"
    }
}

fn leaf_name_range(name: &Name) -> TextRange {
    let leaf = name.text.rsplit('.').next().unwrap_or(&name.text);
    let length = u32::try_from(leaf.len()).unwrap_or_default();
    TextRange::new(name.span.end().saturating_sub(length), name.span.end()).unwrap_or(name.span)
}

fn encode_uri_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
}

/// Static dotted names may designate packages. A local receiver blocks this
/// interpretation, and no object type is inferred from an arbitrary receiver.
fn qualified_name(expression: &Expr) -> Option<(String, &str, &Name)> {
    let ExprKind::Field {
        target,
        name: Some(name),
    } = &expression.kind
    else {
        return None;
    };
    match &target.kind {
        ExprKind::Name(root) => Some((format!("{root}.{}", name.text), root, name)),
        ExprKind::Field { .. } => {
            let (prefix, root, _) = qualified_name(target)?;
            Some((format!("{prefix}.{}", name.text), root, name))
        }
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct Scope {
    id: usize,
    locals: BTreeMap<String, usize>,
    binding_kinds: BTreeMap<String, ScopeBindingKind>,
    blocked_globals: BTreeSet<String>,
    allow_globals: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatementContext {
    Root,
    Nested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FunctionContext {
    TopLevel,
    Method,
    Nested,
}

fn collect_local_declarations(
    statements: &[Stmt],
    declarations: &mut BTreeMap<String, Vec<TextRange>>,
    binding_kinds: &mut BTreeMap<String, ScopeBindingKind>,
    binding_definitions: &mut BTreeMap<String, TextRange>,
) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Assignment { target, .. } => {
                collect_assignment_names(target, declarations, binding_kinds);
            }
            StmtKind::Declaration(declaration) if declaration_is_scope_binding(declaration) => {
                collect_explicit_declaration(
                    declaration,
                    declarations,
                    binding_kinds,
                    binding_definitions,
                );
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for branch in branches {
                    collect_local_declarations(
                        &branch.body,
                        declarations,
                        binding_kinds,
                        binding_definitions,
                    );
                }
                collect_local_declarations(
                    else_body,
                    declarations,
                    binding_kinds,
                    binding_definitions,
                );
            }
            StmtKind::For { variable, body, .. } => {
                collect_assignment_names(variable, declarations, binding_kinds);
                collect_local_declarations(body, declarations, binding_kinds, binding_definitions);
            }
            StmtKind::While { body, .. } => {
                collect_local_declarations(body, declarations, binding_kinds, binding_definitions);
            }
            StmtKind::Try(try_statement) => {
                collect_local_declarations(
                    &try_statement.body,
                    declarations,
                    binding_kinds,
                    binding_definitions,
                );
                if let Some(catch) = &try_statement.catch {
                    if let Some(variable) = &catch.variable
                        && is_indexable_name(&variable.text)
                    {
                        declarations
                            .entry(variable.text.clone())
                            .or_default()
                            .push(variable.span);
                        merge_scope_binding(
                            binding_kinds,
                            &variable.text,
                            ScopeBindingKind::Ordinary,
                        );
                    }
                    collect_local_declarations(
                        &catch.body,
                        declarations,
                        binding_kinds,
                        binding_definitions,
                    );
                }
            }
            StmtKind::Switch {
                cases, otherwise, ..
            } => {
                for case in cases {
                    collect_local_declarations(
                        &case.body,
                        declarations,
                        binding_kinds,
                        binding_definitions,
                    );
                }
                if let Some(otherwise) = otherwise {
                    collect_local_declarations(
                        &otherwise.body,
                        declarations,
                        binding_kinds,
                        binding_definitions,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_explicit_declaration(
    declaration: &DeclarationStatement,
    declarations: &mut BTreeMap<String, Vec<TextRange>>,
    binding_kinds: &mut BTreeMap<String, ScopeBindingKind>,
    binding_definitions: &mut BTreeMap<String, TextRange>,
) {
    let kind = match declaration.kind {
        DeclarationKind::Global => ScopeBindingKind::Global,
        DeclarationKind::Persistent => ScopeBindingKind::Persistent,
    };
    for name in &declaration.names {
        if !is_indexable_name(&name.text) {
            continue;
        }
        declarations
            .entry(name.text.clone())
            .or_default()
            .push(name.span);
        binding_definitions
            .entry(name.text.clone())
            .and_modify(|definition| {
                if (name.span.start(), name.span.end()) < (definition.start(), definition.end()) {
                    *definition = name.span;
                }
            })
            .or_insert(name.span);
        merge_scope_binding(binding_kinds, &name.text, kind);
    }
}

fn collect_assignment_names(
    target: &Expr,
    declarations: &mut BTreeMap<String, Vec<TextRange>>,
    binding_kinds: &mut BTreeMap<String, ScopeBindingKind>,
) {
    match &target.kind {
        ExprKind::Name(name) if is_indexable_name(name) => {
            declarations
                .entry(name.clone())
                .or_default()
                .push(target.span);
            merge_scope_binding(binding_kinds, name, ScopeBindingKind::Ordinary);
        }
        ExprKind::Matrix(rows) if rows.len() == 1 => {
            for element in &rows[0] {
                if let ExprKind::Name(name) = &element.kind
                    && is_indexable_name(name)
                {
                    declarations
                        .entry(name.clone())
                        .or_default()
                        .push(element.span);
                    merge_scope_binding(binding_kinds, name, ScopeBindingKind::Ordinary);
                }
            }
        }
        _ => {}
    }
}

fn merge_scope_binding(
    bindings: &mut BTreeMap<String, ScopeBindingKind>,
    name: &str,
    incoming: ScopeBindingKind,
) {
    bindings
        .entry(name.to_owned())
        .and_modify(|current| {
            *current = match (*current, incoming) {
                (ScopeBindingKind::Ordinary, explicit) | (explicit, ScopeBindingKind::Ordinary) => {
                    explicit
                }
                (left, right) if left == right => left,
                _ => ScopeBindingKind::ConflictingDeclarations,
            };
        })
        .or_insert(incoming);
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

fn collect_nested_declaration_names(statements: &[Stmt], names: &mut BTreeSet<String>) {
    for statement in statements {
        match &statement.kind {
            StmtKind::Function(function) => {
                if let Some(name) = &function.name {
                    names.insert(name.text.clone());
                }
            }
            StmtKind::Class(class) => {
                if let Some(name) = &class.name {
                    names.insert(name.text.clone());
                }
            }
            _ => {}
        }
    }
}

fn is_indexable_name(name: &str) -> bool {
    !name.is_empty() && name != "~"
}

pub(crate) fn is_valid_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
        && !is_keyword(name)
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "classdef"
            | "continue"
            | "else"
            | "elseif"
            | "end"
            | "enumeration"
            | "events"
            | "for"
            | "function"
            | "global"
            | "if"
            | "methods"
            | "otherwise"
            | "parfor"
            | "persistent"
            | "properties"
            | "return"
            | "spmd"
            | "switch"
            | "try"
            | "while"
    )
}

fn compare_occurrences(left: &IndexedOccurrence, right: &IndexedOccurrence) -> std::cmp::Ordering {
    left.location
        .uri
        .cmp(&right.location.uri)
        .then_with(|| left.location.range.start.cmp(&right.location.range.start))
        .then_with(|| left.location.range.end.cmp(&right.location.range.end))
}

fn same_location(left: &Location, right: &Location) -> bool {
    left.uri == right.uri && left.range == right.range
}

fn range_contains(range: crate::protocol::Range, position: Position) -> bool {
    (range.start <= position && position < range.end)
        || (range.start < range.end && position == range.end)
}

#[cfg(test)]
mod tests {
    use super::{SymbolIndex, is_valid_identifier};
    use crate::document::{DocumentStore, WorkspaceDocument, WorkspaceSnapshot};
    use crate::protocol::Position;

    const WORKSPACE_ROOT: &str = "openmat-workspace://root-1/project/";

    fn workspace(files: &[(&str, &str)], search_paths: &[&str]) -> DocumentStore {
        let mut documents = DocumentStore::new();
        documents
            .replace_workspace_documents(WorkspaceSnapshot {
                root_uri: WORKSPACE_ROOT.to_owned(),
                current_directory: String::new(),
                search_paths: search_paths.iter().map(|path| (*path).to_owned()).collect(),
                complete: true,
                documents: files
                    .iter()
                    .map(|(path, text)| WorkspaceDocument {
                        uri: format!("{WORKSPACE_ROOT}{path}"),
                        relative_path: (*path).to_owned(),
                        text: (*text).to_owned(),
                    })
                    .collect(),
            })
            .expect("workspace snapshot");
        documents
    }

    fn reference_position(documents: &DocumentStore, path: &str, reference: &str) -> Position {
        let uri = format!("{WORKSPACE_ROOT}{path}");
        let document = documents.get_effective(&uri).expect("fixture document");
        let offset = document
            .text()
            .find(reference)
            .expect("reference substring");
        document.byte_to_position(u32::try_from(offset).expect("fixture offset"))
    }

    fn target(
        index: &SymbolIndex,
        documents: &DocumentStore,
        path: &str,
        reference: &str,
    ) -> Option<String> {
        index
            .definition(
                &format!("{WORKSPACE_ROOT}{path}"),
                reference_position(documents, path, reference),
            )
            .map(|location| location.uri)
    }

    #[test]
    fn workspace_exports_only_filename_matching_primary_declarations() {
        let documents = workspace(
            &[
                (
                    "main.m",
                    "a = declared();\nb = wrong();\nc = localOnly();\nd = helper();\n",
                ),
                ("wrong.m", "function y = declared()\ny = 1;\nend\n"),
                (
                    "container.m",
                    "function y = container()\ny = helper();\nend\nfunction y = helper()\ny = 2;\nend\nfunction y = localOnly()\ny = 3;\nend\n",
                ),
                ("helper.m", "function y = helper()\ny = 4;\nend\n"),
            ],
            &[],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(target(&index, &documents, "main.m", "declared()"), None);
        assert_eq!(target(&index, &documents, "main.m", "wrong()"), None);
        assert_eq!(target(&index, &documents, "main.m", "localOnly()"), None);
        assert_eq!(
            target(&index, &documents, "main.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}helper.m"))
        );
        assert_eq!(
            target(&index, &documents, "container.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}container.m"))
        );
        let references = index
            .references(
                &format!("{WORKSPACE_ROOT}container.m"),
                reference_position(&documents, "container.m", "helper()"),
                true,
            )
            .expect("local helper references");
        assert_eq!(references.len(), 2);
        assert!(
            references
                .iter()
                .all(|location| location.uri.ends_with("container.m"))
        );
    }

    #[test]
    fn caller_private_functions_do_not_escape_their_parent_directory() {
        let documents = workspace(
            &[
                ("a/caller.m", "x = helper();\ny = secret();\n"),
                ("b/caller.m", "x = helper();\ny = secret();\n"),
                ("a/helper.m", "function y = helper()\ny = 0;\nend\n"),
                ("helper.m", "function y = helper()\ny = 1;\nend\n"),
                ("a/private/helper.m", "function y = helper()\ny = 2;\nend\n"),
                (
                    "a/private/secret.m",
                    "function y = secret()\ny = helper();\nend\n",
                ),
            ],
            &["a/private"],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "a/caller.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}a/private/helper.m"))
        );
        assert_eq!(
            target(&index, &documents, "a/caller.m", "secret()"),
            Some(format!("{WORKSPACE_ROOT}a/private/secret.m"))
        );
        assert_eq!(
            target(&index, &documents, "b/caller.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}helper.m"))
        );
        assert_eq!(target(&index, &documents, "b/caller.m", "secret()"), None);
        assert_eq!(
            target(&index, &documents, "a/private/secret.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}a/private/helper.m"))
        );
    }

    #[test]
    fn recursive_indexing_respects_caller_directory_and_search_path_order() {
        let files = [
            ("main.m", "x = helper();\ny = unrelated();\n"),
            ("a/caller.m", "x = helper();\n"),
            ("a/helper.m", "function y = helper()\ny = 1;\nend\n"),
            ("b/helper.m", "function y = helper()\ny = 2;\nend\n"),
            (
                "elsewhere/unrelated.m",
                "function y = unrelated()\ny = 3;\nend\n",
            ),
        ];
        let documents = workspace(&files, &[]);
        let index = SymbolIndex::build(&documents);
        assert_eq!(target(&index, &documents, "main.m", "helper()"), None);
        assert_eq!(target(&index, &documents, "main.m", "unrelated()"), None);
        let documents = workspace(&files, &["b", "a"]);
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "main.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}b/helper.m"))
        );
        assert_eq!(
            target(&index, &documents, "a/caller.m", "helper()"),
            Some(format!("{WORKSPACE_ROOT}a/helper.m"))
        );
        assert_eq!(target(&index, &documents, "main.m", "unrelated()"), None);
    }

    #[test]
    fn package_qualification_resolves_leaf_symbols_without_inferring_object_members() {
        let documents = workspace(
            &[
                (
                    "main.m",
                    "a = pkg.helper();\nb = helper();\nc = pkg.inner.deep();\n",
                ),
                ("object.m", "pkg = 1;\na = pkg.helper();\n"),
                ("+pkg/helper.m", "function y = helper()\ny = 1;\nend\n"),
                (
                    "+pkg/caller.m",
                    "function y = caller()\ny = helper();\nz = pkg.helper();\nend\n",
                ),
                ("+pkg/+inner/deep.m", "function y = deep()\ny = 2;\nend\n"),
            ],
            &["+pkg"],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "main.m", "helper();"),
            Some(format!("{WORKSPACE_ROOT}+pkg/helper.m"))
        );
        assert_eq!(target(&index, &documents, "main.m", "helper();\nc"), None);
        assert_eq!(
            target(&index, &documents, "main.m", "deep();"),
            Some(format!("{WORKSPACE_ROOT}+pkg/+inner/deep.m"))
        );
        assert_eq!(target(&index, &documents, "object.m", "helper();"), None);
        assert_eq!(
            target(&index, &documents, "+pkg/caller.m", "helper();\nz"),
            None
        );
        assert_eq!(
            target(&index, &documents, "+pkg/caller.m", "helper();\nend"),
            Some(format!("{WORKSPACE_ROOT}+pkg/helper.m"))
        );
        let rename = index
            .rename(
                &documents,
                &format!("{WORKSPACE_ROOT}main.m"),
                reference_position(&documents, "main.m", "helper();"),
                "calculate",
            )
            .expect("package leaf rename");
        let main_edit = rename
            .document_changes
            .iter()
            .find(|edit| edit.uri.ends_with("main.m"))
            .expect("main edit");
        assert_eq!(main_edit.version, None);
        assert_eq!(main_edit.edits.len(), 1);
        assert_eq!(main_edit.edits[0].range.start, Position::new(0, 8));
        assert_eq!(main_edit.edits[0].range.end, Position::new(0, 14));
        assert_eq!(rename.file_renames.len(), 1);
        assert_eq!(
            rename.file_renames[0].old_uri,
            format!("{WORKSPACE_ROOT}+pkg/helper.m")
        );
        assert_eq!(
            rename.file_renames[0].new_uri,
            format!("{WORKSPACE_ROOT}+pkg/calculate.m")
        );
    }

    #[test]
    fn workspace_class_constructors_and_superclasses_use_visible_source_paths() {
        let documents = workspace(
            &[
                (
                    "main.m",
                    "a = Thing();\nb = pkg.Base();\nc = Thing.method();\n",
                ),
                ("@Thing/Thing.m", "classdef Thing\nend\n"),
                ("Child.m", "classdef Child < pkg.Base\nend\n"),
                ("+pkg/Base.m", "classdef Base\nend\n"),
                ("@Thing/method.m", "function y = method(obj)\ny = 1;\nend\n"),
            ],
            &[],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "main.m", "Thing();"),
            Some(format!("{WORKSPACE_ROOT}@Thing/Thing.m"))
        );
        assert_eq!(
            target(&index, &documents, "main.m", "Base();"),
            Some(format!("{WORKSPACE_ROOT}+pkg/Base.m"))
        );
        assert_eq!(
            target(&index, &documents, "Child.m", "Base"),
            Some(format!("{WORKSPACE_ROOT}+pkg/Base.m"))
        );
        assert_eq!(target(&index, &documents, "main.m", "method();"), None);
        let position = reference_position(&documents, "main.m", "Thing();");
        assert!(
            index
                .rename_block_reason(&format!("{WORKSPACE_ROOT}main.m"), position)
                .is_some()
        );
        assert!(
            index
                .prepare_rename(&format!("{WORKSPACE_ROOT}main.m"), position)
                .is_none()
        );
        assert!(
            index
                .rename(
                    &documents,
                    &format!("{WORKSPACE_ROOT}main.m"),
                    position,
                    "Renamed"
                )
                .is_none()
        );
    }

    #[test]
    fn prior_root_buffers_keep_local_navigation_without_cross_root_resolution() {
        const OLD_URI: &str = "openmat-workspace://root-1/previous/main.m";
        let mut documents = workspace(
            &[
                ("main.m", "x = helper();\ny = previousOnly();\n"),
                ("helper.m", "function y = helper()\ny = 1;\nend\n"),
            ],
            &[],
        );
        documents
            .did_open(
                OLD_URI,
                7,
                "x = helper();\ny = previousOnly();\nfunction y = previousOnly()\ny = 1;\nend\n",
            )
            .expect("previous root buffer");
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "main.m", "helper();"),
            Some(format!("{WORKSPACE_ROOT}helper.m"))
        );
        assert_eq!(
            target(&index, &documents, "main.m", "previousOnly();"),
            None
        );
        assert!(index.definition(OLD_URI, Position::new(0, 5)).is_none());
        assert_eq!(
            index
                .definition(OLD_URI, Position::new(1, 5))
                .expect("old root file local")
                .uri,
            OLD_URI
        );
    }

    #[test]
    fn first_existing_source_blocks_fallback_when_it_is_not_a_valid_function() {
        let documents = workspace(
            &[
                ("main.m", "x = helper();\n"),
                ("helper.m", "x = 1;\n"),
                ("library/helper.m", "function y = helper()\ny = 2;\nend\n"),
            ],
            &["library"],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(target(&index, &documents, "main.m", "helper();"), None);
    }

    #[test]
    fn exported_function_rename_moves_its_source_and_rejects_existing_scripts() {
        let mut documents = workspace(
            &[
                ("main.m", "x = helper();\n"),
                ("helper.m", "function y = helper()\ny = 1;\nend\n"),
                ("occupied.m", "x = 1;\n"),
                ("处理.m", "x = 2;\n"),
            ],
            &[],
        );
        let main_uri = format!("{WORKSPACE_ROOT}main.m");
        documents
            .did_open(&main_uri, 5, "x = helper();\n")
            .expect("open calling buffer");
        let index = SymbolIndex::build(&documents);
        let position = reference_position(&documents, "main.m", "helper();");
        assert!(
            index
                .rename(&documents, &main_uri, position, "occupied")
                .is_none()
        );
        assert!(
            index
                .rename(&documents, &main_uri, position, "处理")
                .is_none()
        );
        let edit = index
            .rename(&documents, &main_uri, position, "变量")
            .expect("Unicode rename");
        assert_eq!(
            edit.file_renames[0].new_uri,
            format!("{WORKSPACE_ROOT}%E5%8F%98%E9%87%8F.m")
        );
        assert_eq!(
            edit.document_changes
                .iter()
                .find(|edit| edit.uri == main_uri)
                .expect("caller edit")
                .version,
            Some(5)
        );
        assert_eq!(
            edit.document_changes
                .iter()
                .find(|edit| edit.uri.ends_with("helper.m"))
                .expect("closed definition edit")
                .version,
            None
        );
        assert!(
            index
                .rename(&documents, &main_uri, position, "helper")
                .expect("unchanged name")
                .file_renames
                .is_empty()
        );
    }

    #[test]
    fn file_local_and_exported_renames_cannot_capture_unrelated_calls() {
        let documents = workspace(
            &[
                (
                    "container.m",
                    "function y = container()\ny = helper();\nend\nfunction y = localOnly()\ny = 1;\nend\n",
                ),
                ("main.m", "x = missing();\ny = pkg.absent();\n"),
                ("helper.m", "function y = helper()\ny = 2;\nend\n"),
                (
                    "+pkg/available.m",
                    "function y = available()\ny = 3;\nend\n",
                ),
            ],
            &[],
        );
        let index = SymbolIndex::build(&documents);
        let container_uri = format!("{WORKSPACE_ROOT}container.m");
        assert!(
            index
                .rename(
                    &documents,
                    &container_uri,
                    reference_position(&documents, "container.m", "localOnly()"),
                    "helper"
                )
                .is_none()
        );
        assert!(
            index
                .rename(
                    &documents,
                    &format!("{WORKSPACE_ROOT}helper.m"),
                    reference_position(&documents, "helper.m", "helper()"),
                    "missing"
                )
                .is_none()
        );
        assert!(
            index
                .rename(
                    &documents,
                    &format!("{WORKSPACE_ROOT}+pkg/available.m"),
                    reference_position(&documents, "+pkg/available.m", "available()"),
                    "absent"
                )
                .is_none()
        );
    }

    #[test]
    fn packages_nested_beneath_private_keep_their_qualified_lookup_root() {
        let documents = workspace(
            &[
                (
                    "a/private/+pkg/caller.m",
                    "function y = caller()\ny = pkg.helper();\nend\n",
                ),
                (
                    "a/private/+pkg/helper.m",
                    "function y = helper()\ny = 1;\nend\n",
                ),
                ("main.m", "x = pkg.helper();\n"),
            ],
            &[],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "a/private/+pkg/caller.m", "helper();"),
            Some(format!("{WORKSPACE_ROOT}a/private/+pkg/helper.m"))
        );
        assert_eq!(target(&index, &documents, "main.m", "helper();"), None);
    }

    #[test]
    #[cfg(windows)]
    fn windows_filename_matching_does_not_ignore_function_identifier_case() {
        let documents = workspace(
            &[
                ("main.m", "a = Foo();\nb = foo();\nc = lower();\n"),
                ("Foo.M", "function y = Foo()\ny = 1;\nend\n"),
                ("LOWER.m", "function y = lower()\ny = 2;\nend\n"),
            ],
            &[],
        );
        let index = SymbolIndex::build(&documents);
        assert_eq!(
            target(&index, &documents, "main.m", "Foo();"),
            Some(format!("{WORKSPACE_ROOT}Foo.M"))
        );
        assert_eq!(target(&index, &documents, "main.m", "foo();"), None);
        assert_eq!(
            target(&index, &documents, "main.m", "lower();"),
            Some(format!("{WORKSPACE_ROOT}LOWER.m"))
        );
    }

    #[test]
    fn indexes_open_documents_deterministically_and_respects_local_shadowing() {
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                "file:///z-use.m",
                3,
                "value = helper(1);\nbare = helper + 1;\nhelper(1) = 2;\nfunction y = local(x)\ny = x;\nend\n",
            )
            .expect("use document");
        documents
            .did_open(
                "file:///a-helper.m",
                7,
                "function y = helper(x)\ny = x;\nend\n",
            )
            .expect("definition document");
        let mut index = SymbolIndex::default();
        index.rebuild(&documents);

        let definition = index
            .definition("file:///z-use.m", Position::new(0, 10))
            .expect("global definition");
        assert_eq!(definition.uri, "file:///a-helper.m");
        let references = index
            .references("file:///a-helper.m", Position::new(0, 15), true)
            .expect("references");
        assert_eq!(references.len(), 2);
        assert!(
            index
                .definition("file:///z-use.m", Position::new(1, 9))
                .is_none()
        );
        assert!(
            index
                .definition("file:///z-use.m", Position::new(2, 2))
                .is_none()
        );

        let local_definition = index
            .definition("file:///a-helper.m", Position::new(1, 4))
            .expect("local definition");
        assert_eq!(local_definition.range.start, Position::new(0, 20));
    }

    #[test]
    fn duplicate_globals_and_members_are_conservative() {
        let mut documents = DocumentStore::new();
        documents
            .did_open("file:///a.m", 1, "function a = same()\na = 1;\nend")
            .expect("first");
        documents
            .did_open("file:///b.m", 1, "function b = same()\nb = 2;\nend")
            .expect("second");
        let mut index = SymbolIndex::default();
        index.rebuild(&documents);
        assert!(
            index
                .definition("file:///a.m", Position::new(0, 15))
                .is_none()
        );
        assert!(
            index
                .prepare_rename("file:///a.m", Position::new(0, 15))
                .is_none()
        );
    }

    #[test]
    fn rename_is_versioned_and_validates_identifiers_and_collisions() {
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                "file:///local.m",
                9,
                "function y = calc(input)\ny = input + sin(1);\nend",
            )
            .expect("open");
        let mut index = SymbolIndex::default();
        index.rebuild(&documents);
        let edit = index
            .rename(
                &documents,
                "file:///local.m",
                Position::new(0, 22),
                "renamed",
            )
            .expect("rename");
        assert_eq!(edit.document_changes[0].version, Some(9));
        assert_eq!(edit.document_changes[0].edits.len(), 2);
        assert!(
            index
                .rename(&documents, "file:///local.m", Position::new(0, 22), "y",)
                .is_none()
        );
        assert!(
            index
                .rename(&documents, "file:///local.m", Position::new(0, 22), "sin",)
                .is_none()
        );
        assert!(!is_valid_identifier("while"));
        assert!(!is_valid_identifier("1bad"));
        assert!(is_valid_identifier("变量_2"));
    }

    #[test]
    fn try_catch_bindings_and_bodies_participate_in_navigation() {
        const URI: &str = "file:///try-catch.m";
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                URI,
                4,
                concat!(
                    "try\n",
                    "  protected_value = missing;\n",
                    "  try\n",
                    "    nested_value = 1;\n",
                    "  catch nested_error\n",
                    "    nested_use = nested_error;\n",
                    "  end\n",
                    "catch caught\n",
                    "  caught_use = caught;\n",
                    "end\n",
                    "after = caught;\n",
                    "try\n",
                    "end\n",
                    "plain = absent;\n",
                    "try\n",
                    "catch empty_error\n",
                    "end\n",
                    "empty_after = empty_error;\n",
                ),
            )
            .expect("try/catch document");
        let index = SymbolIndex::build(&documents);

        let caught_definition = index
            .definition(URI, Position::new(10, 10))
            .expect("catch binding should remain visible after the construct");
        assert_eq!(caught_definition.range.start, Position::new(7, 6));
        assert_eq!(
            index
                .references(URI, Position::new(7, 8), true)
                .expect("caught references")
                .len(),
            3
        );

        let nested_definition = index
            .definition(URI, Position::new(5, 20))
            .expect("nested catch reference");
        assert_eq!(nested_definition.range.start, Position::new(4, 8));
        assert_eq!(
            index
                .references(URI, Position::new(4, 10), true)
                .expect("nested catch references")
                .len(),
            2
        );

        let empty_definition = index
            .definition(URI, Position::new(17, 17))
            .expect("an empty catch body still declares its binding");
        assert_eq!(empty_definition.range.start, Position::new(15, 6));
        assert!(index.definition(URI, Position::new(1, 22)).is_none());
        assert!(index.definition(URI, Position::new(13, 10)).is_none());

        let rename = index
            .rename(&documents, URI, Position::new(10, 10), "recovered")
            .expect("catch binding rename");
        assert_eq!(rename.document_changes[0].version, Some(4));
        assert_eq!(rename.document_changes[0].edits.len(), 3);
        assert!(
            index
                .rename(&documents, URI, Position::new(4, 10), "caught")
                .is_none(),
            "nested catch bindings share the ordinary surrounding script scope"
        );
    }

    #[test]
    fn global_and_persistent_declarations_define_distinct_scope_bindings() {
        const URI: &str = "file:///declarations.m";
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                URI,
                5,
                concat!(
                    "function y = worker(input)\n",
                    "  shared = input;\n",
                    "  switch input\n",
                    "  case 1\n",
                    "    global shared\n",
                    "  otherwise\n",
                    "    persistent cache\n",
                    "  end\n",
                    "  cache = shared;\n",
                    "  y = cache;\n",
                    "end\n",
                ),
            )
            .expect("declaration document");
        let index = SymbolIndex::build(&documents);

        let shared_definition = index
            .definition(URI, Position::new(8, 12))
            .expect("global declaration definition");
        let cache_definition = index
            .definition(URI, Position::new(9, 8))
            .expect("persistent declaration definition");
        assert_eq!(shared_definition.range.start, Position::new(4, 11));
        assert_eq!(cache_definition.range.start, Position::new(6, 15));
        assert_eq!(
            index
                .references(URI, Position::new(4, 13), true)
                .expect("global declaration references")
                .len(),
            3
        );
        assert_eq!(
            index
                .references(URI, Position::new(6, 17), true)
                .expect("persistent declaration references")
                .len(),
            3
        );
    }

    #[test]
    fn invalid_script_persistent_does_not_create_a_navigable_binding() {
        const URI: &str = "file:///invalid-persistent.m";
        let mut documents = DocumentStore::new();
        documents
            .did_open(URI, 1, "persistent cache\nvalue = cache;\n")
            .expect("invalid persistent document");
        let index = SymbolIndex::build(&documents);

        assert!(index.definition(URI, Position::new(1, 10)).is_none());
    }

    #[test]
    fn catch_bindings_use_the_surrounding_function_scope_only() {
        const URI: &str = "file:///catch-function-scope.m";
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                URI,
                1,
                concat!(
                    "function out = guarded(input)\n",
                    "try\n",
                    "  use = input;\n",
                    "catch caught\n",
                    "  use = caught;\n",
                    "end\n",
                    "out = caught;\n",
                    "end\n",
                    "outside = caught;\n",
                ),
            )
            .expect("function catch document");
        let index = SymbolIndex::build(&documents);

        let definition = index
            .definition(URI, Position::new(6, 8))
            .expect("catch binding after try/catch in the function");
        assert_eq!(definition.range.start, Position::new(3, 6));
        assert_eq!(
            index
                .references(URI, Position::new(3, 8), true)
                .expect("function-local catch references")
                .len(),
            3
        );
        assert!(
            index.definition(URI, Position::new(8, 12)).is_none(),
            "the catch binding must not escape its surrounding function"
        );
    }

    #[test]
    fn catch_assignment_obeys_existing_parameter_shadowing_rules() {
        const URI: &str = "file:///catch-parameter.m";
        let mut documents = DocumentStore::new();
        documents
            .did_open(
                URI,
                2,
                concat!(
                    "function out = guarded(caught)\n",
                    "try\n",
                    "catch caught\n",
                    "end\n",
                    "out = caught;\n",
                    "end\n",
                ),
            )
            .expect("parameter catch document");
        let index = SymbolIndex::build(&documents);

        let definition = index
            .definition(URI, Position::new(2, 8))
            .expect("catch assignment to an existing parameter");
        assert_eq!(definition.range.start, Position::new(0, 23));
        assert_eq!(
            index
                .references(URI, Position::new(0, 25), true)
                .expect("parameter and catch occurrences")
                .len(),
            3
        );
    }
}
