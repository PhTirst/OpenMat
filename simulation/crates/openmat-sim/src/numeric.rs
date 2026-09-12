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
                Instruction::Add(a, b) | Instruction::Multiply(a, b) => a < index && b < index,
                Instruction::Negate(a) => a < index,
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
