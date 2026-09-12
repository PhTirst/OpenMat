#[allow(dead_code)]
mod support;
use openmat_sim::{CollectionLimits, Runner, compile_with_sources, numeric::ReferenceKernel};
use openmat_sim_slx::{ControlModel, ImportedSlx};
use support::{Fixture, SYSTEM};

fn block(kind: &str, sid: usize, properties: &str) -> String {
    format!(r#"<Block BlockType="{kind}" Name="{kind}{sid}" SID="{sid}">{properties}</Block>"#)
}
fn property(name: &str, text: &str) -> String {
    format!(r#"<P Name="{name}">{text}</P>"#)
}
fn edge(from: usize, out: usize, to: usize, input: usize) -> String {
    format!(r#"<Line><P Name="Src">{from}#out:{out}</P><P Name="Dst">{to}#in:{input}</P></Line>"#)
}
fn fixture(body: &str) -> Fixture {
    let mut f = Fixture::feedback();
    f.parts
        .insert(SYSTEM.into(), format!("<System>{body}</System>"));
    f
}
fn lower(body: &str, parameters: &str) -> ControlModel {
    ImportedSlx::read(&fixture(body).package(), "control")
        .unwrap()
        .lower_control(parameters)
        .unwrap()
}
fn run(m: &ControlModel) -> openmat_sim::SimulationResult {
    let plan = compile_with_sources(&m.model, &m.sources).unwrap();
    let update = plan.update_program().cloned().map(ReferenceKernel::new);
    Runner::new_with_update(
        plan.clone(),
        ReferenceKernel::new(plan.program().clone()),
        update,
    )
    .unwrap()
    .collect(CollectionLimits::default())
    .unwrap()
}

#[test]
fn parameters_are_explicit_and_legacy_profile_remains_strict() {
    let mut f = Fixture::feedback();
    f.edit(SYSTEM, "<P Name=\"Gain\">1</P>", "<P Name=\"Gain\">K</P>");
    let imported = ImportedSlx::read(&f.package(), "feedback").unwrap();
    assert!(imported.lower().is_err());
    let issue = imported.lower_control("").err().unwrap().remove(0);
    assert_eq!(issue.parameter.as_deref(), Some("Gain"));
    assert!(issue.message.contains("feedback/feedback"));
    let m = imported.lower_control("K=2;").unwrap();
    let result = run(&m);
    let end = result.frames.last().unwrap().values[0];
    assert!((end - 0.5 * (1.0 - (-2.0_f64).exp())).abs() < 2e-7);
    let mut f = Fixture::feedback();
    f.edit(
        support::CONFIG,
        "<P Name=\"FixedStep\">0.05</P>",
        "<P Name=\"FixedStep\">Ts</P>",
    );
    let errors = ImportedSlx::read(&f.package(), "unresolved")
        .unwrap()
        .lower_control("")
        .err()
        .unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "parameter_unresolved");
    assert_eq!(errors[0].parameter.as_deref(), Some("FixedStep"));
}

#[test]
fn virtual_boundary_fanout_and_nested_state_feedback() {
    let sub = block("Inport", 4, "")
        + &block("Integrator", 5, "")
        + &block("Outport", 6, "")
        + &edge(4, 1, 5, 1)
        + &edge(5, 1, 6, 1);
    let body = block("Constant", 1, "")
        + &block("SubSystem", 2, &format!("<System>{sub}</System>"))
        + &block("Scope", 3, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1);
    let imported = ImportedSlx::read(&fixture(&body).package(), "nested").unwrap();
    assert!(imported.lower().is_err());
    let m = imported.lower_control("").unwrap();
    assert_eq!(m.model.blocks.len(), 3);
    assert_eq!(m.block_paths["5"], "nested/SubSystem2/Integrator5");
    assert!((run(&m).frames.last().unwrap().values[0] - 1.0).abs() < 1e-12);
    let atomic = body.replace(
        "<System><Block BlockType=\"Inport\"",
        "<P Name=\"TreatAsAtomicUnit\">on</P><System><Block BlockType=\"Inport\"",
    );
    assert!(
        ImportedSlx::read(&fixture(&atomic).package(), "atomic")
            .unwrap()
            .lower_control("")
            .is_err()
    );
}

#[test]
fn routed_vectors_product_broadcast_and_terminator() {
    let body = block("Constant", 1, &property("Value", "[2 3]"))
        + &block("Demux", 2, &property("Outputs", "2"))
        + &block("Terminator", 3, "")
        + &block("Ground", 4, "")
        + &block("Mux", 5, &property("Inputs", "2"))
        + &block("Constant", 6, &property("Value", "4"))
        + &block("Product", 7, &property("Inputs", "**"))
        + &block("Scope", 8, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1)
        + &edge(2, 2, 5, 1)
        + &edge(4, 1, 5, 2)
        + &edge(5, 1, 7, 1)
        + &edge(6, 1, 7, 2)
        + &edge(7, 1, 8, 1);
    let m = lower(&body, "");
    assert_eq!(run(&m).frames[0].values, vec![12.0, 0.0]);
}

#[test]
fn pure_time_source_bias_division_and_zero_order_transfer() {
    let body = block(
        "Sin",
        1,
        &(property("SampleTime", "Ts0") + &property("Amplitude", "K") + &property("Phase", "pi/2")),
    ) + &block("Bias", 2, &property("Bias", "1"))
        + &block("Constant", 3, &property("Value", "2"))
        + &block("Product", 4, &property("Inputs", "*/"))
        + &block(
            "TransferFcn",
            5,
            &(property("Numerator", "3") + &property("Denominator", "2")),
        )
        + &block("Scope", 6, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 4, 1)
        + &edge(3, 1, 4, 2)
        + &edge(4, 1, 5, 1)
        + &edge(5, 1, 6, 1);
    let model = lower(&body, "K=2; Ts0=0;");
    let result = run(&model);
    for frame in result.frames {
        let expected = (2.0 * (frame.time + std::f64::consts::FRAC_PI_2).sin() + 1.0) * 0.75;
        assert!((frame.values[0] - expected).abs() < 2e-14);
    }
    let mut discrete = fixture(&body);
    discrete.edit(support::CONFIG, "ode4", "FixedStepDiscrete");
    let model = ImportedSlx::read(&discrete.package(), "static-transfer")
        .unwrap()
        .lower_control("K=2; Ts0=0;")
        .unwrap();
    assert!((run(&model).frames[0].values[0] - 2.25).abs() < 1e-14);
}

#[test]
fn omitted_transfer_coefficients_follow_r2022b_built_in_defaults() {
    let body = block("Constant", 1, &property("Value", "2"))
        + &block("TransferFcn", 2, "")
        + &block("Scope", 3, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1);
    let mut model = lower(&body, "");
    model.model.settings.max_step = 0.1;
    model.model.settings.stop_time = 0.1;
    assert!((run(&model).frames[1].values[0] - 0.009_358_333_333_333_333).abs() < 1e-14);
}

#[test]
fn state_space_initial_conditions_and_feedthrough() {
    let ss = property("A", "[-1 0;0 -2]")
        + &property("B", "[1;2]")
        + &property("C", "[1 1]")
        + &property("D", "3")
        + &property("InitialCondition", "[1;-1]");
    let body = block("Constant", 1, &property("Value", "2"))
        + &block("StateSpace", 2, &ss)
        + &block("Scope", 3, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1);
    assert_eq!(run(&lower(&body, "")).frames[0].values, vec![6.0]);
    let invalid = body.replace("[1;2]", "[1 2]");
    assert!(
        ImportedSlx::read(&fixture(&invalid).package(), "shape")
            .unwrap()
            .lower_control("")
            .is_err()
    );
    let feedback = block(
        "StateSpace",
        1,
        &(property("A", "-1") + &property("B", "1") + &property("C", "1") + &property("D", "0")),
    ) + &block("Scope", 2, "")
        + &edge(1, 1, 1, 1)
        + &edge(1, 1, 2, 1);
    assert!(
        ImportedSlx::read(&fixture(&feedback).package(), "feedback")
            .unwrap()
            .lower_control("")
            .is_ok()
    );
    let direct = feedback.replace("<P Name=\"D\">0</P>", "<P Name=\"D\">1</P>");
    let errors = ImportedSlx::read(&fixture(&direct).package(), "direct")
        .unwrap()
        .lower_control("")
        .err()
        .unwrap();
    assert_eq!(errors[0].code, "algebraic_loop");
}

#[test]
fn step_uses_r2022b_rk4_stage_time_rule() {
    let body = block(
        "Step",
        1,
        &(property("Time", "0.2") + &property("SampleTime", "0")),
    ) + &block("Integrator", 2, "")
        + &block("Scope", 3, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1);
    let mut m = lower(&body, "");
    m.model.settings.max_step = 0.1;
    m.model.settings.stop_time = 0.4;
    let result = run(&m);
    assert!((result.frames[2].values[0] - 1.0 / 60.0).abs() < 1e-14);
    assert!((result.frames[4].values[0] - (0.2 + 1.0 / 60.0)).abs() < 1e-14);
    m.model.schema_version = 3;
    assert!(compile_with_sources(&m.model, &m.sources).is_err());
}

#[test]
fn unsupported_configuration_and_missing_routing_width_are_diagnosed() {
    let body = block("Sin", 1, &property("TimeSource", "Use external signal"))
        + &block("Scope", 2, "")
        + &edge(1, 1, 2, 1);
    assert!(
        ImportedSlx::read(&fixture(&body).package(), "source")
            .unwrap()
            .lower_control("")
            .is_err()
    );
    let body = block("Constant", 1, &property("Value", "[1 2 3]"))
        + &block("Demux", 2, &property("Outputs", "2"))
        + &block("Scope", 3, "")
        + &edge(1, 1, 2, 1)
        + &edge(2, 1, 3, 1);
    assert!(
        ImportedSlx::read(&fixture(&body).package(), "routing")
            .unwrap()
            .lower_control("")
            .is_err()
    );
}
