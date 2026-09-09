#![doc = "Lossless tokens and concrete syntax tree data structures."]

use openmat_source::{SourceId, TextRange};
use std::ops::Range;

/// Lexical token categories for the release-one MATLAB-compatible frontend.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TokenKind {
    Whitespace,
    Newline,
    Comment,
    Identifier,
    Number,
    CharLiteral,
    StringLiteral,
    KwBreak,
    KwCase,
    KwCatch,
    KwClassdef,
    KwContinue,
    KwElse,
    KwElseif,
    KwEnd,
    KwEnumeration,
    KwEvents,
    KwFor,
    KwFunction,
    KwGlobal,
    KwIf,
    KwMethods,
    KwOtherwise,
    KwParfor,
    KwPersistent,
    KwProperties,
    KwReturn,
    KwSpmd,
    KwSwitch,
    KwTry,
    KwWhile,
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    DotStar,
    DotSlash,
    DotBackslash,
    DotCaret,
    Equal,
    EqualEqual,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    AndAnd,
    Or,
    OrOr,
    Tilde,
    Colon,
    ConjugateTranspose,
    DotTranspose,
    Dot,
    At,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Ellipsis,
    Error,
    Eof,
}

impl TokenKind {
    /// Returns whether this token is retained trivia rather than syntax.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::Newline | Self::Comment | Self::Ellipsis
        )
    }

    /// Returns whether this token is a line boundary for statement parsing.
    #[must_use]
    pub const fn is_line_break(self) -> bool {
        matches!(self, Self::Newline)
    }

    /// Returns whether this token is a reserved word.
    #[must_use]
    pub const fn is_keyword(self) -> bool {
        matches!(
            self,
            Self::KwBreak
                | Self::KwCase
                | Self::KwCatch
                | Self::KwClassdef
                | Self::KwContinue
                | Self::KwElse
                | Self::KwElseif
                | Self::KwEnd
                | Self::KwEnumeration
                | Self::KwEvents
                | Self::KwFor
                | Self::KwFunction
                | Self::KwGlobal
                | Self::KwIf
                | Self::KwMethods
                | Self::KwOtherwise
                | Self::KwParfor
                | Self::KwPersistent
                | Self::KwProperties
                | Self::KwReturn
                | Self::KwSpmd
                | Self::KwSwitch
                | Self::KwTry
                | Self::KwWhile
        )
    }

    /// Returns whether this token may begin an expression in the supported subset.
    #[must_use]
    pub const fn can_start_expression(self) -> bool {
        matches!(
            self,
            Self::Identifier
                | Self::Number
                | Self::CharLiteral
                | Self::StringLiteral
                | Self::KwEnd
                | Self::LParen
                | Self::LBracket
                | Self::LBrace
                | Self::Colon
                | Self::Plus
                | Self::Minus
                | Self::Tilde
                | Self::At
        )
    }

    /// Returns whether this token can be followed by MATLAB's postfix apostrophe.
    #[must_use]
    pub const fn can_end_expression(self) -> bool {
        matches!(
            self,
            Self::Identifier
                | Self::Number
                | Self::CharLiteral
                | Self::StringLiteral
                | Self::KwEnd
                | Self::RParen
                | Self::RBracket
                | Self::RBrace
                | Self::ConjugateTranspose
                | Self::DotTranspose
        )
    }
}

/// A lossless token, including its exact source spelling and byte range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub range: TextRange,
}

impl Token {
    #[must_use]
    pub fn new(kind: TokenKind, text: impl Into<String>, range: TextRange) -> Self {
        Self {
            kind,
            text: text.into(),
            range,
        }
    }
}

/// Concrete syntax categories. Nodes reference ranges in the lossless token stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SyntaxKind {
    Root,
    Error,
    EmptyStmt,
    Block,
    AssignmentStmt,
    ExprStmt,
    /// MATLAB command-form invocation such as `hold on`.
    CommandStmt,
    ClearStmt,
    GlobalStmt,
    PersistentStmt,
    BreakStmt,
    ContinueStmt,
    ReturnStmt,
    IfStmt,
    ElseifClause,
    ElseClause,
    ForStmt,
    WhileStmt,
    TryStmt,
    CatchClause,
    SwitchStmt,
    CaseClause,
    OtherwiseClause,
    FunctionDef,
    FunctionSignature,
    ArgumentsBlock,
    ArgumentDecl,
    ArgumentDimensions,
    ArgumentClass,
    ArgumentValidators,
    ArgumentDefault,
    NameValueArgument,
    ClassDef,
    ClassHeader,
    PropertiesBlock,
    MethodsBlock,
    /// A class `enumeration ... end` member block.
    EnumerationBlock,
    /// One named enumeration member and its constructor arguments.
    EnumMemberDecl,
    /// A contextual class `events ... end` declaration block.
    EventsBlock,
    /// One event name declaration.
    EventDecl,
    /// A body-less signature declared in a `methods (Abstract)` block.
    MethodDecl,
    PropertyDecl,
    AttributeList,
    Attribute,
    ParameterList,
    OutputList,
    NameExpr,
    NumberExpr,
    CharExpr,
    StringExpr,
    FunctionHandleExpr,
    AnonymousFunctionExpr,
    ParenExpr,
    ParenApplyExpr,
    BraceApplyExpr,
    SuperclassConstructorCallExpr,
    FieldExpr,
    DynamicFieldExpr,
    UnaryExpr,
    BinaryExpr,
    ColonExpr,
    EndIndexExpr,
    RangeExpr,
    TransposeExpr,
    MatrixExpr,
    MatrixRow,
    CellExpr,
    CellRow,
}

/// An arena index identifying a concrete syntax node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeId(usize);

impl NodeId {
    #[must_use]
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> usize {
        self.0
    }
}

/// A CST node with a source span, token interval, and ordered child nodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNode {
    pub kind: SyntaxKind,
    pub range: TextRange,
    pub token_range: Range<usize>,
    pub children: Vec<NodeId>,
}

impl SyntaxNode {
    #[must_use]
    pub fn new(
        kind: SyntaxKind,
        range: TextRange,
        token_range: Range<usize>,
        children: Vec<NodeId>,
    ) -> Self {
        Self {
            kind,
            range,
            token_range,
            children,
        }
    }
}

/// Lossless concrete syntax tree backed by token and node arenas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cst {
    source_id: SourceId,
    tokens: Vec<Token>,
    nodes: Vec<SyntaxNode>,
    root: NodeId,
}

impl Cst {
    #[must_use]
    pub fn new(
        source_id: SourceId,
        tokens: Vec<Token>,
        nodes: Vec<SyntaxNode>,
        root: NodeId,
    ) -> Self {
        Self {
            source_id,
            tokens,
            nodes,
            root,
        }
    }

    #[must_use]
    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    #[must_use]
    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&SyntaxNode> {
        self.nodes.get(id.raw())
    }

    #[must_use]
    pub fn node_tokens(&self, id: NodeId) -> &[Token] {
        let Some(node) = self.node(id) else {
            return &[];
        };
        self.tokens.get(node.token_range.clone()).unwrap_or(&[])
    }

    /// Reconstructs the input exactly, excluding the zero-width EOF marker.
    #[must_use]
    pub fn text(&self) -> String {
        self.tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cst, NodeId, SyntaxKind, SyntaxNode, Token, TokenKind};
    use openmat_source::{SourceId, TextRange};

    #[test]
    fn cst_reconstructs_trivia_exactly() {
        let tokens = vec![
            Token::new(
                TokenKind::Identifier,
                "x",
                TextRange::new(0, 1).expect("valid range"),
            ),
            Token::new(
                TokenKind::Whitespace,
                " ",
                TextRange::new(1, 2).expect("valid range"),
            ),
            Token::new(
                TokenKind::Comment,
                "% note",
                TextRange::new(2, 8).expect("valid range"),
            ),
            Token::new(
                TokenKind::Newline,
                "\r\n",
                TextRange::new(8, 10).expect("valid range"),
            ),
        ];
        let node = SyntaxNode::new(
            SyntaxKind::Root,
            TextRange::new(0, 10).expect("valid range"),
            0..tokens.len(),
            Vec::new(),
        );
        let tree = Cst::new(SourceId::new(1), tokens, vec![node], NodeId::new(0));

        assert_eq!(tree.text(), "x % note\r\n");
        assert_eq!(tree.node_tokens(tree.root()).len(), 4);
    }

    #[test]
    fn token_classification_exposes_trivia_and_expression_boundaries() {
        assert!(TokenKind::Whitespace.is_trivia());
        assert!(TokenKind::Ellipsis.is_trivia());
        assert!(TokenKind::KwClassdef.is_keyword());
        assert!(TokenKind::KwSwitch.is_keyword());
        assert!(TokenKind::KwCase.is_keyword());
        assert!(TokenKind::KwOtherwise.is_keyword());
        assert!(TokenKind::KwTry.is_keyword());
        assert!(TokenKind::KwCatch.is_keyword());
        assert!(TokenKind::LBracket.can_start_expression());
        assert!(TokenKind::Colon.can_start_expression());
        assert!(TokenKind::RParen.can_end_expression());
        assert!(!TokenKind::Comment.can_end_expression());
    }
}
