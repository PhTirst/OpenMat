//! Verified f64 SSA shared by the reference and native execution backends.
use std::fmt;

pub const KERNEL_ABI_VERSION: u32 = 1;
pub(crate) const MAX_VALUES: usize = 1_000_000;

#[derive(Clone, Debug)]
pub enum Instruction {
    Input(usize),
    Constant(f64),
    Add(usize, usize),
    Multiply(usize, usize),
    Negate(usize),
    Subtract(usize, usize),
    Divide(usize, usize),
    Math(MathFunction, usize),
    Compare(Comparison, usize, usize),
    Select(usize, usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    #[must_use]
    #[allow(clippy::float_cmp)] // Language comparisons use IEEE equality, not tolerances.
    pub fn evaluate(self, a: f64, b: f64) -> f64 {
        f64::from(match self {
            Self::Equal => a == b,
            Self::NotEqual => a != b,
            Self::Less => a < b,
            Self::LessEqual => a <= b,
            Self::Greater => a > b,
            Self::GreaterEqual => a >= b,
        })
    }
}

impl Instruction {
    #[must_use]
    pub fn operands(&self) -> Vec<usize> {
        match *self {
            Self::Input(_) | Self::Constant(_) => vec![],
            Self::Negate(a) | Self::Math(_, a) => vec![a],
            Self::Add(a, b)
            | Self::Subtract(a, b)
            | Self::Multiply(a, b)
            | Self::Divide(a, b)
            | Self::Compare(_, a, b) => vec![a, b],
            Self::Select(c, a, b) => vec![c, a, b],
        }
    }

    pub(crate) fn remap(&self, mut map: impl FnMut(usize) -> usize) -> Self {
        match *self {
            Self::Input(i) => Self::Input(i),
            Self::Constant(v) => Self::Constant(v),
            Self::Negate(a) => Self::Negate(map(a)),
            Self::Math(f, a) => Self::Math(f, map(a)),
            Self::Add(a, b) => Self::Add(map(a), map(b)),
            Self::Subtract(a, b) => Self::Subtract(map(a), map(b)),
            Self::Multiply(a, b) => Self::Multiply(map(a), map(b)),
            Self::Divide(a, b) => Self::Divide(map(a), map(b)),
            Self::Compare(c, a, b) => Self::Compare(c, map(a), map(b)),
            Self::Select(c, a, b) => Self::Select(map(c), map(a), map(b)),
        }
    }

    pub(crate) fn constant(&self, values: &[Option<f64>]) -> Option<f64> {
        Some(match *self {
            Self::Input(_) => return None,
            Self::Constant(v) => v,
            Self::Add(a, b) => values[a]? + values[b]?,
            Self::Subtract(a, b) => values[a]? - values[b]?,
            Self::Multiply(a, b) => values[a]? * values[b]?,
            Self::Divide(a, b) => values[a]? / values[b]?,
            Self::Negate(a) => -values[a]?,
            Self::Math(f, a) => f.evaluate(values[a]?),
            Self::Compare(c, a, b) => c.evaluate(values[a]?, values[b]?),
            Self::Select(c, a, b) => values[if values[c]? == 0.0 { b } else { a }]?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathFunction {
    Sin,
    Cos,
    Exp,
    Sqrt,
    Abs,
    Tanh,
}

impl MathFunction {
    #[must_use]
    pub fn evaluate(self, value: f64) -> f64 {
        match self {
            Self::Sin => value.sin(),
            Self::Cos => value.cos(),
            Self::Exp => value.exp(),
            Self::Sqrt => value.sqrt(),
            Self::Abs => value.abs(),
            Self::Tanh => value.tanh(),
        }
    }
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Exp => "exp",
            Self::Sqrt => "sqrt",
            Self::Abs => "abs",
            Self::Tanh => "tanh",
        }
    }
}

impl PartialEq for Instruction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Input(a), Self::Input(b)) | (Self::Negate(a), Self::Negate(b)) => a == b,
            // Program identity includes signed zero; IEEE numeric equality alone
            // could attach a kernel with different literal bits to a model.
            (Self::Constant(a), Self::Constant(b)) => a.to_bits() == b.to_bits(),
            (Self::Add(a, b), Self::Add(c, d)) | (Self::Multiply(a, b), Self::Multiply(c, d)) => {
                a == c && b == d
            }
            (Self::Subtract(a, b), Self::Subtract(c, d))
            | (Self::Divide(a, b), Self::Divide(c, d)) => a == c && b == d,
            (Self::Math(f, a), Self::Math(g, b)) => f == g && a == b,
            (Self::Compare(lhs, a, b), Self::Compare(rhs, c, d)) => lhs == rhs && a == c && b == d,
            (Self::Select(cond, a, b), Self::Select(other_cond, c, d)) => {
                cond == other_cond && a == c && b == d
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    input_count: usize,
    instructions: Vec<Instruction>,
    outputs: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError(pub String);

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for EvalError {}

impl Program {
    /// Verifies bounds and SSA dominance before either backend can execute IR.
    ///
    /// # Errors
    /// Returns an error for invalid references, non-finite literals or size limits.
    pub fn new(
        input_count: usize,
        instructions: Vec<Instruction>,
        outputs: Vec<usize>,
    ) -> Result<Self, EvalError> {
        if input_count > MAX_VALUES || instructions.len() > MAX_VALUES || outputs.len() > MAX_VALUES
        {
            return Err(EvalError(
                "numerical program exceeds the v0 value limit".into(),
            ));
        }
        for (index, instruction) in instructions.iter().enumerate() {
            let valid = match *instruction {
                Instruction::Input(input) => input < input_count,
                Instruction::Constant(value) => value.is_finite(),
                Instruction::Add(a, b)
                | Instruction::Multiply(a, b)
                | Instruction::Subtract(a, b)
                | Instruction::Divide(a, b)
                | Instruction::Compare(_, a, b) => a < index && b < index,
                Instruction::Negate(a) | Instruction::Math(_, a) => a < index,
                Instruction::Select(c, a, b) => c < index && a < index && b < index,
            };
            if !valid {
                return Err(EvalError(format!("invalid numerical instruction {index}")));
            }
        }
        if outputs.iter().any(|&id| id >= instructions.len()) {
            return Err(EvalError("invalid numerical output reference".into()));
        }
        Ok(Self {
            input_count,
            instructions,
            outputs,
        })
    }

    #[must_use]
    pub fn input_count(&self) -> usize {
        self.input_count
    }
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    #[must_use]
    pub fn outputs(&self) -> &[usize] {
        &self.outputs
    }

    /// Remove instructions that cannot contribute to the selected outputs.
    #[must_use]
    pub fn pruned(&self) -> Self {
        let mut used = vec![false; self.instructions.len()];
        for &id in &self.outputs {
            used[id] = true;
        }
        for i in (0..used.len()).rev() {
            if used[i] {
                for id in self.instructions[i].operands() {
                    used[id] = true;
                }
            }
        }
        let mut mapping = vec![0; used.len()];
        let mut instructions = Vec::new();
        for (i, op) in self.instructions.iter().enumerate() {
            if used[i] {
                mapping[i] = instructions.len();
                instructions.push(op.remap(|id| mapping[id]));
            }
        }
        Self {
            input_count: self.input_count,
            instructions,
            outputs: self.outputs.iter().map(|&id| mapping[id]).collect(),
        }
    }
}

/// Implementations must evaluate without changing simulation state or inputs.
pub trait Kernel {
    fn program(&self) -> &Program;
    /// # Errors
    /// Returns an error on incompatible buffers or a backend execution failure.
    fn evaluate(&mut self, inputs: &[f64], outputs: &mut [f64]) -> Result<(), EvalError>;
}

pub struct ReferenceKernel {
    program: Program,
    registers: Vec<f64>,
}

impl ReferenceKernel {
    #[must_use]
    pub fn new(program: Program) -> Self {
        let registers = vec![0.0; program.instructions.len()];
        Self { program, registers }
    }
}

impl Kernel for ReferenceKernel {
    fn program(&self) -> &Program {
        &self.program
    }

    fn evaluate(&mut self, inputs: &[f64], outputs: &mut [f64]) -> Result<(), EvalError> {
        if inputs.len() != self.program.input_count || outputs.len() != self.program.output_count()
        {
            return Err(EvalError("numerical kernel buffer length mismatch".into()));
        }
        for (index, instruction) in self.program.instructions.iter().enumerate() {
            self.registers[index] = match *instruction {
                Instruction::Input(input) => inputs[input],
                Instruction::Constant(value) => value,
                Instruction::Add(a, b) => self.registers[a] + self.registers[b],
                Instruction::Multiply(a, b) => self.registers[a] * self.registers[b],
                Instruction::Negate(a) => -self.registers[a],
                Instruction::Subtract(a, b) => self.registers[a] - self.registers[b],
                Instruction::Divide(a, b) => self.registers[a] / self.registers[b],
                Instruction::Math(function, a) => function.evaluate(self.registers[a]),
                Instruction::Compare(c, a, b) => c.evaluate(self.registers[a], self.registers[b]),
                Instruction::Select(c, a, b) => {
                    self.registers[if self.registers[c] == 0.0 { b } else { a }]
                }
            };
        }
        for (output, &id) in outputs.iter_mut().zip(&self.program.outputs) {
            *output = self.registers[id];
        }
        Ok(())
    }
}

impl<T: Kernel + ?Sized> Kernel for Box<T> {
    fn program(&self) -> &Program {
        (**self).program()
    }
    fn evaluate(&mut self, inputs: &[f64], outputs: &mut [f64]) -> Result<(), EvalError> {
        (**self).evaluate(inputs, outputs)
    }
}
