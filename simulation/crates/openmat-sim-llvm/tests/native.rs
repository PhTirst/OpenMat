//! Explicit acceptance: these tests never replace missing LLVM with an interpreter.
use std::path::PathBuf;

use openmat_sim::model::{BlockKind, Model};
use openmat_sim::numeric::{Instruction, Kernel, Program, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, compile};
use openmat_sim_llvm::LlvmKernel;

fn library() -> PathBuf {
    std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY")
        .expect("native acceptance requires OPENMAT_SIM_LLVM_LIBRARY; run simulation/tools/Prepare-Llvm.ps1")
        .into()
}

#[test]
fn missing_native_library_is_an_error() {
    let program = Program::new(1, vec![Instruction::Input(0)], vec![0]).unwrap();
    assert!(
        LlvmKernel::compile(
            program,
            std::path::Path::new("missing-openmat-llvm-test-library.dll")
        )
        .is_err()
    );
}

#[test]
#[ignore = "explicit native acceptance requires LLVM 22; CI runs with --ignored"]
fn native_trajectories_match_reference_for_continuous_discrete_and_mixed_models() {
    let mut models: Vec<Model> = [
        include_str!("../../../examples/first-order.omsim.json"),
        include_str!("../../../examples/delay-counter.omsim.json"),
        include_str!("../../../examples/sampled-feedback.omsim.json"),
    ]
    .iter()
    .map(|source| serde_json::from_str(source).unwrap())
    .collect();
    let mut vector = models[0].clone();
    for block in &mut vector.blocks {
        match &mut block.kind {
            BlockKind::Constant { value } => *value = vec![1.0, 2.0],
            BlockKind::Integrator { initial } => *initial = vec![0.0, 0.0],
            BlockKind::Gain { gain } => *gain = vec![1.0, 2.0],
            _ => {}
        }
    }
    models.push(vector);
    for model in models {
        let plan = compile(&model).unwrap();
        let native = LlvmKernel::compile(plan.program().clone(), &library()).unwrap();
        native.verify_abi_guards().unwrap();
        assert!(native.version().starts_with("22."));
        assert!(native.target_triple().starts_with("x86_64"));
        println!(
            "native model={} LLVM={} target={}",
            model.name,
            native.version(),
            native.target_triple()
        );
        let reference = ReferenceKernel::new(plan.program().clone());
        let expected = Runner::new(plan.clone(), reference)
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        let actual = Runner::new(plan, native)
            .unwrap()
            .collect(CollectionLimits::default())
            .unwrap();
        assert_eq!(actual.scopes, expected.scopes);
        assert_eq!(actual.frames.len(), expected.frames.len());
        for (actual, expected) in actual.frames.iter().zip(&expected.frames) {
            assert_eq!(actual.time.to_bits(), expected.time.to_bits());
            assert_eq!(actual.sample_hit, expected.sample_hit);
            assert_eq!(actual.values.len(), expected.values.len());
            for (&a, &b) in actual.values.iter().zip(&expected.values) {
                assert!((a - b).abs() <= 1e-12, "native={a} reference={b}");
            }
        }
    }
}

#[test]
#[ignore = "explicit native acceptance requires LLVM 22; CI runs with --ignored"]
fn native_arithmetic_preserves_signed_zero_and_strict_operation_order() {
    let program = Program::new(
        2,
        vec![
            Instruction::Input(0),
            Instruction::Input(1),
            Instruction::Constant(-0.0),
            Instruction::Negate(0),
            Instruction::Add(0, 1),
            Instruction::Multiply(0, 1),
            Instruction::Multiply(4, 5),
            Instruction::Add(6, 2),
        ],
        vec![2, 3, 4, 5, 6, 7],
    )
    .unwrap();
    let mut native = LlvmKernel::compile(program.clone(), &library()).unwrap();
    let mut reference = ReferenceKernel::new(program);
    for input in [
        [0.0, -0.0],
        [-0.0, 1.0],
        [1e-300, 1e-20],
        [0.1, 0.2],
        [-3.5, 7.25],
        [1e50, 1e-50],
    ] {
        let mut expected = [0.0; 6];
        let mut actual = [0.0; 6];
        reference.evaluate(&input, &mut expected).unwrap();
        native.evaluate(&input, &mut actual).unwrap();
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
    }
    assert!(native.evaluate(&[], &mut [0.0; 6]).is_err());
    native.verify_abi_guards().unwrap();
}

#[test]
#[ignore = "explicit native acceptance requires LLVM 22; CI runs with --ignored"]
fn native_empty_program_and_zero_length_buffers_are_valid() {
    let program = Program::new(0, vec![], vec![]).unwrap();
    let mut kernel = LlvmKernel::compile(program, &library()).unwrap();
    kernel.verify_abi_guards().unwrap();
    kernel.evaluate(&[], &mut []).unwrap();
}

#[test]
#[ignore = "explicit native acceptance requires LLVM 22; CI runs with --ignored"]
fn native_code_and_context_lifetimes_survive_independent_concurrent_instances() {
    let workers: Vec<_> = (0..4)
        .map(|worker| {
            std::thread::spawn(move || {
                for iteration in 0..8 {
                    let constant = f64::from(worker * 8 + iteration);
                    let program = Program::new(
                        1,
                        vec![
                            Instruction::Input(0),
                            Instruction::Constant(constant),
                            Instruction::Add(0, 1),
                        ],
                        vec![2],
                    )
                    .unwrap();
                    let mut kernel = LlvmKernel::compile(program, &library()).unwrap();
                    let mut output = [0.0];
                    kernel.evaluate(&[3.0], &mut output).unwrap();
                    assert_eq!(output[0].to_bits(), (3.0 + constant).to_bits());
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}
