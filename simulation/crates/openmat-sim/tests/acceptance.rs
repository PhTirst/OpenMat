use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use openmat_sim::model::{Block, BlockKind, Connection, Model, Port, Position};
use openmat_sim::numeric::{EvalError, Instruction, Kernel, Program, ReferenceKernel};
use openmat_sim::{CollectionLimits, Runner, SimulationResult, compile};

fn first_order() -> Model {
    serde_json::from_str(include_str!("../../../examples/first-order.omsim.json")).unwrap()
}
fn counter() -> Model {
    serde_json::from_str(include_str!("../../../examples/delay-counter.omsim.json")).unwrap()
}
fn sampled() -> Model {
    serde_json::from_str(include_str!(
        "../../../examples/sampled-feedback.omsim.json"
    ))
    .unwrap()
}
fn simulate(model: &Model) -> SimulationResult {
    let plan = compile(model).unwrap();
    let kernel = ReferenceKernel::new(plan.program().clone());
    Runner::new(plan, kernel)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap()
}
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

#[test]
fn feedback_matches_analytic_solution_and_converges_at_fourth_order() {
    let mut model = first_order();
    let mut errors = Vec::new();
    for step in [0.2, 0.1, 0.05] {
        model.settings.max_step = step;
        let result = simulate(&model);
        let last = result.frames.last().unwrap();
        close(last.time, 1.0, 0.0);
        errors.push((last.values[0] - (1.0 - (-1.0_f64).exp())).abs());
        for frame in result.frames {
            close(frame.values[0], 1.0 - (-frame.time).exp(), 6e-6);
        }
    }
    assert!(errors[0] / errors[1] > 15.0);
    assert!(errors[1] / errors[2] > 15.0);
}

#[test]
fn delay_counter_changes_only_at_ticks_and_delays_exactly_one_period() {
    let result = simulate(&counter());
    let hits: Vec<_> = result.frames.iter().filter(|f| f.sample_hit).collect();
    assert_eq!(hits.len(), 4);
    for (index, frame) in hits.iter().enumerate() {
        let k = f64::from(u32::try_from(index).unwrap());
        close(frame.time, k * 0.1, 1e-15);
        close(frame.values[0], k, 0.0);
    }
    for frames in result.frames.windows(2) {
        assert!(frames[1].time > frames[0].time);
        if !frames[1].sample_hit {
            close(frames[1].values[0], frames[0].values[0], 0.0);
        }
    }
    close(result.frames.last().unwrap().time, 0.3, 0.0);
}

#[test]
fn sampled_feedback_is_independent_of_rk_trial_count() {
    let mut model = sampled();
    for step in [0.03, 0.07, 0.25] {
        model.settings.max_step = step;
        let result = simulate(&model);
        let hits: Vec<_> = result.frames.iter().filter(|f| f.sample_hit).collect();
        assert_eq!(hits.len(), 5);
        for ((frame, plant), held) in hits
            .iter()
            .zip([0.0, 0.1, 0.2, 0.29, 0.37])
            .zip([0.0, 0.0, 0.1, 0.2, 0.29])
        {
            close(frame.values[0], plant, 2e-15);
            close(frame.values[1], held, 2e-15);
        }
        close(result.frames.last().unwrap().time, 0.4, 0.0);
    }
}

#[test]
fn simultaneous_delays_do_not_depend_on_block_order() {
    let json = r#"{
      "schemaVersion":1,"name":"swap","settings":{"startTime":0,"stopTime":3,"maxStep":1,"sampleTime":1},
      "blocks":[
        {"id":"a","kind":{"type":"unitDelay","initial":[1]}},
        {"id":"b","kind":{"type":"unitDelay","initial":[2]}},
        {"id":"a_scope","kind":{"type":"scope"}},
        {"id":"b_scope","kind":{"type":"scope"}}],
      "connections":[
        {"from":{"block":"a","port":"out"},"to":{"block":"b","port":"in"}},
        {"from":{"block":"b","port":"out"},"to":{"block":"a","port":"in"}},
        {"from":{"block":"a","port":"out"},"to":{"block":"a_scope","port":"in"}},
        {"from":{"block":"b","port":"out"},"to":{"block":"b_scope","port":"in"}}] }"#;
    let mut model: Model = serde_json::from_str(json).unwrap();
    for _ in 0..2 {
        let result = simulate(&model);
        assert_eq!(
            result
                .frames
                .iter()
                .map(|f| f.values.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![1.0, 2.0],
                vec![2.0, 1.0],
                vec![1.0, 2.0],
                vec![2.0, 1.0]
            ]
        );
        model.blocks.reverse();
    }
}

#[test]
fn vector_signals_and_elementwise_gain_preserve_component_order() {
    let mut model = first_order();
    for block in &mut model.blocks {
        match &mut block.kind {
            BlockKind::Constant { value } => *value = vec![1.0, 2.0],
            BlockKind::Integrator { initial } => *initial = vec![0.0, 0.0],
            BlockKind::Gain { gain } => *gain = vec![1.0, 2.0],
            _ => {}
        }
    }
    let result = simulate(&model);
    assert_eq!(result.scopes[0].width, 2);
    for frame in result.frames {
        close(frame.values[0], 1.0 - (-frame.time).exp(), 1e-7);
        close(frame.values[1], 1.0 - (-2.0 * frame.time).exp(), 1e-6);
    }
}

#[test]
fn model_roundtrip_and_editor_changes_do_not_change_program() {
    let mut model = first_order();
    let original = compile(&model).unwrap();
    let json = serde_json::to_string(&model).unwrap();
    assert_eq!(serde_json::from_str::<Model>(&json).unwrap(), model);
    model.blocks.reverse();
    model.connections.reverse();
    model.blocks[0].position = Some(Position { x: 22.0, y: 7.0 });
    let changed = compile(&model).unwrap();
    assert_eq!(original.program(), changed.program());
    assert_eq!(original.execution_order(), changed.execution_order());
}

#[test]
fn instantaneous_loop_has_a_specific_block_and_port() {
    let mut model = first_order();
    let connection = model
        .connections
        .iter_mut()
        .find(|c| c.to.block == "gain")
        .unwrap();
    connection.from.block = "sum".into();
    let error = compile(&model).unwrap_err().0;
    assert_eq!(error.code, "algebraic_loop");
    assert!(["gain", "sum"].contains(&error.block.as_deref().unwrap()));
    assert!(error.port.is_some());
}

#[test]
fn diagnostics_cover_connection_and_shape_failures() {
    let mut cases = Vec::new();
    let mut model = first_order();
    model.blocks.push(model.blocks[0].clone());
    cases.push((model, "duplicate_block"));
    let mut model = first_order();
    model.connections[0].from.block = "absent".into();
    cases.push((model, "unknown_block"));
    let mut model = first_order();
    model.connections[0].from.port = "wrong".into();
    cases.push((model, "unknown_port"));
    let mut model = first_order();
    model.connections[0].to.port = "wrong".into();
    cases.push((model, "unknown_port"));
    let mut model = first_order();
    model.connections.push(model.connections[0].clone());
    cases.push((model, "multiple_drivers"));
    let mut model = first_order();
    model.connections.remove(0);
    cases.push((model, "missing_input"));
    let mut model = first_order();
    model
        .blocks
        .iter_mut()
        .find(|b| b.id == "state")
        .unwrap()
        .kind = BlockKind::Integrator {
        initial: vec![0.0, 0.0],
    };
    cases.push((model, "signal_width"));
    let mut model = first_order();
    model
        .blocks
        .iter_mut()
        .find(|b| b.id == "gain")
        .unwrap()
        .kind = BlockKind::Gain {
        gain: vec![1.0, 2.0],
    };
    cases.push((model, "signal_width"));
    for (model, code) in cases {
        let error = compile(&model).unwrap_err().0;
        assert_eq!(error.code, code);
        assert!(error.block.is_some(), "{code}");
        if code != "duplicate_block" {
            assert!(error.port.is_some(), "{code}");
        }
    }
}

#[test]
fn invalid_time_and_parameters_are_rejected_before_execution() {
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut model = first_order();
        model.settings.max_step = invalid;
        assert_eq!(compile(&model).unwrap_err().0.code, "time_step");
        let mut model = counter();
        model.settings.sample_time = Some(invalid);
        assert_eq!(compile(&model).unwrap_err().0.code, "sample_time");
    }
    let mut model = counter();
    model.settings.sample_time = None;
    assert_eq!(compile(&model).unwrap_err().0.code, "sample_time");
    let mut model = first_order();
    model.settings.stop_time = model.settings.start_time;
    assert_eq!(compile(&model).unwrap_err().0.code, "time_range");
    let mut model = first_order();
    model.schema_version = 999;
    assert_eq!(compile(&model).unwrap_err().0.code, "schema_version");
    let mut model = first_order();
    model.blocks[0].kind = BlockKind::Constant {
        value: vec![f64::NAN],
    };
    assert_eq!(compile(&model).unwrap_err().0.code, "parameter");
    let mut model = first_order();
    model.blocks[1].kind = BlockKind::Sum { signs: vec![0] };
    assert_eq!(compile(&model).unwrap_err().0.code, "sum_signs");
}

#[test]
fn schema_rejects_misspelled_properties() {
    let mut value = serde_json::to_value(first_order()).unwrap();
    value["settings"]["maxStepp"] = 1.0.into();
    assert!(serde_json::from_value::<Model>(value).is_err());
    let mut value = serde_json::to_value(first_order()).unwrap();
    value["blocks"][0]["kind"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<Model>(value).is_err());
}

fn connect(from: &str, to: &str, port: &str) -> Connection {
    Connection {
        from: Port {
            block: from.into(),
            port: "out".into(),
        },
        to: Port {
            block: to.into(),
            port: port.into(),
        },
    }
}

fn constant_model(width: usize) -> Model {
    let mut model = first_order();
    model.blocks = vec![Block {
        parent: None,
        id: "source".into(),
        kind: BlockKind::Constant {
            value: vec![1.0; width],
        },
        position: None,
    }];
    model.connections.clear();
    model
}

#[test]
fn compiler_bounds_large_operations_before_lowering_them() {
    let mut model = constant_model(20_000);
    model.blocks.push(Block {
        parent: None,
        id: "wide_sum".into(),
        kind: BlockKind::Sum { signs: vec![1; 64] },
        position: None,
    });
    model.connections = (0..64)
        .map(|i| connect("source", "wide_sum", &format!("in{i}")))
        .collect();
    let error = compile(&model).unwrap_err().0;
    assert_eq!(error.code, "model_limit");
    assert_eq!(error.block.as_deref(), Some("wide_sum"));
}

#[test]
fn compiler_bounds_fanout_observations_and_alias_signal_storage() {
    for kind in [BlockKind::Scope, BlockKind::Sum { signs: vec![1] }] {
        let mut model = constant_model(2048);
        for index in 0..500 {
            let id = format!("consumer_{index:03}");
            model.connections.push(connect(
                "source",
                &id,
                if matches!(kind, BlockKind::Scope) {
                    "in"
                } else {
                    "in0"
                },
            ));
            model.blocks.push(Block {
                parent: None,
                id,
                kind: kind.clone(),
                position: None,
            });
        }
        let error = compile(&model).unwrap_err().0;
        assert_eq!(error.code, "model_limit");
        assert!(error.block.unwrap().starts_with("consumer_"));
        if matches!(kind, BlockKind::Scope) {
            assert_eq!(error.port.as_deref(), Some("in"));
        }
    }
}

#[test]
fn nonzero_start_and_partial_final_interval_are_handled_once() {
    let mut model = counter();
    model.settings.start_time = 2.0;
    model.settings.stop_time = 2.25;
    let result = simulate(&model);
    let hits: Vec<_> = result.frames.iter().filter(|f| f.sample_hit).collect();
    assert_eq!(hits.len(), 3);
    close(hits[0].time, 2.0, 0.0);
    close(hits[2].time, 2.2, 1e-15);
    let final_frame = result.frames.last().unwrap();
    close(final_frame.time, 2.25, 0.0);
    close(final_frame.values[0], 2.0, 0.0);
    assert!(!final_frame.sample_hit);
}

#[test]
fn small_time_scales_are_not_coalesced_to_zero() {
    let mut model = counter();
    model.settings.stop_time = 3e-15;
    model.settings.sample_time = Some(1e-15);
    model.settings.max_step = 3e-16;
    let result = simulate(&model);
    let hits: Vec<_> = result.frames.iter().filter(|f| f.sample_hit).collect();
    assert_eq!(hits.len(), 4);
    close(hits[3].values[0], 3.0, 0.0);
    close(result.frames.last().unwrap().time, 3e-15, 0.0);
}

#[test]
fn numerical_verifier_rejects_invalid_ssa_and_lengths() {
    assert!(Program::new(1, vec![Instruction::Input(1)], vec![0]).is_err());
    assert!(Program::new(1, vec![Instruction::Add(0, 0)], vec![0]).is_err());
    assert!(Program::new(1, vec![Instruction::Constant(f64::NAN)], vec![0]).is_err());
    assert!(Program::new(1, vec![Instruction::Input(0)], vec![1]).is_err());
    let program = Program::new(1, vec![Instruction::Input(0)], vec![0]).unwrap();
    let mut kernel = ReferenceKernel::new(program);
    assert!(kernel.evaluate(&[], &mut [0.0]).is_err());
    assert!(kernel.evaluate(&[1.0], &mut []).is_err());
}

#[test]
fn mismatched_backend_and_unbounded_trajectory_are_rejected() {
    let plan = compile(&first_order()).unwrap();
    let other = compile(&counter()).unwrap();
    assert!(Runner::new(plan.clone(), ReferenceKernel::new(other.program().clone())).is_err());
    let runner = Runner::new(plan.clone(), ReferenceKernel::new(plan.program().clone())).unwrap();
    let error = runner
        .collect(CollectionLimits {
            max_samples: 2,
            max_values: 10,
        })
        .unwrap_err();
    assert_eq!(error.code, "output_limit");
}

#[test]
fn program_identity_distinguishes_positive_and_negative_zero_literals() {
    let mut model = constant_model(1);
    model.blocks[0].kind = BlockKind::Constant { value: vec![0.0] };
    let positive = compile(&model).unwrap();
    model.blocks[0].kind = BlockKind::Constant { value: vec![-0.0] };
    let negative = compile(&model).unwrap();
    assert_ne!(positive.program(), negative.program());
    let kernel = ReferenceKernel::new(negative.program().clone());
    let Err(error) = Runner::new(positive, kernel) else {
        panic!("a different constant sign must invalidate the compiled kernel");
    };
    assert_eq!(error.code, "kernel_mismatch");
}

struct CancellingKernel {
    inner: ReferenceKernel,
    flag: Arc<AtomicBool>,
    count: usize,
}
impl Kernel for CancellingKernel {
    fn program(&self) -> &Program {
        self.inner.program()
    }
    fn evaluate(&mut self, input: &[f64], output: &mut [f64]) -> Result<(), EvalError> {
        self.inner.evaluate(input, output)?;
        self.count += 1;
        if self.count == 3 {
            self.flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[test]
fn cancellation_during_rk_trials_is_terminal_and_does_not_commit_state() {
    let plan = compile(&first_order()).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let kernel = CancellingKernel {
        inner: ReferenceKernel::new(plan.program().clone()),
        flag: flag.clone(),
        count: 0,
    };
    let mut runner = Runner::new(plan, kernel).unwrap();
    assert_eq!(
        runner.advance_with_cancel(&flag).unwrap_err().code,
        "cancelled"
    );
    close(runner.time(), 0.0, 0.0);
    assert_eq!(runner.continuous_state(), &[0.0]);
    assert_eq!(runner.current_frame().values, vec![0.0]);
    assert!(runner.is_failed());
    assert_eq!(runner.advance().unwrap_err().code, "failed_run");
}

struct FailingKernel {
    inner: ReferenceKernel,
    count: usize,
    fail_on: usize,
}
impl Kernel for FailingKernel {
    fn program(&self) -> &Program {
        self.inner.program()
    }
    fn evaluate(&mut self, input: &[f64], output: &mut [f64]) -> Result<(), EvalError> {
        self.inner.evaluate(input, output)?;
        self.count += 1;
        if self.count == self.fail_on {
            return Err(EvalError("injected final-output failure".into()));
        }
        Ok(())
    }
}

#[test]
fn final_evaluation_failure_keeps_the_last_accepted_snapshot() {
    for (mut model, fail_on, successful_steps) in [
        (first_order(), 6, 0),
        (first_order(), 11, 1),
        (counter(), 2, 0),
        (counter(), 3, 1),
    ] {
        model.settings.max_step = 0.1;
        let plan = compile(&model).unwrap();
        let kernel = FailingKernel {
            inner: ReferenceKernel::new(plan.program().clone()),
            count: 0,
            fail_on,
        };
        let mut runner = Runner::new(plan, kernel).unwrap();
        for _ in 0..successful_steps {
            runner.advance().unwrap().unwrap();
        }
        let before = runner.current_frame();
        let continuous = runner.continuous_state().to_vec();
        let discrete = runner.discrete_output().to_vec();
        assert_eq!(runner.advance().unwrap_err().code, "kernel");
        let after = runner.current_frame();
        close(after.time, before.time, 0.0);
        assert_eq!(after.sample_hit, before.sample_hit);
        assert_eq!(after.values, before.values);
        assert_eq!(runner.continuous_state(), continuous);
        assert_eq!(runner.discrete_output(), discrete);
        assert!(runner.is_failed());
    }
}

#[test]
fn nonfinite_computation_is_diagnosed_at_observing_port() {
    let mut model = counter();
    model
        .blocks
        .iter_mut()
        .find(|b| b.id == "one")
        .unwrap()
        .kind = BlockKind::Constant {
        value: vec![f64::MAX],
    };
    model
        .blocks
        .iter_mut()
        .find(|b| b.id == "delay")
        .unwrap()
        .kind = BlockKind::UnitDelay {
        initial: vec![f64::MAX],
    };
    let plan = compile(&model).unwrap();
    let kernel = ReferenceKernel::new(plan.program().clone());
    let Err(error) = Runner::new(plan, kernel) else {
        panic!("overflow must fail");
    };
    assert_eq!(error.code, "non_finite_output");
    assert_eq!(error.block.as_deref(), Some("delay"));
    assert_eq!(error.port.as_deref(), Some("in"));
}
