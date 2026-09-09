#![doc = "Handwritten lossless lexer and error-tolerant MATLAB subset parser."]

use openmat_source::{Diagnostic, SourceId, TextRange};
use openmat_syntax::{Cst, NodeId, SyntaxKind, SyntaxNode, Token, TokenKind};

/// Result of lossless tokenization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexResult {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of parsing. A CST is returned even when diagnostics are present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseResult {
    pub syntax: Cst,
    pub diagnostics: Vec<Diagnostic>,
}

/// Tokenizes a UTF-8 source buffer without discarding trivia.
#[must_use]
pub fn lex(source_id: SourceId, source: &str) -> LexResult {
    Lexer::new(source_id, source).run()
}

/// Parses a UTF-8 source buffer into a lossless, error-tolerant CST.
#[must_use]
pub fn parse(source_id: SourceId, source: &str) -> ParseResult {
    let lexed = lex(source_id, source);
    Parser::new(source_id, lexed.tokens, lexed.diagnostics).run()
}

struct Lexer<'source> {
    source_id: SourceId,
    source: &'source str,
    offset: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    previous_significant: Option<TokenKind>,
    matrix_depth: usize,
    horizontal_trivia_since_significant: bool,
    line_continuation_pending: bool,
}

impl<'source> Lexer<'source> {
    fn new(source_id: SourceId, source: &'source str) -> Self {
        Self {
            source_id,
            source,
            offset: 0,
            tokens: Vec::new(),
            diagnostics: Vec::new(),
            previous_significant: None,
            matrix_depth: 0,
            horizontal_trivia_since_significant: false,
            line_continuation_pending: false,
        }
    }

    fn run(mut self) -> LexResult {
        if u32::try_from(self.source.len()).is_err() {
            self.diagnostics.push(
                Diagnostic::error(
                    self.source_id,
                    TextRange::empty(u32::MAX),
                    "source exceeds the 32-bit byte-range limit",
                )
                .with_code("OML0001"),
            );
        }
        while self.offset < self.source.len() {
            self.lex_one();
        }
        let end = u32::try_from(self.source.len()).unwrap_or(u32::MAX);
        self.tokens
            .push(Token::new(TokenKind::Eof, "", TextRange::empty(end)));
        LexResult {
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    fn lex_one(&mut self) {
        let start = self.offset;
        let Some(first) = self.remaining().chars().next() else {
            return;
        };
        if first == '\r' || first == '\n' {
            let continued = self.line_continuation_pending;
            self.offset += if self.remaining().starts_with("\r\n") {
                2
            } else {
                first.len_utf8()
            };
            self.push(TokenKind::Newline, start, self.offset);
            if !continued {
                self.previous_significant = None;
            }
            self.horizontal_trivia_since_significant = false;
            self.line_continuation_pending = false;
            return;
        }
        if first.is_whitespace() {
            self.consume_while(|character| {
                character.is_whitespace() && character != '\r' && character != '\n'
            });
            self.push(TokenKind::Whitespace, start, self.offset);
            self.horizontal_trivia_since_significant = true;
            return;
        }
        if self.remaining().starts_with("%{") {
            self.lex_block_comment(start);
            return;
        }
        if first == '%' {
            self.consume_until_line_end();
            self.push(TokenKind::Comment, start, self.offset);
            return;
        }
        if self.remaining().starts_with("...") {
            self.consume_while(|character| character == '.');
            self.push(TokenKind::Ellipsis, start, self.offset);
            let trailing_start = self.offset;
            self.consume_until_line_end();
            if self.offset > trailing_start {
                self.push(TokenKind::Comment, trailing_start, self.offset);
            }
            self.line_continuation_pending = self.offset < self.source.len();
            if !self.line_continuation_pending {
                self.diagnostics.push(
                    Diagnostic::error(
                        self.source_id,
                        Self::range(start, self.offset),
                        "line continuation is not followed by another physical line",
                    )
                    .with_code("OML0005"),
                );
            }
            return;
        }
        if first.is_ascii_digit()
            || (first == '.'
                && self
                    .remaining()
                    .as_bytes()
                    .get(1)
                    .is_some_and(u8::is_ascii_digit))
        {
            self.lex_number(start);
            return;
        }
        if first == '_' || first.is_alphabetic() {
            self.consume_while(|character| character == '_' || character.is_alphanumeric());
            let kind = keyword_kind(&self.source[start..self.offset]);
            self.push(kind, start, self.offset);
            return;
        }
        if first == '\'' {
            let matrix_element_boundary =
                self.matrix_depth > 0 && self.horizontal_trivia_since_significant;
            if !matrix_element_boundary
                && self
                    .previous_significant
                    .is_some_and(TokenKind::can_end_expression)
            {
                self.offset += 1;
                self.push(TokenKind::ConjugateTranspose, start, self.offset);
            } else {
                self.lex_quoted(start, '\'', TokenKind::CharLiteral);
            }
            return;
        }
        if first == '"' {
            self.lex_quoted(start, '"', TokenKind::StringLiteral);
            return;
        }

        self.lex_operator_or_error(start, first);
    }

    fn lex_operator_or_error(&mut self, start: usize, first: char) {
        let (kind, width) = if self.remaining().starts_with(".*") {
            (TokenKind::DotStar, 2)
        } else if self.remaining().starts_with("./") {
            (TokenKind::DotSlash, 2)
        } else if self.remaining().starts_with(".\\") {
            (TokenKind::DotBackslash, 2)
        } else if self.remaining().starts_with(".^") {
            (TokenKind::DotCaret, 2)
        } else if self.remaining().starts_with(".'") {
            (TokenKind::DotTranspose, 2)
        } else if self.remaining().starts_with("==") {
            (TokenKind::EqualEqual, 2)
        } else if self.remaining().starts_with("~=") {
            (TokenKind::NotEqual, 2)
        } else if self.remaining().starts_with("<=") {
            (TokenKind::LessEqual, 2)
        } else if self.remaining().starts_with(">=") {
            (TokenKind::GreaterEqual, 2)
        } else if self.remaining().starts_with("&&") {
            (TokenKind::AndAnd, 2)
        } else if self.remaining().starts_with("||") {
            (TokenKind::OrOr, 2)
        } else {
            match first {
                '+' => (TokenKind::Plus, 1),
                '-' => (TokenKind::Minus, 1),
                '*' => (TokenKind::Star, 1),
                '/' => (TokenKind::Slash, 1),
                '\\' => (TokenKind::Backslash, 1),
                '^' => (TokenKind::Caret, 1),
                '=' => (TokenKind::Equal, 1),
                '<' => (TokenKind::Less, 1),
                '>' => (TokenKind::Greater, 1),
                '&' => (TokenKind::And, 1),
                '|' => (TokenKind::Or, 1),
                '~' => (TokenKind::Tilde, 1),
                ':' => (TokenKind::Colon, 1),
                '.' => (TokenKind::Dot, 1),
                '@' => (TokenKind::At, 1),
                '(' => (TokenKind::LParen, 1),
                ')' => (TokenKind::RParen, 1),
                '[' => (TokenKind::LBracket, 1),
                ']' => (TokenKind::RBracket, 1),
                '{' => (TokenKind::LBrace, 1),
                '}' => (TokenKind::RBrace, 1),
                ',' => (TokenKind::Comma, 1),
                ';' => (TokenKind::Semicolon, 1),
                _ => (TokenKind::Error, first.len_utf8()),
            }
        };
        self.offset += width;
        self.push(kind, start, self.offset);
        if kind == TokenKind::Error {
            self.diagnostics.push(
                Diagnostic::error(
                    self.source_id,
                    Self::range(start, self.offset),
                    format!("unrecognized character `{first}`"),
                )
                .with_code("OML0002"),
            );
        }
    }

    fn lex_block_comment(&mut self, start: usize) {
        self.offset += 2;
        if let Some(relative_end) = self.remaining().find("%}") {
            self.offset += relative_end + 2;
        } else {
            self.offset = self.source.len();
            self.diagnostics.push(
                Diagnostic::error(
                    self.source_id,
                    Self::range(start, self.offset),
                    "unterminated block comment",
                )
                .with_code("OML0003"),
            );
        }
        self.push(TokenKind::Comment, start, self.offset);
    }

    fn lex_number(&mut self, start: usize) {
        let bytes = self.source.as_bytes();
        let mut cursor = self.offset;
        if bytes.get(cursor) == Some(&b'.') {
            cursor += 1;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'.')
            && !matches!(
                bytes.get(cursor + 1),
                Some(b'*' | b'/' | b'\\' | b'^' | b'\'')
            )
        {
            cursor += 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
        }
        if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
            let exponent = cursor;
            cursor += 1;
            if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
                cursor += 1;
            }
            let digits = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
            if cursor == digits {
                cursor = exponent;
            }
        }
        if matches!(bytes.get(cursor), Some(b'i' | b'j')) {
            cursor += 1;
        }
        self.offset = cursor;
        self.push(TokenKind::Number, start, self.offset);
    }

    fn lex_quoted(&mut self, start: usize, quote: char, kind: TokenKind) {
        self.offset += quote.len_utf8();
        let mut terminated = false;
        while self.offset < self.source.len() {
            let Some(character) = self.remaining().chars().next() else {
                break;
            };
            if character == '\r' || character == '\n' {
                break;
            }
            self.offset += character.len_utf8();
            if character == quote {
                if self.remaining().starts_with(quote) {
                    self.offset += quote.len_utf8();
                } else {
                    terminated = true;
                    break;
                }
            }
        }
        self.push(kind, start, self.offset);
        if !terminated {
            self.diagnostics.push(
                Diagnostic::error(
                    self.source_id,
                    Self::range(start, self.offset),
                    "unterminated quoted literal",
                )
                .with_code("OML0004"),
            );
        }
    }

    fn consume_while(&mut self, mut predicate: impl FnMut(char) -> bool) {
        while let Some(character) = self.remaining().chars().next() {
            if !predicate(character) {
                break;
            }
            self.offset += character.len_utf8();
        }
    }

    fn consume_until_line_end(&mut self) {
        while let Some(character) = self.remaining().chars().next() {
            if character == '\r' || character == '\n' {
                break;
            }
            self.offset += character.len_utf8();
        }
    }

    fn remaining(&self) -> &'source str {
        &self.source[self.offset..]
    }

    fn range(start: usize, end: usize) -> TextRange {
        TextRange::from_usize(start, end).unwrap_or(TextRange::empty(u32::MAX))
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        let text = self.source[start..end].to_owned();
        let contains_line_break = text.contains('\n') || text.contains('\r');
        self.tokens
            .push(Token::new(kind, text, Self::range(start, end)));
        if !kind.is_trivia() {
            self.line_continuation_pending = false;
            match kind {
                TokenKind::LBracket | TokenKind::LBrace => self.matrix_depth += 1,
                TokenKind::RBracket | TokenKind::RBrace => {
                    self.matrix_depth = self.matrix_depth.saturating_sub(1);
                }
                _ => {}
            }
            self.previous_significant = Some(kind);
            self.horizontal_trivia_since_significant = false;
        } else if kind == TokenKind::Comment && contains_line_break {
            self.previous_significant = None;
            self.horizontal_trivia_since_significant = false;
            self.line_continuation_pending = false;
        }
    }
}

fn keyword_kind(text: &str) -> TokenKind {
    // `events` is intentionally absent: it introduces a block only in class-definition
    // context and remains an ordinary identifier everywhere else.
    match text {
        "break" => TokenKind::KwBreak,
        "case" => TokenKind::KwCase,
        "catch" => TokenKind::KwCatch,
        "classdef" => TokenKind::KwClassdef,
        "continue" => TokenKind::KwContinue,
        "else" => TokenKind::KwElse,
        "elseif" => TokenKind::KwElseif,
        "end" => TokenKind::KwEnd,
        "enumeration" => TokenKind::KwEnumeration,
        "for" => TokenKind::KwFor,
        "function" => TokenKind::KwFunction,
        "global" => TokenKind::KwGlobal,
        "if" => TokenKind::KwIf,
        "methods" => TokenKind::KwMethods,
        "otherwise" => TokenKind::KwOtherwise,
        "parfor" => TokenKind::KwParfor,
        "persistent" => TokenKind::KwPersistent,
        "properties" => TokenKind::KwProperties,
        "return" => TokenKind::KwReturn,
        "spmd" => TokenKind::KwSpmd,
        "switch" => TokenKind::KwSwitch,
        "try" => TokenKind::KwTry,
        "while" => TokenKind::KwWhile,
        _ => TokenKind::Identifier,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionContext {
    Normal,
    ApplicationArgument,
    NestedApplicationArgument,
    Matrix,
    MatrixApplicationArgument,
}

impl ExpressionContext {
    const fn nested(self) -> Self {
        match self {
            Self::ApplicationArgument | Self::NestedApplicationArgument => {
                Self::NestedApplicationArgument
            }
            Self::Normal | Self::Matrix | Self::MatrixApplicationArgument => self,
        }
    }

    const fn matrix(self) -> Self {
        if self.binds_end_index() {
            Self::MatrixApplicationArgument
        } else {
            Self::Matrix
        }
    }

    const fn allows_all_index(self) -> bool {
        matches!(self, Self::ApplicationArgument)
    }

    const fn binds_end_index(self) -> bool {
        matches!(
            self,
            Self::ApplicationArgument
                | Self::NestedApplicationArgument
                | Self::MatrixApplicationArgument
        )
    }

    const fn is_matrix(self) -> bool {
        matches!(self, Self::Matrix | Self::MatrixApplicationArgument)
    }
}

struct Parser {
    source_id: SourceId,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    nodes: Vec<SyntaxNode>,
    position: usize,
}

impl Parser {
    fn new(source_id: SourceId, tokens: Vec<Token>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            source_id,
            tokens,
            diagnostics,
            nodes: Vec::new(),
            position: 0,
        }
    }

    fn run(mut self) -> ParseResult {
        let mut children = Vec::new();
        while self.peek_kind() != TokenKind::Eof {
            let before = self.position;
            if let Some(statement) = self.parse_statement() {
                children.push(statement);
            }
            if self.position <= before {
                let index = self.peek_index();
                self.error_at(index, "parser could not make progress", "OMP0001");
                self.position = index.saturating_add(1).min(self.tokens.len());
            }
        }
        let root = self.add_node(SyntaxKind::Root, 0, self.tokens.len(), children);
        ParseResult {
            syntax: Cst::new(self.source_id, self.tokens, self.nodes, root),
            diagnostics: self.diagnostics,
        }
    }

    fn parse_statement(&mut self) -> Option<NodeId> {
        self.skip_trivia();
        if self.tokens[self.peek_index()].text == "arguments"
            && self
                .tokens
                .iter()
                .skip(self.peek_index() + 1)
                .find(|token| !matches!(token.kind, TokenKind::Whitespace | TokenKind::Comment))
                .is_some_and(|token| matches!(token.kind, TokenKind::Newline | TokenKind::LParen))
        {
            return Some(self.parse_arguments_block());
        }
        if self.is_clear_command() {
            return Some(self.parse_clear_statement());
        }
        if self.is_command_form() {
            return Some(self.parse_command_statement());
        }
        match self.peek_kind() {
            TokenKind::Eof => None,
            TokenKind::Semicolon | TokenKind::Comma => {
                let start = self.bump_index();
                Some(self.add_node(SyntaxKind::EmptyStmt, start, self.position, Vec::new()))
            }
            TokenKind::KwIf => Some(self.parse_if()),
            TokenKind::KwFor => Some(self.parse_for()),
            TokenKind::KwWhile => Some(self.parse_while()),
            TokenKind::KwTry => Some(self.parse_try()),
            TokenKind::KwSwitch => Some(self.parse_switch()),
            TokenKind::KwFunction => Some(self.parse_function()),
            TokenKind::KwClassdef => Some(self.parse_classdef()),
            TokenKind::KwBreak => Some(self.parse_keyword_statement(SyntaxKind::BreakStmt)),
            TokenKind::KwContinue => Some(self.parse_keyword_statement(SyntaxKind::ContinueStmt)),
            TokenKind::KwReturn => Some(self.parse_keyword_statement(SyntaxKind::ReturnStmt)),
            TokenKind::KwGlobal => Some(self.parse_declaration_statement(SyntaxKind::GlobalStmt)),
            TokenKind::KwPersistent => {
                Some(self.parse_declaration_statement(SyntaxKind::PersistentStmt))
            }
            TokenKind::KwCatch => Some(self.parse_unexpected_catch()),
            TokenKind::KwParfor
            | TokenKind::KwSpmd
            | TokenKind::KwCase
            | TokenKind::KwOtherwise
            | TokenKind::KwEnumeration
            | TokenKind::KwEvents
            | TokenKind::KwElse
            | TokenKind::KwElseif => Some(self.parse_unsupported_statement()),
            TokenKind::KwEnd => {
                let start = self.bump_index();
                self.error_at(start, "unmatched `end`", "OMP0002");
                Some(self.add_node(SyntaxKind::Error, start, self.position, Vec::new()))
            }
            _ => self.parse_simple_statement(),
        }
    }

    fn parse_simple_statement(&mut self) -> Option<NodeId> {
        self.skip_trivia();
        let start = self.peek_index();
        let left = self.parse_expression(0, ExpressionContext::Normal)?;
        let mut children = vec![left];
        let kind = if self.peek_on_same_line() == TokenKind::Equal {
            self.bump_index();
            if let Some(right) = self.parse_expression(0, ExpressionContext::Normal) {
                children.push(right);
            } else {
                self.error_here("expected expression after `=`", "OMP0003");
            }
            SyntaxKind::AssignmentStmt
        } else {
            SyntaxKind::ExprStmt
        };
        self.consume_simple_terminator();
        Some(self.add_node(kind, start, self.position, children))
    }

    fn is_command_form(&self) -> bool {
        let command = self.peek_index();
        if self.tokens.get(command).map(|token| token.kind) != Some(TokenKind::Identifier) {
            return false;
        }
        let argument = self.next_significant(command.saturating_add(1));
        if self.has_line_break(command.saturating_add(1), argument)
            || !self.has_horizontal_space(command.saturating_add(1), argument)
        {
            return false;
        }
        let Some(argument_kind) = self.tokens.get(argument).map(|token| token.kind) else {
            return false;
        };
        if matches!(
            argument_kind,
            TokenKind::Equal
                | TokenKind::LParen
                | TokenKind::Semicolon
                | TokenKind::Comma
                | TokenKind::Eof
        ) {
            return false;
        }

        // In command syntax an operator attached to the argument text remains literal
        // (`save -v7.3 file.mat`), while whitespace on both sides makes it an
        // expression operator (`a + b`). A continued logical line follows the same
        // rule. This distinction is observable in MATLAB R2022b for arithmetic,
        // relational, logical, range, and element-wise operators.
        if binary_binding_power(argument_kind).is_some() {
            let following = self.next_significant(argument.saturating_add(1));
            if !self.has_line_break(argument.saturating_add(1), following)
                && self.has_horizontal_space(argument.saturating_add(1), following)
            {
                return false;
            }
        }

        true
    }

    fn parse_command_statement(&mut self) -> NodeId {
        let start = self.bump_index();
        let command = self.add_node(SyntaxKind::NameExpr, start, self.position, Vec::new());
        let mut quoted = false;
        let mut continuation_pending = false;

        while self.position < self.tokens.len() {
            let index = self.position;
            let token = &self.tokens[index];
            match token.kind {
                TokenKind::Newline if quoted => {
                    self.error_at(index, "unterminated quoted command argument", "OMP0004");
                    break;
                }
                TokenKind::Newline if continuation_pending => {
                    continuation_pending = false;
                    self.position += 1;
                    continue;
                }
                TokenKind::Eof | TokenKind::Newline => break,
                TokenKind::Comment if !quoted && !continuation_pending => break,
                TokenKind::Comment if !quoted && continuation_pending => {
                    self.position += 1;
                    continue;
                }
                TokenKind::Ellipsis if !quoted => {
                    continuation_pending = true;
                    self.position += 1;
                    continue;
                }
                TokenKind::Semicolon | TokenKind::Comma if !quoted => {
                    self.position += 1;
                    break;
                }
                _ => {}
            }
            quoted = toggled_command_quote_state(quoted, &token.text);
            self.position += 1;
        }
        if quoted && self.peek_kind() == TokenKind::Eof {
            self.error_here("unterminated quoted command argument", "OMP0004");
        }
        self.add_node(SyntaxKind::CommandStmt, start, self.position, vec![command])
    }

    fn is_clear_command(&self) -> bool {
        let clear = self.peek_index();
        if !matches!(
            self.tokens.get(clear),
            Some(Token {
                kind: TokenKind::Identifier,
                text,
                ..
            }) if text == "clear"
        ) {
            return false;
        }
        let argument = self.next_significant(clear.saturating_add(1));
        if self.has_line_break(clear.saturating_add(1), argument) {
            return true;
        }
        match self.tokens.get(argument).map(|token| token.kind) {
            Some(TokenKind::Equal) => false,
            Some(TokenKind::Identifier) => {
                self.has_horizontal_space(clear.saturating_add(1), argument)
            }
            Some(TokenKind::LParen | TokenKind::Semicolon | TokenKind::Comma | TokenKind::Eof)
            | None => true,
            Some(_) => self.has_horizontal_space(clear.saturating_add(1), argument),
        }
    }

    fn parse_clear_statement(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        let mut import_option = false;
        loop {
            let argument = self.peek_index();
            if self.has_line_break(self.position, argument)
                || self.tokens.get(argument).map(|token| token.kind) == Some(TokenKind::Eof)
            {
                break;
            }
            let kind = self.tokens[argument].kind;
            if matches!(kind, TokenKind::Semicolon | TokenKind::Comma) {
                self.position = argument + 1;
                break;
            }
            if import_option {
                self.error_at(
                    argument,
                    "`clear import` does not accept additional names",
                    "OMP0006",
                );
                let error_start = argument;
                self.recover_to_line_end();
                children.push(self.add_node(
                    SyntaxKind::Error,
                    error_start,
                    self.position,
                    Vec::new(),
                ));
                break;
            }
            if kind == TokenKind::KwGlobal && children.is_empty() {
                self.position = argument + 1;
                continue;
            }
            let is_option = kind == TokenKind::Identifier
                && is_unsupported_clear_option(&self.tokens[argument].text);
            if kind == TokenKind::Identifier
                && children.is_empty()
                && self.tokens[argument].text == "import"
            {
                self.position = argument + 1;
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    argument,
                    self.position,
                    Vec::new(),
                ));
                import_option = true;
                continue;
            }
            if kind == TokenKind::Identifier && !is_option {
                self.position = argument + 1;
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    argument,
                    self.position,
                    Vec::new(),
                ));
                continue;
            }

            self.error_at(
                argument,
                if is_option {
                    "clear options are not supported; expected a list of variable names"
                } else {
                    "unsupported clear form; expected a whitespace-separated list of variable names"
                },
                "OMP0006",
            );
            let error_start = argument;
            self.recover_to_line_end();
            children.push(self.add_node(SyntaxKind::Error, error_start, self.position, Vec::new()));
            break;
        }
        self.add_node(SyntaxKind::ClearStmt, start, self.position, children)
    }

    fn parse_keyword_statement(&mut self, kind: SyntaxKind) -> NodeId {
        let start = self.bump_index();
        self.consume_simple_terminator();
        self.add_node(kind, start, self.position, Vec::new())
    }

    fn parse_declaration_statement(&mut self, kind: SyntaxKind) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        let mut persistent_names = Vec::<String>::new();

        loop {
            let item_start = self.peek_index();
            if self.has_line_break(self.position, item_start) {
                self.recover_to_line_end();
                break;
            }
            match self.tokens.get(item_start).map(|token| token.kind) {
                Some(TokenKind::Eof) | None => {
                    self.position = item_start;
                    break;
                }
                Some(TokenKind::Comma | TokenKind::Semicolon) => {
                    self.position = item_start + 1;
                    break;
                }
                Some(_) => {}
            }

            let item_end = self.declaration_item_end(item_start);
            self.position = item_end;
            let is_name =
                item_end == item_start + 1 && self.tokens[item_start].kind == TokenKind::Identifier;
            if is_name {
                let name = self.tokens[item_start].text.clone();
                if kind == SyntaxKind::PersistentStmt && persistent_names.contains(&name) {
                    self.error_at(
                        item_start,
                        format!("duplicate persistent declaration `{name}`"),
                        "OMP0009",
                    );
                }
                persistent_names.push(name);
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    item_start,
                    item_end,
                    Vec::new(),
                ));
            } else {
                self.error_at(
                    item_start,
                    "expected a whitespace-separated variable name in declaration",
                    "OMP0008",
                );
                children.push(self.add_node(SyntaxKind::Error, item_start, item_end, Vec::new()));
            }
        }

        if children.is_empty() {
            self.error_at(
                start,
                "declaration requires at least one variable name",
                "OMP0007",
            );
        }
        self.add_node(kind, start, self.position, children)
    }

    fn declaration_item_end(&self, start: usize) -> usize {
        let mut end = start.saturating_add(1).min(self.tokens.len());
        loop {
            let next = self.next_significant(end);
            if self.has_line_break(end, next)
                || matches!(
                    self.tokens.get(next).map(|token| token.kind),
                    Some(TokenKind::Comma | TokenKind::Semicolon | TokenKind::Eof) | None
                )
                || self.has_declaration_item_separator(end, next)
            {
                return end;
            }
            let next_end = next.saturating_add(1).min(self.tokens.len());
            if next_end <= end {
                return end;
            }
            end = next_end;
        }
    }

    fn has_declaration_item_separator(&self, start: usize, end: usize) -> bool {
        self.tokens.get(start..end).is_some_and(|tokens| {
            tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::Whitespace | TokenKind::Ellipsis))
        })
    }

    fn parse_unsupported_statement(&mut self) -> NodeId {
        let start = self.peek_index();
        let spelling = self
            .tokens
            .get(start)
            .map_or("token", |token| token.text.as_str())
            .to_owned();
        self.error_at(
            start,
            format!("`{spelling}` is not supported by this frontend milestone"),
            "OMP0004",
        );
        self.bump_index();
        self.recover_to_line_end();
        self.add_node(SyntaxKind::Error, start, self.position, Vec::new())
    }

    fn parse_if(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if let Some(condition) = self.parse_expression(0, ExpressionContext::Normal) {
            children.push(condition);
        } else {
            self.error_here("expected condition after `if`", "OMP0010");
        }
        self.consume_header_terminator();
        children.push(self.parse_block_until(&[
            TokenKind::KwElseif,
            TokenKind::KwElse,
            TokenKind::KwEnd,
        ]));
        while self.peek_kind() == TokenKind::KwElseif {
            let clause_start = self.bump_index();
            let mut clause_children = Vec::new();
            if let Some(condition) = self.parse_expression(0, ExpressionContext::Normal) {
                clause_children.push(condition);
            } else {
                self.error_here("expected condition after `elseif`", "OMP0011");
            }
            self.consume_header_terminator();
            clause_children.push(self.parse_block_until(&[
                TokenKind::KwElseif,
                TokenKind::KwElse,
                TokenKind::KwEnd,
            ]));
            children.push(self.add_node(
                SyntaxKind::ElseifClause,
                clause_start,
                self.position,
                clause_children,
            ));
        }
        if self.peek_kind() == TokenKind::KwElse {
            let clause_start = self.bump_index();
            self.consume_header_terminator();
            let body = self.parse_block_until(&[TokenKind::KwEnd]);
            children.push(self.add_node(
                SyntaxKind::ElseClause,
                clause_start,
                self.position,
                vec![body],
            ));
        }
        self.expect_end("`if` block");
        self.add_node(SyntaxKind::IfStmt, start, self.position, children)
    }

    fn parse_for(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if let Some(variable) = self.parse_expression(0, ExpressionContext::Normal) {
            children.push(variable);
        } else {
            self.error_here("expected loop variable after `for`", "OMP0020");
        }
        if self.peek_on_same_line() == TokenKind::Equal {
            self.bump_index();
        } else {
            self.error_here("expected `=` in `for` header", "OMP0021");
        }
        if let Some(iterable) = self.parse_expression(0, ExpressionContext::Normal) {
            children.push(iterable);
        } else {
            self.error_here("expected iteration expression", "OMP0022");
        }
        self.consume_header_terminator();
        children.push(self.parse_block_until(&[TokenKind::KwEnd]));
        self.expect_end("`for` block");
        self.add_node(SyntaxKind::ForStmt, start, self.position, children)
    }

    fn parse_while(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if let Some(condition) = self.parse_expression(0, ExpressionContext::Normal) {
            children.push(condition);
        } else {
            self.error_here("expected condition after `while`", "OMP0030");
        }
        self.consume_header_terminator();
        children.push(self.parse_block_until(&[TokenKind::KwEnd]));
        self.expect_end("`while` block");
        self.add_node(SyntaxKind::WhileStmt, start, self.position, children)
    }

    fn parse_try(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        self.consume_required_header_terminator("`try`", "OMP0038");
        children.push(self.parse_block_until(&[TokenKind::KwCatch, TokenKind::KwEnd]));

        let mut saw_catch = false;
        while self.peek_kind() == TokenKind::KwCatch {
            if saw_catch {
                self.error_here("`catch` may appear only once in a `try` block", "OMP0036");
            }
            saw_catch = true;
            children.push(self.parse_catch_clause());
        }
        self.expect_end("`try` block");
        self.add_node(SyntaxKind::TryStmt, start, self.position, children)
    }

    fn parse_catch_clause(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::Identifier {
            let identifier = self.bump_index();
            children.push(self.add_node(
                SyntaxKind::NameExpr,
                identifier,
                self.position,
                Vec::new(),
            ));
        }
        self.consume_required_header_terminator("`catch` header", "OMP0038");
        children.push(self.parse_block_until(&[TokenKind::KwCatch, TokenKind::KwEnd]));
        self.add_node(SyntaxKind::CatchClause, start, self.position, children)
    }

    fn parse_unexpected_catch(&mut self) -> NodeId {
        let start = self.bump_index();
        self.error_at(start, "unmatched `catch`", "OMP0037");
        self.recover_to_line_end();
        self.add_node(SyntaxKind::Error, start, self.position, Vec::new())
    }

    fn parse_switch(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line().can_start_expression() {
            if let Some(selector) = self.parse_expression(0, ExpressionContext::Normal) {
                children.push(selector);
            } else {
                self.error_here("expected selector after `switch`", "OMP0031");
            }
        } else {
            self.error_here("expected selector after `switch`", "OMP0031");
        }
        self.consume_header_terminator();

        let mut saw_otherwise = false;
        loop {
            self.skip_trivia();
            match self.peek_kind() {
                TokenKind::KwCase => {
                    if saw_otherwise {
                        self.error_here("`case` cannot follow `otherwise`", "OMP0034");
                    }
                    children.push(self.parse_case_clause());
                }
                TokenKind::KwOtherwise => {
                    if saw_otherwise {
                        self.error_here(
                            "`otherwise` may appear only once in a `switch` block",
                            "OMP0033",
                        );
                    }
                    saw_otherwise = true;
                    children.push(self.parse_otherwise_clause());
                }
                TokenKind::KwEnd | TokenKind::Eof => break,
                _ => {
                    self.error_here(
                        "expected `case`, `otherwise`, or `end` in `switch` block",
                        "OMP0035",
                    );
                    let before = self.position;
                    if let Some(statement) = self.parse_statement() {
                        children.push(statement);
                    }
                    if self.position <= before {
                        self.bump_index();
                    }
                }
            }
        }

        self.expect_end("`switch` block");
        self.add_node(SyntaxKind::SwitchStmt, start, self.position, children)
    }

    fn parse_case_clause(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line().can_start_expression() {
            if let Some(expression) = self.parse_expression(0, ExpressionContext::Normal) {
                children.push(expression);
            } else {
                self.error_here("expected expression after `case`", "OMP0032");
            }
        } else {
            self.error_here("expected expression after `case`", "OMP0032");
        }
        self.consume_header_terminator();
        children.push(self.parse_block_until(&[
            TokenKind::KwCase,
            TokenKind::KwOtherwise,
            TokenKind::KwEnd,
        ]));
        self.add_node(SyntaxKind::CaseClause, start, self.position, children)
    }

    fn parse_otherwise_clause(&mut self) -> NodeId {
        let start = self.bump_index();
        self.consume_header_terminator();
        let body =
            self.parse_block_until(&[TokenKind::KwCase, TokenKind::KwOtherwise, TokenKind::KwEnd]);
        self.add_node(
            SyntaxKind::OtherwiseClause,
            start,
            self.position,
            vec![body],
        )
    }

    fn parse_function(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut signature_children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LBracket {
            signature_children.push(self.parse_name_list(
                SyntaxKind::OutputList,
                TokenKind::LBracket,
                TokenKind::RBracket,
            ));
            if self.peek_on_same_line() == TokenKind::Equal {
                self.bump_index();
            } else {
                self.error_here("expected `=` after function outputs", "OMP0040");
            }
        } else if self.looks_like_single_output() {
            let output_start = self.bump_index();
            let output = self.add_node(
                SyntaxKind::NameExpr,
                output_start,
                self.position,
                Vec::new(),
            );
            signature_children.push(self.add_node(
                SyntaxKind::OutputList,
                output_start,
                self.position,
                vec![output],
            ));
            self.bump_index();
        }
        if self.peek_on_same_line() == TokenKind::Identifier {
            let name_start = self.bump_index();
            if self.peek_on_same_line() == TokenKind::Dot {
                self.bump_index();
                if self.peek_on_same_line() == TokenKind::Identifier {
                    self.bump_index();
                } else {
                    self.error_here(
                        "expected accessor name after `.` in function name",
                        "OMP0042",
                    );
                }
            }
            signature_children.push(self.add_node(
                SyntaxKind::NameExpr,
                name_start,
                self.position,
                Vec::new(),
            ));
        } else {
            self.error_here("expected function name", "OMP0041");
        }
        if self.peek_on_same_line() == TokenKind::LParen {
            signature_children.push(self.parse_name_list(
                SyntaxKind::ParameterList,
                TokenKind::LParen,
                TokenKind::RParen,
            ));
        }
        let signature = self.add_node(
            SyntaxKind::FunctionSignature,
            start,
            self.position,
            signature_children,
        );
        self.consume_header_terminator();
        let body = self.parse_block_until(&[TokenKind::KwEnd]);
        self.expect_end("function");
        self.add_node(
            SyntaxKind::FunctionDef,
            start,
            self.position,
            vec![signature, body],
        )
    }

    fn consume_qualified_name_tail(&mut self) {
        while self.peek_on_same_line() == TokenKind::Dot {
            self.bump_index();
            if self.peek_on_same_line() == TokenKind::Identifier {
                self.bump_index();
            } else {
                self.error_here("expected name after package separator", "OMP0052");
                break;
            }
        }
    }

    fn parse_classdef(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut header_children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            header_children.push(self.parse_attributes());
        }
        if self.peek_on_same_line() == TokenKind::Identifier {
            let name_start = self.bump_index();
            header_children.push(self.add_node(
                SyntaxKind::NameExpr,
                name_start,
                self.position,
                Vec::new(),
            ));
        } else {
            self.error_here("expected class name", "OMP0050");
        }
        if self.peek_on_same_line() == TokenKind::Less {
            self.bump_index();
            if self.peek_on_same_line() == TokenKind::Identifier {
                let superclass_start = self.bump_index();
                self.consume_qualified_name_tail();
                header_children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    superclass_start,
                    self.position,
                    Vec::new(),
                ));
                if self.peek_on_same_line() == TokenKind::And {
                    self.error_here(
                        "multiple inheritance is outside the release-one contract",
                        "OMP0051",
                    );
                }
            } else {
                self.error_here("expected superclass name", "OMP0052");
            }
        }
        let header = self.add_node(
            SyntaxKind::ClassHeader,
            start,
            self.position,
            header_children,
        );
        self.consume_header_terminator();
        let mut children = vec![header];
        loop {
            self.skip_trivia();
            let before = self.position;
            match self.peek_kind() {
                TokenKind::KwProperties => children.push(self.parse_properties_block()),
                TokenKind::KwMethods => children.push(self.parse_methods_block()),
                TokenKind::KwEnumeration => children.push(self.parse_enumeration_block()),
                TokenKind::Identifier if self.peek_text() == Some("events") => {
                    children.push(self.parse_events_block());
                }
                TokenKind::KwEnd | TokenKind::Eof => break,
                _ => {
                    self.error_here(
                        "expected `properties`, `methods`, `enumeration`, `events`, or `end` in class body",
                        "OMP0053",
                    );
                    if let Some(statement) = self.parse_statement() {
                        children.push(statement);
                    }
                }
            }
            if self.position == before {
                self.bump_index();
            }
        }
        self.expect_end("class definition");
        self.add_node(SyntaxKind::ClassDef, start, self.position, children)
    }

    fn parse_arguments_block(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            children.push(self.parse_attributes());
        }
        self.consume_header_terminator();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::KwEnd | TokenKind::Eof) {
                break;
            }
            let declaration_start = self.peek_index();
            let mut parts = Vec::new();
            if self.peek_kind() != TokenKind::Identifier {
                self.error_here("expected argument name", "OMP0160");
                self.recover_to_line_end();
                continue;
            }
            let name_start = self.bump_index();
            self.consume_qualified_name_tail();
            parts.push(self.add_node(SyntaxKind::NameExpr, name_start, self.position, Vec::new()));
            for (open, close, kind) in [
                (
                    TokenKind::LParen,
                    TokenKind::RParen,
                    SyntaxKind::ArgumentDimensions,
                ),
                (
                    TokenKind::Identifier,
                    TokenKind::Eof,
                    SyntaxKind::ArgumentClass,
                ),
                (
                    TokenKind::LBrace,
                    TokenKind::RBrace,
                    SyntaxKind::ArgumentValidators,
                ),
                (
                    TokenKind::Equal,
                    TokenKind::Eof,
                    SyntaxKind::ArgumentDefault,
                ),
            ] {
                if self.peek_on_same_line() != open {
                    continue;
                }
                let part_start = self.bump_index();
                let mut expressions = Vec::new();
                if kind == SyntaxKind::ArgumentClass {
                    self.consume_qualified_name_tail();
                } else if kind == SyntaxKind::ArgumentDefault {
                    if let Some(value) = self.parse_expression(0, ExpressionContext::Normal) {
                        expressions.push(value);
                    } else {
                        self.error_here("expected argument default", "OMP0161");
                    }
                } else {
                    expressions = self.parse_application_arguments(
                        close,
                        "expected validation expression",
                        "OMP0162",
                    );
                    self.expect(close, "unclosed argument validation list", "OMP0163");
                    if kind == SyntaxKind::ArgumentDimensions && expressions.is_empty() {
                        self.error_here("argument dimensions cannot be empty", "OMP0167");
                    }
                }
                parts.push(self.add_node(kind, part_start, self.position, expressions));
            }
            self.consume_simple_terminator();
            children.push(self.add_node(
                SyntaxKind::ArgumentDecl,
                declaration_start,
                self.position,
                parts,
            ));
        }
        self.expect_end("arguments block");
        self.add_node(SyntaxKind::ArgumentsBlock, start, self.position, children)
    }

    fn parse_properties_block(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            children.push(self.parse_attributes());
        }
        self.consume_header_terminator();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::KwEnd | TokenKind::Eof) {
                break;
            }
            let declaration_start = self.peek_index();
            let mut declaration_children = Vec::new();
            if let Some(name) = self.parse_expression(0, ExpressionContext::Normal) {
                declaration_children.push(name);
            } else {
                self.error_here("expected property declaration", "OMP0060");
                self.recover_to_line_end();
            }
            if self.peek_on_same_line() == TokenKind::Equal {
                self.bump_index();
                if let Some(value) = self.parse_expression(0, ExpressionContext::Normal) {
                    declaration_children.push(value);
                } else {
                    self.error_here("expected property default value", "OMP0061");
                }
            }
            self.consume_simple_terminator();
            children.push(self.add_node(
                SyntaxKind::PropertyDecl,
                declaration_start,
                self.position,
                declaration_children,
            ));
        }
        self.expect_end("properties block");
        self.add_node(SyntaxKind::PropertiesBlock, start, self.position, children)
    }

    fn parse_methods_block(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            children.push(self.parse_attributes());
        }
        self.consume_header_terminator();
        loop {
            self.skip_trivia();
            let before = self.position;
            match self.peek_kind() {
                TokenKind::KwEnd | TokenKind::Eof => break,
                TokenKind::KwFunction => children.push(self.parse_function()),
                TokenKind::Identifier | TokenKind::LBracket => {
                    children.push(self.parse_method_declaration());
                }
                _ => {
                    self.error_here(
                        "expected a method definition, method signature, or `end`",
                        "OMP0070",
                    );
                    if let Some(statement) = self.parse_statement() {
                        children.push(statement);
                    }
                }
            }
            // Unexpected class-block keywords are recovery boundaries for
            // parse_statement. Consume one token if recovery made no progress.
            if self.position == before {
                self.bump_index();
            }
        }
        self.expect_end("methods block");
        self.add_node(SyntaxKind::MethodsBlock, start, self.position, children)
    }

    fn parse_enumeration_block(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            children.push(self.parse_attributes());
        }
        self.consume_header_terminator();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::KwEnd | TokenKind::Eof) {
                break;
            }
            let declaration_start = self.peek_index();
            let mut declaration_children = Vec::new();
            if self.peek_on_same_line() == TokenKind::Identifier {
                let name_start = self.bump_index();
                declaration_children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    name_start,
                    self.position,
                    Vec::new(),
                ));
            } else {
                self.error_here("expected enumeration member name", "OMP0073");
                self.recover_to_line_end();
            }
            if self.peek_on_same_line() == TokenKind::LParen {
                self.bump_index();
                loop {
                    if matches!(self.peek_on_same_line(), TokenKind::RParen | TokenKind::Eof) {
                        break;
                    }
                    if let Some(argument) =
                        self.parse_expression(0, ExpressionContext::ApplicationArgument)
                    {
                        declaration_children.push(argument);
                    } else {
                        self.error_here("expected enumeration constructor argument", "OMP0074");
                        self.recover_until(&[TokenKind::Comma, TokenKind::RParen]);
                    }
                    if self.peek_on_same_line() == TokenKind::Comma {
                        self.bump_index();
                    } else if self.peek_on_same_line() != TokenKind::RParen {
                        self.error_here(
                            "expected `,` or `)` after enumeration constructor argument",
                            "OMP0075",
                        );
                        self.recover_until(&[TokenKind::Comma, TokenKind::RParen]);
                        if self.peek_kind() == TokenKind::Comma {
                            self.bump_index();
                        }
                    }
                }
                self.expect(
                    TokenKind::RParen,
                    "expected `)` after enumeration constructor arguments",
                    "OMP0076",
                );
            }
            self.consume_simple_terminator();
            children.push(self.add_node(
                SyntaxKind::EnumMemberDecl,
                declaration_start,
                self.position,
                declaration_children,
            ));
        }
        self.expect_end("enumeration block");
        self.add_node(SyntaxKind::EnumerationBlock, start, self.position, children)
    }

    fn parse_events_block(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LParen {
            children.push(self.parse_attributes());
        }
        self.consume_header_terminator();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::KwEnd | TokenKind::Eof) {
                break;
            }
            let declaration_start = self.peek_index();
            let mut declaration_children = Vec::new();
            if self.peek_on_same_line() == TokenKind::Identifier {
                let name_start = self.bump_index();
                declaration_children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    name_start,
                    self.position,
                    Vec::new(),
                ));
            } else {
                self.error_here("expected event name", "OMP0077");
                self.recover_to_line_end();
            }
            self.consume_simple_terminator();
            children.push(self.add_node(
                SyntaxKind::EventDecl,
                declaration_start,
                self.position,
                declaration_children,
            ));
        }
        self.expect_end("events block");
        self.add_node(SyntaxKind::EventsBlock, start, self.position, children)
    }

    fn parse_method_declaration(&mut self) -> NodeId {
        let start = self.peek_index();
        let mut signature_children = Vec::new();
        if self.peek_on_same_line() == TokenKind::LBracket {
            signature_children.push(self.parse_name_list(
                SyntaxKind::OutputList,
                TokenKind::LBracket,
                TokenKind::RBracket,
            ));
            if self.peek_on_same_line() == TokenKind::Equal {
                self.bump_index();
            } else {
                self.error_here("expected `=` after abstract method outputs", "OMP0071");
            }
        } else if self.looks_like_single_output() {
            let output_start = self.bump_index();
            let output = self.add_node(
                SyntaxKind::NameExpr,
                output_start,
                self.position,
                Vec::new(),
            );
            signature_children.push(self.add_node(
                SyntaxKind::OutputList,
                output_start,
                self.position,
                vec![output],
            ));
            self.bump_index();
        }
        if self.peek_on_same_line() == TokenKind::Identifier {
            let name_start = self.bump_index();
            signature_children.push(self.add_node(
                SyntaxKind::NameExpr,
                name_start,
                self.position,
                Vec::new(),
            ));
        } else {
            self.error_here("expected abstract method name", "OMP0072");
        }
        if self.peek_on_same_line() == TokenKind::LParen {
            signature_children.push(self.parse_name_list(
                SyntaxKind::ParameterList,
                TokenKind::LParen,
                TokenKind::RParen,
            ));
        }
        self.consume_header_terminator();
        self.add_node(
            SyntaxKind::MethodDecl,
            start,
            self.position,
            signature_children,
        )
    }

    fn parse_attributes(&mut self) -> NodeId {
        let start = self.bump_index();
        let mut children = Vec::new();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                break;
            }
            let attribute_start = self.peek_index();
            let mut attribute_children = Vec::new();
            if self.peek_kind() == TokenKind::Identifier {
                let name_start = self.bump_index();
                attribute_children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    name_start,
                    self.position,
                    Vec::new(),
                ));
            } else {
                self.error_here("expected attribute name", "OMP0080");
                self.bump_index();
            }
            if self.peek_on_same_line() == TokenKind::Equal {
                self.bump_index();
                if let Some(value) = self.parse_expression(0, ExpressionContext::Normal) {
                    attribute_children.push(value);
                } else {
                    self.error_here("expected attribute value", "OMP0081");
                }
            }
            children.push(self.add_node(
                SyntaxKind::Attribute,
                attribute_start,
                self.position,
                attribute_children,
            ));
            if self.peek_kind() == TokenKind::Comma {
                self.bump_index();
            } else if self.peek_kind() != TokenKind::RParen {
                self.error_here("expected `,` or `)` after attribute", "OMP0082");
                self.recover_until(&[TokenKind::Comma, TokenKind::RParen]);
                if self.peek_kind() == TokenKind::Comma {
                    self.bump_index();
                }
            }
        }
        self.expect(
            TokenKind::RParen,
            "expected `)` after attributes",
            "OMP0083",
        );
        self.add_node(SyntaxKind::AttributeList, start, self.position, children)
    }

    fn parse_name_list(&mut self, kind: SyntaxKind, open: TokenKind, close: TokenKind) -> NodeId {
        let allows_space_separator = open == TokenKind::LBracket;
        let start = self.peek_index();
        self.expect(open, "expected list opener", "OMP0090");
        let mut children = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek_kind() == TokenKind::Eof || self.peek_kind() == close {
                break;
            }
            if matches!(self.peek_kind(), TokenKind::Identifier | TokenKind::Tilde) {
                let name_start = self.bump_index();
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    name_start,
                    self.position,
                    Vec::new(),
                ));
            } else {
                self.error_here("expected a name in list", "OMP0091");
                self.bump_index();
            }
            if self.peek_kind() == TokenKind::Comma {
                self.bump_index();
            } else if self.peek_kind() != close {
                let next = self.peek_index();
                if allows_space_separator
                    && self.has_horizontal_space(self.position, next)
                    && !self.has_line_break(self.position, next)
                    && matches!(
                        self.tokens.get(next).map(|token| token.kind),
                        Some(TokenKind::Identifier | TokenKind::Tilde)
                    )
                {
                    continue;
                }
                self.error_here("expected `,` or list closer", "OMP0092");
                self.recover_until(&[TokenKind::Comma, close]);
                if self.peek_kind() == TokenKind::Comma {
                    self.bump_index();
                }
            }
        }
        self.expect(close, "expected list closer", "OMP0093");
        self.add_node(kind, start, self.position, children)
    }

    fn parse_block_until(&mut self, terminators: &[TokenKind]) -> NodeId {
        self.skip_trivia();
        let start = self.peek_index();
        let mut children = Vec::new();
        loop {
            self.skip_trivia();
            if terminators.contains(&self.peek_kind()) || self.peek_kind() == TokenKind::Eof {
                break;
            }
            let before = self.position;
            if let Some(statement) = self.parse_statement() {
                children.push(statement);
            }
            if self.position <= before {
                self.error_here("could not recover statement", "OMP0094");
                self.bump_index();
            }
        }
        self.add_node(SyntaxKind::Block, start, self.position, children)
    }

    fn parse_expression(
        &mut self,
        minimum_binding_power: u8,
        context: ExpressionContext,
    ) -> Option<NodeId> {
        let (start, left) = self.parse_prefix(context)?;
        if self
            .nodes
            .get(left.raw())
            .is_some_and(|node| node.kind == SyntaxKind::ColonExpr)
        {
            return Some(left);
        }
        Some(self.parse_expression_tail(start, left, minimum_binding_power, context))
    }

    fn parse_prefix(&mut self, context: ExpressionContext) -> Option<(usize, NodeId)> {
        self.skip_trivia();
        let start = self.peek_index();
        let expression = match self.peek_kind() {
            TokenKind::Identifier => {
                self.bump_index();
                self.add_node(SyntaxKind::NameExpr, start, self.position, Vec::new())
            }
            TokenKind::KwEnd => self.parse_end_expression(start, context),
            TokenKind::Number => {
                self.bump_index();
                self.add_node(SyntaxKind::NumberExpr, start, self.position, Vec::new())
            }
            TokenKind::CharLiteral => {
                self.bump_index();
                self.add_node(SyntaxKind::CharExpr, start, self.position, Vec::new())
            }
            TokenKind::StringLiteral => {
                self.bump_index();
                self.add_node(SyntaxKind::StringExpr, start, self.position, Vec::new())
            }
            TokenKind::Colon if context.allows_all_index() => {
                self.bump_index();
                self.add_node(SyntaxKind::ColonExpr, start, self.position, Vec::new())
            }
            TokenKind::Plus | TokenKind::Minus | TokenKind::Tilde => {
                self.bump_index();
                let mut children = Vec::new();
                if let Some(operand) = self.parse_expression(18, context.nested()) {
                    children.push(operand);
                } else {
                    self.error_here("expected unary operand", "OMP0100");
                }
                self.add_node(SyntaxKind::UnaryExpr, start, self.position, children)
            }
            TokenKind::At => self.parse_function_handle_or_anonymous(start),
            TokenKind::LParen => {
                self.bump_index();
                let mut children = Vec::new();
                if let Some(expression) = self.parse_expression(0, context.nested()) {
                    children.push(expression);
                } else {
                    self.error_here("expected parenthesized expression", "OMP0102");
                }
                self.expect(
                    TokenKind::RParen,
                    "expected `)` after expression",
                    "OMP0103",
                );
                self.add_node(SyntaxKind::ParenExpr, start, self.position, children)
            }
            TokenKind::LBracket => self.parse_matrix(false, context),
            TokenKind::LBrace => self.parse_matrix(true, context),
            _ => return None,
        };
        Some((start, expression))
    }

    fn parse_function_handle_or_anonymous(&mut self, start: usize) -> NodeId {
        self.bump_index();
        if self.peek_kind() == TokenKind::LParen {
            let parameters = self.parse_anonymous_parameter_list();
            let mut children = vec![parameters];
            let body_index = self.peek_index();
            if self.has_line_break(self.position, body_index)
                || !self.peek_kind().can_start_expression()
            {
                self.error_here("expected anonymous-function body expression", "OMP0104");
            } else if let Some(body) = self.parse_expression(0, ExpressionContext::Normal) {
                children.push(body);
            } else {
                self.error_here("expected anonymous-function body expression", "OMP0104");
            }
            return self.add_node(
                SyntaxKind::AnonymousFunctionExpr,
                start,
                self.position,
                children,
            );
        }

        let mut children = Vec::new();
        if self.peek_kind() == TokenKind::Identifier {
            let name_start = self.bump_index();
            children.push(self.add_node(
                SyntaxKind::NameExpr,
                name_start,
                self.position,
                Vec::new(),
            ));
            loop {
                if self.peek_on_same_line() != TokenKind::Dot {
                    break;
                }
                self.bump_index();
                if self.peek_on_same_line() != TokenKind::Identifier {
                    self.error_here("expected qualified function name after `.`", "OMP0101");
                    break;
                }
                let component_start = self.bump_index();
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    component_start,
                    self.position,
                    Vec::new(),
                ));
            }
        } else {
            self.error_here("expected function name after `@`", "OMP0101");
        }
        self.add_node(
            SyntaxKind::FunctionHandleExpr,
            start,
            self.position,
            children,
        )
    }

    fn parse_end_expression(&mut self, start: usize, context: ExpressionContext) -> NodeId {
        self.bump_index();
        let kind = if context.binds_end_index() {
            SyntaxKind::EndIndexExpr
        } else {
            SyntaxKind::NameExpr
        };
        self.add_node(kind, start, self.position, Vec::new())
    }

    fn parse_anonymous_parameter_list(&mut self) -> NodeId {
        let start = self.peek_index();
        self.expect(TokenKind::LParen, "expected `(` after `@`", "OMP0106");
        let mut children = Vec::new();
        let mut names = Vec::new();
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::RParen | TokenKind::Eof) {
                break;
            }
            if matches!(self.peek_kind(), TokenKind::Identifier | TokenKind::Tilde) {
                let name_start = self.bump_index();
                let text = self.tokens[name_start].text.clone();
                if text != "~" && names.iter().any(|name| name == &text) {
                    self.error_at(
                        name_start,
                        format!("duplicate anonymous-function parameter `{text}`"),
                        "OMP0105",
                    );
                } else if text != "~" {
                    names.push(text);
                }
                children.push(self.add_node(
                    SyntaxKind::NameExpr,
                    name_start,
                    self.position,
                    Vec::new(),
                ));
            } else {
                self.error_here("expected an anonymous-function parameter", "OMP0107");
                self.bump_index();
            }
            if self.peek_kind() == TokenKind::Comma {
                self.bump_index();
            } else if self.peek_kind() != TokenKind::RParen {
                self.error_here(
                    "expected `,` or `)` after anonymous-function parameter",
                    "OMP0108",
                );
                self.recover_until(&[TokenKind::Comma, TokenKind::RParen]);
                if self.peek_kind() == TokenKind::Comma {
                    self.bump_index();
                }
            }
        }
        self.expect(
            TokenKind::RParen,
            "expected `)` after anonymous-function parameters",
            "OMP0109",
        );
        self.add_node(SyntaxKind::ParameterList, start, self.position, children)
    }

    fn parse_expression_tail(
        &mut self,
        start: usize,
        mut left: NodeId,
        minimum_binding_power: u8,
        context: ExpressionContext,
    ) -> NodeId {
        loop {
            let operator_index = self.peek_index();
            if self.has_line_break(self.position, operator_index) {
                break;
            }
            let operator = self.peek_kind();
            if matches!(operator, TokenKind::LParen | TokenKind::LBrace) {
                if 24 < minimum_binding_power {
                    break;
                }
                left = self.parse_postfix_apply(start, left, operator == TokenKind::LBrace);
                continue;
            }
            if operator == TokenKind::At {
                if 24 < minimum_binding_power {
                    break;
                }
                left = self.parse_superclass_constructor_call(start, left);
                continue;
            }
            if operator == TokenKind::Dot {
                if 24 < minimum_binding_power {
                    break;
                }
                self.bump_index();
                let mut children = vec![left];
                if self.peek_kind() == TokenKind::LParen {
                    self.bump_index();
                    if let Some(field) = self.parse_expression(0, context.nested()) {
                        children.push(field);
                    } else {
                        self.error_here("expected dynamic field expression", "OMP0122");
                    }
                    self.expect(
                        TokenKind::RParen,
                        "expected `)` after dynamic field expression",
                        "OMP0123",
                    );
                    left =
                        self.add_node(SyntaxKind::DynamicFieldExpr, start, self.position, children);
                } else if is_field_name_token(self.peek_kind()) {
                    let field_start = self.bump_index();
                    children.push(self.add_node(
                        SyntaxKind::NameExpr,
                        field_start,
                        self.position,
                        Vec::new(),
                    ));
                    left = self.add_node(SyntaxKind::FieldExpr, start, self.position, children);
                } else {
                    self.error_here("expected field name after `.`", "OMP0112");
                    left = self.add_node(SyntaxKind::FieldExpr, start, self.position, children);
                }
                continue;
            }
            if matches!(
                operator,
                TokenKind::ConjugateTranspose | TokenKind::DotTranspose
            ) {
                if 22 < minimum_binding_power {
                    break;
                }
                self.bump_index();
                left = self.add_node(SyntaxKind::TransposeExpr, start, self.position, vec![left]);
                continue;
            }
            let Some((left_power, right_power, node_kind)) = binary_binding_power(operator) else {
                break;
            };
            if left_power < minimum_binding_power {
                break;
            }
            if context.is_matrix()
                && matches!(operator, TokenKind::Plus | TokenKind::Minus)
                && self.is_matrix_signed_separator(operator_index)
            {
                break;
            }
            self.bump_index();
            let Some(right) = self.parse_expression(right_power, context.nested()) else {
                self.error_here("expected right-hand operand", "OMP0113");
                break;
            };
            left = self.add_node(node_kind, start, self.position, vec![left, right]);
        }
        left
    }

    fn parse_postfix_apply(&mut self, start: usize, target: NodeId, brace: bool) -> NodeId {
        let (close, argument_message, argument_code, close_message, close_code, kind) = if brace {
            (
                TokenKind::RBrace,
                "expected brace-application argument",
                "OMP0118",
                "expected `}` after brace-application arguments",
                "OMP0119",
                SyntaxKind::BraceApplyExpr,
            )
        } else {
            (
                TokenKind::RParen,
                "expected application argument",
                "OMP0110",
                "expected `)` after application arguments",
                "OMP0111",
                SyntaxKind::ParenApplyExpr,
            )
        };
        self.bump_index();
        let mut children = vec![target];
        children.extend(self.parse_application_arguments(close, argument_message, argument_code));
        self.expect(close, close_message, close_code);
        self.add_node(kind, start, self.position, children)
    }

    fn parse_application_arguments(
        &mut self,
        close: TokenKind,
        missing_message: &str,
        missing_code: &str,
    ) -> Vec<NodeId> {
        let mut arguments = Vec::new();
        let mut saw_named = false;
        loop {
            self.skip_trivia();
            if matches!(self.peek_kind(), TokenKind::Eof) || self.peek_kind() == close {
                break;
            }
            if let Some(argument) = self.parse_expression(0, ExpressionContext::ApplicationArgument)
            {
                if self.peek_kind() == TokenKind::Equal {
                    let start = self.nodes[argument.raw()].token_range.start;
                    if close != TokenKind::RParen
                        || self.nodes[argument.raw()].kind != SyntaxKind::NameExpr
                    {
                        self.error_here(
                            "Name=Value requires an identifier in a parenthesized call",
                            "OMP0164",
                        );
                    }
                    self.bump_index();
                    let mut children = vec![argument];
                    if let Some(value) =
                        self.parse_expression(0, ExpressionContext::ApplicationArgument)
                    {
                        children.push(value);
                    } else {
                        self.error_here("expected value after named argument", "OMP0165");
                    }
                    arguments.push(self.add_node(
                        SyntaxKind::NameValueArgument,
                        start,
                        self.position,
                        children,
                    ));
                    saw_named = true;
                } else {
                    if saw_named {
                        self.error_here("positional arguments cannot follow Name=Value", "OMP0166");
                    }
                    arguments.push(argument);
                }
            } else {
                self.error_here(missing_message, missing_code);
                self.recover_until(&[TokenKind::Comma, close, TokenKind::Eof]);
            }
            if self.peek_kind() == TokenKind::Comma {
                self.bump_index();
            } else {
                break;
            }
        }
        arguments
    }

    fn parse_superclass_constructor_call(&mut self, start: usize, object: NodeId) -> NodeId {
        self.bump_index();
        let mut children = vec![object];
        if self.peek_kind() == TokenKind::Identifier {
            let superclass_start = self.bump_index();
            self.consume_qualified_name_tail();
            children.push(self.add_node(
                SyntaxKind::NameExpr,
                superclass_start,
                self.position,
                Vec::new(),
            ));
        } else {
            self.error_here("expected superclass name after `@`", "OMP0114");
        }
        if self.peek_kind() == TokenKind::LParen {
            self.bump_index();
            children.extend(self.parse_application_arguments(
                TokenKind::RParen,
                "expected superclass constructor argument",
                "OMP0115",
            ));
            self.expect(
                TokenKind::RParen,
                "expected `)` after superclass constructor arguments",
                "OMP0116",
            );
        } else {
            self.error_here("expected `(` after superclass name", "OMP0117");
        }
        self.add_node(
            SyntaxKind::SuperclassConstructorCallExpr,
            start,
            self.position,
            children,
        )
    }

    fn parse_matrix(&mut self, cell: bool, context: ExpressionContext) -> NodeId {
        let start = self.bump_index();
        let assignment_target = !cell && self.bracket_is_assignment_target(start);
        let close = if cell {
            TokenKind::RBrace
        } else {
            TokenKind::RBracket
        };
        let row_kind = if cell {
            SyntaxKind::CellRow
        } else {
            SyntaxKind::MatrixRow
        };
        let expression_kind = if cell {
            SyntaxKind::CellExpr
        } else {
            SyntaxKind::MatrixExpr
        };
        let mut rows = Vec::new();
        let mut row_children = Vec::new();
        let mut row_start = self.peek_index();
        loop {
            let next = self.peek_index();
            if self.has_line_break(self.position, next) && !row_children.is_empty() {
                rows.push(self.add_node(row_kind, row_start, self.position, row_children));
                row_children = Vec::new();
                self.position = next;
                row_start = next;
            } else {
                self.skip_trivia();
            }
            match self.peek_kind() {
                kind if kind == close => {
                    if !row_children.is_empty() {
                        rows.push(self.add_node(row_kind, row_start, self.position, row_children));
                    }
                    self.bump_index();
                    break;
                }
                TokenKind::Eof => {
                    if !row_children.is_empty() {
                        rows.push(self.add_node(row_kind, row_start, self.position, row_children));
                    }
                    self.error_here("unterminated matrix or cell literal", "OMP0120");
                    break;
                }
                TokenKind::Semicolon => {
                    self.bump_index();
                    rows.push(self.add_node(row_kind, row_start, self.position, row_children));
                    row_children = Vec::new();
                    row_start = self.peek_index();
                }
                TokenKind::Comma => {
                    self.bump_index();
                }
                TokenKind::Tilde if assignment_target => {
                    let name_start = self.bump_index();
                    row_children.push(self.add_node(
                        SyntaxKind::NameExpr,
                        name_start,
                        self.position,
                        Vec::new(),
                    ));
                }
                _ => {
                    let before = self.position;
                    if let Some(element) = self.parse_expression(0, context.matrix()) {
                        row_children.push(element);
                    } else {
                        self.error_here("expected matrix or cell element", "OMP0121");
                        self.bump_index();
                    }
                    if self.position <= before {
                        self.bump_index();
                    }
                }
            }
        }
        self.add_node(expression_kind, start, self.position, rows)
    }

    fn bracket_is_assignment_target(&self, open: usize) -> bool {
        let mut depth = 0_usize;
        for (index, token) in self.tokens.iter().enumerate().skip(open) {
            match token.kind {
                TokenKind::LBracket => depth = depth.saturating_add(1),
                TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let equal = self.next_significant(index.saturating_add(1));
                        return !self.has_line_break(index.saturating_add(1), equal)
                            && self.tokens.get(equal).map(|token| token.kind)
                                == Some(TokenKind::Equal);
                    }
                }
                TokenKind::Eof => break,
                _ => {}
            }
        }
        false
    }

    fn looks_like_single_output(&self) -> bool {
        let first = self.peek_index();
        if self.tokens.get(first).map(|token| token.kind) != Some(TokenKind::Identifier) {
            return false;
        }
        let second = self.next_significant(first.saturating_add(1));
        self.tokens.get(second).map(|token| token.kind) == Some(TokenKind::Equal)
            && !self.has_line_break(first.saturating_add(1), second)
    }

    fn is_matrix_signed_separator(&self, operator_index: usize) -> bool {
        if !self.has_horizontal_space(self.position, operator_index) {
            return false;
        }
        let following = self.next_significant(operator_index.saturating_add(1));
        !self.has_horizontal_space(operator_index.saturating_add(1), following)
            && !self.has_line_break(operator_index.saturating_add(1), following)
    }

    fn node_range(&self, start: usize, end: usize) -> TextRange {
        if start < end {
            let first = self.tokens.get(start).map(|token| token.range.start());
            let last = self
                .tokens
                .get(end.saturating_sub(1))
                .map(|token| token.range.end());
            if let (Some(first), Some(last)) = (first, last) {
                return TextRange::new(first, last).unwrap_or(TextRange::empty(first));
            }
        }
        let offset = self
            .tokens
            .get(start)
            .or_else(|| self.tokens.last())
            .map_or(0, |token| token.range.start());
        TextRange::empty(offset)
    }

    fn add_node(
        &mut self,
        kind: SyntaxKind,
        start: usize,
        end: usize,
        children: Vec<NodeId>,
    ) -> NodeId {
        let range = self.node_range(start, end);
        let id = NodeId::new(self.nodes.len());
        self.nodes
            .push(SyntaxNode::new(kind, range, start..end, children));
        id
    }

    fn expect_end(&mut self, construct: &str) {
        if self.peek_kind() == TokenKind::KwEnd {
            self.bump_index();
            self.consume_simple_terminator();
        } else {
            self.error_here(format!("expected `end` for {construct}"), "OMP0200");
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str, code: &str) {
        if self.peek_kind() == kind {
            self.bump_index();
        } else {
            self.error_here(message, code);
        }
    }

    fn consume_header_terminator(&mut self) {
        let index = self.peek_index();
        if matches!(
            self.tokens.get(index).map(|token| token.kind),
            Some(TokenKind::Comma | TokenKind::Semicolon)
        ) {
            self.position = index + 1;
        }
    }

    fn consume_required_header_terminator(&mut self, construct: &str, code: &str) {
        let index = self.peek_index();
        if self.has_line_break(self.position, index) {
            return;
        }
        if matches!(
            self.tokens.get(index).map(|token| token.kind),
            Some(TokenKind::Comma | TokenKind::Semicolon)
        ) {
            self.position = index + 1;
            return;
        }
        self.error_at(
            index,
            format!("expected a line break, `,`, or `;` after {construct}"),
            code,
        );
    }

    fn consume_simple_terminator(&mut self) {
        let index = self.peek_index();
        if self.has_line_break(self.position, index) {
            return;
        }
        if matches!(
            self.tokens.get(index).map(|token| token.kind),
            Some(TokenKind::Comma | TokenKind::Semicolon)
        ) {
            self.position = index + 1;
        }
    }

    fn recover_to_line_end(&mut self) {
        while self.position < self.tokens.len() {
            match self.tokens[self.position].kind {
                TokenKind::Newline | TokenKind::Eof => break,
                TokenKind::Semicolon | TokenKind::Comma => {
                    self.position += 1;
                    break;
                }
                _ => self.position += 1,
            }
        }
    }

    fn recover_until(&mut self, kinds: &[TokenKind]) {
        while self.peek_kind() != TokenKind::Eof && !kinds.contains(&self.peek_kind()) {
            self.bump_index();
        }
    }

    fn skip_trivia(&mut self) {
        while self
            .tokens
            .get(self.position)
            .is_some_and(|token| token.kind.is_trivia())
        {
            self.position += 1;
        }
    }

    fn peek_kind(&self) -> TokenKind {
        self.tokens
            .get(self.peek_index())
            .map_or(TokenKind::Eof, |token| token.kind)
    }

    fn peek_text(&self) -> Option<&str> {
        self.tokens
            .get(self.peek_index())
            .map(|token| token.text.as_str())
    }

    fn peek_on_same_line(&self) -> TokenKind {
        let index = self.peek_index();
        if self.has_line_break(self.position, index) {
            TokenKind::Eof
        } else {
            self.tokens
                .get(index)
                .map_or(TokenKind::Eof, |token| token.kind)
        }
    }

    fn peek_index(&self) -> usize {
        self.next_significant(self.position)
    }

    fn next_significant(&self, mut index: usize) -> usize {
        while self
            .tokens
            .get(index)
            .is_some_and(|token| token.kind.is_trivia())
        {
            index += 1;
        }
        index.min(self.tokens.len().saturating_sub(1))
    }

    fn bump_index(&mut self) -> usize {
        let index = self.peek_index();
        self.position = index.saturating_add(1).min(self.tokens.len());
        index
    }

    fn has_line_break(&self, start: usize, end: usize) -> bool {
        let Some(tokens) = self.tokens.get(start..end) else {
            return false;
        };
        let mut continuation_pending = false;
        for token in tokens {
            match token.kind {
                TokenKind::Ellipsis => continuation_pending = true,
                TokenKind::Newline => {
                    if continuation_pending {
                        continuation_pending = false;
                    } else {
                        return true;
                    }
                }
                TokenKind::Comment => {
                    let line_breaks = physical_line_break_count(&token.text);
                    if line_breaks > usize::from(continuation_pending) {
                        return true;
                    }
                    if line_breaks > 0 {
                        continuation_pending = false;
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn has_horizontal_space(&self, start: usize, end: usize) -> bool {
        self.tokens.get(start..end).is_some_and(|tokens| {
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Whitespace)
        })
    }

    fn error_here(&mut self, message: impl Into<String>, code: &str) {
        let index = self.peek_index();
        self.error_at(index, message, code);
    }

    fn error_at(&mut self, index: usize, message: impl Into<String>, code: &str) {
        let range = self
            .tokens
            .get(index)
            .map_or(TextRange::empty(0), |token| token.range);
        self.diagnostics.push(
            Diagnostic::error(self.source_id, range, message.into()).with_code(code.to_owned()),
        );
    }
}

const fn is_field_name_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::KwBreak
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

fn physical_line_break_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                count += 1;
                index += usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1;
            }
            b'\n' => {
                count += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    count
}

fn toggled_command_quote_state(mut quoted: bool, text: &str) -> bool {
    for character in text.chars() {
        if character == '\'' {
            quoted = !quoted;
        }
    }
    quoted
}

fn is_unsupported_clear_option(text: &str) -> bool {
    matches!(
        text,
        "all" | "classes" | "functions" | "global" | "import" | "java" | "mex" | "variables"
    )
}

fn binary_binding_power(kind: TokenKind) -> Option<(u8, u8, SyntaxKind)> {
    let result = match kind {
        TokenKind::OrOr => (2, 3, SyntaxKind::BinaryExpr),
        TokenKind::AndAnd => (4, 5, SyntaxKind::BinaryExpr),
        TokenKind::Or => (6, 7, SyntaxKind::BinaryExpr),
        TokenKind::And => (8, 9, SyntaxKind::BinaryExpr),
        TokenKind::EqualEqual
        | TokenKind::NotEqual
        | TokenKind::Less
        | TokenKind::LessEqual
        | TokenKind::Greater
        | TokenKind::GreaterEqual => (10, 11, SyntaxKind::BinaryExpr),
        TokenKind::Colon => (12, 13, SyntaxKind::RangeExpr),
        TokenKind::Plus | TokenKind::Minus => (14, 15, SyntaxKind::BinaryExpr),
        TokenKind::Star
        | TokenKind::Slash
        | TokenKind::Backslash
        | TokenKind::DotStar
        | TokenKind::DotSlash
        | TokenKind::DotBackslash => (16, 17, SyntaxKind::BinaryExpr),
        TokenKind::Caret | TokenKind::DotCaret => (20, 21, SyntaxKind::BinaryExpr),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::{lex, parse};
    use openmat_source::SourceId;
    use openmat_syntax::{Cst, NodeId, SyntaxKind, TokenKind};

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(SourceId::new(1), source)
            .tokens
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    fn node_kinds(source: &str) -> Vec<SyntaxKind> {
        parse(SourceId::new(1), source)
            .syntax
            .nodes()
            .iter()
            .map(|node| node.kind)
            .collect()
    }

    fn binary_operator(tree: &Cst, id: NodeId) -> Option<TokenKind> {
        let node = tree.node(id)?;
        let left_end = tree.node(*node.children.first()?)?.token_range.end;
        let right_start = tree.node(*node.children.get(1)?)?.token_range.start;
        tree.tokens()[left_end..right_start]
            .iter()
            .find(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
    }

    #[test]
    fn parses_general_command_form_without_stealing_functional_or_bare_calls() {
        let source = "figure;\nhold on; % trailing comment\ngrid off\nbox off\nhold('on');\nvalue = left + right;\nstrcmp alpha ... continuation comment\n beta;\nstrcmp 'alpha beta' \"gamma delta\";\n";
        let result = parse(SourceId::new(41), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");
        let statements = root
            .children
            .iter()
            .map(|id| result.syntax.node(*id).expect("statement node").kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(
            statements,
            [
                SyntaxKind::ExprStmt,
                SyntaxKind::CommandStmt,
                SyntaxKind::CommandStmt,
                SyntaxKind::CommandStmt,
                SyntaxKind::ExprStmt,
                SyntaxKind::AssignmentStmt,
                SyntaxKind::CommandStmt,
                SyntaxKind::CommandStmt,
            ]
        );
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::ParenApplyExpr)
                .count(),
            1
        );
    }

    #[test]
    fn command_form_preserves_options_quotes_comments_continuations_and_suppression() {
        let source = "close all % trailing comment\nsave -v7.3 file.mat;\ndisp 'quoted argument' \"string argument\";\nstrcmp alpha ... continuation comment\n beta;\nsave ... option continuation\n -v7.3 continued.mat;\n";
        let result = parse(SourceId::new(43), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");
        let statements = root
            .children
            .iter()
            .map(|id| result.syntax.node(*id).expect("statement node").kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(statements, [SyntaxKind::CommandStmt; 5]);
        assert_eq!(
            result
                .syntax
                .tokens()
                .iter()
                .filter(|token| token.kind == TokenKind::Ellipsis)
                .count(),
            2
        );
        assert_eq!(
            result
                .syntax
                .tokens()
                .iter()
                .filter(|token| token.kind == TokenKind::Semicolon)
                .count(),
            4
        );
    }

    #[test]
    fn command_form_does_not_steal_spaced_binary_expressions() {
        let source = "a + b;\na - b;\na * b;\na / b;\na \\ b;\na ^ b;\na .* b;\na ./ b;\na .\\ b;\na .^ b;\na : b;\na == b;\na ~= b;\na < b;\na <= b;\na > b;\na >= b;\na & b;\na && b;\na | b;\na || b;\n";
        let result = parse(SourceId::new(44), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");
        let statements = root
            .children
            .iter()
            .map(|id| result.syntax.node(*id).expect("statement node").kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(statements.len(), 21);
        assert!(statements.iter().all(|kind| *kind == SyntaxKind::ExprStmt));
        assert!(
            !result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::CommandStmt)
        );
    }

    #[test]
    fn command_form_does_not_steal_postfix_index_or_member_expressions() {
        let source = "a';\na.';\na(1);\na{1};\na.field;\na.(field);\na. field;\n";
        let result = parse(SourceId::new(45), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");
        let statements = root
            .children
            .iter()
            .map(|id| result.syntax.node(*id).expect("statement node").kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(statements, [SyntaxKind::ExprStmt; 7]);
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::TransposeExpr)
                .count(),
            2
        );
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| matches!(
                    node.kind,
                    SyntaxKind::ParenApplyExpr | SyntaxKind::BraceApplyExpr
                ))
                .count(),
            2
        );
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| matches!(
                    node.kind,
                    SyntaxKind::FieldExpr | SyntaxKind::DynamicFieldExpr
                ))
                .count(),
            3
        );
    }

    #[test]
    fn attached_operator_text_remains_a_command_argument() {
        let source =
            "save -v7.3 file.mat;\ntool +flag;\ntool ==flag;\ntool .field;\ntool {literal};\n";
        let result = parse(SourceId::new(46), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");
        let statements = root
            .children
            .iter()
            .map(|id| result.syntax.node(*id).expect("statement node").kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(statements, [SyntaxKind::CommandStmt; 5]);
    }

    #[test]
    fn unterminated_command_quote_recovers_at_the_physical_line_boundary() {
        let source = "hold 'on\nvalue = 1;\n";
        let result = parse(SourceId::new(42), source);
        let root = result
            .syntax
            .node(result.syntax.root())
            .expect("root node must exist");

        assert_eq!(result.syntax.text(), source);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0004"))
        );
        assert_eq!(root.children.len(), 2);
        assert_eq!(
            result.syntax.node(root.children[0]).expect("command").kind,
            SyntaxKind::CommandStmt
        );
        assert_eq!(
            result
                .syntax
                .node(root.children[1])
                .expect("assignment")
                .kind,
            SyntaxKind::AssignmentStmt
        );
    }

    #[test]
    fn lexer_is_lossless_and_keeps_comments_and_newlines() {
        let source = "x = 1; % comment\r\ny = x + 2\n";
        let result = lex(SourceId::new(9), source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result
                .tokens
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            source
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Comment)
        );
        assert_eq!(
            result
                .tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Newline)
                .count(),
            2
        );
    }

    #[test]
    fn lexer_retains_both_index_and_range_colons() {
        let result = lex(SourceId::new(9), "selected = matrix(:, 1:2);\n");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result
                .tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Colon)
                .count(),
            2
        );
    }

    #[test]
    fn lexer_classifies_try_catch_end_keywords_losslessly() {
        let source = "try\r\ncatch issue % retained\r\nend\r\n";
        let result = lex(SourceId::new(9), source);
        let significant = result
            .tokens
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            significant,
            [
                TokenKind::KwTry,
                TokenKind::KwCatch,
                TokenKind::Identifier,
                TokenKind::KwEnd,
                TokenKind::Eof,
            ]
        );
        assert_eq!(
            result
                .tokens
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            source
        );
    }

    #[test]
    fn events_remains_an_identifier_outside_class_block_context() {
        let source = "function events = read_events(case_id)\n\
events = reshape(case_id, 1, []);\n\
end\n";
        let lexed = lex(SourceId::new(9), source);
        let parsed = parse(SourceId::new(9), source);

        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        assert_eq!(
            lexed
                .tokens
                .iter()
                .filter(|token| token.text == "events")
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            [TokenKind::Identifier, TokenKind::Identifier]
        );
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert_eq!(parsed.syntax.text(), source);
    }

    #[test]
    fn lexer_keeps_line_continuation_text_and_ranges() {
        let source = "x = 1 ... % retained comment\r\n    + 2;\n";
        let result = lex(SourceId::new(9), source);
        let continuation = result
            .tokens
            .iter()
            .find(|token| token.kind == TokenKind::Ellipsis)
            .expect("ellipsis token");
        let range = continuation.range;
        let continued_newline = result
            .tokens
            .iter()
            .find(|token| token.kind == TokenKind::Newline)
            .expect("continued newline token");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(continuation.text, "...");
        assert!(result.tokens.iter().any(|token| {
            token.kind == TokenKind::Comment && token.text == " % retained comment"
        }));
        assert_eq!(&source[range.start() as usize..range.end() as usize], "...");
        assert_eq!(continued_newline.text, "\r\n");
        assert_eq!(
            &source
                [continued_newline.range.start() as usize..continued_newline.range.end() as usize],
            "\r\n"
        );
        assert_eq!(
            result
                .tokens
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            source
        );
    }

    #[test]
    fn ellipsis_comments_out_the_remainder_of_its_physical_line() {
        let source = "x = 1 ... ignored + 2;\n    + 3;\n";
        let lexed = lex(SourceId::new(9), source);
        let parsed = parse(SourceId::new(9), source);
        let assignment = parsed
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::AssignmentStmt)
            .expect("continued assignment");

        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        assert!(
            lexed
                .tokens
                .iter()
                .any(|token| { token.kind == TokenKind::Comment && token.text == " ignored + 2;" })
        );
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert_eq!(parsed.syntax.text(), source);
        assert_eq!(
            binary_operator(&parsed.syntax, assignment.children[1]),
            Some(TokenKind::Plus)
        );
    }

    #[test]
    fn continuation_at_end_of_file_is_diagnosed() {
        let result = lex(SourceId::new(9), "x = 1 ...");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OML0005"))
        );
    }

    #[test]
    fn distinguishes_character_literals_from_postfix_transpose() {
        let token_kinds = kinds("a = 'it''s'; b = a'; c = a.';");
        assert!(token_kinds.contains(&TokenKind::CharLiteral));
        assert!(token_kinds.contains(&TokenKind::ConjugateTranspose));
        assert!(token_kinds.contains(&TokenKind::DotTranspose));
    }

    #[test]
    fn quote_context_resets_at_lines_and_matrix_whitespace() {
        let result = lex(SourceId::new(1), "a'\n'line'\nx = ['a' 'b' A'];\n");
        let token_kinds: Vec<_> = result.tokens.iter().map(|token| token.kind).collect();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            token_kinds
                .iter()
                .filter(|kind| **kind == TokenKind::ConjugateTranspose)
                .count(),
            2
        );
        assert_eq!(
            token_kinds
                .iter()
                .filter(|kind| **kind == TokenKind::CharLiteral)
                .count(),
            3
        );
    }

    #[test]
    fn paren_application_stays_unresolved_in_the_cst() {
        let result = parse(SourceId::new(1), "y = f(x, 2);\n");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::ParenApplyExpr)
        );
    }

    #[test]
    fn qualified_named_function_handle_stays_one_handle_expression() {
        let source = "f = @alpha.nested.inc;\n";
        let result = parse(SourceId::new(1), source);
        let handle = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::FunctionHandleExpr)
            .expect("qualified function-handle CST node");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(handle.children.len(), 3);
        assert!(
            !result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::FieldExpr)
        );
    }

    #[test]
    fn parses_anonymous_function_parameters_body_and_application_losslessly() {
        let source = "scale = 3; f = @(x, y) x + y * scale; result = f(1, 2);\n";
        let result = parse(SourceId::new(1), source);
        let anonymous = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::AnonymousFunctionExpr)
            .expect("anonymous-function CST node");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(anonymous.children.len(), 2);
        assert_eq!(
            result
                .syntax
                .node(anonymous.children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::ParameterList)
        );
        assert_eq!(
            result
                .syntax
                .node(anonymous.children[0])
                .unwrap()
                .children
                .len(),
            2
        );
        assert_eq!(
            result
                .syntax
                .node(anonymous.children[1])
                .map(|node| node.kind),
            Some(SyntaxKind::BinaryExpr)
        );
    }

    #[test]
    fn anonymous_function_recovery_has_stable_missing_and_duplicate_diagnostics() {
        let duplicate = parse(SourceId::new(1), "f = @(x, x) x;\n");
        assert!(
            duplicate
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0105"))
        );
        assert!(
            duplicate
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::AnonymousFunctionExpr)
        );
        assert_eq!(duplicate.syntax.text(), "f = @(x, x) x;\n");

        let duplicate_varargin = parse(SourceId::new(1), "f = @(varargin, varargin) varargin;\n");
        assert!(
            duplicate_varargin
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0105"))
        );

        let positioned_varargin = parse(
            SourceId::new(1),
            "fixed = @(varargin, tail) varargin; variadic = @(head, varargin) varargin;\n",
        );
        assert!(
            positioned_varargin.diagnostics.is_empty(),
            "{:?}",
            positioned_varargin.diagnostics
        );

        let missing_body = parse(SourceId::new(1), "f = @(x);\n");
        assert!(
            missing_body
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0104"))
        );
        let missing_paren = parse(SourceId::new(1), "f = @(x\n");
        assert!(
            missing_paren
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0109"))
        );
    }

    #[test]
    fn parses_property_read_write_and_unresolved_object_member_application() {
        let source = "read = obj.prop; obj.prop = 2; result = obj.method(3);\n";
        let result = parse(SourceId::new(1), source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::FieldExpr)
                .count(),
            3
        );
        assert!(result.syntax.nodes().iter().any(|node| {
            node.kind == SyntaxKind::ParenApplyExpr
                && result
                    .syntax
                    .node(node.children[0])
                    .is_some_and(|target| target.kind == SyntaxKind::FieldExpr)
        }));
    }

    #[test]
    fn distinguishes_prefix_cell_literals_from_lossless_postfix_brace_application() {
        let source = "empty_literal = {}; literal = {1, 2; 3, 4}; empty_contents = literal{}; value = literal { 1, end };\n";
        let result = parse(SourceId::new(1), source);
        let cell_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::CellExpr)
            .count();
        let brace_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::BraceApplyExpr)
            .count();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(cell_count, 2);
        assert_eq!(brace_count, 2);
        let mut brace_child_counts = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::BraceApplyExpr)
            .map(|node| node.children.len())
            .collect::<Vec<_>>();
        brace_child_counts.sort_unstable();
        assert_eq!(brace_child_counts, vec![1, 3]);
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::EndIndexExpr)
                .count(),
            1
        );
    }

    #[test]
    fn postfix_chain_nests_in_source_order_with_static_and_dynamic_fields() {
        let source = "out = a(1){2}.f.(name);\n";
        let result = parse(SourceId::new(1), source);
        let dynamic = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::DynamicFieldExpr)
            .expect("dynamic field");
        let static_field = result
            .syntax
            .node(dynamic.children[0])
            .expect("static field target");
        let brace = result
            .syntax
            .node(static_field.children[0])
            .expect("brace target");
        let paren = result.syntax.node(brace.children[0]).expect("paren target");
        let chain_text = result
            .syntax
            .tokens()
            .get(dynamic.token_range.clone())
            .expect("dynamic field tokens")
            .iter()
            .map(|token| token.text.as_str())
            .collect::<String>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(dynamic.children.len(), 2);
        assert_eq!(static_field.kind, SyntaxKind::FieldExpr);
        assert_eq!(brace.kind, SyntaxKind::BraceApplyExpr);
        assert_eq!(paren.kind, SyntaxKind::ParenApplyExpr);
        assert_eq!(chain_text, "a(1){2}.f.(name)");
        assert_eq!(
            result
                .syntax
                .node(static_field.children[1])
                .map(|node| node.range),
            Some(openmat_source::TextRange::new(14, 15).expect("field span"))
        );
    }

    #[test]
    fn assignment_targets_preserve_paren_brace_and_chained_expressions() {
        let source = "C{end} = value; D(1){2}.f = other; P(end) = rhs; S.(field) = dynamic;\n";
        let result = parse(SourceId::new(1), source);
        let assignments = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::AssignmentStmt)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(assignments.len(), 4);
        assert_eq!(
            result
                .syntax
                .node(assignments[0].children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::BraceApplyExpr)
        );
        assert_eq!(
            result
                .syntax
                .node(assignments[1].children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::FieldExpr)
        );
        assert_eq!(
            result
                .syntax
                .node(assignments[2].children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::ParenApplyExpr)
        );
        assert_eq!(
            result
                .syntax
                .node(assignments[3].children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::DynamicFieldExpr)
        );
    }

    #[test]
    fn bracketed_assignment_accepts_discarded_outputs_without_weakening_rhs_recovery() {
        let source = "[warning_text, ~] = lastwarn; following = 1;\n";
        let result = parse(SourceId::new(1), source);
        let assignments = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::AssignmentStmt)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(assignments.len(), 2);
        let target = result
            .syntax
            .node(assignments[0].children[0])
            .expect("matrix target");
        let row = result
            .syntax
            .node(target.children[0])
            .expect("matrix target row");
        let discard = row.children[1];
        assert_eq!(
            result.syntax.node(discard).map(|node| node.kind),
            Some(SyntaxKind::NameExpr)
        );
        assert_eq!(result.syntax.node_tokens(discard)[0].text, "~");

        let invalid_rhs = parse(SourceId::new(1), "value = [1, ~]; following = 2;\n");
        assert!(
            invalid_rhs
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0100"))
        );
        assert_eq!(
            invalid_rhs.syntax.text(),
            "value = [1, ~]; following = 2;\n"
        );
        assert_eq!(
            invalid_rhs
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::AssignmentStmt)
                .count(),
            2
        );
    }

    #[test]
    fn end_and_colon_bind_only_inside_nested_apply_arguments() {
        let source = "outer = A(1:end-1, B(:, end), [end 2]); plain = end; literal = {end}; dynamic = obj.(end); contents = C{end, :};\n";
        let result = parse(SourceId::new(1), source);
        let end_index_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::EndIndexExpr)
            .count();
        let ordinary_end_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| {
                node.kind == SyntaxKind::NameExpr
                    && result
                        .syntax
                        .tokens()
                        .get(node.token_range.clone())
                        .expect("name tokens")
                        .iter()
                        .any(|token| token.kind == TokenKind::KwEnd)
            })
            .count();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(end_index_count, 4);
        assert_eq!(ordinary_end_count, 3);
        assert_eq!(
            result
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::ColonExpr)
                .count(),
            2
        );
    }

    #[test]
    fn missing_postfix_delimiters_recover_to_lossless_nodes() {
        for (source, expected_kind, expected_code) in [
            ("value = C{1, 2", SyntaxKind::BraceApplyExpr, "OMP0119"),
            (
                "value = obj.(field",
                SyntaxKind::DynamicFieldExpr,
                "OMP0123",
            ),
            ("value = obj.()", SyntaxKind::DynamicFieldExpr, "OMP0122"),
        ] {
            let result = parse(SourceId::new(1), source);

            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .syntax
                    .nodes()
                    .iter()
                    .any(|node| node.kind == expected_kind),
                "missing recovered {expected_kind:?} for {source:?}"
            );
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_deref() == Some(expected_code)),
                "missing {expected_code} for {source:?}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn standalone_colon_is_an_application_argument_not_a_range() {
        let result = parse(SourceId::new(1), "selected = matrix(:, 2);\n");
        let application = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::ParenApplyExpr)
            .expect("parenthesized application");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(application.children.len(), 3);
        assert_eq!(
            result
                .syntax
                .node(application.children[1])
                .map(|node| node.kind),
            Some(SyntaxKind::ColonExpr)
        );
        assert_eq!(
            result
                .syntax
                .node(application.children[2])
                .map(|node| node.kind),
            Some(SyntaxKind::NumberExpr)
        );
    }

    #[test]
    fn ordinary_ranges_do_not_become_standalone_colons() {
        let result = parse(SourceId::new(1), "a = 1:2; b = 1:2:9;\n");
        let range_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::RangeExpr)
            .count();
        let colon_count = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::ColonExpr)
            .count();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(range_count, 3);
        assert_eq!(colon_count, 0);
    }

    #[test]
    fn pratt_parser_respects_power_unary_and_product_precedence() {
        let result = parse(SourceId::new(1), "x = -2^2 + 3*4;\n");
        let assignment = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::AssignmentStmt)
            .expect("assignment node");
        let top = result
            .syntax
            .node(assignment.children[1])
            .expect("assignment value");
        let left = result
            .syntax
            .node(top.children[0])
            .expect("addition left operand");
        let right = result
            .syntax
            .node(top.children[1])
            .expect("addition right operand");
        let powered = result.syntax.node(left.children[0]).expect("unary operand");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(top.kind, SyntaxKind::BinaryExpr);
        assert_eq!(left.kind, SyntaxKind::UnaryExpr);
        assert_eq!(powered.kind, SyntaxKind::BinaryExpr);
        assert_eq!(right.kind, SyntaxKind::BinaryExpr);
    }

    #[test]
    fn power_operators_are_left_associative_with_unary_and_transpose_precedence() {
        let result = parse(
            SourceId::new(1),
            "a = 2^3^2; b = 2.^3.^2; c = -2^3^2; d = A'^2;\n",
        );
        let assignments: Vec<_> = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::AssignmentStmt)
            .collect();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(assignments.len(), 4);
        for (assignment, operator) in assignments[..2]
            .iter()
            .zip([TokenKind::Caret, TokenKind::DotCaret])
        {
            let outer_id = assignment.children[1];
            let outer = result.syntax.node(outer_id).expect("outer power");
            let inner_id = outer.children[0];
            assert_eq!(outer.kind, SyntaxKind::BinaryExpr);
            assert_eq!(binary_operator(&result.syntax, outer_id), Some(operator));
            assert_eq!(
                result.syntax.node(inner_id).map(|node| node.kind),
                Some(SyntaxKind::BinaryExpr)
            );
            assert_eq!(binary_operator(&result.syntax, inner_id), Some(operator));
        }

        let unary = result
            .syntax
            .node(assignments[2].children[1])
            .expect("unary expression");
        let unary_power = result
            .syntax
            .node(unary.children[0])
            .expect("power beneath unary minus");
        assert_eq!(unary.kind, SyntaxKind::UnaryExpr);
        assert_eq!(unary_power.kind, SyntaxKind::BinaryExpr);
        assert_eq!(
            result
                .syntax
                .node(unary_power.children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::BinaryExpr)
        );

        let transpose_power_id = assignments[3].children[1];
        let transpose_power = result
            .syntax
            .node(transpose_power_id)
            .expect("power above transpose");
        assert_eq!(
            binary_operator(&result.syntax, transpose_power_id),
            Some(TokenKind::Caret)
        );
        assert_eq!(
            result
                .syntax
                .node(transpose_power.children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::TransposeExpr)
        );
    }

    #[test]
    fn conformance_short_circuit_matrix_continuation_parses_losslessly() {
        let source =
            include_str!("../../../tests/conformance/cases/programs/logical_short_circuit.m");
        let result = parse(SourceId::new(20), source);
        let matrix = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::MatrixExpr)
            .expect("matrix expression");
        let row = result
            .syntax
            .node(matrix.children[0])
            .expect("continued matrix row");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(matrix.children.len(), 1);
        assert_eq!(row.kind, SyntaxKind::MatrixRow);
        assert_eq!(row.children.len(), 4);
        assert_eq!(
            result
                .syntax
                .tokens()
                .iter()
                .filter(|token| token.kind == TokenKind::Ellipsis)
                .count(),
            1
        );
        assert_eq!(
            result
                .syntax
                .tokens()
                .iter()
                .filter(|token| token.kind == TokenKind::AndAnd)
                .count(),
            2
        );
        assert_eq!(
            result
                .syntax
                .tokens()
                .iter()
                .filter(|token| token.kind == TokenKind::OrOr)
                .count(),
            2
        );
    }

    #[test]
    fn short_circuit_operators_parse_on_both_sides_of_continuation() {
        let source = "a = true && ...\n    false; b = false ...\n    || true;\n";
        let result = parse(SourceId::new(21), source);
        let assignments: Vec<_> = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::AssignmentStmt)
            .collect();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(assignments.len(), 2);
        assert_eq!(
            binary_operator(&result.syntax, assignments[0].children[1]),
            Some(TokenKind::AndAnd)
        );
        assert_eq!(
            binary_operator(&result.syntax, assignments[1].children[1]),
            Some(TokenKind::OrOr)
        );
    }

    #[test]
    fn matrix_whitespace_creates_elements_and_preserves_signed_separator() {
        let result = parse(SourceId::new(1), "a = [1 2; 3 -4];\n");
        let rows: Vec<_> = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::MatrixRow)
            .collect();
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].children.len(), 2);
        assert_eq!(rows[1].children.len(), 2);
        assert_eq!(result.syntax.text(), "a = [1 2; 3 -4];\n");
    }

    #[test]
    fn parses_control_flow_function_and_regular_program() {
        let source = "function y = sum_to(n)\ny = 0;\nfor i = 1:n\nif i > 0\ny = y + i;\nelse\nbreak\nend\nend\nend\n";
        let result = parse(SourceId::new(2), source);
        let syntax_kinds = node_kinds(source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(syntax_kinds.contains(&SyntaxKind::FunctionDef));
        assert!(syntax_kinds.contains(&SyntaxKind::ForStmt));
        assert!(syntax_kinds.contains(&SyntaxKind::IfStmt));
        assert!(syntax_kinds.contains(&SyntaxKind::AssignmentStmt));
    }

    #[test]
    fn parses_no_catch_plain_bound_and_nested_try_forms_losslessly() {
        let source = "try\nvalue = 1;\nend\ntry; catch; end\ntry, before = 1; catch issue, after = 2; end\ntry % outer\ntry; catch inner; end\ncatch outer\nend\n";
        let result = parse(SourceId::new(25), source);
        let tries = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::TryStmt)
            .collect::<Vec<_>>();
        let catches = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::CatchClause)
            .collect::<Vec<_>>();
        let no_catch = tries
            .iter()
            .copied()
            .find(|node| node.range.start() == 0)
            .expect("no-catch try");
        let outer = tries
            .iter()
            .copied()
            .max_by_key(|node| node.range.end() - node.range.start())
            .expect("outer try");
        let outer_body = result
            .syntax
            .node(outer.children[0])
            .expect("outer try body");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(tries.len(), 5);
        assert_eq!(catches.len(), 4);
        assert_eq!(no_catch.children.len(), 1);
        assert_eq!(
            result
                .syntax
                .node(no_catch.children[0])
                .map(|node| node.kind),
            Some(SyntaxKind::Block)
        );
        assert!(catches.iter().any(|clause| {
            clause.children.len() == 1
                && result
                    .syntax
                    .node(clause.children[0])
                    .is_some_and(|node| node.kind == SyntaxKind::Block)
        }));
        assert_eq!(
            catches
                .iter()
                .filter(|clause| {
                    clause.children.first().is_some_and(|child| {
                        result
                            .syntax
                            .node(*child)
                            .is_some_and(|node| node.kind == SyntaxKind::NameExpr)
                    })
                })
                .count(),
            3
        );
        assert!(outer_body.children.iter().any(|child| {
            result
                .syntax
                .node(*child)
                .is_some_and(|node| node.kind == SyntaxKind::TryStmt)
        }));
    }

    #[test]
    fn try_headers_accept_line_comments_commas_and_semicolons() {
        let cases = [
            "try, value = 1; catch, value = 2; end\n",
            "try; value = 1; catch issue; value = 2; end\n",
            "try % try note\r\nvalue = 1;\r\ncatch issue % catch note\r\nvalue = 2;\r\nend\r\n",
            "try; catch; end\n",
        ];

        for source in cases {
            let result = parse(SourceId::new(26), source);
            assert!(
                result.diagnostics.is_empty(),
                "unexpected diagnostics for {source:?}: {:?}",
                result.diagnostics
            );
            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .syntax
                    .nodes()
                    .iter()
                    .any(|node| node.kind == SyntaxKind::TryStmt)
            );
        }
    }

    #[test]
    fn malformed_try_forms_recover_losslessly_with_stable_diagnostics() {
        let cases = [
            (
                "try\ncatch first\nvalue = 1;\ncatch second\nvalue = 2;\nend\nafter = 3;\n",
                "OMP0036",
            ),
            ("catch issue\nafter = 1;\nend\n", "OMP0037"),
            ("try value = 1; catch; end\n", "OMP0038"),
            ("try; catch issue value = 1; end\n", "OMP0038"),
            ("try\ncatch\nvalue = 1;\n", "OMP0200"),
            ("end\nafter = 1;\n", "OMP0002"),
        ];

        for (source, expected_code) in cases {
            let result = parse(SourceId::new(27), source);
            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_deref() == Some(expected_code)),
                "missing {expected_code} in {:?}",
                result.diagnostics
            );
            assert_eq!(
                result
                    .syntax
                    .node(result.syntax.root())
                    .map(|node| node.kind),
                Some(SyntaxKind::Root)
            );
        }

        let duplicate = parse(SourceId::new(27), "try\ncatch first\ncatch second\nend\n");
        assert_eq!(
            duplicate
                .syntax
                .nodes()
                .iter()
                .filter(|node| node.kind == SyntaxKind::CatchClause)
                .count(),
            2
        );
    }

    #[test]
    fn parses_nested_switch_clauses_without_leaking_terminators() {
        let source = "switch key\ncase 1\nif flag\nfor i = 1:2\nwhile i < 2\nswitch inner\ncase 2\nvalue = i;\notherwise\nvalue = 0;\nend\nend\nend\nelse\nvalue = -1;\nend\ncase {3, 4}\nvalue = 3;\notherwise\nvalue = 4;\nend\n";
        let result = parse(SourceId::new(22), source);
        let switches = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::SwitchStmt)
            .collect::<Vec<_>>();
        let outer = switches
            .iter()
            .copied()
            .max_by_key(|node| node.range.end() - node.range.start())
            .expect("outer switch");
        let outer_children = outer
            .children
            .iter()
            .map(|child| result.syntax.node(*child).expect("switch child").kind)
            .collect::<Vec<_>>();
        let syntax_kinds = result
            .syntax
            .nodes()
            .iter()
            .map(|node| node.kind)
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(switches.len(), 2);
        assert_eq!(
            outer_children,
            vec![
                SyntaxKind::NameExpr,
                SyntaxKind::CaseClause,
                SyntaxKind::CaseClause,
                SyntaxKind::OtherwiseClause,
            ]
        );
        assert_eq!(
            syntax_kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::CaseClause)
                .count(),
            3
        );
        assert_eq!(
            syntax_kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::OtherwiseClause)
                .count(),
            2
        );
        assert!(syntax_kinds.contains(&SyntaxKind::IfStmt));
        assert!(syntax_kinds.contains(&SyntaxKind::ForStmt));
        assert!(syntax_kinds.contains(&SyntaxKind::WhileStmt));
    }

    #[test]
    fn parses_switch_with_no_clauses() {
        let source = "switch value\nend\n";
        let result = parse(SourceId::new(23), source);
        let switch = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::SwitchStmt)
            .expect("switch statement");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(switch.children.len(), 1);
        assert_eq!(result.syntax.text(), source);
    }

    #[test]
    fn malformed_switches_return_lossless_trees_and_stable_diagnostics() {
        let cases = [
            ("switch\ncase 1\nend\n", "OMP0031"),
            ("switch =\ncase 1\nend\n", "OMP0031"),
            ("switch (value\ncase 1\nend\n", "OMP0103"),
            ("switch value\ncase\nanswer = 1;\nend\n", "OMP0032"),
            ("switch value\ncase (1\nend\n", "OMP0103"),
            ("switch value\notherwise\notherwise\nend\n", "OMP0033"),
            ("switch value\notherwise\ncase 1\nend\n", "OMP0034"),
            ("switch value\ncase 1\nanswer = 1;\n", "OMP0200"),
        ];

        for (source, expected_code) in cases {
            let result = parse(SourceId::new(24), source);
            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_deref() == Some(expected_code)),
                "missing {expected_code} in {:?}",
                result.diagnostics
            );
            assert_eq!(
                result
                    .syntax
                    .node(result.syntax.root())
                    .map(|node| node.kind),
                Some(SyntaxKind::Root)
            );
            assert!(
                result
                    .syntax
                    .nodes()
                    .iter()
                    .any(|node| node.kind == SyntaxKind::SwitchStmt)
            );
            if expected_code == "OMP0033" {
                assert_eq!(
                    result
                        .syntax
                        .nodes()
                        .iter()
                        .filter(|node| node.kind == SyntaxKind::OtherwiseClause)
                        .count(),
                    2
                );
            }
            if expected_code == "OMP0034" {
                let switch = result
                    .syntax
                    .nodes()
                    .iter()
                    .find(|node| node.kind == SyntaxKind::SwitchStmt)
                    .expect("switch statement");
                let child_kinds = switch
                    .children
                    .iter()
                    .map(|child| result.syntax.node(*child).expect("switch child").kind)
                    .collect::<Vec<_>>();
                assert_eq!(
                    child_kinds,
                    vec![
                        SyntaxKind::NameExpr,
                        SyntaxKind::OtherwiseClause,
                        SyntaxKind::CaseClause,
                    ]
                );
            }
        }
    }

    #[test]
    fn parses_clear_identifier_lists_losslessly() {
        let source = "clear first second\nkept = 1;\n";
        let result = parse(SourceId::new(2), source);
        let clear = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::ClearStmt)
            .expect("clear statement");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(clear.children.len(), 2);
        assert!(clear.children.iter().all(|child| {
            result
                .syntax
                .node(*child)
                .is_some_and(|node| node.kind == SyntaxKind::NameExpr)
        }));
        assert_eq!(result.syntax.text(), source);
    }

    #[test]
    fn parses_global_and_persistent_declarations_losslessly() {
        let source = "global first second ... % retained\r\n    third, next = 1;\nfunction f()\n persistent cache state;\nend\n";
        let result = parse(SourceId::new(31), source);
        let global = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::GlobalStmt)
            .expect("global declaration");
        let persistent = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::PersistentStmt)
            .expect("persistent declaration");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(global.children.len(), 3);
        assert_eq!(persistent.children.len(), 2);
        assert!(global.children.iter().all(|child| {
            result
                .syntax
                .node(*child)
                .is_some_and(|node| node.kind == SyntaxKind::NameExpr)
        }));
        let global_text = result
            .syntax
            .node_tokens(NodeId::new(
                result
                    .syntax
                    .nodes()
                    .iter()
                    .position(|node| std::ptr::eq(node, global))
                    .expect("global node index"),
            ))
            .iter()
            .map(|token| token.text.as_str())
            .collect::<String>();
        assert!(global_text.contains("... % retained\r\n"));
        assert!(global_text.ends_with("third,"));
    }

    #[test]
    fn malformed_declarations_recover_names_and_following_statements() {
        let source = "global\npersistent p(1) good\npersistent q q\nkept = 1;\n";
        let result = parse(SourceId::new(32), source);
        let codes = result
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.code.as_deref())
            .collect::<Vec<_>>();
        let declaration_kinds = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    SyntaxKind::GlobalStmt | SyntaxKind::PersistentStmt
                )
            })
            .map(|node| node.kind)
            .collect::<Vec<_>>();

        assert_eq!(result.syntax.text(), source);
        assert!(codes.contains(&"OMP0007"));
        assert!(codes.contains(&"OMP0008"));
        assert!(codes.contains(&"OMP0009"));
        assert_eq!(declaration_kinds.len(), 3);
        assert!(node_kinds(source).contains(&SyntaxKind::AssignmentStmt));
    }

    #[test]
    fn nested_class_blocks_in_methods_recover_without_stalling() {
        for keyword in ["properties", "methods", "enumeration"] {
            let source = format!("classdef Broken\nmethods\n{keyword}\n member\nend\nend\nend\n");
            let result = parse(SourceId::new(33), &source);
            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0070"))
            );
            assert!(result.diagnostics.len() < 20);
        }
    }

    #[test]
    fn declarations_in_class_body_remain_explicit_recovery_nodes() {
        let source = "classdef Example\n global shared\n persistent cached\nend\n";
        let result = parse(SourceId::new(33), source);

        assert_eq!(result.syntax.text(), source);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0053"))
        );
        assert!(node_kinds(source).contains(&SyntaxKind::GlobalStmt));
        assert!(node_kinds(source).contains(&SyntaxKind::PersistentStmt));
    }

    #[test]
    fn clear_unsupported_forms_recover_with_stable_diagnostics() {
        let cases = [
            ("clear all\nnext = 1;\n", "OMP0006"),
            ("clear item*\nnext = 1;\n", "OMP0006"),
            ("clear('item')\nnext = 1;\n", "OMP0006"),
        ];
        for (source, code) in cases {
            let result = parse(SourceId::new(2), source);
            assert_eq!(
                result
                    .diagnostics
                    .first()
                    .and_then(|item| item.code.as_deref()),
                Some(code)
            );
            assert_eq!(result.syntax.text(), source);
            assert!(
                result
                    .syntax
                    .nodes()
                    .iter()
                    .any(|node| node.kind == SyntaxKind::AssignmentStmt)
            );
        }
    }

    #[test]
    fn parses_clear_without_names_as_a_complete_statement() {
        let source = "clear\nnext = 1;\n";
        let result = parse(SourceId::new(2), source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        let clear = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::ClearStmt)
            .expect("clear statement");
        assert!(clear.children.is_empty());
    }

    #[test]
    fn parses_clear_import_as_a_dedicated_clear_statement() {
        let source = "clear import;\nnext = 1;\n";
        let result = parse(SourceId::new(2), source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let clear = result
            .syntax
            .nodes()
            .iter()
            .find(|node| node.kind == SyntaxKind::ClearStmt)
            .expect("clear import statement");
        assert_eq!(clear.children.len(), 1);
        assert_eq!(result.syntax.text(), source);
    }

    #[test]
    fn function_outputs_accept_comma_or_matrix_space_separators() {
        let comma = parse(
            SourceId::new(2),
            "function [sum, product] = pair(a, b)\nsum = a+b;\nproduct = a*b;\nend\n",
        );
        let space = parse(
            SourceId::new(2),
            "function [sum product] = pair(a, b)\nsum = a+b;\nproduct = a*b;\nend\n",
        );

        assert!(comma.diagnostics.is_empty(), "{:?}", comma.diagnostics);
        assert!(space.diagnostics.is_empty(), "{:?}", space.diagnostics);
        let output_count = |result: &super::ParseResult| {
            result
                .syntax
                .nodes()
                .iter()
                .find(|node| node.kind == SyntaxKind::OutputList)
                .map_or(0, |node| node.children.len())
        };
        assert_eq!(output_count(&comma), 2);
        assert_eq!(output_count(&space), 2);
    }

    #[test]
    fn parses_core_classdef_skeleton_and_attributes() {
        let source = "classdef (Sealed) Counter < handle\nproperties (Access = private)\nvalue = 0\nend\nmethods (Static)\nfunction y = make(x)\ny = x;\nend\nend\nend\n";
        let result = parse(SourceId::new(3), source);
        let syntax_kinds: Vec<_> = result.syntax.nodes().iter().map(|node| node.kind).collect();
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(syntax_kinds.contains(&SyntaxKind::ClassDef));
        assert!(syntax_kinds.contains(&SyntaxKind::PropertiesBlock));
        assert!(syntax_kinds.contains(&SyntaxKind::MethodsBlock));
        assert!(syntax_kinds.contains(&SyntaxKind::PropertyDecl));
        assert_eq!(
            syntax_kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::AttributeList)
                .count(),
            3
        );
    }

    #[test]
    fn parses_abstract_method_signatures_losslessly_without_function_bodies() {
        let source = "classdef (Abstract, Sealed) Contract\nmethods (Abstract, Static, Access = protected)\n[left, right] = pair(value)\ntouch(value)\nend\nend\n";
        let result = parse(SourceId::new(34), source);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        let declarations = result
            .syntax
            .nodes()
            .iter()
            .filter(|node| node.kind == SyntaxKind::MethodDecl)
            .count();
        assert_eq!(declarations, 2);
        assert!(
            result
                .syntax
                .nodes()
                .iter()
                .all(|node| node.kind != SyntaxKind::FunctionDef)
        );
    }

    #[test]
    fn parses_enumeration_members_and_events_blocks_losslessly() {
        let source = "classdef Traffic < uint8\nenumeration\nStop (0)\nReady(1 + 1, 'amber')\nGo\nend\nevents (ListenAccess = protected, NotifyAccess = private)\nChanged, Reset\nend\nend\n";
        let result = parse(SourceId::new(36), source);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        let kinds = result
            .syntax
            .nodes()
            .iter()
            .map(|node| node.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::EnumMemberDecl)
                .count(),
            3
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::EventDecl)
                .count(),
            2
        );
        assert!(kinds.contains(&SyntaxKind::EnumerationBlock));
        assert!(kinds.contains(&SyntaxKind::EventsBlock));
    }

    #[test]
    fn malformed_enumeration_and_event_members_recover_losslessly() {
        let source =
            "classdef Broken\nenumeration\nBad(1, )\n42\nend\nevents\nChanged = 1\nend\nend\n";
        let result = parse(SourceId::new(37), source);

        assert_eq!(result.syntax.text(), source);
        assert!(!result.diagnostics.is_empty());
        assert!(
            result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::EnumerationBlock)
        );
        assert!(
            result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::EventsBlock)
        );
    }

    #[test]
    fn malformed_abstract_signature_keeps_a_lossless_error_tree() {
        let source =
            "classdef (Abstract) Broken\nmethods (Abstract)\n[value] = (input)\nend\nend\n";
        let result = parse(SourceId::new(35), source);

        assert_eq!(result.syntax.text(), source);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_deref() == Some("OMP0072"))
        );
        assert!(
            result
                .syntax
                .nodes()
                .iter()
                .any(|node| node.kind == SyntaxKind::MethodDecl)
        );
    }

    #[test]
    fn parses_static_constant_class_and_keeps_class_member_apply_unresolved() {
        let source = "classdef Scale\nproperties (Constant)\nFactor = 3\nend\nmethods (Static)\nfunction result = scale(value)\nresult = Scale.Factor * value;\nend\nend\nend\nout = Scale.scale(5);\n";
        let result = parse(SourceId::new(3), source);
        let attributes = result
            .syntax
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind == SyntaxKind::Attribute)
            .map(|(index, _)| {
                result
                    .syntax
                    .node_tokens(NodeId::new(index))
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.syntax.text(), source);
        assert_eq!(attributes, ["Constant", "Static"]);
        assert!(result.syntax.nodes().iter().any(|node| {
            node.kind == SyntaxKind::ParenApplyExpr
                && result
                    .syntax
                    .node(node.children[0])
                    .is_some_and(|target| target.kind == SyntaxKind::FieldExpr)
        }));
    }

    #[test]
    fn parses_dotted_dependent_accessor_method_names_without_recovery() {
        let source = "classdef Box\nproperties (Dependent)\nTwice\nend\nmethods\nfunction value = get.Twice(obj)\nvalue = 1;\nend\nfunction obj = set.Twice(obj, value)\nend\nend\nend\n";
        let result = parse(SourceId::new(3), source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let names = result
            .syntax
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind == SyntaxKind::NameExpr)
            .map(|(index, _)| {
                result
                    .syntax
                    .node_tokens(openmat_syntax::NodeId::new(index))
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(names.contains(&"get.Twice".to_owned()));
        assert!(names.contains(&"set.Twice".to_owned()));
        assert_eq!(
            names
                .iter()
                .filter(|name| matches!(name.as_str(), "get.Twice" | "set.Twice"))
                .count(),
            2
        );
        assert_eq!(result.syntax.text(), source);
    }

    #[test]
    fn parses_qualified_superclasses_and_recovers_malformed_class_headers() {
        for source in [
            "classdef Widget < matlab.ui.componentcontainer.ComponentContainer\nend\n",
            "classdef Widget < tools.Base\nmethods\nfunction obj = Widget()\nobj@tools.Base();\nend\nend\nend\n",
        ] {
            let parsed = parse(SourceId::new(1), source);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            assert_eq!(parsed.syntax.text(), source);
        }
        let malformed = parse(SourceId::new(1), "classdef Widget < tools.\n)\nend\n");
        assert!(!malformed.diagnostics.is_empty());
    }

    #[test]
    fn parses_explicit_superclass_constructor_call_losslessly() {
        let source = "classdef Child < Base\nmethods\nfunction object = Child(value)\nobject@Base(value);\nend\nend\nend\n";
        let result = parse(SourceId::new(3), source);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let call = result
            .syntax
            .nodes()
            .iter()
            .enumerate()
            .find(|(_, node)| node.kind == SyntaxKind::SuperclassConstructorCallExpr)
            .map(|(index, _)| openmat_syntax::NodeId::new(index))
            .expect("superclass constructor CST node");
        assert_eq!(
            result
                .syntax
                .node_tokens(call)
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            "object@Base(value)"
        );
        assert_eq!(result.syntax.text(), source);
    }

    #[test]
    fn malformed_input_returns_a_tree_and_diagnostics() {
        let source = "if (x +\ny = [1, @];\n";
        let result = parse(SourceId::new(4), source);
        assert_eq!(result.syntax.text(), source);
        assert!(!result.diagnostics.is_empty());
        assert_eq!(
            result
                .syntax
                .node(result.syntax.root())
                .map(|node| node.kind),
            Some(SyntaxKind::Root)
        );
    }
}
