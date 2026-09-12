use std::fmt::Write;

use openmat_sim::numeric::{Instruction, KERNEL_ABI_VERSION, Program};

/// Emit standalone LLVM IR for a verified program and C kernel ABI v1.
#[must_use]
pub fn emit_llvm(program: &Program) -> String {
    let mut ir = String::from(
        "; OpenMat numerical kernel, C ABI v1. Strict f64 operations.\n\ndefine i32 @openmat_sim_eval(i32 %abi, ptr %inputs, i64 %input_count, ptr %outputs, i64 %output_count) {\nentry:\n",
    );
    writeln!(ir, "  %abi_ok = icmp eq i32 %abi, {KERNEL_ABI_VERSION}\n  br i1 %abi_ok, label %lengths, label %bad_abi\nlengths:").unwrap();
    writeln!(ir, "  %input_ok = icmp eq i64 %input_count, {}\n  %output_ok = icmp eq i64 %output_count, {}\n  %length_ok = and i1 %input_ok, %output_ok\n  br i1 %length_ok, label %pointers, label %bad_length\npointers:", program.input_count(), program.output_count()).unwrap();
    writeln!(ir, "  %input_null = icmp eq ptr %inputs, null\n  %output_null = icmp eq ptr %outputs, null\n  %need_input = and i1 %input_null, {}\n  %need_output = and i1 %output_null, {}\n  %invalid_pointer = or i1 %need_input, %need_output\n  br i1 %invalid_pointer, label %bad_pointer, label %compute\ncompute:", program.input_count() != 0, program.output_count() != 0).unwrap();
    // Constants are operands rather than fadd aliases, preserving signed zero.
    let operands: Vec<String> = program
        .instructions()
        .iter()
        .enumerate()
        .map(|(index, op)| {
            if let Instruction::Constant(value) = op {
                format!("0x{:016X}", value.to_bits())
            } else {
                format!("%v{index}")
            }
        })
        .collect();
    for (index, instruction) in program.instructions().iter().enumerate() {
        match *instruction {
            Instruction::Input(input) => {
                writeln!(ir, "  %p{index} = getelementptr double, ptr %inputs, i64 {input}\n  %v{index} = load double, ptr %p{index}, align 8").unwrap();
            }
            Instruction::Constant(_) => {}
            Instruction::Add(a, b) => {
                writeln!(
                    ir,
                    "  %v{index} = fadd double {}, {}",
                    operands[a], operands[b]
                )
                .unwrap();
            }
            Instruction::Multiply(a, b) => {
                writeln!(
                    ir,
                    "  %v{index} = fmul double {}, {}",
                    operands[a], operands[b]
                )
                .unwrap();
            }
            Instruction::Negate(a) => {
                writeln!(ir, "  %v{index} = fneg double {}", operands[a]).unwrap();
            }
        }
    }
    for (index, &value) in program.outputs().iter().enumerate() {
        writeln!(ir, "  %o{index} = getelementptr double, ptr %outputs, i64 {index}\n  store double {}, ptr %o{index}, align 8", operands[value]).unwrap();
    }
    ir.push_str("  ret i32 0\nbad_abi:\n  ret i32 1\nbad_length:\n  ret i32 2\nbad_pointer:\n  ret i32 3\n}\n");
    ir
}
