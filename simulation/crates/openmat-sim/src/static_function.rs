//! Bounded, shape-checked callback lowering using the ordinary m parser and HIR.
use std::collections::BTreeMap;

use openmat_hir::{BinaryOp, ConditionalBranch, Expr, ExprKind, Stmt, StmtKind, UnaryOp};
use openmat_source::{SourceId, TextRange};

use crate::component::Callback;
use crate::numeric::{Comparison, Instruction, MAX_VALUES, MathFunction, Program};
use crate::{ModelError, SourceBundle};

const MAX_WIDTH: usize = 4096;

pub(crate) struct Signature<'a> {
    pub arguments: &'a [(&'a str, usize)],
    pub parameters: &'a [f64],
    pub output_width: usize,
    pub conditions: bool,
}

#[derive(Clone)]
struct Array {
    rows: usize,
    cols: usize,
    values: Vec<usize>,
}

impl Array {
    fn column(values: Vec<usize>) -> Self {
        Self {
            rows: values.len(),
            cols: 1,
            values,
        }
    }
    fn same_shape(&self, other: &Self) -> bool {
        self.rows == other.rows && self.cols == other.cols
    }
}

pub(crate) fn compile(
    block: &str,
    callback: &Callback,
    signature: &Signature<'_>,
    sources: &SourceBundle,
) -> Result<Program, ModelError> {
    let text = sources.get(&callback.source).ok_or_else(|| {
        ModelError::new(
            "source_missing",
            format!("missing source snapshot: {}", callback.source),
        )
        .at(block, None)
    })?;
    let mut compiler = Compiler {
        block,
        callback,
        text,
        conditions: signature.conditions,
        instructions: Vec::new(),
        constants: Vec::new(),
        locals: BTreeMap::new(),
        stored: 0,
        statements: 0,
    };
    compiler.preflight()?;
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
            "expected exactly one function and no script statements",
        ));
    };
    let StmtKind::Function(function) = &statement.kind else {
        return Err(compiler.error(
            statement.span,
            "function_file",
            "expected a function declaration",
        ));
    };
    let expected = signature
        .arguments
        .iter()
        .map(|(name, _)| *name)
        .chain(std::iter::once("p"));
    if function.name.as_ref().map(|n| n.text.as_str()) != Some(callback.entry.as_str())
        || function.outputs.len() != 1
        || !function.inputs.iter().map(|n| n.text.as_str()).eq(expected)
    {
        return Err(compiler.error(function.span, "function_signature", "callback name and ordered arguments must match its lifecycle signature, with exactly one output"));
    }
    let mut offset = 0;
    for &(name, width) in signature.arguments {
        let ids = (offset..offset + width)
            .map(|i| compiler.push(Instruction::Input(i), function.span))
            .collect::<Result<Vec<_>, _>>()?;
        compiler.bind(name, Array::column(ids), function.span)?;
        offset += width;
    }
    let parameters = signature
        .parameters
        .iter()
        .map(|&v| compiler.push(Instruction::Constant(v), function.span))
        .collect::<Result<Vec<_>, _>>()?;
    compiler.bind("p", Array::column(parameters), function.span)?;
    compiler.body(&function.body, 0)?;
    let output = compiler
        .locals
        .get(&function.outputs[0].text)
        .ok_or_else(|| {
            compiler.error(
                function.span,
                "function_output",
                "output must be assigned on every path",
            )
        })?;
    if output.values.len() != signature.output_width
        || (output.cols != 1 && !output.values.is_empty())
    {
        return Err(compiler.error(
            function.span,
            "signal_width",
            &format!(
                "expected a column output with {} elements, found {} by {}",
                signature.output_width, output.rows, output.cols
            ),
        ));
    }
    Program::new(offset, compiler.instructions, output.values.clone())
        .map(|p| p.pruned())
        .map_err(|e| ModelError::new("numerical_ir", e.to_string()).at(block, None))
}

struct Compiler<'a> {
    block: &'a str,
    callback: &'a Callback,
    text: &'a str,
    conditions: bool,
    instructions: Vec<Instruction>,
    constants: Vec<Option<f64>>,
    locals: BTreeMap<String, Array>,
    stored: usize,
    statements: usize,
}

impl Compiler<'_> {
    fn error(&self, span: TextRange, code: &str, message: &str) -> ModelError {
        let mut error = ModelError::new(code, message).at(self.block, None);
        let prefix = &self.text[..span.start() as usize];
        error.0.source_path = Some(self.callback.source.clone());
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

    fn preflight(&self) -> Result<(), ModelError> {
        use openmat_syntax::TokenKind;
        let tokens = openmat_parser::lex(SourceId::new(1), self.text);
        let (mut depth, mut control, mut count, mut functions) =
            (0_usize, 0_usize, 0_usize, 0_usize);
        let mut continued = false;
        for token in tokens.tokens {
            if token.kind.is_keyword()
                && !matches!(
                    token.kind,
                    TokenKind::KwFunction
                        | TokenKind::KwEnd
                        | TokenKind::KwFor
                        | TokenKind::KwIf
                        | TokenKind::KwElseif
                        | TokenKind::KwElse
                )
            {
                return Err(self.error(
                    token.range,
                    "function_subset",
                    "only assignments, static for loops and discrete if branches are supported",
                ));
            }
            match token.kind {
                TokenKind::KwFunction => {
                    functions += 1;
                    control += 1;
                }
                TokenKind::KwFor | TokenKind::KwIf => control += 1,
                TokenKind::KwEnd if depth == 0 => control = control.saturating_sub(1),
                TokenKind::Ellipsis => continued = true,
                TokenKind::Newline => {
                    if depth == 0 && !continued {
                        count = 0;
                    }
                    continued = false;
                }
                _ => {}
            }
            if token.kind.is_trivia() {
                continue;
            }
            count += 1;
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Comma | TokenKind::Semicolon if depth == 0 => count = 0,
                _ => {}
            }
            if depth > 32 || control > 16 || count > 256 || functions > 1 {
                return Err(self.error(token.range, "function_complexity", "one function, at most 16 control levels, 32 delimiter levels and 256 tokens per statement are supported"));
            }
        }
        Ok(())
    }

    fn push(&mut self, instruction: Instruction, span: TextRange) -> Result<usize, ModelError> {
        if self.instructions.len() >= MAX_VALUES {
            return Err(self.error(span, "model_limit", "numerical instruction limit exceeded"));
        }
        let id = self.instructions.len();
        self.constants.push(instruction.constant(&self.constants));
        self.instructions.push(instruction);
        Ok(id)
    }

    fn bind(&mut self, name: &str, value: Array, span: TextRange) -> Result<(), ModelError> {
        self.stored += value.values.len();
        if self.stored > MAX_VALUES || value.values.len() > MAX_WIDTH {
            return Err(self.error(
                span,
                "model_limit",
                "callback local storage or fixed array size limit exceeded",
            ));
        }
        self.locals.insert(name.into(), value);
        Ok(())
    }

    fn body(&mut self, body: &[Stmt], depth: usize) -> Result<(), ModelError> {
        if depth > 16 {
            return Err(self.error(
                body[0].span,
                "function_complexity",
                "control nesting exceeds 16",
            ));
        }
        for statement in body {
            self.statements += 1;
            if self.statements > 100_000 {
                return Err(self.error(
                    statement.span,
                    "function_complexity",
                    "unrolled statement budget exceeded",
                ));
            }
            match &statement.kind {
                StmtKind::Assignment { target, value } => {
                    let value = self.expression(value, 0)?;
                    self.assignment(target, value)?;
                }
                StmtKind::For {
                    variable,
                    iterable,
                    body,
                } => self.loop_body(variable, iterable, body, depth)?,
                StmtKind::If {
                    branches,
                    else_body,
                } if self.conditions => self.conditional(branches, else_body, depth)?,
                _ => return Err(self.error(
                    statement.span,
                    "function_subset",
                    "expected assignment or static for; if is only supported in initialize/update",
                )),
            }
        }
        Ok(())
    }

    fn assignment(&mut self, target: &Expr, value: Array) -> Result<(), ModelError> {
        match &target.kind {
            ExprKind::Name(name) => self.bind(name, value, target.span),
            ExprKind::ParenApply {
                target: base,
                arguments,
            } => {
                let ExprKind::Name(name) = &base.kind else {
                    return Err(self.error(
                        target.span,
                        "function_assignment",
                        "indexed assignment requires a fixed local array",
                    ));
                };
                let mut array = self.locals.get(name).cloned().ok_or_else(|| {
                    self.error(
                        target.span,
                        "function_assignment",
                        "create the fixed array before indexed assignment",
                    )
                })?;
                let index = self.index(&array, arguments, target.span, 0)?;
                if value.values.len() != 1 {
                    return Err(self.error(
                        target.span,
                        "function_assignment",
                        "indexed assignment requires a scalar value",
                    ));
                }
                array.values[index] = value.values[0];
                self.bind(name, array, target.span)
            }
            _ => Err(self.error(
                target.span,
                "function_assignment",
                "unsupported assignment target",
            )),
        }
    }

    fn loop_body(
        &mut self,
        variable: &Expr,
        iterable: &Expr,
        body: &[Stmt],
        depth: usize,
    ) -> Result<(), ModelError> {
        let ExprKind::Name(name) = &variable.kind else {
            return Err(self.error(
                variable.span,
                "function_loop",
                "for requires a local loop variable",
            ));
        };
        let ExprKind::Range { start, step, end } = &iterable.kind else {
            return Err(self.error(
                iterable.span,
                "function_loop",
                "for requires a compile-time integer range",
            ));
        };
        let start = self.integer(start, 0)?;
        let step = step.as_ref().map_or(Ok(1), |s| self.integer(s, 0))?;
        let end = self.integer(end, 0)?;
        if step == 0 {
            return Err(self.error(iterable.span, "function_loop", "for step cannot be zero"));
        }
        let count = if (step > 0 && start <= end) || (step < 0 && start >= end) {
            (end - start) / step + 1
        } else {
            0
        };
        if count > 1024 {
            return Err(self.error(
                iterable.span,
                "function_loop",
                "for supports at most 1024 static iterations",
            ));
        }
        // MATLAB leaves the final loop variable in the local scope.
        for i in 0..count {
            #[allow(clippy::cast_precision_loss)] // Compile-time integers are bounded to +/- 2^31.
            let id = self.push(
                Instruction::Constant((start + i * step) as f64),
                variable.span,
            )?;
            self.bind(name, Array::column(vec![id]), variable.span)?;
            self.body(body, depth + 1)?;
        }
        if count == 0 {
            self.bind(
                name,
                Array {
                    rows: 0,
                    cols: 0,
                    values: Vec::new(),
                },
                variable.span,
            )?;
        }
        Ok(())
    }

    fn conditional(
        &mut self,
        branches: &[ConditionalBranch],
        otherwise: &[Stmt],
        depth: usize,
    ) -> Result<(), ModelError> {
        let incoming = self.locals.clone();
        let mut cases = Vec::new();
        for branch in branches {
            self.locals.clone_from(&incoming);
            let condition = self.expression(&branch.condition, 0)?;
            if condition.values.len() != 1 {
                return Err(self.error(
                    branch.condition.span,
                    "function_condition",
                    "if requires a scalar condition",
                ));
            }
            self.body(&branch.body, depth + 1)?;
            cases.push((condition.values[0], self.locals.clone(), branch.span));
        }
        self.locals = incoming;
        self.body(otherwise, depth + 1)?;
        for (condition, branch, span) in cases.into_iter().rev() {
            let fallback = std::mem::take(&mut self.locals);
            for (name, a) in branch {
                let Some(b) = fallback.get(&name) else {
                    continue;
                };
                if !a.same_shape(b) {
                    return Err(self.error(
                        span,
                        "function_shape",
                        "if branches must assign identical fixed shapes",
                    ));
                }
                let values = a
                    .values
                    .iter()
                    .zip(&b.values)
                    .map(|(&a, &b)| {
                        if a == b {
                            Ok(a)
                        } else {
                            self.push(Instruction::Select(condition, a, b), span)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.bind(&name, Array { values, ..a }, span)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // HIR dispatch explicitly rejects dynamic language features.
    fn expression(&mut self, expr: &Expr, depth: usize) -> Result<Array, ModelError> {
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
                Ok(Array::column(vec![
                    self.push(Instruction::Constant(value), expr.span)?,
                ]))
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
                let mut array = self.expression(operand, depth + 1)?;
                if *operator == UnaryOp::Not && (!self.conditions || array.values.len() != 1) {
                    return Err(self.error(
                        expr.span,
                        "function_condition",
                        "logical not requires a scalar in initialize/update",
                    ));
                }
                if *operator == UnaryOp::Plus {
                    return Ok(array);
                }
                let zero = self.push(Instruction::Constant(0.0), expr.span)?;
                for value in &mut array.values {
                    let op = match operator {
                        UnaryOp::Minus => Instruction::Negate(*value),
                        UnaryOp::Not => Instruction::Compare(Comparison::Equal, *value, zero),
                        _ => {
                            return Err(self.error(
                                expr.span,
                                "function_operator",
                                "unsupported unary operator",
                            ));
                        }
                    };
                    *value = self.push(op, expr.span)?;
                }
                Ok(array)
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
            ExprKind::Matrix(rows) => {
                let mut pieces = Vec::new();
                for row in rows {
                    let items = row
                        .iter()
                        .map(|e| self.expression(e, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?;
                    pieces.push(self.concatenate(&items, true, expr.span)?);
                }
                self.concatenate(&pieces, false, expr.span)
            }
            ExprKind::Transpose { operand, .. } => {
                let a = self.expression(operand, depth + 1)?;
                let mut values = Vec::with_capacity(a.values.len());
                for col in 0..a.rows {
                    for row in 0..a.cols {
                        values.push(a.values[col + row * a.rows]);
                    }
                }
                Ok(Array {
                    rows: a.cols,
                    cols: a.rows,
                    values,
                })
            }
            ExprKind::ParenApply { target, arguments } => {
                let ExprKind::Name(name) = &target.kind else {
                    return Err(self.error(
                        target.span,
                        "function_call",
                        "only fixed local indexing and supported intrinsics are allowed",
                    ));
                };
                if let Some(a) = self.locals.get(name).cloned() {
                    let index = self.index(&a, arguments, expr.span, depth + 1)?;
                    return Ok(Array::column(vec![a.values[index]]));
                }
                if name == &self.callback.entry {
                    return Err(self.error(
                        expr.span,
                        "function_call",
                        "recursive calls are not supported",
                    ));
                }
                self.intrinsic(name, arguments, expr.span, depth + 1)
            }
            _ => Err(self.error(
                expr.span,
                "function_subset",
                "expression is outside the fixed-shape callback subset",
            )),
        }
    }

    fn integer(&mut self, expr: &Expr, depth: usize) -> Result<i64, ModelError> {
        let a = self.expression(expr, depth + 1)?;
        let value = (a.values.len() == 1)
            .then(|| self.constants[a.values[0]])
            .flatten()
            .filter(|v| v.is_finite() && v.fract() == 0.0 && v.abs() <= f64::from(i32::MAX))
            .ok_or_else(|| {
                self.error(
                    expr.span,
                    "function_constant",
                    "expected a compile-time integer within +/- 2^31",
                )
            })?;
        #[allow(clippy::cast_possible_truncation)] // Checked finite integer range above.
        Ok(value as i64)
    }

    fn index(
        &mut self,
        array: &Array,
        arguments: &[Expr],
        span: TextRange,
        depth: usize,
    ) -> Result<usize, ModelError> {
        let bounds = match arguments.len() {
            1 => vec![array.values.len()],
            2 => vec![array.rows, array.cols],
            _ => {
                return Err(self.error(
                    span,
                    "function_index",
                    "expected one or two static indices",
                ));
            }
        };
        let mut indices = Vec::new();
        for (argument, bound) in arguments.iter().zip(bounds) {
            let i = usize::try_from(self.integer(argument, depth)?)
                .ok()
                .filter(|&i| i >= 1 && i <= bound)
                .ok_or_else(|| {
                    self.error(
                        argument.span,
                        "function_index",
                        "index is outside one-based fixed array bounds",
                    )
                })?;
            indices.push(i - 1);
        }
        Ok(indices[0] + indices.get(1).copied().unwrap_or(0) * array.rows)
    }

    fn concatenate(
        &self,
        arrays: &[Array],
        horizontal: bool,
        span: TextRange,
    ) -> Result<Array, ModelError> {
        let mut nonempty: Vec<_> = arrays.iter().filter(|a| !a.values.is_empty()).collect();
        if nonempty.is_empty() {
            // Preserve shaped empty arrays; literal [] and empty colon ranges are 0-by-0.
            nonempty = arrays
                .iter()
                .filter(|a| a.rows != 0 || a.cols != 0)
                .collect();
        }
        let Some(first) = nonempty.first() else {
            return Ok(Array {
                rows: 0,
                cols: 0,
                values: Vec::new(),
            });
        };
        let (rows, cols) = if horizontal {
            if nonempty.iter().any(|a| a.rows != first.rows) {
                return Err(self.error(
                    span,
                    "function_shape",
                    "horizontal concatenation requires equal row counts",
                ));
            }
            (first.rows, nonempty.iter().map(|a| a.cols).sum::<usize>())
        } else {
            if nonempty.iter().any(|a| a.cols != first.cols) {
                return Err(self.error(
                    span,
                    "function_shape",
                    "vertical concatenation requires equal column counts",
                ));
            }
            (nonempty.iter().map(|a| a.rows).sum::<usize>(), first.cols)
        };
        if rows * cols > MAX_WIDTH {
            return Err(self.error(
                span,
                "function_shape",
                "fixed arrays support at most 4096 elements",
            ));
        }
        let mut values = Vec::with_capacity(rows * cols);
        if horizontal {
            for a in nonempty {
                values.extend(&a.values);
            }
        } else {
            for col in 0..cols {
                for a in &nonempty {
                    values.extend(&a.values[col * a.rows..(col + 1) * a.rows]);
                }
            }
        }
        Ok(Array { rows, cols, values })
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        a: &Array,
        b: &Array,
        span: TextRange,
    ) -> Result<Array, ModelError> {
        if matches!(
            op,
            BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
        ) && (!self.conditions || a.values.len() != 1 || b.values.len() != 1)
        {
            return Err(self.error(
                span,
                "function_condition",
                "comparisons require scalar operands in initialize/update",
            ));
        }
        if op == BinaryOp::Multiply && a.values.len() != 1 && b.values.len() != 1 {
            return self.matmul(a, b, span);
        }
        if !a.same_shape(b) && a.values.len() != 1 && b.values.len() != 1 {
            return Err(self.error(
                span,
                "function_shape",
                "elementwise operands require identical shapes or a scalar",
            ));
        }
        if op == BinaryOp::RightDivide && b.values.len() != 1 {
            return Err(self.error(
                span,
                "function_shape",
                "matrix division is not supported; use ./ for elementwise division",
            ));
        }
        let shape = if a.values.len() == 1 { b } else { a };
        let mut values = Vec::new();
        for i in 0..shape.values.len() {
            let x = a.values[if a.values.len() == 1 { 0 } else { i }];
            let y = b.values[if b.values.len() == 1 { 0 } else { i }];
            let instruction = match op {
                BinaryOp::Add => Instruction::Add(x, y),
                BinaryOp::Subtract => Instruction::Subtract(x, y),
                BinaryOp::Multiply | BinaryOp::ElementMultiply => Instruction::Multiply(x, y),
                BinaryOp::RightDivide | BinaryOp::ElementRightDivide => Instruction::Divide(x, y),
                BinaryOp::Equal => Instruction::Compare(Comparison::Equal, x, y),
                BinaryOp::NotEqual => Instruction::Compare(Comparison::NotEqual, x, y),
                BinaryOp::Less => Instruction::Compare(Comparison::Less, x, y),
                BinaryOp::LessEqual => Instruction::Compare(Comparison::LessEqual, x, y),
                BinaryOp::Greater => Instruction::Compare(Comparison::Greater, x, y),
                BinaryOp::GreaterEqual => Instruction::Compare(Comparison::GreaterEqual, x, y),
                _ => {
                    return Err(self.error(
                        span,
                        "function_operator",
                        "unsupported fixed-shape operator",
                    ));
                }
            };
            values.push(self.push(instruction, span)?);
        }
        Ok(Array {
            rows: shape.rows,
            cols: shape.cols,
            values,
        })
    }

    fn matmul(&mut self, a: &Array, b: &Array, span: TextRange) -> Result<Array, ModelError> {
        if a.cols != b.rows || a.rows * b.cols > MAX_WIDTH {
            return Err(self.error(
                span,
                "function_shape",
                "matrix product dimensions disagree or exceed 4096 elements",
            ));
        }
        let mut values = Vec::new();
        for col in 0..b.cols {
            for row in 0..a.rows {
                let mut result = None;
                for k in 0..a.cols {
                    let term = self.push(
                        Instruction::Multiply(
                            a.values[row + k * a.rows],
                            b.values[k + col * b.rows],
                        ),
                        span,
                    )?;
                    result = Some(match result {
                        None => term,
                        Some(sum) => self.push(Instruction::Add(sum, term), span)?,
                    });
                }
                values.push(match result {
                    Some(id) => id,
                    None => self.push(Instruction::Constant(0.0), span)?,
                });
            }
        }
        Ok(Array {
            rows: a.rows,
            cols: b.cols,
            values,
        })
    }

    fn dimensions(
        &mut self,
        args: &[Expr],
        span: TextRange,
        depth: usize,
    ) -> Result<(usize, usize), ModelError> {
        if args.is_empty() || args.len() > 2 {
            return Err(self.error(
                span,
                "function_shape",
                "expected one or two static dimensions",
            ));
        }
        let mut dimensions = Vec::new();
        for expr in args {
            let n = usize::try_from(self.integer(expr, depth)?)
                .ok()
                .filter(|&n| n <= MAX_WIDTH)
                .ok_or_else(|| {
                    self.error(
                        expr.span,
                        "function_shape",
                        "dimensions must be static integers 0..4096",
                    )
                })?;
            dimensions.push(n);
        }
        let (rows, cols) = (
            dimensions[0],
            dimensions.get(1).copied().unwrap_or(dimensions[0]),
        );
        if rows * cols > MAX_WIDTH {
            return Err(self.error(span, "function_shape", "fixed array exceeds 4096 elements"));
        }
        Ok((rows, cols))
    }

    fn intrinsic(
        &mut self,
        name: &str,
        args: &[Expr],
        span: TextRange,
        depth: usize,
    ) -> Result<Array, ModelError> {
        if name == "zeros" || name == "ones" {
            let (rows, cols) = self.dimensions(args, span, depth)?;
            let id = self.push(
                Instruction::Constant(if name == "zeros" { 0.0 } else { 1.0 }),
                span,
            )?;
            return Ok(Array {
                rows,
                cols,
                values: vec![id; rows * cols],
            });
        }
        if name == "reshape" && args.len() == 3 {
            let a = self.expression(&args[0], depth + 1)?;
            let (rows, cols) = self.dimensions(&args[1..], span, depth)?;
            if rows * cols != a.values.len() {
                return Err(self.error(
                    span,
                    "function_shape",
                    "reshape must preserve the number of elements",
                ));
            }
            return Ok(Array {
                rows,
                cols,
                values: a.values,
            });
        }
        if (name == "numel" && args.len() == 1) || (name == "size" && args.len() == 2) {
            let a = self.expression(&args[0], depth + 1)?;
            let size = if name == "numel" {
                a.values.len()
            } else {
                match self.integer(&args[1], depth)? {
                    1 => a.rows,
                    2 => a.cols,
                    _ => {
                        return Err(self.error(
                            span,
                            "function_shape",
                            "size dimension must be 1 or 2",
                        ));
                    }
                }
            };
            #[allow(clippy::cast_precision_loss)]
            // Fixed arrays have at most 4096 elements per dimension.
            return Ok(Array::column(vec![
                self.push(Instruction::Constant(size as f64), span)?,
            ]));
        }
        let function = match name {
            "sin" => MathFunction::Sin,
            "cos" => MathFunction::Cos,
            "exp" => MathFunction::Exp,
            "sqrt" => MathFunction::Sqrt,
            "abs" => MathFunction::Abs,
            "tanh" => MathFunction::Tanh,
            _ => {
                return Err(self.error(
                    span,
                    "function_call",
                    &format!("call {name} is outside the fixed-shape callback subset"),
                ));
            }
        };
        let [argument] = args else {
            return Err(self.error(
                span,
                "function_call",
                "math intrinsic requires exactly one argument",
            ));
        };
        let mut a = self.expression(argument, depth + 1)?;
        for value in &mut a.values {
            *value = self.push(Instruction::Math(function, *value), span)?;
        }
        Ok(a)
    }
}
