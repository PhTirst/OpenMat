#![doc = "Stable first-version HIR and lowering from the lossless CST."]

use openmat_source::{Diagnostic, SourceId, TextRange};
use openmat_syntax::{Cst, NodeId, SyntaxKind, TokenKind};

/// A lowered source file. Every contained item retains a source byte range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HirFile {
    pub source_id: SourceId,
    pub span: TextRange,
    pub statements: Vec<Stmt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LowerResult {
    pub file: HirFile,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: TextRange,
    /// Whether a trailing semicolon suppresses this statement's automatic display.
    ///
    /// Evaluation still occurs and expression statements may still update the
    /// implicit `ans` binding. A comma or line boundary does not suppress output.
    pub suppress_output: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StmtKind {
    Assignment {
        target: Expr,
        value: Expr,
    },
    Expr(Expr),
    Command(CommandStatement),
    Clear(ClearStatement),
    Declaration(DeclarationStatement),
    If {
        branches: Vec<ConditionalBranch>,
        else_body: Vec<Stmt>,
    },
    For {
        variable: Expr,
        iterable: Expr,
        body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Try(TryStatement),
    Switch {
        selector: Expr,
        cases: Vec<SwitchCase>,
        otherwise: Option<OtherwiseBranch>,
    },
    Break,
    Continue,
    Return,
    Function(FunctionDef),
    Arguments(ArgumentsBlock),
    Class(ClassDef),
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClearStatement {
    pub names: Vec<Name>,
    pub form: ClearForm,
}

/// A MATLAB command-form call whose arguments are character text rather than
/// workspace expressions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandStatement {
    pub callee: Option<Name>,
    pub arguments: Vec<CommandArgument>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandArgument {
    pub text: String,
    pub span: TextRange,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClearForm {
    IdentifierList,
    MissingNames,
    UnsupportedArguments,
}

/// A source declaration that changes how names bind without assigning values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationStatement {
    pub kind: DeclarationKind,
    pub names: Vec<Name>,
    pub form: DeclarationForm,
    /// The lexical container in which the declaration appeared.
    ///
    /// This is retained so executable lowering can enforce scope semantics
    /// without reconstructing the frontend tree.
    pub context: DeclarationContext,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeclarationKind {
    Global,
    Persistent,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeclarationForm {
    IdentifierList,
    MissingNames,
    InvalidItems,
    DuplicatePersistentNames,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeclarationContext {
    Script,
    Function,
    ClassDefinition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionalBranch {
    pub condition: Expr,
    pub body: Vec<Stmt>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchCase {
    pub expression: Expr,
    pub body: Vec<Stmt>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtherwiseBranch {
    pub body: Vec<Stmt>,
    pub span: TextRange,
}

/// A `try` statement, including the source range occupied by its protected body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TryStatement {
    pub body: Vec<Stmt>,
    pub body_span: TextRange,
    /// MATLAB R2022b permits `try ... end`, so the entire catch clause is optional.
    pub catch: Option<CatchClause>,
}

/// A catch clause with its optional exception binding and independently ranged body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatchClause {
    pub variable: Option<Name>,
    pub body: Vec<Stmt>,
    pub body_span: TextRange,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Name {
    pub text: String,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionDef {
    pub name: Option<Name>,
    pub outputs: Vec<Name>,
    pub inputs: Vec<Name>,
    pub body: Vec<Stmt>,
    pub span: TextRange,
}

/// Source-backed function argument declarations. Expressions remain executable HIR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentsBlock {
    pub attributes: Vec<Attribute>,
    pub declarations: Vec<ArgumentDeclaration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentDeclaration {
    pub name: Name,
    pub dimensions: Vec<Expr>,
    pub class: Option<Name>,
    pub validators: Vec<Expr>,
    pub default: Option<Expr>,
    pub span: TextRange,
}

/// A body-less method signature from an abstract block or an `@Class` folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MethodDeclaration {
    pub name: Option<Name>,
    pub outputs: Vec<Name>,
    pub inputs: Vec<Name>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassDef {
    pub name: Option<Name>,
    pub superclass: Option<Name>,
    pub attributes: Vec<Attribute>,
    pub property_blocks: Vec<PropertyBlock>,
    pub method_blocks: Vec<MethodBlock>,
    pub enumeration_blocks: Vec<EnumerationBlock>,
    pub event_blocks: Vec<EventBlock>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribute {
    pub name: Option<Name>,
    pub value: Option<Expr>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyBlock {
    pub attributes: Vec<Attribute>,
    pub properties: Vec<PropertyDef>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyDef {
    pub name: Option<Name>,
    pub default: Option<Expr>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MethodBlock {
    pub attributes: Vec<Attribute>,
    pub methods: Vec<FunctionDef>,
    pub declarations: Vec<MethodDeclaration>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumerationBlock {
    pub attributes: Vec<Attribute>,
    pub members: Vec<EnumMemberDef>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumMemberDef {
    pub name: Option<Name>,
    pub arguments: Vec<Expr>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventBlock {
    pub attributes: Vec<Attribute>,
    pub events: Vec<EventDef>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDef {
    pub name: Option<Name>,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: TextRange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExprKind {
    Name(String),
    Number(String),
    Char(String),
    String(String),
    /// MATLAB's standalone `:` index selecting every element of one dimension.
    AllIndex,
    /// MATLAB's `end` keyword bound to the innermost paren/brace index target.
    EndIndex,
    FunctionHandle(Option<Name>),
    AnonymousFunction {
        parameters: Vec<Name>,
        body: Box<Expr>,
    },
    Paren(Box<Expr>),
    /// Unresolved MATLAB `f(x)` syntax: name resolution/runtime decides call vs index.
    ParenApply {
        target: Box<Expr>,
        arguments: Vec<Expr>,
    },
    /// MATLAB `C{...}` syntax, distinct from both calls and prefix cell literals.
    BraceApply {
        target: Box<Expr>,
        arguments: Vec<Expr>,
    },
    /// Explicit `object@Base(args)` syntax retained for constructor validation.
    SuperclassConstructorCall {
        object: Box<Expr>,
        superclass: Option<Name>,
        arguments: Vec<Expr>,
    },
    Field {
        target: Box<Expr>,
        name: Option<Name>,
    },
    /// MATLAB `target.(expression)` syntax with a source-backed selector expression.
    DynamicField {
        target: Box<Expr>,
        name: Box<Expr>,
    },
    Unary {
        operator: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        operator: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Range {
        start: Box<Expr>,
        step: Option<Box<Expr>>,
        end: Box<Expr>,
    },
    Transpose {
        kind: TransposeKind,
        operand: Box<Expr>,
    },
    Matrix(Vec<Vec<Expr>>),
    Cell(Vec<Vec<Expr>>),
    Error,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    RightDivide,
    LeftDivide,
    Power,
    ElementMultiply,
    ElementRightDivide,
    ElementLeftDivide,
    ElementPower,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    ShortCircuitAnd,
    Or,
    ShortCircuitOr,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TransposeKind {
    Conjugate,
    NonConjugate,
    Unknown,
}

/// Lowers all supported CST nodes. Malformed nodes become error HIR plus diagnostics.
#[must_use]
pub fn lower(cst: &Cst) -> LowerResult {
    Lowerer::new(cst).run()
}

struct Lowerer<'cst> {
    cst: &'cst Cst,
    diagnostics: Vec<Diagnostic>,
}

impl<'cst> Lowerer<'cst> {
    fn new(cst: &'cst Cst) -> Self {
        Self {
            cst,
            diagnostics: Vec::new(),
        }
    }

    fn run(mut self) -> LowerResult {
        let root_id = self.cst.root();
        let (span, children) = self
            .cst
            .node(root_id)
            .map_or((TextRange::empty(0), Vec::new()), |root| {
                (root.range, root.children.clone())
            });
        let statements = children
            .into_iter()
            .filter_map(|child| self.lower_stmt(child, DeclarationContext::Script))
            .collect();
        LowerResult {
            file: HirFile {
                source_id: self.cst.source_id(),
                span,
                statements,
            },
            diagnostics: self.diagnostics,
        }
    }

    fn lower_stmt(&mut self, id: NodeId, context: DeclarationContext) -> Option<Stmt> {
        let node = self.cst.node(id)?.clone();
        let kind = match node.kind {
            SyntaxKind::AssignmentStmt => {
                let target = self.lower_required_expr(node.children.first().copied(), node.range);
                let value = self.lower_required_expr(node.children.get(1).copied(), node.range);
                StmtKind::Assignment { target, value }
            }
            SyntaxKind::ExprStmt => {
                StmtKind::Expr(self.lower_required_expr(node.children.first().copied(), node.range))
            }
            SyntaxKind::CommandStmt => StmtKind::Command(self.lower_command(id, &node)),
            SyntaxKind::ClearStmt => StmtKind::Clear(self.lower_clear(&node)),
            SyntaxKind::GlobalStmt | SyntaxKind::PersistentStmt => {
                StmtKind::Declaration(self.lower_declaration(&node, context))
            }
            SyntaxKind::BreakStmt => StmtKind::Break,
            SyntaxKind::ContinueStmt => StmtKind::Continue,
            SyntaxKind::ReturnStmt => StmtKind::Return,
            SyntaxKind::IfStmt => self.lower_if(&node, context),
            SyntaxKind::ForStmt => {
                let variable = self.lower_required_expr(node.children.first().copied(), node.range);
                let iterable = self.lower_required_expr(node.children.get(1).copied(), node.range);
                let body = node
                    .children
                    .get(2)
                    .copied()
                    .map_or_else(Vec::new, |child| self.lower_block(child, context));
                StmtKind::For {
                    variable,
                    iterable,
                    body,
                }
            }
            SyntaxKind::WhileStmt => {
                let condition =
                    self.lower_required_expr(node.children.first().copied(), node.range);
                let body = node
                    .children
                    .get(1)
                    .copied()
                    .map_or_else(Vec::new, |child| self.lower_block(child, context));
                StmtKind::While { condition, body }
            }
            SyntaxKind::TryStmt => StmtKind::Try(self.lower_try(&node, context)),
            SyntaxKind::SwitchStmt => self.lower_switch(&node, context),
            SyntaxKind::FunctionDef => StmtKind::Function(self.lower_function(&node)),
            SyntaxKind::ArgumentsBlock => StmtKind::Arguments(self.lower_arguments(&node)),
            SyntaxKind::ClassDef => StmtKind::Class(self.lower_class(&node)),
            SyntaxKind::Error => StmtKind::Error,
            _ => return None,
        };
        let suppress_output = matches!(
            node.kind,
            SyntaxKind::AssignmentStmt | SyntaxKind::ExprStmt | SyntaxKind::CommandStmt
        ) && self
            .cst
            .node_tokens(id)
            .iter()
            .rev()
            .find(|token| !token.kind.is_trivia() && token.kind != TokenKind::Eof)
            .is_some_and(|token| token.kind == TokenKind::Semicolon);
        Some(Stmt {
            kind,
            span: node.range,
            suppress_output,
        })
    }

    fn lower_command(&self, id: NodeId, node: &openmat_syntax::SyntaxNode) -> CommandStatement {
        let callee = node
            .children
            .first()
            .copied()
            .and_then(|child| self.lower_name(child));
        let tokens = self.cst.node_tokens(id);
        let tail = tokens
            .iter()
            .position(|token| token.kind == TokenKind::Identifier)
            .and_then(|command| tokens.get(command.saturating_add(1)..))
            .unwrap_or(&[]);
        let base = tail
            .first()
            .map_or(node.range.end(), |token| token.range.start());
        let raw = tail
            .iter()
            .map(|token| token.text.as_str())
            .collect::<String>();
        CommandStatement {
            callee,
            arguments: parse_command_arguments(&raw, base),
        }
    }

    fn lower_clear(&self, node: &openmat_syntax::SyntaxNode) -> ClearStatement {
        let mut names = Vec::new();
        let mut unsupported = false;
        for child in node.children.iter().copied() {
            match self.cst.node(child).map(|node| node.kind) {
                Some(SyntaxKind::NameExpr) => {
                    if let Some(name) = self.lower_name(child) {
                        names.push(name);
                    }
                }
                _ => unsupported = true,
            }
        }
        let form = if unsupported {
            ClearForm::UnsupportedArguments
        } else if names.is_empty() {
            ClearForm::MissingNames
        } else {
            ClearForm::IdentifierList
        };
        ClearStatement { names, form }
    }

    fn lower_declaration(
        &mut self,
        node: &openmat_syntax::SyntaxNode,
        context: DeclarationContext,
    ) -> DeclarationStatement {
        let kind = if node.kind == SyntaxKind::GlobalStmt {
            DeclarationKind::Global
        } else {
            DeclarationKind::Persistent
        };
        let mut names = Vec::new();
        let mut invalid_items = false;
        let mut duplicate_persistent_name = false;
        for child in node.children.iter().copied() {
            match self.cst.node(child).map(|child| child.kind) {
                Some(SyntaxKind::NameExpr) => {
                    if let Some(name) = self.lower_name(child) {
                        if kind == DeclarationKind::Persistent
                            && names
                                .iter()
                                .any(|existing: &Name| existing.text == name.text)
                        {
                            duplicate_persistent_name = true;
                        }
                        names.push(name);
                    }
                }
                _ => invalid_items = true,
            }
        }
        let form = if invalid_items {
            DeclarationForm::InvalidItems
        } else if names.is_empty() {
            DeclarationForm::MissingNames
        } else if duplicate_persistent_name {
            DeclarationForm::DuplicatePersistentNames
        } else {
            DeclarationForm::IdentifierList
        };

        let context_is_valid = match kind {
            DeclarationKind::Global => {
                matches!(
                    context,
                    DeclarationContext::Script | DeclarationContext::Function
                )
            }
            DeclarationKind::Persistent => context == DeclarationContext::Function,
        };
        if !context_is_valid {
            let declaration = match kind {
                DeclarationKind::Global => "global",
                DeclarationKind::Persistent => "persistent",
            };
            self.diagnostics.push(
                Diagnostic::error(
                    self.cst.source_id(),
                    node.range,
                    format!("`{declaration}` declaration is not valid in this lexical context"),
                )
                .with_code("OMH0004"),
            );
        }

        DeclarationStatement {
            kind,
            names,
            form,
            context,
        }
    }

    fn lower_if(
        &mut self,
        node: &openmat_syntax::SyntaxNode,
        context: DeclarationContext,
    ) -> StmtKind {
        let mut branches = Vec::new();
        let condition = self.lower_required_expr(node.children.first().copied(), node.range);
        let body = node
            .children
            .get(1)
            .copied()
            .map_or_else(Vec::new, |child| self.lower_block(child, context));
        branches.push(ConditionalBranch {
            condition,
            body,
            span: node.range,
        });
        let mut else_body = Vec::new();
        for child in node.children.iter().skip(2).copied() {
            let Some(clause) = self.cst.node(child).cloned() else {
                continue;
            };
            match clause.kind {
                SyntaxKind::ElseifClause => {
                    let condition =
                        self.lower_required_expr(clause.children.first().copied(), clause.range);
                    let body = clause
                        .children
                        .get(1)
                        .copied()
                        .map_or_else(Vec::new, |block| self.lower_block(block, context));
                    branches.push(ConditionalBranch {
                        condition,
                        body,
                        span: clause.range,
                    });
                }
                SyntaxKind::ElseClause => {
                    else_body = clause
                        .children
                        .first()
                        .copied()
                        .map_or_else(Vec::new, |block| self.lower_block(block, context));
                }
                _ => {}
            }
        }
        StmtKind::If {
            branches,
            else_body,
        }
    }

    fn lower_switch(
        &mut self,
        node: &openmat_syntax::SyntaxNode,
        context: DeclarationContext,
    ) -> StmtKind {
        let selector_id = node.children.first().copied().filter(|child| {
            self.cst.node(*child).is_some_and(|child| {
                !matches!(
                    child.kind,
                    SyntaxKind::CaseClause | SyntaxKind::OtherwiseClause
                )
            })
        });
        let selector = self.lower_required_expr(selector_id, node.range);
        let mut cases = Vec::new();
        let mut otherwise = None;

        for child in node.children.iter().copied() {
            let Some(clause) = self.cst.node(child).cloned() else {
                continue;
            };
            match clause.kind {
                SyntaxKind::CaseClause => {
                    let expression_id = clause.children.iter().copied().find(|part| {
                        self.cst
                            .node(*part)
                            .is_some_and(|part| part.kind != SyntaxKind::Block)
                    });
                    let expression = self.lower_required_expr(expression_id, clause.range);
                    let body = clause
                        .children
                        .iter()
                        .copied()
                        .find(|part| {
                            self.cst
                                .node(*part)
                                .is_some_and(|part| part.kind == SyntaxKind::Block)
                        })
                        .map_or_else(Vec::new, |block| self.lower_block(block, context));
                    cases.push(SwitchCase {
                        expression,
                        body,
                        span: clause.range,
                    });
                }
                SyntaxKind::OtherwiseClause if otherwise.is_none() => {
                    let body = clause
                        .children
                        .iter()
                        .copied()
                        .find(|part| {
                            self.cst
                                .node(*part)
                                .is_some_and(|part| part.kind == SyntaxKind::Block)
                        })
                        .map_or_else(Vec::new, |block| self.lower_block(block, context));
                    otherwise = Some(OtherwiseBranch {
                        body,
                        span: clause.range,
                    });
                }
                _ => {}
            }
        }

        StmtKind::Switch {
            selector,
            cases,
            otherwise,
        }
    }

    fn lower_try(
        &mut self,
        node: &openmat_syntax::SyntaxNode,
        context: DeclarationContext,
    ) -> TryStatement {
        let body_id = node.children.iter().copied().find(|child| {
            self.cst
                .node(*child)
                .is_some_and(|child| child.kind == SyntaxKind::Block)
        });
        let (body, body_span) = body_id.map_or_else(
            || (Vec::new(), TextRange::empty(node.range.start())),
            |body| {
                let span = self
                    .cst
                    .node(body)
                    .map_or(TextRange::empty(node.range.start()), |body| body.range);
                (self.lower_block(body, context), span)
            },
        );
        let catch = node
            .children
            .iter()
            .copied()
            .find_map(|child| {
                self.cst
                    .node(child)
                    .filter(|child| child.kind == SyntaxKind::CatchClause)
                    .cloned()
            })
            .map(|clause| self.lower_catch_clause(&clause, context));

        TryStatement {
            body,
            body_span,
            catch,
        }
    }

    fn lower_catch_clause(
        &mut self,
        clause: &openmat_syntax::SyntaxNode,
        context: DeclarationContext,
    ) -> CatchClause {
        let variable = clause.children.iter().copied().find_map(|child| {
            self.cst
                .node(child)
                .filter(|child| child.kind == SyntaxKind::NameExpr)
                .and_then(|_| self.lower_name(child))
        });
        let body_id = clause.children.iter().copied().find(|child| {
            self.cst
                .node(*child)
                .is_some_and(|child| child.kind == SyntaxKind::Block)
        });
        let (body, body_span) = body_id.map_or_else(
            || (Vec::new(), TextRange::empty(clause.range.end())),
            |body| {
                let span = self
                    .cst
                    .node(body)
                    .map_or(TextRange::empty(clause.range.end()), |body| body.range);
                (self.lower_block(body, context), span)
            },
        );

        CatchClause {
            variable,
            body,
            body_span,
            span: clause.range,
        }
    }

    fn lower_block(&mut self, id: NodeId, context: DeclarationContext) -> Vec<Stmt> {
        let children = self
            .cst
            .node(id)
            .map_or_else(Vec::new, |node| node.children.clone());
        children
            .into_iter()
            .filter_map(|child| self.lower_stmt(child, context))
            .collect()
    }

    fn lower_function(&mut self, node: &openmat_syntax::SyntaxNode) -> FunctionDef {
        let mut name = None;
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut body = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::FunctionSignature => {
                    for part in child_node.children.iter().copied() {
                        let Some(part_node) = self.cst.node(part).cloned() else {
                            continue;
                        };
                        match part_node.kind {
                            SyntaxKind::OutputList => outputs = self.lower_name_list(&part_node),
                            SyntaxKind::ParameterList => inputs = self.lower_name_list(&part_node),
                            SyntaxKind::NameExpr => name = self.lower_name(part),
                            _ => {}
                        }
                    }
                }
                SyntaxKind::Block => {
                    body = self.lower_block(child, DeclarationContext::Function);
                }
                _ => {}
            }
        }
        FunctionDef {
            name,
            outputs,
            inputs,
            body,
            span: node.range,
        }
    }

    fn lower_arguments(&mut self, node: &openmat_syntax::SyntaxNode) -> ArgumentsBlock {
        let mut block = ArgumentsBlock {
            attributes: Vec::new(),
            declarations: Vec::new(),
        };
        for child in &node.children {
            let Some(part) = self.cst.node(*child).cloned() else {
                continue;
            };
            if part.kind == SyntaxKind::AttributeList {
                block.attributes = self.lower_attributes(*child);
            } else if part.kind == SyntaxKind::ArgumentDecl {
                let Some(name) = part
                    .children
                    .first()
                    .and_then(|child| self.lower_name(*child))
                else {
                    continue;
                };
                let mut declaration = ArgumentDeclaration {
                    name,
                    dimensions: Vec::new(),
                    class: None,
                    validators: Vec::new(),
                    default: None,
                    span: part.range,
                };
                for child in part.children.iter().skip(1) {
                    let Some(item) = self.cst.node(*child).cloned() else {
                        continue;
                    };
                    match item.kind {
                        SyntaxKind::ArgumentDimensions => {
                            declaration.dimensions = item
                                .children
                                .iter()
                                .map(|child| self.lower_expr(*child))
                                .collect();
                        }
                        SyntaxKind::ArgumentValidators => {
                            declaration.validators = item
                                .children
                                .iter()
                                .map(|child| self.lower_expr(*child))
                                .collect();
                        }
                        SyntaxKind::ArgumentClass => {
                            declaration.class = Some(Name {
                                text: self
                                    .cst
                                    .node_tokens(*child)
                                    .iter()
                                    .filter(|token| !token.kind.is_trivia())
                                    .map(|token| token.text.as_str())
                                    .collect(),
                                span: item.range,
                            });
                        }
                        SyntaxKind::ArgumentDefault => {
                            declaration.default =
                                item.children.first().map(|child| self.lower_expr(*child));
                        }
                        _ => {}
                    }
                }
                block.declarations.push(declaration);
            }
        }
        block
    }

    fn lower_name_list(&self, node: &openmat_syntax::SyntaxNode) -> Vec<Name> {
        node.children
            .iter()
            .filter_map(|child| self.lower_name(*child))
            .collect()
    }

    fn lower_class(&mut self, node: &openmat_syntax::SyntaxNode) -> ClassDef {
        let mut name = None;
        let mut superclass = None;
        let mut attributes = Vec::new();
        let mut property_blocks = Vec::new();
        let mut method_blocks = Vec::new();
        let mut enumeration_blocks = Vec::new();
        let mut event_blocks = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::ClassHeader => {
                    let mut names = Vec::new();
                    for header_child in child_node.children.iter().copied() {
                        match self.cst.node(header_child).map(|item| item.kind) {
                            Some(SyntaxKind::AttributeList) => {
                                attributes = self.lower_attributes(header_child);
                            }
                            Some(SyntaxKind::NameExpr) => {
                                if let Some(lowered) = self.lower_name(header_child) {
                                    names.push(lowered);
                                }
                            }
                            _ => {}
                        }
                    }
                    name = names.first().cloned();
                    superclass = names.get(1).cloned();
                }
                SyntaxKind::PropertiesBlock => {
                    property_blocks.push(self.lower_property_block(&child_node));
                }
                SyntaxKind::MethodsBlock => {
                    method_blocks.push(self.lower_method_block(&child_node));
                }
                SyntaxKind::EnumerationBlock => {
                    enumeration_blocks.push(self.lower_enumeration_block(&child_node));
                }
                SyntaxKind::EventsBlock => {
                    event_blocks.push(self.lower_event_block(&child_node));
                }
                SyntaxKind::GlobalStmt | SyntaxKind::PersistentStmt => {
                    let _ = self.lower_stmt(child, DeclarationContext::ClassDefinition);
                }
                _ => {}
            }
        }
        ClassDef {
            name,
            superclass,
            attributes,
            property_blocks,
            method_blocks,
            enumeration_blocks,
            event_blocks,
            span: node.range,
        }
    }

    fn lower_enumeration_block(&mut self, node: &openmat_syntax::SyntaxNode) -> EnumerationBlock {
        let mut attributes = Vec::new();
        let mut members = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::AttributeList => attributes = self.lower_attributes(child),
                SyntaxKind::EnumMemberDecl => {
                    let name = child_node
                        .children
                        .first()
                        .copied()
                        .and_then(|name| self.lower_name(name));
                    let arguments = child_node
                        .children
                        .iter()
                        .skip(1)
                        .copied()
                        .map(|argument| self.lower_expr(argument))
                        .collect();
                    members.push(EnumMemberDef {
                        name,
                        arguments,
                        span: child_node.range,
                    });
                }
                _ => {}
            }
        }
        EnumerationBlock {
            attributes,
            members,
            span: node.range,
        }
    }

    fn lower_event_block(&mut self, node: &openmat_syntax::SyntaxNode) -> EventBlock {
        let mut attributes = Vec::new();
        let mut events = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::AttributeList => attributes = self.lower_attributes(child),
                SyntaxKind::EventDecl => events.push(EventDef {
                    name: child_node
                        .children
                        .first()
                        .copied()
                        .and_then(|name| self.lower_name(name)),
                    span: child_node.range,
                }),
                _ => {}
            }
        }
        EventBlock {
            attributes,
            events,
            span: node.range,
        }
    }

    fn lower_property_block(&mut self, node: &openmat_syntax::SyntaxNode) -> PropertyBlock {
        let mut attributes = Vec::new();
        let mut properties = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::AttributeList => attributes = self.lower_attributes(child),
                SyntaxKind::PropertyDecl => {
                    let name = child_node
                        .children
                        .first()
                        .copied()
                        .and_then(|name_id| self.lower_name(name_id));
                    let default = child_node
                        .children
                        .get(1)
                        .copied()
                        .map(|value| self.lower_expr(value));
                    properties.push(PropertyDef {
                        name,
                        default,
                        span: child_node.range,
                    });
                }
                _ => {}
            }
        }
        PropertyBlock {
            attributes,
            properties,
            span: node.range,
        }
    }

    fn lower_method_block(&mut self, node: &openmat_syntax::SyntaxNode) -> MethodBlock {
        let mut attributes = Vec::new();
        let mut methods = Vec::new();
        let mut declarations = Vec::new();
        for child in node.children.iter().copied() {
            let Some(child_node) = self.cst.node(child).cloned() else {
                continue;
            };
            match child_node.kind {
                SyntaxKind::AttributeList => attributes = self.lower_attributes(child),
                SyntaxKind::FunctionDef => methods.push(self.lower_function(&child_node)),
                SyntaxKind::MethodDecl => {
                    let mut name = None;
                    let mut outputs = Vec::new();
                    let mut inputs = Vec::new();
                    for part in child_node.children.iter().copied() {
                        let Some(part_node) = self.cst.node(part).cloned() else {
                            continue;
                        };
                        match part_node.kind {
                            SyntaxKind::OutputList => outputs = self.lower_name_list(&part_node),
                            SyntaxKind::ParameterList => inputs = self.lower_name_list(&part_node),
                            SyntaxKind::NameExpr => name = self.lower_name(part),
                            _ => {}
                        }
                    }
                    declarations.push(MethodDeclaration {
                        name,
                        outputs,
                        inputs,
                        span: child_node.range,
                    });
                }
                SyntaxKind::GlobalStmt | SyntaxKind::PersistentStmt => {
                    let _ = self.lower_stmt(child, DeclarationContext::ClassDefinition);
                }
                _ => {}
            }
        }
        MethodBlock {
            attributes,
            methods,
            declarations,
            span: node.range,
        }
    }

    fn lower_attributes(&mut self, id: NodeId) -> Vec<Attribute> {
        let children = self
            .cst
            .node(id)
            .map_or_else(Vec::new, |node| node.children.clone());
        children
            .into_iter()
            .filter_map(|attribute_id| {
                let node = self.cst.node(attribute_id)?.clone();
                if node.kind != SyntaxKind::Attribute {
                    return None;
                }
                let name = node
                    .children
                    .first()
                    .copied()
                    .and_then(|name_id| self.lower_name(name_id));
                let value = node
                    .children
                    .get(1)
                    .copied()
                    .map(|value_id| self.lower_expr(value_id));
                Some(Attribute {
                    name,
                    value,
                    span: node.range,
                })
            })
            .collect()
    }

    fn lower_name(&self, id: NodeId) -> Option<Name> {
        let node = self.cst.node(id)?;
        if node.kind != SyntaxKind::NameExpr {
            return None;
        }
        let tokens = self.cst.node_tokens(id);
        tokens.iter().find(|token| {
            token.kind == TokenKind::Identifier
                || token.kind == TokenKind::Tilde
                || is_keyword_name_token(token.kind)
        })?;
        let mut text = String::new();
        for token in tokens.iter().filter(|token| {
            matches!(
                token.kind,
                TokenKind::Identifier | TokenKind::Tilde | TokenKind::Dot
            ) || is_keyword_name_token(token.kind)
        }) {
            text.push_str(&token.text);
        }
        Some(Name {
            text,
            span: node.range,
        })
    }

    fn lower_qualified_name(&self, ids: &[NodeId]) -> Option<Name> {
        let components = ids
            .iter()
            .copied()
            .map(|id| self.lower_name(id))
            .collect::<Option<Vec<_>>>()?;
        let first = components.first()?;
        let last = components.last()?;
        Some(Name {
            text: components
                .iter()
                .map(|component| component.text.as_str())
                .collect::<Vec<_>>()
                .join("."),
            span: TextRange::new(first.span.start(), last.span.end()).ok()?,
        })
    }

    fn lower_required_expr(&mut self, id: Option<NodeId>, fallback: TextRange) -> Expr {
        if let Some(id) = id {
            return self.lower_expr(id);
        }
        self.diagnostics.push(
            Diagnostic::error(
                self.cst.source_id(),
                fallback,
                "missing expression during HIR lowering",
            )
            .with_code("OMH0001"),
        );
        Expr {
            kind: ExprKind::Error,
            span: fallback,
        }
    }

    fn lower_expr(&mut self, id: NodeId) -> Expr {
        let Some(node) = self.cst.node(id).cloned() else {
            return self.invalid_expr();
        };
        let kind = match node.kind {
            SyntaxKind::NameExpr => ExprKind::Name(self.first_token_text(id)),
            SyntaxKind::NumberExpr => ExprKind::Number(self.first_token_text(id)),
            SyntaxKind::CharExpr => ExprKind::Char(self.first_token_text(id)),
            SyntaxKind::StringExpr => ExprKind::String(self.first_token_text(id)),
            SyntaxKind::ColonExpr => ExprKind::AllIndex,
            SyntaxKind::EndIndexExpr => ExprKind::EndIndex,
            SyntaxKind::FunctionHandleExpr => {
                ExprKind::FunctionHandle(self.lower_qualified_name(&node.children))
            }
            SyntaxKind::AnonymousFunctionExpr => self.lower_anonymous_function(&node),
            SyntaxKind::ParenExpr => ExprKind::Paren(Box::new(
                self.lower_required_expr(node.children.first().copied(), node.range),
            )),
            SyntaxKind::ParenApplyExpr => self.lower_apply(&node, false),
            SyntaxKind::BraceApplyExpr => self.lower_apply(&node, true),
            SyntaxKind::SuperclassConstructorCallExpr => {
                self.lower_superclass_constructor_call(&node)
            }
            SyntaxKind::FieldExpr => self.lower_field(&node, false),
            SyntaxKind::DynamicFieldExpr => self.lower_field(&node, true),
            SyntaxKind::UnaryExpr => {
                let operand = self.lower_required_expr(node.children.first().copied(), node.range);
                let operator = self
                    .operator_before(node.children.first().copied())
                    .map_or(UnaryOp::Unknown, unary_operator);
                ExprKind::Unary {
                    operator,
                    operand: Box::new(operand),
                }
            }
            SyntaxKind::BinaryExpr => {
                let left_id = node.children.first().copied();
                let right_id = node.children.get(1).copied();
                let left = self.lower_required_expr(left_id, node.range);
                let right = self.lower_required_expr(right_id, node.range);
                let operator = self
                    .operator_between(left_id, right_id)
                    .map_or(BinaryOp::Unknown, binary_operator);
                ExprKind::Binary {
                    operator,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            }
            SyntaxKind::RangeExpr => self.lower_range(&node),
            SyntaxKind::TransposeExpr => {
                let operand_id = node.children.first().copied();
                let operand = self.lower_required_expr(operand_id, node.range);
                let kind = self
                    .operator_after(operand_id)
                    .map_or(TransposeKind::Unknown, transpose_operator);
                ExprKind::Transpose {
                    kind,
                    operand: Box::new(operand),
                }
            }
            SyntaxKind::MatrixExpr => ExprKind::Matrix(self.lower_rows(&node)),
            SyntaxKind::CellExpr => ExprKind::Cell(self.lower_rows(&node)),
            SyntaxKind::Error => ExprKind::Error,
            _ => {
                self.diagnostics.push(
                    Diagnostic::error(
                        self.cst.source_id(),
                        node.range,
                        "CST node is not an expression",
                    )
                    .with_code("OMH0003"),
                );
                ExprKind::Error
            }
        };
        Expr {
            kind,
            span: node.range,
        }
    }

    fn lower_anonymous_function(&mut self, node: &openmat_syntax::SyntaxNode) -> ExprKind {
        let parameters = node
            .children
            .first()
            .and_then(|parameters| self.cst.node(*parameters))
            .map_or_else(Vec::new, |parameters| {
                parameters
                    .children
                    .iter()
                    .filter_map(|parameter| self.lower_name(*parameter))
                    .collect()
            });
        let body = self.lower_required_expr(node.children.get(1).copied(), node.range);
        ExprKind::AnonymousFunction {
            parameters,
            body: Box::new(body),
        }
    }

    fn lower_superclass_constructor_call(&mut self, node: &openmat_syntax::SyntaxNode) -> ExprKind {
        let object = self.lower_required_expr(node.children.first().copied(), node.range);
        let superclass = node
            .children
            .get(1)
            .copied()
            .and_then(|child| self.lower_name(child));
        let arguments = self.lower_call_arguments(node.children.get(2..).unwrap_or(&[]));
        ExprKind::SuperclassConstructorCall {
            object: Box::new(object),
            superclass,
            arguments,
        }
    }

    fn lower_apply(&mut self, node: &openmat_syntax::SyntaxNode, brace: bool) -> ExprKind {
        let target = self.lower_required_expr(node.children.first().copied(), node.range);
        let arguments = self.lower_call_arguments(node.children.get(1..).unwrap_or(&[]));
        if brace {
            ExprKind::BraceApply {
                target: Box::new(target),
                arguments,
            }
        } else {
            ExprKind::ParenApply {
                target: Box::new(target),
                arguments,
            }
        }
    }

    fn lower_call_arguments(&mut self, children: &[NodeId]) -> Vec<Expr> {
        let mut arguments = Vec::new();
        for child in children {
            if let Some(node) = self.cst.node(*child).cloned()
                && node.kind == SyntaxKind::NameValueArgument
            {
                if let Some(name) = node.children.first().and_then(|id| self.lower_name(*id)) {
                    arguments.push(Expr {
                        kind: ExprKind::String(format!("\"{}\"", name.text)),
                        span: name.span,
                    });
                }
                arguments.push(self.lower_required_expr(node.children.get(1).copied(), node.range));
            } else {
                arguments.push(self.lower_expr(*child));
            }
        }
        arguments
    }

    fn lower_field(&mut self, node: &openmat_syntax::SyntaxNode, dynamic: bool) -> ExprKind {
        let target = self.lower_required_expr(node.children.first().copied(), node.range);
        if dynamic {
            let name = self.lower_required_expr(node.children.get(1).copied(), node.range);
            ExprKind::DynamicField {
                target: Box::new(target),
                name: Box::new(name),
            }
        } else {
            let name = node
                .children
                .get(1)
                .copied()
                .and_then(|child| self.lower_name(child));
            ExprKind::Field {
                target: Box::new(target),
                name,
            }
        }
    }

    fn invalid_expr(&mut self) -> Expr {
        let span = TextRange::empty(0);
        self.diagnostics.push(
            Diagnostic::error(self.cst.source_id(), span, "invalid CST node reference")
                .with_code("OMH0002"),
        );
        Expr {
            kind: ExprKind::Error,
            span,
        }
    }

    fn lower_range(&mut self, node: &openmat_syntax::SyntaxNode) -> ExprKind {
        let left = self.lower_required_expr(node.children.first().copied(), node.range);
        let right = self.lower_required_expr(node.children.get(1).copied(), node.range);
        if let ExprKind::Range {
            start,
            step: None,
            end,
        } = left.kind
        {
            ExprKind::Range {
                start,
                step: Some(end),
                end: Box::new(right),
            }
        } else {
            ExprKind::Range {
                start: Box::new(left),
                step: None,
                end: Box::new(right),
            }
        }
    }

    fn lower_rows(&mut self, node: &openmat_syntax::SyntaxNode) -> Vec<Vec<Expr>> {
        node.children
            .iter()
            .filter_map(|row_id| self.cst.node(*row_id).cloned())
            .map(|row| {
                row.children
                    .into_iter()
                    .map(|element| self.lower_expr(element))
                    .collect()
            })
            .collect()
    }

    fn first_token_text(&self, id: NodeId) -> String {
        self.cst
            .node_tokens(id)
            .iter()
            .find(|token| !token.kind.is_trivia())
            .map_or_else(String::new, |token| token.text.clone())
    }

    fn operator_before(&self, child: Option<NodeId>) -> Option<TokenKind> {
        let child_start = child
            .and_then(|id| self.cst.node(id))
            .map_or(0, |node| node.token_range.start);
        self.cst
            .tokens()
            .get(..child_start)?
            .iter()
            .rev()
            .find(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
    }

    fn operator_after(&self, child: Option<NodeId>) -> Option<TokenKind> {
        let child_end = child
            .and_then(|id| self.cst.node(id))
            .map_or(0, |node| node.token_range.end);
        self.cst
            .tokens()
            .get(child_end..)?
            .iter()
            .find(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
    }

    fn operator_between(&self, left: Option<NodeId>, right: Option<NodeId>) -> Option<TokenKind> {
        let start = left
            .and_then(|id| self.cst.node(id))
            .map_or(0, |node| node.token_range.end);
        let end = right
            .and_then(|id| self.cst.node(id))
            .map_or(start, |node| node.token_range.start);
        self.cst
            .tokens()
            .get(start..end)?
            .iter()
            .find(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
    }
}

const fn is_keyword_name_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::KwBreak
            | TokenKind::KwCase
            | TokenKind::KwCatch
            | TokenKind::KwClassdef
            | TokenKind::KwContinue
            | TokenKind::KwElse
            | TokenKind::KwElseif
            | TokenKind::KwEnd
            | TokenKind::KwEnumeration
            | TokenKind::KwEvents
            | TokenKind::KwFor
            | TokenKind::KwFunction
            | TokenKind::KwGlobal
            | TokenKind::KwIf
            | TokenKind::KwMethods
            | TokenKind::KwOtherwise
            | TokenKind::KwParfor
            | TokenKind::KwPersistent
            | TokenKind::KwProperties
            | TokenKind::KwReturn
            | TokenKind::KwSpmd
            | TokenKind::KwSwitch
            | TokenKind::KwTry
            | TokenKind::KwWhile
    )
}

fn parse_command_arguments(raw: &str, base: u32) -> Vec<CommandArgument> {
    let mut arguments = Vec::new();
    let mut text = String::new();
    let mut active = false;
    let mut start = base;
    let mut end = base;
    let mut quoted = false;
    let mut index = 0;

    while index < raw.len() {
        let character = raw[index..]
            .chars()
            .next()
            .expect("the byte index remains on a UTF-8 boundary");
        let next = index + character.len_utf8();
        if quoted {
            if character == '\'' {
                if raw[next..].starts_with('\'') {
                    let escaped_end = next + '\''.len_utf8();
                    text.push('\'');
                    end = command_offset(base, escaped_end);
                    index = escaped_end;
                } else {
                    end = command_offset(base, next);
                    quoted = false;
                    index = next;
                }
            } else {
                text.push(character);
                end = command_offset(base, next);
                index = next;
            }
            continue;
        }

        if raw[index..].starts_with("...") {
            finish_command_argument(&mut arguments, &mut text, &mut active, start, end);
            index += 3;
            while index < raw.len() {
                let skipped = raw[index..]
                    .chars()
                    .next()
                    .expect("the byte index remains on a UTF-8 boundary");
                index += skipped.len_utf8();
                if skipped == '\n' {
                    break;
                }
                if skipped == '\r' {
                    if raw[index..].starts_with('\n') {
                        index += 1;
                    }
                    break;
                }
            }
            continue;
        }

        match character {
            '\'' => {
                if !active {
                    active = true;
                    start = command_offset(base, index);
                }
                end = command_offset(base, next);
                quoted = true;
                index = next;
            }
            '%' | ';' | ',' | '\r' | '\n' => {
                finish_command_argument(&mut arguments, &mut text, &mut active, start, end);
                break;
            }
            value if value.is_whitespace() => {
                finish_command_argument(&mut arguments, &mut text, &mut active, start, end);
                index = next;
            }
            value => {
                if !active {
                    active = true;
                    start = command_offset(base, index);
                }
                text.push(value);
                end = command_offset(base, next);
                index = next;
            }
        }
    }
    finish_command_argument(&mut arguments, &mut text, &mut active, start, end);
    arguments
}

fn finish_command_argument(
    arguments: &mut Vec<CommandArgument>,
    text: &mut String,
    active: &mut bool,
    start: u32,
    end: u32,
) {
    if !*active {
        return;
    }
    let span = TextRange::new(start, end).unwrap_or(TextRange::empty(start));
    arguments.push(CommandArgument {
        text: std::mem::take(text),
        span,
    });
    *active = false;
}

fn command_offset(base: u32, relative: usize) -> u32 {
    base.saturating_add(u32::try_from(relative).unwrap_or(u32::MAX))
}

fn unary_operator(kind: TokenKind) -> UnaryOp {
    match kind {
        TokenKind::Plus => UnaryOp::Plus,
        TokenKind::Minus => UnaryOp::Minus,
        TokenKind::Tilde => UnaryOp::Not,
        _ => UnaryOp::Unknown,
    }
}

fn binary_operator(kind: TokenKind) -> BinaryOp {
    match kind {
        TokenKind::Plus => BinaryOp::Add,
        TokenKind::Minus => BinaryOp::Subtract,
        TokenKind::Star => BinaryOp::Multiply,
        TokenKind::Slash => BinaryOp::RightDivide,
        TokenKind::Backslash => BinaryOp::LeftDivide,
        TokenKind::Caret => BinaryOp::Power,
        TokenKind::DotStar => BinaryOp::ElementMultiply,
        TokenKind::DotSlash => BinaryOp::ElementRightDivide,
        TokenKind::DotBackslash => BinaryOp::ElementLeftDivide,
        TokenKind::DotCaret => BinaryOp::ElementPower,
        TokenKind::EqualEqual => BinaryOp::Equal,
        TokenKind::NotEqual => BinaryOp::NotEqual,
        TokenKind::Less => BinaryOp::Less,
        TokenKind::LessEqual => BinaryOp::LessEqual,
        TokenKind::Greater => BinaryOp::Greater,
        TokenKind::GreaterEqual => BinaryOp::GreaterEqual,
        TokenKind::And => BinaryOp::And,
        TokenKind::AndAnd => BinaryOp::ShortCircuitAnd,
        TokenKind::Or => BinaryOp::Or,
        TokenKind::OrOr => BinaryOp::ShortCircuitOr,
        _ => BinaryOp::Unknown,
    }
}

fn transpose_operator(kind: TokenKind) -> TransposeKind {
    match kind {
        TokenKind::ConjugateTranspose => TransposeKind::Conjugate,
        TokenKind::DotTranspose => TransposeKind::NonConjugate,
        _ => TransposeKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BinaryOp, ClearForm, DeclarationContext, DeclarationForm, DeclarationKind, ExprKind,
        StmtKind, TransposeKind, lower, parse_command_arguments,
    };
    use openmat_source::{SourceId, TextRange};
    use openmat_syntax::{Cst, NodeId, SyntaxKind, SyntaxNode, Token, TokenKind};

    fn tokens(specification: &[(TokenKind, &str)]) -> Vec<Token> {
        let mut offset = 0_u32;
        specification
            .iter()
            .map(|(kind, text)| {
                let start = offset;
                offset += u32::try_from(text.len()).expect("small test token");
                Token::new(
                    *kind,
                    *text,
                    TextRange::new(start, offset).expect("ordered test range"),
                )
            })
            .collect()
    }

    fn node(
        tokens: &[Token],
        kind: SyntaxKind,
        start: usize,
        end: usize,
        children: Vec<NodeId>,
    ) -> SyntaxNode {
        let range_start = tokens[start].range.start();
        let range_end = tokens[end - 1].range.end();
        SyntaxNode::new(
            kind,
            TextRange::new(range_start, range_end).expect("ordered node range"),
            start..end,
            children,
        )
    }

    #[test]
    fn lowers_command_arguments_as_r2022b_character_text() {
        let arguments = parse_command_arguments(
            " 'alpha beta' \"gamma delta\" ... continuation\r\n plain% trailing",
            10,
        );
        assert_eq!(
            arguments
                .iter()
                .map(|argument| argument.text.as_str())
                .collect::<Vec<_>>(),
            ["alpha beta", "\"gamma", "delta\"", "plain"]
        );

        let tokens = tokens(&[
            (TokenKind::Identifier, "hold"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "on"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::CommandStmt, 0, 4, vec![NodeId::new(0)]),
            node(&tokens, SyntaxKind::Root, 0, 4, vec![NodeId::new(1)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(40), tokens, nodes, NodeId::new(2)));
        let StmtKind::Command(command) = &lowered.file.statements[0].kind else {
            panic!("expected command-form statement");
        };
        assert_eq!(
            command.callee.as_ref().map(|name| name.text.as_str()),
            Some("hold")
        );
        assert_eq!(command.arguments[0].text, "on");
        assert!(lowered.file.statements[0].suppress_output);
    }

    fn empty_node(tokens: &[Token], kind: SyntaxKind, at: usize) -> SyntaxNode {
        SyntaxNode::new(
            kind,
            TextRange::empty(tokens[at].range.start()),
            at..at,
            Vec::new(),
        )
    }

    fn expression_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::Identifier, "y"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "f"),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "x"),
            (TokenKind::RParen, ")"),
            (TokenKind::Plus, "+"),
            (TokenKind::Number, "3"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Identifier, "z"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "A"),
            (TokenKind::DotTranspose, ".'"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                2,
                6,
                vec![NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::NumberExpr, 7, 8, vec![]),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                2,
                8,
                vec![NodeId::new(3), NodeId::new(4)],
            ),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                0,
                9,
                vec![NodeId::new(0), NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 9, 10, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 11, 12, vec![]),
            node(
                &tokens,
                SyntaxKind::TransposeExpr,
                11,
                13,
                vec![NodeId::new(8)],
            ),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                9,
                14,
                vec![NodeId::new(7), NodeId::new(9)],
            ),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                15,
                vec![NodeId::new(6), NodeId::new(10)],
            ),
        ];
        Cst::new(SourceId::new(11), tokens, nodes, NodeId::new(11))
    }

    fn switch_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwSwitch, "switch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "key"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCase, "case"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Number, "1"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Identifier, "a"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "10"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCase, "case"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Number, "2"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwBreak, "break"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwOtherwise, "otherwise"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Identifier, "a"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "30"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 6, 7, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 10, 11, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                8,
                12,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::Block, 8, 13, vec![NodeId::new(4)]),
            node(
                &tokens,
                SyntaxKind::CaseClause,
                4,
                13,
                vec![NodeId::new(1), NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::NumberExpr, 15, 16, vec![]),
            node(&tokens, SyntaxKind::BreakStmt, 17, 18, vec![]),
            node(&tokens, SyntaxKind::Block, 17, 19, vec![NodeId::new(8)]),
            node(
                &tokens,
                SyntaxKind::CaseClause,
                13,
                19,
                vec![NodeId::new(7), NodeId::new(9)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 21, 22, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 23, 24, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                21,
                25,
                vec![NodeId::new(11), NodeId::new(12)],
            ),
            node(&tokens, SyntaxKind::Block, 21, 26, vec![NodeId::new(13)]),
            node(
                &tokens,
                SyntaxKind::OtherwiseClause,
                19,
                26,
                vec![NodeId::new(14)],
            ),
            node(
                &tokens,
                SyntaxKind::SwitchStmt,
                0,
                28,
                vec![
                    NodeId::new(0),
                    NodeId::new(6),
                    NodeId::new(10),
                    NodeId::new(15),
                ],
            ),
            node(&tokens, SyntaxKind::Root, 0, 29, vec![NodeId::new(16)]),
        ];
        Cst::new(SourceId::new(19), tokens, nodes, NodeId::new(17))
    }

    fn malformed_switch_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwSwitch, "switch"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCase, "case"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let empty_body = SyntaxNode::new(
            SyntaxKind::Block,
            TextRange::empty(tokens[4].range.start()),
            4..4,
            Vec::new(),
        );
        let nodes = vec![
            empty_body,
            node(&tokens, SyntaxKind::CaseClause, 2, 4, vec![NodeId::new(0)]),
            node(&tokens, SyntaxKind::SwitchStmt, 0, 6, vec![NodeId::new(1)]),
            node(&tokens, SyntaxKind::Root, 0, 7, vec![NodeId::new(2)]),
        ];
        Cst::new(SourceId::new(20), tokens, nodes, NodeId::new(3))
    }

    #[allow(clippy::too_many_lines)]
    fn try_forms_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwTry, "try"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Identifier, "x"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "1"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwTry, "try"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwTry, "try"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Identifier, "x"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "3"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "caught"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Identifier, "y"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "4"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                2,
                6,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::Block, 2, 7, vec![NodeId::new(2)]),
            node(&tokens, SyntaxKind::TryStmt, 0, 8, vec![NodeId::new(3)]),
            empty_node(&tokens, SyntaxKind::Block, 11),
            empty_node(&tokens, SyntaxKind::Block, 13),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                11,
                13,
                vec![NodeId::new(6)],
            ),
            node(
                &tokens,
                SyntaxKind::TryStmt,
                9,
                14,
                vec![NodeId::new(5), NodeId::new(7)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 17, 18, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 19, 20, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                17,
                21,
                vec![NodeId::new(9), NodeId::new(10)],
            ),
            node(&tokens, SyntaxKind::Block, 17, 22, vec![NodeId::new(11)]),
            node(&tokens, SyntaxKind::NameExpr, 24, 25, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 26, 27, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 28, 29, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                26,
                30,
                vec![NodeId::new(14), NodeId::new(15)],
            ),
            node(&tokens, SyntaxKind::Block, 26, 31, vec![NodeId::new(16)]),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                22,
                31,
                vec![NodeId::new(13), NodeId::new(17)],
            ),
            node(
                &tokens,
                SyntaxKind::TryStmt,
                15,
                32,
                vec![NodeId::new(12), NodeId::new(18)],
            ),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                34,
                vec![NodeId::new(4), NodeId::new(8), NodeId::new(19)],
            ),
        ];
        Cst::new(SourceId::new(28), tokens, nodes, NodeId::new(20))
    }

    fn nested_try_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwTry, "try"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwTry, "try"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "inner"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "outer"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            empty_node(&tokens, SyntaxKind::Block, 4),
            node(&tokens, SyntaxKind::NameExpr, 6, 7, vec![]),
            empty_node(&tokens, SyntaxKind::Block, 8),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                4,
                8,
                vec![NodeId::new(1), NodeId::new(2)],
            ),
            node(
                &tokens,
                SyntaxKind::TryStmt,
                2,
                9,
                vec![NodeId::new(0), NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::Block, 2, 10, vec![NodeId::new(4)]),
            node(&tokens, SyntaxKind::NameExpr, 12, 13, vec![]),
            empty_node(&tokens, SyntaxKind::Block, 14),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                10,
                14,
                vec![NodeId::new(6), NodeId::new(7)],
            ),
            node(
                &tokens,
                SyntaxKind::TryStmt,
                0,
                15,
                vec![NodeId::new(5), NodeId::new(8)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 17, vec![NodeId::new(9)]),
        ];
        Cst::new(SourceId::new(29), tokens, nodes, NodeId::new(10))
    }

    fn duplicate_catch_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwTry, "try"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "first"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwCatch, "catch"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "second"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            empty_node(&tokens, SyntaxKind::Block, 2),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            empty_node(&tokens, SyntaxKind::Block, 6),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                2,
                6,
                vec![NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            empty_node(&tokens, SyntaxKind::Block, 10),
            node(
                &tokens,
                SyntaxKind::CatchClause,
                6,
                10,
                vec![NodeId::new(4), NodeId::new(5)],
            ),
            node(
                &tokens,
                SyntaxKind::TryStmt,
                0,
                11,
                vec![NodeId::new(0), NodeId::new(3), NodeId::new(6)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 13, vec![NodeId::new(7)]),
        ];
        Cst::new(SourceId::new(30), tokens, nodes, NodeId::new(8))
    }

    fn range_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::Number, "1"),
            (TokenKind::Colon, ":"),
            (TokenKind::Number, "2"),
            (TokenKind::Colon, ":"),
            (TokenKind::Number, "9"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NumberExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 2, 3, vec![]),
            node(
                &tokens,
                SyntaxKind::RangeExpr,
                0,
                3,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NumberExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::RangeExpr,
                0,
                5,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 6, vec![NodeId::new(4)]),
            node(&tokens, SyntaxKind::Root, 0, 7, vec![NodeId::new(5)]),
        ];
        Cst::new(SourceId::new(12), tokens, nodes, NodeId::new(6))
    }

    fn two_operand_range_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::Number, "1"),
            (TokenKind::Colon, ":"),
            (TokenKind::Number, "2"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NumberExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 2, 3, vec![]),
            node(
                &tokens,
                SyntaxKind::RangeExpr,
                0,
                3,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 4, vec![NodeId::new(2)]),
            node(&tokens, SyntaxKind::Root, 0, 5, vec![NodeId::new(3)]),
        ];
        Cst::new(SourceId::new(13), tokens, nodes, NodeId::new(4))
    }

    fn all_index_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::Identifier, "matrix"),
            (TokenKind::LParen, "("),
            (TokenKind::Colon, ":"),
            (TokenKind::Comma, ","),
            (TokenKind::Number, "2"),
            (TokenKind::RParen, ")"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::ColonExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                0,
                6,
                vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 7, vec![NodeId::new(3)]),
            node(&tokens, SyntaxKind::Root, 0, 8, vec![NodeId::new(4)]),
        ];
        Cst::new(SourceId::new(14), tokens, nodes, NodeId::new(5))
    }

    fn power_chain_cst(operator: TokenKind, spelling: &str, source_id: SourceId) -> Cst {
        let tokens = tokens(&[
            (TokenKind::Number, "2"),
            (operator, spelling),
            (TokenKind::Number, "3"),
            (operator, spelling),
            (TokenKind::Number, "2"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NumberExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 2, 3, vec![]),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                0,
                3,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NumberExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                0,
                5,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 6, vec![NodeId::new(4)]),
            node(&tokens, SyntaxKind::Root, 0, 7, vec![NodeId::new(5)]),
        ];
        Cst::new(source_id, tokens, nodes, NodeId::new(6))
    }

    fn short_circuit_tokens() -> Vec<Token> {
        tokens(&[
            (TokenKind::LBracket, "["),
            (TokenKind::Identifier, "false"),
            (TokenKind::Whitespace, " "),
            (TokenKind::AndAnd, "&&"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Ellipsis, "..."),
            (TokenKind::Newline, "\n"),
            (TokenKind::Whitespace, "    "),
            (TokenKind::Identifier, "missing"),
            (TokenKind::LParen, "("),
            (TokenKind::RParen, ")"),
            (TokenKind::Comma, ","),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "true"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Ellipsis, "..."),
            (TokenKind::Newline, "\n"),
            (TokenKind::Whitespace, "    "),
            (TokenKind::OrOr, "||"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "missing"),
            (TokenKind::LParen, "("),
            (TokenKind::RParen, ")"),
            (TokenKind::RBracket, "]"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ])
    }

    fn short_circuit_continuation_cst() -> Cst {
        let tokens = short_circuit_tokens();
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                8,
                11,
                vec![NodeId::new(1)],
            ),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                1,
                11,
                vec![NodeId::new(0), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 13, 14, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 20, 21, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                20,
                23,
                vec![NodeId::new(5)],
            ),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                13,
                23,
                vec![NodeId::new(4), NodeId::new(6)],
            ),
            node(
                &tokens,
                SyntaxKind::MatrixRow,
                1,
                23,
                vec![NodeId::new(3), NodeId::new(7)],
            ),
            node(&tokens, SyntaxKind::MatrixExpr, 0, 24, vec![NodeId::new(8)]),
            node(&tokens, SyntaxKind::ExprStmt, 0, 25, vec![NodeId::new(9)]),
            node(&tokens, SyntaxKind::Root, 0, 26, vec![NodeId::new(10)]),
        ];
        Cst::new(SourceId::new(15), tokens, nodes, NodeId::new(11))
    }

    fn class_tokens() -> Vec<Token> {
        tokens(&[
            (TokenKind::KwClassdef, "classdef"),
            (TokenKind::Identifier, "Counter"),
            (TokenKind::Less, "<"),
            (TokenKind::Identifier, "handle"),
            (TokenKind::KwProperties, "properties"),
            (TokenKind::Identifier, "value"),
            (TokenKind::Equal, "="),
            (TokenKind::Number, "0"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::KwMethods, "methods"),
            (TokenKind::KwFunction, "function"),
            (TokenKind::Identifier, "y"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "make"),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "x"),
            (TokenKind::RParen, ")"),
            (TokenKind::Identifier, "y"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "x"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Eof, ""),
        ])
    }

    fn class_cst() -> Cst {
        let tokens = class_tokens();
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 3, 4, vec![]),
            node(
                &tokens,
                SyntaxKind::ClassHeader,
                0,
                4,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 5, 6, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 7, 8, vec![]),
            node(
                &tokens,
                SyntaxKind::PropertyDecl,
                5,
                8,
                vec![NodeId::new(3), NodeId::new(4)],
            ),
            node(
                &tokens,
                SyntaxKind::PropertiesBlock,
                4,
                9,
                vec![NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 11, 12, vec![]),
            node(
                &tokens,
                SyntaxKind::OutputList,
                11,
                13,
                vec![NodeId::new(7)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 13, 14, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 15, 16, vec![]),
            node(
                &tokens,
                SyntaxKind::ParameterList,
                14,
                17,
                vec![NodeId::new(10)],
            ),
            node(
                &tokens,
                SyntaxKind::FunctionSignature,
                10,
                17,
                vec![NodeId::new(8), NodeId::new(9), NodeId::new(11)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 17, 18, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 19, 20, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                17,
                21,
                vec![NodeId::new(13), NodeId::new(14)],
            ),
            node(&tokens, SyntaxKind::Block, 17, 21, vec![NodeId::new(15)]),
            node(
                &tokens,
                SyntaxKind::FunctionDef,
                10,
                22,
                vec![NodeId::new(12), NodeId::new(16)],
            ),
            node(
                &tokens,
                SyntaxKind::MethodsBlock,
                9,
                23,
                vec![NodeId::new(17)],
            ),
            node(
                &tokens,
                SyntaxKind::ClassDef,
                0,
                24,
                vec![NodeId::new(2), NodeId::new(6), NodeId::new(18)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 25, vec![NodeId::new(19)]),
        ];
        Cst::new(SourceId::new(13), tokens, nodes, NodeId::new(20))
    }

    fn enum_event_class_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwClassdef, "classdef"),
            (TokenKind::Identifier, "Traffic"),
            (TokenKind::KwEnumeration, "enumeration"),
            (TokenKind::Identifier, "Stop"),
            (TokenKind::LParen, "("),
            (TokenKind::Number, "0"),
            (TokenKind::RParen, ")"),
            (TokenKind::Identifier, "events"),
            (TokenKind::Identifier, "Changed"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::ClassHeader, 0, 2, vec![NodeId::new(0)]),
            node(&tokens, SyntaxKind::NameExpr, 3, 4, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 5, 6, vec![]),
            node(
                &tokens,
                SyntaxKind::EnumMemberDecl,
                3,
                7,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
            node(
                &tokens,
                SyntaxKind::EnumerationBlock,
                2,
                7,
                vec![NodeId::new(4)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            node(&tokens, SyntaxKind::EventDecl, 8, 9, vec![NodeId::new(6)]),
            node(&tokens, SyntaxKind::EventsBlock, 7, 9, vec![NodeId::new(7)]),
            node(
                &tokens,
                SyntaxKind::ClassDef,
                0,
                10,
                vec![NodeId::new(1), NodeId::new(5), NodeId::new(8)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 11, vec![NodeId::new(9)]),
        ];
        Cst::new(SourceId::new(33), tokens, nodes, NodeId::new(10))
    }

    fn malformed_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::Identifier, "x"),
            (TokenKind::Equal, "="),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                0,
                2,
                vec![NodeId::new(0)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 3, vec![NodeId::new(1)]),
        ];
        Cst::new(SourceId::new(14), tokens, nodes, NodeId::new(2))
    }

    fn clear_cst(unsupported: bool) -> Cst {
        let trailing = if unsupported {
            (TokenKind::Star, "*")
        } else {
            (TokenKind::Identifier, "second")
        };
        let tokens = tokens(&[
            (TokenKind::Identifier, "clear"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "first"),
            (TokenKind::Whitespace, " "),
            trailing,
            (TokenKind::Eof, ""),
        ]);
        let mut nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(
                &tokens,
                if unsupported {
                    SyntaxKind::Error
                } else {
                    SyntaxKind::NameExpr
                },
                4,
                5,
                vec![],
            ),
            node(
                &tokens,
                SyntaxKind::ClearStmt,
                0,
                5,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
        ];
        nodes.push(node(&tokens, SyntaxKind::Root, 0, 6, vec![NodeId::new(2)]));
        Cst::new(SourceId::new(17), tokens, nodes, NodeId::new(3))
    }

    fn declaration_cst() -> Cst {
        let tokens = tokens(&[
            (TokenKind::KwGlobal, "global"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "first"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "second"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwPersistent, "persistent"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "script_cache"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwFunction, "function"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "worker"),
            (TokenKind::LParen, "("),
            (TokenKind::RParen, ")"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwPersistent, "persistent"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "cache"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "cache"),
            (TokenKind::Newline, "\n"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Newline, "\n"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::GlobalStmt,
                0,
                5,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            node(
                &tokens,
                SyntaxKind::PersistentStmt,
                6,
                9,
                vec![NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 12, 13, vec![]),
            node(
                &tokens,
                SyntaxKind::FunctionSignature,
                10,
                15,
                vec![NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 18, 19, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 20, 21, vec![]),
            node(
                &tokens,
                SyntaxKind::PersistentStmt,
                16,
                21,
                vec![NodeId::new(7), NodeId::new(8)],
            ),
            node(&tokens, SyntaxKind::Block, 16, 22, vec![NodeId::new(9)]),
            node(
                &tokens,
                SyntaxKind::FunctionDef,
                10,
                23,
                vec![NodeId::new(6), NodeId::new(10)],
            ),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                25,
                vec![NodeId::new(2), NodeId::new(4), NodeId::new(11)],
            ),
        ];
        Cst::new(SourceId::new(31), tokens, nodes, NodeId::new(12))
    }

    #[test]
    fn simple_statement_semicolon_suppresses_output_but_comma_and_eof_do_not() {
        let tokens = tokens(&[
            (TokenKind::Number, "1"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Number, "2"),
            (TokenKind::Comma, ","),
            (TokenKind::Number, "3"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NumberExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::ExprStmt, 0, 2, vec![NodeId::new(0)]),
            node(&tokens, SyntaxKind::NumberExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::ExprStmt, 2, 4, vec![NodeId::new(2)]),
            node(&tokens, SyntaxKind::NumberExpr, 4, 5, vec![]),
            node(&tokens, SyntaxKind::ExprStmt, 4, 5, vec![NodeId::new(4)]),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                6,
                vec![NodeId::new(1), NodeId::new(3), NodeId::new(5)],
            ),
        ];
        let lowered = lower(&Cst::new(SourceId::new(18), tokens, nodes, NodeId::new(6)));

        assert!(lowered.diagnostics.is_empty());
        assert_eq!(
            lowered
                .file
                .statements
                .iter()
                .map(|statement| statement.suppress_output)
                .collect::<Vec<_>>(),
            [true, false, false]
        );
    }

    #[test]
    fn lowers_discarded_multiple_assignment_target_as_a_name() {
        let tokens = tokens(&[
            (TokenKind::LBracket, "["),
            (TokenKind::Identifier, "warning_text"),
            (TokenKind::Comma, ","),
            (TokenKind::Tilde, "~"),
            (TokenKind::RBracket, "]"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "lastwarn"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 3, 4, vec![]),
            node(
                &tokens,
                SyntaxKind::MatrixRow,
                1,
                4,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::MatrixExpr, 0, 5, vec![NodeId::new(2)]),
            node(&tokens, SyntaxKind::NameExpr, 6, 7, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                0,
                8,
                vec![NodeId::new(3), NodeId::new(4)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 9, vec![NodeId::new(5)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(41), tokens, nodes, NodeId::new(6)));
        let StmtKind::Assignment { target, value } = &lowered.file.statements[0].kind else {
            panic!("expected assignment");
        };
        let ExprKind::Matrix(rows) = &target.kind else {
            panic!("expected multiple-assignment target");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][0].kind, ExprKind::Name(name) if name == "warning_text"));
        assert!(matches!(&rows[0][1].kind, ExprKind::Name(name) if name == "~"));
        assert!(matches!(&value.kind, ExprKind::Name(name) if name == "lastwarn"));
    }

    #[test]
    fn lowers_regular_program_and_preserves_unresolved_application() {
        let lowered = lower(&expression_cst());
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(lowered.file.source_id, SourceId::new(11));
        assert_eq!(lowered.file.statements.len(), 2);

        let StmtKind::Assignment { value, .. } = &lowered.file.statements[0].kind else {
            panic!("expected assignment");
        };
        let ExprKind::Binary {
            operator: BinaryOp::Add,
            left,
            ..
        } = &value.kind
        else {
            panic!("expected addition");
        };
        assert!(matches!(left.kind, ExprKind::ParenApply { .. }));

        let StmtKind::Assignment { value, .. } = &lowered.file.statements[1].kind else {
            panic!("expected assignment");
        };
        assert!(matches!(
            value.kind,
            ExprKind::Transpose {
                kind: TransposeKind::NonConjugate,
                ..
            }
        ));
        assert_eq!(lowered.file.statements[0].span.start(), 0);
    }

    #[test]
    fn lowers_clear_names_in_order_and_preserves_unsupported_form() {
        let lowered = lower(&clear_cst(false));
        let StmtKind::Clear(clear) = &lowered.file.statements[0].kind else {
            panic!("clear HIR");
        };
        assert_eq!(clear.form, ClearForm::IdentifierList);
        assert_eq!(
            clear
                .names
                .iter()
                .map(|name| name.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );

        let unsupported = lower(&clear_cst(true));
        let StmtKind::Clear(clear) = &unsupported.file.statements[0].kind else {
            panic!("unsupported clear HIR");
        };
        assert_eq!(clear.form, ClearForm::UnsupportedArguments);
        assert_eq!(clear.names.len(), 1);
    }

    #[test]
    fn lowers_declarations_with_explicit_kind_context_names_and_recovery_form() {
        let lowered = lower(&declaration_cst());
        let StmtKind::Declaration(global) = &lowered.file.statements[0].kind else {
            panic!("global declaration HIR");
        };
        let StmtKind::Declaration(script_persistent) = &lowered.file.statements[1].kind else {
            panic!("script persistent declaration HIR");
        };
        let StmtKind::Function(function) = &lowered.file.statements[2].kind else {
            panic!("function HIR");
        };
        let StmtKind::Declaration(function_persistent) = &function.body[0].kind else {
            panic!("function persistent declaration HIR");
        };

        assert_eq!(global.kind, DeclarationKind::Global);
        assert_eq!(global.context, DeclarationContext::Script);
        assert_eq!(global.form, DeclarationForm::IdentifierList);
        assert_eq!(
            global
                .names
                .iter()
                .map(|name| name.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(global.names[0].span, TextRange::new(7, 12).unwrap());
        assert_eq!(script_persistent.kind, DeclarationKind::Persistent);
        assert_eq!(script_persistent.context, DeclarationContext::Script);
        assert_eq!(function_persistent.context, DeclarationContext::Function);
        assert_eq!(
            function_persistent.form,
            DeclarationForm::DuplicatePersistentNames
        );
        assert_eq!(function_persistent.names.len(), 2);
        assert_eq!(
            lowered
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.as_deref() == Some("OMH0004"))
                .count(),
            1
        );
    }

    #[test]
    fn malformed_declaration_children_lower_without_panicking() {
        let tokens = tokens(&[
            (TokenKind::KwGlobal, "global"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Number, "1"),
            (TokenKind::Whitespace, " "),
            (TokenKind::Identifier, "recovered"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::Error, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::GlobalStmt,
                0,
                5,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::GlobalStmt, 0, 1, vec![]),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                6,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
        ];
        let lowered = lower(&Cst::new(SourceId::new(32), tokens, nodes, NodeId::new(4)));
        let StmtKind::Declaration(invalid) = &lowered.file.statements[0].kind else {
            panic!("invalid declaration HIR");
        };
        let StmtKind::Declaration(missing) = &lowered.file.statements[1].kind else {
            panic!("missing declaration HIR");
        };

        assert_eq!(invalid.form, DeclarationForm::InvalidItems);
        assert_eq!(invalid.names[0].text, "recovered");
        assert_eq!(missing.form, DeclarationForm::MissingNames);
    }

    #[test]
    fn lowers_switch_selector_ordered_cases_and_otherwise_with_ranges() {
        let lowered = lower(&switch_cst());
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(lowered.file.statements.len(), 1);

        let statement = &lowered.file.statements[0];
        let StmtKind::Switch {
            selector,
            cases,
            otherwise,
        } = &statement.kind
        else {
            panic!("switch statement HIR");
        };
        assert_eq!(statement.span, TextRange::new(0, 57).unwrap());
        assert!(matches!(&selector.kind, ExprKind::Name(name) if name == "key"));
        assert_eq!(selector.span, TextRange::new(7, 10).unwrap());
        assert_eq!(cases.len(), 2);
        assert!(matches!(&cases[0].expression.kind, ExprKind::Number(value) if value == "1"));
        assert!(matches!(&cases[1].expression.kind, ExprKind::Number(value) if value == "2"));
        assert_eq!(cases[0].span, TextRange::new(11, 24).unwrap());
        assert_eq!(cases[1].span, TextRange::new(24, 37).unwrap());
        assert!(matches!(cases[1].body[0].kind, StmtKind::Break));

        let otherwise = otherwise.as_ref().expect("otherwise branch");
        assert_eq!(otherwise.span, TextRange::new(37, 53).unwrap());
        assert_eq!(otherwise.body.len(), 1);
        assert!(matches!(
            otherwise.body[0].kind,
            StmtKind::Assignment { .. }
        ));
    }

    #[test]
    fn malformed_switch_cst_lowers_to_error_expressions_without_panicking() {
        let lowered = lower(&malformed_switch_cst());
        let StmtKind::Switch {
            selector,
            cases,
            otherwise,
        } = &lowered.file.statements[0].kind
        else {
            panic!("switch statement HIR");
        };

        assert_eq!(lowered.diagnostics.len(), 2);
        assert!(
            lowered
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code.as_deref() == Some("OMH0001"))
        );
        assert!(matches!(selector.kind, ExprKind::Error));
        assert_eq!(cases.len(), 1);
        assert!(matches!(cases[0].expression.kind, ExprKind::Error));
        assert!(cases[0].body.is_empty());
        assert!(otherwise.is_none());
    }

    #[test]
    fn lowers_try_forms_with_optional_catch_binding_and_explicit_body_ranges() {
        let lowered = lower(&try_forms_cst());

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(lowered.file.statements.len(), 3);

        let StmtKind::Try(no_catch) = &lowered.file.statements[0].kind else {
            panic!("no-catch try HIR");
        };
        assert_eq!(no_catch.body.len(), 1);
        assert_eq!(no_catch.body_span, TextRange::new(4, 9).unwrap());
        assert!(no_catch.catch.is_none());
        assert_eq!(
            lowered.file.statements[0].span,
            TextRange::new(0, 12).unwrap()
        );

        let StmtKind::Try(plain) = &lowered.file.statements[1].kind else {
            panic!("plain catch try HIR");
        };
        let plain_catch = plain.catch.as_ref().expect("plain catch clause");
        assert!(plain.body.is_empty());
        assert_eq!(plain.body_span, TextRange::empty(17));
        assert!(plain_catch.variable.is_none());
        assert!(plain_catch.body.is_empty());
        assert_eq!(plain_catch.body_span, TextRange::empty(23));
        assert_eq!(plain_catch.span, TextRange::new(17, 23).unwrap());

        let StmtKind::Try(bound) = &lowered.file.statements[2].kind else {
            panic!("bound catch try HIR");
        };
        let bound_catch = bound.catch.as_ref().expect("bound catch clause");
        let variable = bound_catch.variable.as_ref().expect("catch variable");
        assert_eq!(bound.body_span, TextRange::new(31, 36).unwrap());
        assert_eq!(variable.text, "caught");
        assert_eq!(variable.span, TextRange::new(42, 48).unwrap());
        assert_eq!(bound_catch.body.len(), 1);
        assert_eq!(bound_catch.body_span, TextRange::new(49, 54).unwrap());
        assert_eq!(bound_catch.span, TextRange::new(36, 54).unwrap());
    }

    #[test]
    fn lowers_nested_try_to_the_nearest_structural_catch() {
        let lowered = lower(&nested_try_cst());

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let StmtKind::Try(outer) = &lowered.file.statements[0].kind else {
            panic!("outer try");
        };
        let StmtKind::Try(inner) = &outer.body[0].kind else {
            panic!("inner try");
        };
        assert_eq!(
            inner
                .catch
                .as_ref()
                .and_then(|clause| clause.variable.as_ref())
                .map(|name| name.text.as_str()),
            Some("inner")
        );
        assert_eq!(
            outer
                .catch
                .as_ref()
                .and_then(|clause| clause.variable.as_ref())
                .map(|name| name.text.as_str()),
            Some("outer")
        );
    }

    #[test]
    fn recovered_try_csts_lower_without_losing_the_primary_catch() {
        let duplicate_hir = lower(&duplicate_catch_cst());
        let StmtKind::Try(statement) = &duplicate_hir.file.statements[0].kind else {
            panic!("recovered duplicate catch try");
        };

        assert!(duplicate_hir.diagnostics.is_empty());
        assert_eq!(
            statement
                .catch
                .as_ref()
                .and_then(|clause| clause.variable.as_ref())
                .map(|name| name.text.as_str()),
            Some("first")
        );
    }

    #[test]
    fn lowers_three_operand_range_to_explicit_step() {
        let lowered = lower(&range_cst());
        let StmtKind::Expr(range) = &lowered.file.statements[0].kind else {
            panic!("expected expression statement");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(matches!(range.kind, ExprKind::Range { step: Some(_), .. }));
    }

    #[test]
    fn lowers_two_operand_range_without_a_step() {
        let lowered = lower(&two_operand_range_cst());
        let StmtKind::Expr(range) = &lowered.file.statements[0].kind else {
            panic!("expected expression statement");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(matches!(range.kind, ExprKind::Range { step: None, .. }));
    }

    #[test]
    fn lowers_standalone_colon_to_all_index_argument() {
        let lowered = lower(&all_index_cst());
        let StmtKind::Expr(application) = &lowered.file.statements[0].kind else {
            panic!("expected expression statement");
        };
        let ExprKind::ParenApply { arguments, .. } = &application.kind else {
            panic!("expected unresolved parenthesized application");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[0].kind, ExprKind::AllIndex));
        assert!(matches!(arguments[1].kind, ExprKind::Number(_)));
        assert_eq!(arguments[0].span.start(), 7);
        assert_eq!(arguments[0].span.end(), 8);
    }

    #[test]
    fn lowers_left_associative_matrix_and_element_power_chains() {
        for (token, spelling, expected) in [
            (TokenKind::Caret, "^", BinaryOp::Power),
            (TokenKind::DotCaret, ".^", BinaryOp::ElementPower),
        ] {
            let lowered = lower(&power_chain_cst(token, spelling, SourceId::new(15)));
            let StmtKind::Expr(outer) = &lowered.file.statements[0].kind else {
                panic!("expected power expression statement");
            };
            let ExprKind::Binary {
                operator,
                left,
                right,
            } = &outer.kind
            else {
                panic!("expected outer power");
            };
            let ExprKind::Binary {
                operator: inner_operator,
                ..
            } = &left.kind
            else {
                panic!("expected power chain on the left");
            };

            assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
            assert_eq!(*operator, expected);
            assert_eq!(*inner_operator, expected);
            assert!(matches!(right.kind, ExprKind::Number(_)));
        }
    }

    #[test]
    fn lowers_short_circuit_operators_across_lossless_continuation() {
        let cst = short_circuit_continuation_cst();
        let source = "[false && ...\n    missing(), true ...\n    || missing()];";

        assert_eq!(cst.text(), source);
        let continuation_ranges: Vec<_> = cst
            .tokens()
            .iter()
            .filter(|token| token.kind == TokenKind::Ellipsis)
            .map(|token| (token.range.start(), token.range.end()))
            .collect();
        assert_eq!(continuation_ranges, vec![(10, 13), (34, 37)]);

        let lowered = lower(&cst);
        let StmtKind::Expr(matrix) = &lowered.file.statements[0].kind else {
            panic!("expected matrix expression statement");
        };
        let ExprKind::Matrix(rows) = &matrix.kind else {
            panic!("expected matrix HIR");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
        assert!(matches!(
            rows[0][0].kind,
            ExprKind::Binary {
                operator: BinaryOp::ShortCircuitAnd,
                ..
            }
        ));
        assert!(matches!(
            rows[0][1].kind,
            ExprKind::Binary {
                operator: BinaryOp::ShortCircuitOr,
                ..
            }
        ));
        assert_eq!(rows[0][1].span.start(), 29);
    }

    #[test]
    fn lowers_function_and_core_class_members() {
        let lowered = lower(&class_cst());

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let StmtKind::Class(class) = &lowered.file.statements[0].kind else {
            panic!("expected class HIR");
        };
        assert_eq!(
            class.name.as_ref().map(|name| name.text.as_str()),
            Some("Counter")
        );
        assert_eq!(
            class.superclass.as_ref().map(|name| name.text.as_str()),
            Some("handle")
        );
        assert!(class.attributes.is_empty());
        assert_eq!(class.property_blocks[0].properties.len(), 1);
        assert_eq!(class.method_blocks[0].methods.len(), 1);
        assert_eq!(
            class.method_blocks[0].methods[0]
                .name
                .as_ref()
                .map(|name| name.text.as_str()),
            Some("make")
        );
    }

    #[test]
    fn lowers_enumeration_arguments_and_event_names_as_class_metadata() {
        let lowered = lower(&enum_event_class_cst());
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        let StmtKind::Class(class) = &lowered.file.statements[0].kind else {
            panic!("class HIR");
        };
        assert_eq!(class.enumeration_blocks.len(), 1);
        assert_eq!(class.event_blocks.len(), 1);
        let member = &class.enumeration_blocks[0].members[0];
        assert_eq!(
            member.name.as_ref().map(|name| name.text.as_str()),
            Some("Stop")
        );
        assert!(matches!(member.arguments[0].kind, ExprKind::Number(ref value) if value == "0"));
        assert_eq!(
            class.event_blocks[0].events[0]
                .name
                .as_ref()
                .map(|name| name.text.as_str()),
            Some("Changed")
        );
    }

    #[test]
    fn lowers_constant_and_static_attributes_as_source_backed_hir() {
        let tokens = tokens(&[
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "Constant"),
            (TokenKind::RParen, ")"),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "Static"),
            (TokenKind::RParen, ")"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::Attribute, 1, 2, vec![NodeId::new(0)]),
            node(
                &tokens,
                SyntaxKind::AttributeList,
                0,
                3,
                vec![NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(&tokens, SyntaxKind::Attribute, 4, 5, vec![NodeId::new(3)]),
            node(
                &tokens,
                SyntaxKind::AttributeList,
                3,
                6,
                vec![NodeId::new(4)],
            ),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                7,
                vec![NodeId::new(2), NodeId::new(5)],
            ),
        ];
        let cst = Cst::new(SourceId::new(18), tokens, nodes, NodeId::new(6));
        let mut lowerer = super::Lowerer::new(&cst);
        let constant = lowerer.lower_attributes(NodeId::new(2));
        let static_method = lowerer.lower_attributes(NodeId::new(5));

        assert!(lowerer.diagnostics.is_empty());
        assert_eq!(
            constant[0].name.as_ref().map(|name| name.text.as_str()),
            Some("Constant")
        );
        assert_eq!(constant[0].span, TextRange::new(1, 9).unwrap());
        assert_eq!(
            static_method[0]
                .name
                .as_ref()
                .map(|name| name.text.as_str()),
            Some("Static")
        );
        assert_eq!(static_method[0].span, TextRange::new(11, 17).unwrap());
    }

    #[test]
    fn lowers_dotted_accessor_name_as_one_semantic_method_name() {
        for (prefix, expected, end) in [("get", "get.Twice", 9_u32), ("set", "set.Twice", 9_u32)] {
            let tokens = tokens(&[
                (TokenKind::Identifier, prefix),
                (TokenKind::Dot, "."),
                (TokenKind::Identifier, "Twice"),
                (TokenKind::Eof, ""),
            ]);
            let nodes = vec![
                node(&tokens, SyntaxKind::NameExpr, 0, 3, vec![]),
                node(&tokens, SyntaxKind::Root, 0, 4, vec![NodeId::new(0)]),
            ];
            let cst = Cst::new(SourceId::new(15), tokens, nodes, NodeId::new(1));
            let lowered = super::Lowerer::new(&cst)
                .lower_name(NodeId::new(0))
                .expect("dotted accessor name");

            assert_eq!(lowered.text, expected);
            assert_eq!(lowered.span, TextRange::new(0, end).unwrap());
        }
    }

    #[test]
    fn lowers_object_member_application_without_deciding_call_or_index() {
        let tokens = tokens(&[
            (TokenKind::Identifier, "out"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "obj"),
            (TokenKind::Dot, "."),
            (TokenKind::Identifier, "method"),
            (TokenKind::LParen, "("),
            (TokenKind::Number, "2"),
            (TokenKind::RParen, ")"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::FieldExpr,
                2,
                5,
                vec![NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::NumberExpr, 6, 7, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                2,
                8,
                vec![NodeId::new(3), NodeId::new(4)],
            ),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                0,
                9,
                vec![NodeId::new(0), NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::Root, 0, 10, vec![NodeId::new(6)]),
        ];
        let cst = Cst::new(SourceId::new(16), tokens, nodes, NodeId::new(7));
        let lowered = lower(&cst);
        let StmtKind::Assignment { value, .. } = &lowered.file.statements[0].kind else {
            panic!("assignment HIR");
        };
        let ExprKind::ParenApply { target, arguments } = &value.kind else {
            panic!("unresolved member application");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(matches!(target.kind, ExprKind::Field { .. }));
        assert_eq!(arguments.len(), 1);
    }

    #[test]
    fn lowers_long_postfix_chain_with_explicit_brace_end_and_field_selectors() {
        let tokens = tokens(&[
            (TokenKind::Identifier, "a"),
            (TokenKind::LParen, "("),
            (TokenKind::Number, "1"),
            (TokenKind::RParen, ")"),
            (TokenKind::LBrace, "{"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::Comma, ","),
            (TokenKind::Colon, ":"),
            (TokenKind::RBrace, "}"),
            (TokenKind::Dot, "."),
            (TokenKind::Identifier, "f"),
            (TokenKind::Dot, "."),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "name"),
            (TokenKind::RParen, ")"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NumberExpr, 2, 3, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                0,
                4,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::EndIndexExpr, 5, 6, vec![]),
            node(&tokens, SyntaxKind::ColonExpr, 7, 8, vec![]),
            node(
                &tokens,
                SyntaxKind::BraceApplyExpr,
                0,
                9,
                vec![NodeId::new(2), NodeId::new(3), NodeId::new(4)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 10, 11, vec![]),
            node(
                &tokens,
                SyntaxKind::FieldExpr,
                0,
                11,
                vec![NodeId::new(5), NodeId::new(6)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 13, 14, vec![]),
            node(
                &tokens,
                SyntaxKind::DynamicFieldExpr,
                0,
                15,
                vec![NodeId::new(7), NodeId::new(8)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 16, vec![NodeId::new(9)]),
            node(&tokens, SyntaxKind::Root, 0, 17, vec![NodeId::new(10)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(19), tokens, nodes, NodeId::new(11)));
        let StmtKind::Expr(dynamic_field) = &lowered.file.statements[0].kind else {
            panic!("dynamic field expression");
        };
        let ExprKind::DynamicField {
            target,
            name: field_name,
        } = &dynamic_field.kind
        else {
            panic!("dynamic field HIR");
        };
        let ExprKind::Field {
            target: brace,
            name: Some(static_name),
        } = &target.kind
        else {
            panic!("static field HIR");
        };
        let ExprKind::BraceApply {
            target: paren,
            arguments,
        } = &brace.kind
        else {
            panic!("brace apply HIR");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(static_name.text, "f");
        assert_eq!(
            static_name.span,
            TextRange::new(12, 13).expect("field span")
        );
        assert!(matches!(&field_name.kind, ExprKind::Name(name) if name == "name"));
        assert_eq!(
            field_name.span,
            TextRange::new(15, 19).expect("dynamic name span")
        );
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[0].kind, ExprKind::EndIndex));
        assert!(matches!(arguments[1].kind, ExprKind::AllIndex));
        assert!(matches!(paren.kind, ExprKind::ParenApply { .. }));
    }

    #[test]
    fn assignment_hir_keeps_brace_and_unresolved_paren_targets_intact() {
        let tokens = tokens(&[
            (TokenKind::Identifier, "C"),
            (TokenKind::LBrace, "{"),
            (TokenKind::KwEnd, "end"),
            (TokenKind::RBrace, "}"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "v"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Identifier, "P"),
            (TokenKind::LParen, "("),
            (TokenKind::KwEnd, "end"),
            (TokenKind::RParen, ")"),
            (TokenKind::Equal, "="),
            (TokenKind::Identifier, "w"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::EndIndexExpr, 2, 3, vec![]),
            node(
                &tokens,
                SyntaxKind::BraceApplyExpr,
                0,
                4,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 5, 6, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                0,
                7,
                vec![NodeId::new(2), NodeId::new(3)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 7, 8, vec![]),
            node(&tokens, SyntaxKind::EndIndexExpr, 9, 10, vec![]),
            node(
                &tokens,
                SyntaxKind::ParenApplyExpr,
                7,
                11,
                vec![NodeId::new(5), NodeId::new(6)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 12, 13, vec![]),
            node(
                &tokens,
                SyntaxKind::AssignmentStmt,
                7,
                14,
                vec![NodeId::new(7), NodeId::new(8)],
            ),
            node(
                &tokens,
                SyntaxKind::Root,
                0,
                15,
                vec![NodeId::new(4), NodeId::new(9)],
            ),
        ];
        let lowered = lower(&Cst::new(SourceId::new(20), tokens, nodes, NodeId::new(10)));
        let StmtKind::Assignment {
            target: brace_target,
            ..
        } = &lowered.file.statements[0].kind
        else {
            panic!("brace assignment");
        };
        let StmtKind::Assignment {
            target: paren_target,
            ..
        } = &lowered.file.statements[1].kind
        else {
            panic!("paren assignment");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(matches!(
            &brace_target.kind,
            ExprKind::BraceApply { arguments, .. }
                if matches!(arguments[0].kind, ExprKind::EndIndex)
        ));
        assert!(matches!(
            &paren_target.kind,
            ExprKind::ParenApply { arguments, .. }
                if matches!(arguments[0].kind, ExprKind::EndIndex)
        ));
    }

    #[test]
    fn malformed_dynamic_field_lowers_to_an_error_selector_without_panicking() {
        let tokens = tokens(&[
            (TokenKind::Identifier, "value"),
            (TokenKind::Dot, "."),
            (TokenKind::LParen, "("),
            (TokenKind::RParen, ")"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(
                &tokens,
                SyntaxKind::DynamicFieldExpr,
                0,
                4,
                vec![NodeId::new(0)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 4, vec![NodeId::new(1)]),
            node(&tokens, SyntaxKind::Root, 0, 5, vec![NodeId::new(2)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(21), tokens, nodes, NodeId::new(3)));
        let StmtKind::Expr(expression) = &lowered.file.statements[0].kind else {
            panic!("field expression");
        };
        let ExprKind::DynamicField { name, .. } = &expression.kind else {
            panic!("dynamic field selector");
        };

        assert!(matches!(name.kind, ExprKind::Error));
        assert!(
            lowered
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMH0001"))
        );
    }

    #[test]
    fn lowers_explicit_superclass_constructor_call_without_erasing_names() {
        let tokens = tokens(&[
            (TokenKind::Identifier, "object"),
            (TokenKind::At, "@"),
            (TokenKind::Identifier, "Base"),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "value"),
            (TokenKind::RParen, ")"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 0, 1, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::SuperclassConstructorCallExpr,
                0,
                6,
                vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 7, vec![NodeId::new(3)]),
            node(&tokens, SyntaxKind::Root, 0, 8, vec![NodeId::new(4)]),
        ];
        let cst = Cst::new(SourceId::new(17), tokens, nodes, NodeId::new(5));
        let lowered = lower(&cst);
        let StmtKind::Expr(expression) = &lowered.file.statements[0].kind else {
            panic!("expression statement");
        };
        let ExprKind::SuperclassConstructorCall {
            object,
            superclass,
            arguments,
        } = &expression.kind
        else {
            panic!("superclass constructor call");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert!(matches!(&object.kind, ExprKind::Name(name) if name == "object"));
        assert_eq!(
            superclass.as_ref().map(|name| name.text.as_str()),
            Some("Base")
        );
        assert!(matches!(&arguments[0].kind, ExprKind::Name(name) if name == "value"));
        assert_eq!(expression.span, TextRange::new(0, 18).unwrap());
    }

    #[test]
    fn lowers_qualified_named_function_handle_as_one_name() {
        let tokens = tokens(&[
            (TokenKind::At, "@"),
            (TokenKind::Identifier, "alpha"),
            (TokenKind::Dot, "."),
            (TokenKind::Identifier, "nested"),
            (TokenKind::Dot, "."),
            (TokenKind::Identifier, "inc"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 1, 2, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 3, 4, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 5, 6, vec![]),
            node(
                &tokens,
                SyntaxKind::FunctionHandleExpr,
                0,
                6,
                vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 7, vec![NodeId::new(3)]),
            node(&tokens, SyntaxKind::Root, 0, 8, vec![NodeId::new(4)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(22), tokens, nodes, NodeId::new(5)));
        let StmtKind::Expr(expression) = &lowered.file.statements[0].kind else {
            panic!("function-handle expression");
        };
        let ExprKind::FunctionHandle(Some(name)) = &expression.kind else {
            panic!("named function handle");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(name.text, "alpha.nested.inc");
        assert_eq!(name.span, TextRange::new(1, 17).unwrap());
    }

    #[test]
    fn lowers_anonymous_function_with_complete_span_and_explicit_parts() {
        let tokens = tokens(&[
            (TokenKind::At, "@"),
            (TokenKind::LParen, "("),
            (TokenKind::Identifier, "x"),
            (TokenKind::Comma, ","),
            (TokenKind::Identifier, "y"),
            (TokenKind::RParen, ")"),
            (TokenKind::Identifier, "x"),
            (TokenKind::Plus, "+"),
            (TokenKind::Identifier, "y"),
            (TokenKind::Semicolon, ";"),
            (TokenKind::Eof, ""),
        ]);
        let nodes = vec![
            node(&tokens, SyntaxKind::NameExpr, 2, 3, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 4, 5, vec![]),
            node(
                &tokens,
                SyntaxKind::ParameterList,
                1,
                6,
                vec![NodeId::new(0), NodeId::new(1)],
            ),
            node(&tokens, SyntaxKind::NameExpr, 6, 7, vec![]),
            node(&tokens, SyntaxKind::NameExpr, 8, 9, vec![]),
            node(
                &tokens,
                SyntaxKind::BinaryExpr,
                6,
                9,
                vec![NodeId::new(3), NodeId::new(4)],
            ),
            node(
                &tokens,
                SyntaxKind::AnonymousFunctionExpr,
                0,
                9,
                vec![NodeId::new(2), NodeId::new(5)],
            ),
            node(&tokens, SyntaxKind::ExprStmt, 0, 10, vec![NodeId::new(6)]),
            node(&tokens, SyntaxKind::Root, 0, 10, vec![NodeId::new(7)]),
        ];
        let lowered = lower(&Cst::new(SourceId::new(18), tokens, nodes, NodeId::new(8)));
        let StmtKind::Expr(expression) = &lowered.file.statements[0].kind else {
            panic!("expression statement");
        };
        let ExprKind::AnonymousFunction { parameters, body } = &expression.kind else {
            panic!("anonymous-function HIR");
        };

        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(
            parameters
                .iter()
                .map(|name| name.text.as_str())
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert!(matches!(body.kind, ExprKind::Binary { .. }));
        assert_eq!(expression.span, TextRange::new(0, 9).unwrap());
    }

    #[test]
    fn malformed_cst_lowers_without_panicking() {
        let lowered = lower(&malformed_cst());

        assert!(!lowered.diagnostics.is_empty());
        assert!(!lowered.file.statements.is_empty());
        assert!(
            lowered
                .file
                .statements
                .iter()
                .any(|statement| matches!(statement.kind, StmtKind::Assignment { .. }))
        );
    }
}
