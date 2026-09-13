//! Bounded constant initialization, using the ordinary lossless m parser.
//! This evaluator has no filesystem, workspace, callback or general call access.
use std::collections::BTreeMap;

use openmat_hir::{BinaryOp, Expr, ExprKind, Stmt, StmtKind, UnaryOp};
use openmat_source::SourceId;
use openmat_syntax::TokenKind;
use serde::Serialize;

use crate::Issue;

const MAX_ARRAY: usize = 4096;
const MAX_WORK: usize = 1_048_576;
pub const MAX_PARAMETER_TEXT: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParameterArray {
    pub rows: usize,
    pub columns: usize,
    /// Column-major numerical values; dimensions are retained independently.
    pub values: Vec<f64>,
}

impl ParameterArray {
    fn new(rows: usize, columns: usize, values: Vec<f64>) -> Result<Self, Issue> {
        if rows.checked_mul(columns) != Some(values.len()) || values.len() > MAX_ARRAY {
            return Err(fail(
                "parameter_shape",
                "parameter array exceeds its shape budget",
            ));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(fail(
                "parameter_finite",
                "parameter expression produced a non-finite value",
            ));
        }
        Ok(Self {
            rows,
            columns,
            values,
        })
    }
    fn number(value: f64) -> Result<Self, Issue> {
        Self::new(1, 1, vec![value])
    }
    pub(crate) fn scalar(&self) -> Result<f64, Issue> {
        if self.values.len() != 1 {
            return Err(fail("parameter_shape", "expected a scalar parameter"));
        }
        Ok(self.values[0])
    }
    pub(crate) fn vector(&self) -> Result<Vec<f64>, Issue> {
        if self.values.is_empty() || (self.rows != 1 && self.columns != 1) {
            return Err(fail(
                "parameter_shape",
                "expected a nonempty scalar or vector parameter",
            ));
        }
        Ok(self.values.clone())
    }
    pub(crate) fn text(&self) -> String {
        let rows = (0..self.rows)
            .map(|r| {
                (0..self.columns)
                    .map(|c| self.values[r + c * self.rows].to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        format!("[{}]", rows.join("; "))
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Parameters {
    pub values: BTreeMap<String, ParameterArray>,
}

impl Parameters {
    /// Evaluate explicitly supplied assignment-only m text, in source order.
    /// # Errors
    /// Rejects general scripts, unresolved names, unsupported expressions and limits.
    pub fn from_script(text: &str) -> Result<Self, Issue> {
        let statements = parse(text)?;
        let mut evaluator = Evaluator {
            values: BTreeMap::new(),
            work: 0,
        };
        for statement in statements {
            let StmtKind::Assignment { target, value } = statement.kind else {
                return Err(fail(
                    "parameter_statement",
                    "parameter files allow only name = constant-expression assignments",
                ));
            };
            let ExprKind::Name(name) = target.kind else {
                return Err(fail(
                    "parameter_assignment",
                    "parameter assignments require a plain variable name",
                ));
            };
            if name.len() > 63
                || (evaluator.values.len() >= 256 && !evaluator.values.contains_key(&name))
            {
                return Err(fail(
                    "parameter_limit",
                    "parameter names or variable count exceed the profile limit",
                ));
            }
            let array = evaluator.eval(&value, 0).map_err(|mut e| {
                let line = text[..value.span.start() as usize]
                    .bytes()
                    .filter(|c| *c == b'\n')
                    .count()
                    + 1;
                e.parameter = Some(name.clone());
                e.message = format!("parameters.m:{line}: {}", e.message);
                e
            })?;
            evaluator.values.insert(name, array);
            if evaluator
                .values
                .values()
                .map(|a| a.values.len())
                .sum::<usize>()
                > 65_536
            {
                return Err(fail(
                    "parameter_limit",
                    "total stored parameter values exceed 65536",
                ));
            }
        }
        Ok(Self {
            values: evaluator.values,
        })
    }
    /// # Errors
    /// Reports unsupported or unresolved constant expressions without fallback values.
    pub fn evaluate(&self, text: &str) -> Result<ParameterArray, Issue> {
        self.evaluate_with_budget(text, &mut 0)
    }

    pub(crate) fn evaluate_with_budget(
        &self,
        text: &str,
        work: &mut usize,
    ) -> Result<ParameterArray, Issue> {
        let statements = parse(text)?;
        let [
            Stmt {
                kind: StmtKind::Expr(expr),
                ..
            },
        ] = statements.as_slice()
        else {
            return Err(fail(
                "parameter_expression",
                "expected exactly one constant expression",
            ));
        };
        let mut evaluator = Evaluator {
            values: self.values.clone(),
            work: *work,
        };
        let value = evaluator.eval(expr, 0);
        *work = evaluator.work;
        value
    }
}

fn fail(code: &str, message: &str) -> Issue {
    Issue::new(code, message)
}

fn parse(text: &str) -> Result<Vec<Stmt>, Issue> {
    if text.len() > MAX_PARAMETER_TEXT {
        return Err(fail("parameter_limit", "parameter source exceeds 64 KiB"));
    }
    // Use the existing lexer to bound recursive CST/HIR work before parsing.
    // Continued newlines are lexer whitespace, so they cannot reset a chain's
    // budget. Reject statement keywords before nested control syntax is parsed.
    let lexed = openmat_parser::lex(SourceId::new(1), text);
    if !lexed.diagnostics.is_empty() {
        return Err(fail("parameter_syntax", "invalid m parameter tokens"));
    }
    if lexed.tokens.len() > 16_384 {
        return Err(fail("parameter_limit", "parameter token limit exceeded"));
    }
    let (mut nesting, mut operators) = (0usize, 0usize);
    for token in lexed.tokens {
        if token.kind.is_keyword() {
            return Err(fail(
                "parameter_statement",
                "control statements and definitions are outside the constant parameter subset",
            ));
        }
        if token.kind.is_line_break() && nesting == 0 {
            operators = 0;
        }
        if token.kind.is_trivia() {
            continue;
        }
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket => {
                nesting += 1;
                // Also bounds long postfix chains such as f(1)(1)... .
                operators += 1;
            }
            TokenKind::RParen | TokenKind::RBracket => nesting = nesting.saturating_sub(1),
            TokenKind::Semicolon | TokenKind::Comma => {
                if nesting == 0 {
                    operators = 0;
                }
            }
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Caret
            | TokenKind::DotStar
            | TokenKind::DotSlash
            | TokenKind::DotCaret
            | TokenKind::ConjugateTranspose
            | TokenKind::DotTranspose
            | TokenKind::Equal => operators += 1,
            TokenKind::Number | TokenKind::Identifier | TokenKind::Eof => {}
            _ => {
                return Err(fail(
                    "parameter_syntax",
                    "token is outside the constant parameter subset",
                ));
            }
        }
        if nesting > 32 || operators > 128 {
            return Err(fail(
                "parameter_limit",
                "parameter expression is too deeply nested or complex",
            ));
        }
    }
    let parsed = openmat_parser::parse(SourceId::new(1), text);
    if !parsed.diagnostics.is_empty() {
        return Err(fail("parameter_syntax", "invalid m parameter syntax"));
    }
    let lowered = openmat_hir::lower(&parsed.syntax);
    if !lowered.diagnostics.is_empty() || lowered.file.statements.len() > 512 {
        return Err(fail(
            "parameter_syntax",
            "invalid or oversized parameter file",
        ));
    }
    Ok(lowered.file.statements)
}

struct Evaluator {
    values: BTreeMap<String, ParameterArray>,
    work: usize,
}

impl Evaluator {
    fn charge(&mut self, work: usize) -> Result<(), Issue> {
        self.work = self.work.saturating_add(work);
        if self.work > MAX_WORK {
            return Err(fail(
                "parameter_limit",
                "constant evaluation work limit exceeded",
            ));
        }
        Ok(())
    }
    fn eval(&mut self, expr: &Expr, depth: usize) -> Result<ParameterArray, Issue> {
        if depth > 64 {
            return Err(fail(
                "parameter_limit",
                "constant expression depth exceeded",
            ));
        }
        self.charge(1)?;
        let result = match &expr.kind {
            ExprKind::Number(text) => ParameterArray::number(text.parse::<f64>().map_err(|_| fail("parameter_number", "expected a real double number"))?)?,
            ExprKind::Name(name) => self.values.get(name).cloned().map_or_else(|| {
                if name == "pi" { ParameterArray::number(std::f64::consts::PI) }
                else { Err(Issue::new("parameter_unresolved", format!("unresolved parameter {name}; supply its explicit parameter assignment"))) }
            }, Ok)?,
            ExprKind::Paren(inner) => self.eval(inner, depth + 1)?,
            ExprKind::Unary { operator, operand } => {
                let a = self.eval(operand, depth + 1)?;
                match operator {
                    UnaryOp::Plus => a,
                    UnaryOp::Minus => ParameterArray::new(a.rows, a.columns, a.values.iter().map(|v| -v).collect())?,
                    _ => return Err(fail("parameter_operator", "unsupported constant unary operator")),
                }
            }
            ExprKind::Transpose { operand, .. } => {
                let a = self.eval(operand, depth + 1)?;
                let values = (0..a.rows).flat_map(|r| (0..a.columns).map(move |c| (r,c)))
                    .map(|(r,c)| a.values[r + c * a.rows]).collect();
                ParameterArray::new(a.columns, a.rows, values)?
            }
            ExprKind::Binary { operator, left, right } => {
                let a = self.eval(left, depth + 1)?;
                let b = self.eval(right, depth + 1)?;
                self.binary(*operator, &a, &b)?
            }
            ExprKind::Matrix(rows) => {
                let arrays = rows.iter().map(|row| row.iter().map(|e| self.eval(e, depth + 1)).collect::<Result<Vec<_>, _>>())
                    .collect::<Result<Vec<_>, _>>()?;
                concatenate(arrays)?
            }
            ExprKind::ParenApply { target, arguments } => {
                let ExprKind::Name(name) = &target.kind else { return Err(fail("parameter_call", "only named pure constant functions are supported")); };
                let args = arguments.iter().map(|e| self.eval(e, depth + 1)).collect::<Result<Vec<_>, _>>()?;
                if self.values.contains_key(name) {
                    return Err(fail("parameter_index", "parameter indexing is outside control-v1; supply the selected constant explicitly"));
                }
                self.call(name, &args)?
            }
            _ => return Err(fail("parameter_expression", "expression is outside the bounded constant parameter subset")),
        };
        self.charge(result.values.len())?;
        Ok(result)
    }
    fn binary(
        &mut self,
        op: BinaryOp,
        a: &ParameterArray,
        b: &ParameterArray,
    ) -> Result<ParameterArray, Issue> {
        if op == BinaryOp::Multiply && a.values.len() != 1 && b.values.len() != 1 {
            if a.columns != b.rows {
                return Err(fail(
                    "parameter_shape",
                    "matrix multiplication dimensions do not agree",
                ));
            }
            let count = a
                .rows
                .checked_mul(b.columns)
                .filter(|n| *n <= MAX_ARRAY)
                .ok_or_else(|| fail("parameter_limit", "matrix product exceeds array limit"))?;
            self.charge(count.saturating_mul(a.columns))?;
            let mut values = vec![0.0; count];
            for c in 0..b.columns {
                for r in 0..a.rows {
                    for k in 0..a.columns {
                        values[r + c * a.rows] +=
                            a.values[r + k * a.rows] * b.values[k + c * b.rows];
                    }
                }
            }
            return ParameterArray::new(a.rows, b.columns, values);
        }
        if (op == BinaryOp::RightDivide && b.values.len() != 1)
            || (op == BinaryOp::Power && (a.values.len() != 1 || b.values.len() != 1))
        {
            return Err(fail(
                "parameter_operator",
                "matrix division and matrix powers are outside control-v1",
            ));
        }
        let scalar_a = a.values.len() == 1;
        let scalar_b = b.values.len() == 1;
        if !scalar_a && !scalar_b && (a.rows != b.rows || a.columns != b.columns) {
            return Err(fail(
                "parameter_shape",
                "elementwise parameters need matching shapes or a scalar",
            ));
        }
        let shape = if scalar_a {
            (b.rows, b.columns)
        } else {
            (a.rows, a.columns)
        };
        let mut values = Vec::new();
        for i in 0..shape.0 * shape.1 {
            let x = a.values[if scalar_a { 0 } else { i }];
            let y = b.values[if scalar_b { 0 } else { i }];
            values.push(match op {
                BinaryOp::Add => x + y,
                BinaryOp::Subtract => x - y,
                BinaryOp::Multiply | BinaryOp::ElementMultiply => x * y,
                BinaryOp::RightDivide | BinaryOp::ElementRightDivide => x / y,
                BinaryOp::Power | BinaryOp::ElementPower => x.powf(y),
                _ => {
                    return Err(fail(
                        "parameter_operator",
                        "unsupported constant binary operator",
                    ));
                }
            });
        }
        ParameterArray::new(shape.0, shape.1, values)
    }
    fn call(&mut self, name: &str, args: &[ParameterArray]) -> Result<ParameterArray, Issue> {
        if ["zeros", "ones", "eye"].contains(&name) {
            let dimensions = match args {
                [a] if a.values.len() == 1 => vec![a.scalar()?, a.scalar()?],
                [a] if a.values.len() == 2 && a.rows == 1 => a.values.clone(),
                [a, b] => vec![a.scalar()?, b.scalar()?],
                _ => {
                    return Err(fail(
                        "parameter_shape",
                        "array constructor needs one size or two dimensions",
                    ));
                }
            };
            let r = dimension(dimensions[0])?;
            let c = dimension(dimensions[1])?;
            let count = r
                .checked_mul(c)
                .filter(|v| *v <= MAX_ARRAY)
                .ok_or_else(|| {
                    fail("parameter_limit", "constructed array exceeds 4096 elements")
                })?;
            self.charge(count)?;
            let mut values = vec![if name == "ones" { 1.0 } else { 0.0 }; count];
            if name == "eye" {
                for i in 0..r.min(c) {
                    values[i + i * r] = 1.0;
                }
            }
            return ParameterArray::new(r, c, values);
        }
        if name == "reshape" {
            let [a, rows, columns] = args else {
                return Err(fail(
                    "parameter_call",
                    "reshape needs an array and two explicit dimensions",
                ));
            };
            return ParameterArray::new(
                dimension(rows.scalar()?)?,
                dimension(columns.scalar()?)?,
                a.values.clone(),
            );
        }
        let [a] = args else {
            return Err(fail(
                "parameter_call",
                "unsupported constant function or argument count",
            ));
        };
        let function: fn(f64) -> f64 = match name {
            "sin" => f64::sin,
            "cos" => f64::cos,
            "tan" => f64::tan,
            "exp" => f64::exp,
            "log" => f64::ln,
            "sqrt" => f64::sqrt,
            "abs" => f64::abs,
            _ => {
                return Err(Issue::new(
                    "parameter_call",
                    format!("unsupported constant function {name}"),
                ));
            }
        };
        ParameterArray::new(
            a.rows,
            a.columns,
            a.values.iter().copied().map(function).collect(),
        )
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn dimension(value: f64) -> Result<usize, Issue> {
    if !(0.0..=4096.0).contains(&value) || value.fract() != 0.0 {
        return Err(fail(
            "parameter_shape",
            "dimensions must be integers in 0..4096",
        ));
    }
    Ok(value as usize)
}

fn concatenate(rows: Vec<Vec<ParameterArray>>) -> Result<ParameterArray, Issue> {
    let mut stripes = Vec::new();
    for row in rows {
        let nonempty: Vec<_> = row.into_iter().filter(|a| !a.values.is_empty()).collect();
        if nonempty.is_empty() {
            continue;
        }
        let height = nonempty[0].rows;
        if nonempty.iter().any(|a| a.rows != height) {
            return Err(fail(
                "parameter_shape",
                "horizontal concatenation row counts differ",
            ));
        }
        let columns = nonempty.iter().map(|a| a.columns).sum::<usize>();
        if height.saturating_mul(columns) > MAX_ARRAY {
            return Err(fail(
                "parameter_shape",
                "horizontal concatenation exceeds the array limit",
            ));
        }
        stripes.push(ParameterArray::new(
            height,
            columns,
            nonempty.into_iter().flat_map(|a| a.values).collect(),
        )?);
    }
    if stripes.is_empty() {
        return ParameterArray::new(0, 0, vec![]);
    }
    let columns = stripes[0].columns;
    let height = stripes.iter().map(|a| a.rows).sum::<usize>();
    if stripes.iter().any(|a| a.columns != columns) || height.saturating_mul(columns) > MAX_ARRAY {
        return Err(fail(
            "parameter_shape",
            "vertical concatenation dimensions differ or exceed the array limit",
        ));
    }
    let mut values = Vec::with_capacity(height * columns);
    for c in 0..columns {
        for a in &stripes {
            values.extend_from_slice(&a.values[c * a.rows..(c + 1) * a.rows]);
        }
    }
    ParameterArray::new(height, columns, values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrix_shapes_and_column_major_arithmetic() {
        let p = Parameters::from_script(
            "K = 2; A = [-1 1; 0 -2]; B = eye(2); C = A*B; w = 2*pi; z = zeros(2,1);",
        )
        .unwrap();
        assert_eq!(p.evaluate("C").unwrap().values, vec![-1., 0., 1., -2.]);
        assert_eq!(p.evaluate("A'").unwrap().values, vec![-1., 1., 0., -2.]);
        assert_eq!(p.evaluate("[A;B]").unwrap().rows, 4);
        assert_eq!(
            p.evaluate("reshape([1 2 3 4],2,2)").unwrap().text(),
            "[1 3; 2 4]"
        );
        assert_eq!(
            p.evaluate("K+1").unwrap().scalar().unwrap().to_bits(),
            3.0_f64.to_bits()
        );
        assert!(p.evaluate("A").unwrap().vector().is_err());
    }
    #[test]
    fn explicit_errors_for_missing_parameters_side_effects_and_limits() {
        for text in [
            "x = eval('1');",
            "run init",
            "for k=1:3; x=k; end",
            "A(1)=2;",
            "x=ones(100000);",
            "x=1/0;",
            "x=[1 2;3];",
            "x=unknown;",
            "x=system(1);",
            "sin=3; x=sin(1);",
            "x=[ones(64) ones(64)];",
        ] {
            assert!(Parameters::from_script(text).is_err(), "accepted {text}");
        }
        assert_eq!(
            Parameters::default().evaluate("K").unwrap_err().code,
            "parameter_unresolved"
        );
        assert!(Parameters::default().evaluate("1; 2").is_err());
        assert!(
            Parameters::default()
                .evaluate(&format!("{}1{}", "(".repeat(100), ")".repeat(100)))
                .is_err()
        );
        for text in [
            format!("x={}1;", "-".repeat(1000)),
            format!("x={}1;", "~".repeat(1000)),
            format!("x=ones{};", "(1)".repeat(1000)),
            format!("x={}1;", "1+...\n".repeat(1000)),
            format!("{}x=1;{}", "if 1;".repeat(1000), "end;".repeat(1000)),
        ] {
            assert!(Parameters::from_script(&text).is_err());
        }
    }
}
