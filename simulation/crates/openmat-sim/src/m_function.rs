//! Pure, fixed-column m functions lowered through `OpenMat`'s existing HIR.
use std::collections::{BTreeMap, BTreeSet};

use openmat_hir::{BinaryOp, Expr, ExprKind, StmtKind, UnaryOp};
use openmat_source::{SourceId, TextRange};

use crate::ModelError;
use crate::model::{Block, BlockKind};
use crate::numeric::{Instruction, MAX_VALUES, MathFunction};

pub type SourceBundle = BTreeMap<String, String>;
pub const MAX_SOURCE_BYTES: usize = 65_536;
const MAX_WIDTH: usize = 4096;

pub(crate) fn validate_bundle(sources: &SourceBundle) -> Result<(), ModelError> {
    if sources.len() > 64
        || sources.values().map(String::len).sum::<usize>() > 1_048_576
        || sources
            .iter()
            .any(|(path, text)| !valid_path(path) || text.len() > MAX_SOURCE_BYTES)
    {
        return Err(ModelError::new(
            "source_bundle",
            "source bundle requires relative .m paths, at most 64 files, 64 KiB per file and 1 MiB total",
        ));
    }
    Ok(())
}

#[allow(clippy::case_sensitive_file_extension_comparisons)] // Source bundle paths are portable and case-sensitive by contract.
fn valid_path(path: &str) -> bool {
    path.len() <= 512
        && path.ends_with(".m")
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn identifier(name: &str) -> bool {
    name.len() <= 63
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

pub(crate) fn validate_block(block: &Block) -> Result<(), ModelError> {
    let BlockKind::MFunction {
        source,
        entry,
        inputs,
        parameters,
        output_width,
    } = &block.kind
    else {
        return Ok(());
    };
    let mut names = BTreeSet::new();
    if !valid_path(source)
        || !identifier(entry)
        || !(1..=MAX_WIDTH).contains(output_width)
        || inputs.len() + parameters.len() > 64
        || inputs.iter().any(|p| {
            !identifier(&p.name) || !names.insert(&p.name) || !(1..=MAX_WIDTH).contains(&p.width)
        })
        || parameters.iter().any(|p| {
            !identifier(&p.name)
                || !names.insert(&p.name)
                || p.value.is_empty()
                || p.value.len() > MAX_WIDTH
                || p.value.iter().any(|v| !v.is_finite())
        })
    {
        return Err(ModelError::new("function_signature", "M Function requires a relative .m source, valid distinct argument names, finite parameters and widths 1..4096 (at most 64 arguments)").at(&block.id, None));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Validate the signature and lower its straight-line body in source order.
pub(crate) fn lower(
    block: &Block,
    signals: &[&[usize]],
    sources: &SourceBundle,
    instructions: &mut Vec<Instruction>,
) -> Result<Vec<usize>, ModelError> {
    let BlockKind::MFunction {
        source,
        entry,
        inputs,
        parameters,
        output_width,
    } = &block.kind
    else {
        unreachable!()
    };
    let text = sources.get(source).ok_or_else(|| {
        ModelError::new(
            "source_missing",
            format!("missing source snapshot: {source}"),
        )
        .at(&block.id, None)
    })?;
    let mut compiler = FunctionCompiler {
        block,
        path: source,
        text,
        instructions,
        locals: BTreeMap::new(),
        stored: 0,
    };
    compiler.check_parse_budget()?;
    let parsed = openmat_parser::parse(SourceId::new(1), text);
    if let Some(error) = parsed.diagnostics.first() {
        return Err(compiler.error(error.range, "function_syntax", &error.message));
    }
    let lowered = openmat_hir::lower(&parsed.syntax);
    if let Some(error) = lowered.diagnostics.first() {
        return Err(compiler.error(error.range, "function_syntax", &error.message));
    }
    let [statement] = lowered.file.statements.as_slice() else {
        return Err(compiler.error(
            lowered.file.span,
            "function_file",
            "source must contain exactly one function and no script statements",
        ));
    };
    let StmtKind::Function(function) = &statement.kind else {
        return Err(compiler.error(
            statement.span,
            "function_file",
            "expected a function declaration",
        ));
    };
    let arguments: Vec<&str> = inputs
        .iter()
        .map(|p| p.name.as_str())
        .chain(parameters.iter().map(|p| p.name.as_str()))
        .collect();
    if function.name.as_ref().map(|n| n.text.as_str()) != Some(entry)
        || function.outputs.len() != 1
        || !function
            .inputs
            .iter()
            .map(|n| n.text.as_str())
            .eq(arguments)
    {
        return Err(compiler.error(function.span, "function_signature", "function name and ordered arguments must match the block declaration, with exactly one output"));
    }
    for (input, signal) in inputs.iter().zip(signals) {
        if input.width != signal.len() {
            return Err(compiler.error(
                function.span,
                "signal_width",
                &format!(
                    "input {} expects width {}, found {}",
                    input.name,
                    input.width,
                    signal.len()
                ),
            ));
        }
        compiler.bind(&input.name, signal.to_vec(), function.span)?;
    }
    for parameter in parameters {
        let values = parameter
            .value
            .iter()
            .map(|&v| compiler.push(Instruction::Constant(v), function.span))
            .collect::<Result<Vec<_>, _>>()?;
        compiler.bind(&parameter.name, values, function.span)?;
    }
    for statement in &function.body {
        let StmtKind::Assignment { target, value } = &statement.kind else {
            return Err(compiler.error(statement.span, "function_subset", "only local assignments are supported; control flow, state, I/O and statement calls are not permitted"));
        };
        let ExprKind::Name(name) = &target.kind else {
            return Err(compiler.error(
                target.span,
                "function_assignment",
                "assign a whole local variable; indexed or dynamic assignment is not supported",
            ));
        };
        let result = compiler.expression(value, 0)?;
        compiler.bind(name, result, statement.span)?;
    }
    let result = compiler
        .locals
        .get(&function.outputs[0].text)
        .ok_or_else(|| {
            compiler.error(
                function.span,
                "function_output",
                "function output is never assigned",
            )
        })?;
    if result.len() != *output_width {
        return Err(compiler.error(
            function.span,
            "signal_width",
            &format!(
                "output expects width {output_width}, found {}",
                result.len()
            ),
        ));
    }
    Ok(result.clone())
}

struct FunctionCompiler<'a> {
    block: &'a Block,
    path: &'a str,
    text: &'a str,
    instructions: &'a mut Vec<Instruction>,
    locals: BTreeMap<String, Vec<usize>>,
    stored: usize,
}

impl FunctionCompiler<'_> {
    fn check_parse_budget(&self) -> Result<(), ModelError> {
        use openmat_syntax::TokenKind;
        let tokens = openmat_parser::lex(SourceId::new(1), self.text);
        let (mut depth, mut statement_tokens, mut functions) = (0_usize, 0_usize, 0_usize);
        let mut continued = false;
        for token in tokens.tokens {
            if token.kind.is_keyword()
                && !matches!(token.kind, TokenKind::KwFunction | TokenKind::KwEnd)
            {
                return Err(self.error(
                    token.range,
                    "function_subset",
                    "control flow, classes and implicit state are outside the pure function subset",
                ));
            }
            if token.kind == TokenKind::KwFunction {
                functions += 1;
            }
            if token.kind == TokenKind::Ellipsis {
                continued = true;
            }
            if token.kind == TokenKind::Newline {
                if depth == 0 && !continued {
                    statement_tokens = 0;
                }
                continued = false;
            }
            if token.kind.is_trivia() {
                continue;
            }
            statement_tokens += 1;
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Semicolon | TokenKind::Comma if depth == 0 => statement_tokens = 0,
                _ => {}
            }
            if depth > 32 || statement_tokens > 256 || functions > 1 {
                return Err(self.error(token.range, "function_complexity", "use one function, at most 32 delimiter levels and 256 tokens per statement; split long calculations into local assignments"));
            }
        }
        Ok(())
    }
    fn error(&self, span: TextRange, code: &str, message: &str) -> ModelError {
        let mut error = ModelError::new(code, message).at(&self.block.id, None);
        let prefix = &self.text[..span.start() as usize];
        error.0.source_path = Some(self.path.into());
        error.0.line = Some(prefix.bytes().filter(|&c| c == b'\n').count() + 1);
        error.0.column = Some(
            prefix
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .encode_utf16()
                .count()
                + 1,
        );
        error
    }

    fn push(&mut self, op: Instruction, span: TextRange) -> Result<usize, ModelError> {
        if self.instructions.len() >= MAX_VALUES {
            return Err(self.error(span, "model_limit", "numerical instruction limit exceeded"));
        }
        let id = self.instructions.len();
        self.instructions.push(op);
        Ok(id)
    }

    fn bind(&mut self, name: &str, values: Vec<usize>, span: TextRange) -> Result<(), ModelError> {
        self.stored += values.len();
        if self.stored > MAX_VALUES {
            return Err(self.error(span, "model_limit", "function local storage limit exceeded"));
        }
        self.locals.insert(name.into(), values);
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // Exhaustive HIR subset dispatch keeps unsupported syntax explicit.
    fn expression(&mut self, expr: &Expr, depth: usize) -> Result<Vec<usize>, ModelError> {
        if depth > 128 {
            return Err(self.error(
                expr.span,
                "function_depth",
                "expression nesting limit exceeded",
            ));
        }
        match &expr.kind {
            ExprKind::Number(number) => {
                let value = number
                    .replace(['d', 'D'], "e")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        self.error(
                            expr.span,
                            "function_literal",
                            "expected a finite real double literal",
                        )
                    })?;
                Ok(vec![self.push(Instruction::Constant(value), expr.span)?])
            }
            ExprKind::Name(name) => self.locals.get(name).cloned().ok_or_else(|| {
                self.error(
                    expr.span,
                    "function_name",
                    &format!("undefined local variable: {name}"),
                )
            }),
            ExprKind::Paren(value) => self.expression(value, depth + 1),
            ExprKind::Unary { operator, operand } => {
                let values = self.expression(operand, depth + 1)?;
                match operator {
                    UnaryOp::Plus => Ok(values),
                    UnaryOp::Minus => values
                        .into_iter()
                        .map(|v| self.push(Instruction::Negate(v), expr.span))
                        .collect(),
                    _ => Err(self.error(
                        expr.span,
                        "function_operator",
                        "unsupported unary operator",
                    )),
                }
            }
            ExprKind::Matrix(rows) => {
                let mut result = Vec::new();
                for row in rows {
                    let [value] = row.as_slice() else {
                        return Err(self.error(expr.span, "function_shape", "use a column vector [a; b]; row vectors and matrices are not supported yet"));
                    };
                    let values = self.expression(value, depth + 1)?;
                    if result.len() + values.len() > MAX_WIDTH {
                        return Err(self.error(
                            expr.span,
                            "function_shape",
                            "column width exceeds 4096",
                        ));
                    }
                    result.extend(values);
                }
                if result.is_empty() {
                    return Err(self.error(
                        expr.span,
                        "function_shape",
                        "empty arrays are not supported",
                    ));
                }
                Ok(result)
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let a = self.expression(left, depth + 1)?;
                let b = self.expression(right, depth + 1)?;
                self.binary(*operator, &a, &b, expr.span)
            }
            ExprKind::ParenApply { target, arguments } => {
                let ExprKind::Name(name) = &target.kind else {
                    return Err(self.error(
                        target.span,
                        "function_call",
                        "only local indexing or supported math intrinsics are allowed",
                    ));
                };
                let [argument] = arguments.as_slice() else {
                    return Err(self.error(
                        expr.span,
                        "function_call",
                        "expected exactly one argument",
                    ));
                };
                // Resolve call/index ambiguity using the statically known local environment.
                if let Some(values) = self.locals.get(name) {
                    let ExprKind::Number(number) = &argument.kind else {
                        return Err(self.error(
                            argument.span,
                            "function_index",
                            "index must be a constant positive integer literal",
                        ));
                    };
                    let index = number
                        .parse::<usize>()
                        .ok()
                        .filter(|i| *i >= 1 && *i <= values.len())
                        .ok_or_else(|| {
                            self.error(
                                argument.span,
                                "function_index",
                                "index is outside the one-based fixed column bounds",
                            )
                        })?;
                    return Ok(vec![values[index - 1]]);
                }
                if matches!(&self.block.kind, BlockKind::MFunction { entry, .. } if entry == name) {
                    return Err(self.error(
                        target.span,
                        "function_call",
                        "recursive calls are outside the pure function subset",
                    ));
                }
                let function = match name.as_str() {
                    "sin" => MathFunction::Sin,
                    "cos" => MathFunction::Cos,
                    "exp" => MathFunction::Exp,
                    "sqrt" => MathFunction::Sqrt,
                    "abs" => MathFunction::Abs,
                    "tanh" => MathFunction::Tanh,
                    _ => {
                        return Err(self.error(
                            target.span,
                            "function_call",
                            &format!("call {name} is outside the pure function subset"),
                        ));
                    }
                };
                let values = self.expression(argument, depth + 1)?;
                values
                    .into_iter()
                    .map(|v| self.push(Instruction::Math(function, v), expr.span))
                    .collect()
            }
            _ => Err(self.error(
                expr.span,
                "function_subset",
                "expression is outside the pure fixed-column function subset",
            )),
        }
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        a: &[usize],
        b: &[usize],
        span: TextRange,
    ) -> Result<Vec<usize>, ModelError> {
        if a.len() != b.len() && a.len() != 1 && b.len() != 1 {
            return Err(self.error(
                span,
                "function_shape",
                "elementwise operands must have equal widths or a scalar operand",
            ));
        }
        if (op == BinaryOp::Multiply && a.len() != 1 && b.len() != 1)
            || (op == BinaryOp::RightDivide && b.len() != 1)
        {
            return Err(self.error(span, "function_shape", "matrix multiplication/division is not supported; use .* or ./ for elementwise operations"));
        }
        let mut result = Vec::with_capacity(a.len().max(b.len()));
        for i in 0..a.len().max(b.len()) {
            let x = a[if a.len() == 1 { 0 } else { i }];
            let y = b[if b.len() == 1 { 0 } else { i }];
            let instruction = match op {
                BinaryOp::Add => Instruction::Add(x, y),
                BinaryOp::Subtract => Instruction::Subtract(x, y),
                BinaryOp::Multiply | BinaryOp::ElementMultiply => Instruction::Multiply(x, y),
                BinaryOp::RightDivide | BinaryOp::ElementRightDivide => Instruction::Divide(x, y),
                _ => {
                    return Err(self.error(
                        span,
                        "function_operator",
                        "unsupported binary operator; use +, -, *, /, .* or ./",
                    ));
                }
            };
            result.push(self.push(instruction, span)?);
        }
        Ok(result)
    }
}
