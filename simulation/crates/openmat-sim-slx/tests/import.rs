mod support;

use openmat_sim::model::BlockKind;
use openmat_sim::numeric::ReferenceKernel;
use openmat_sim::{CollectionLimits, Runner, SimulationResult, compile};
use openmat_sim_slx::ImportedSlx;
use support::{CONFIG, Fixture, MAIN, RELEASE, SYSTEM};

fn read(fixture: &Fixture) -> ImportedSlx {
    ImportedSlx::read(&fixture.package(), "test-model").unwrap()
}
fn simulate(imported: &ImportedSlx) -> SimulationResult {
    let model = imported.lower().unwrap();
    let plan = compile(&model).unwrap();
    let kernel = ReferenceKernel::new(plan.program().clone());
    Runner::new(plan, kernel)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap()
}
fn rejects(fixture: &Fixture, code: &str) {
    let issues = read(fixture).lower().unwrap_err();
    assert!(
        issues.iter().any(|e| e.code == code),
        "expected {code}, got {issues:?}"
    );
}

#[test]
fn follows_relationships_preserves_display_data_and_flattens_branches() {
    let fixture = Fixture::feedback();
    let imported = read(&fixture);
    let document = imported.document();
    assert_eq!(document.matlab_release.as_deref(), Some("R2022b"));
    assert_eq!(document.systems[0].blocks[0].name, "输入 & source");
    assert_eq!(document.systems[0].lines[2].branches.len(), 2);
    assert_eq!(
        document.configuration_objects[0].properties["Description"],
        "first"
    );
    assert_eq!(
        document.configuration_objects[1].properties["Description"],
        "second"
    );
    let source = &document.systems[0].blocks[2].source;
    let part = imported.package().part(&source.part).unwrap().bytes();
    let text = std::str::from_utf8(&part[source.start..source.end]).unwrap();
    assert!(text.contains("BlockType=\"Integrator\""));
    assert_eq!(part, fixture.parts[SYSTEM].as_bytes());
    let model = imported.lower().unwrap();
    assert_eq!(model.connections.len(), 5);
    assert_eq!(model.blocks[2].id, "slx_3");
    assert_eq!(
        model.blocks[2].position.as_ref().unwrap().x.to_bits(),
        190.0_f64.to_bits()
    );
    let result = simulate(&imported);
    assert_eq!(result.frames.len(), 21);
    for frame in result.frames {
        assert!((frame.values[0] - (1.0 - (-frame.time).exp())).abs() < 3e-8);
    }
}

#[test]
fn vector_feedback_expands_scalar_initial_conditions() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, r#"Name="Value">1"#, r#"Name="Value">[1 2]"#);
    fixture.edit(SYSTEM, r#"Name="Gain">1"#, r#"Name="Gain">[1;2]"#);
    let imported = read(&fixture);
    let model = imported.lower().unwrap();
    match &model.blocks[2].kind {
        BlockKind::Integrator { initial } => assert_eq!(initial.len(), 2),
        _ => panic!("expected Integrator"),
    }
    for frame in simulate(&imported).frames {
        assert_eq!(frame.values.len(), 2);
        for (&actual, rate) in frame.values.iter().zip([1.0, 2.0]) {
            assert!((actual - (1.0 - (-rate * frame.time).exp())).abs() < 4e-7);
        }
    }
}

#[test]
fn delay_emits_initial_state_and_updates_only_at_explicit_ticks() {
    let mut fixture = Fixture::feedback();
    fixture.edit(CONFIG, "ode4", "FixedStepDiscrete");
    fixture.edit(CONFIG, r#"Name="StopTime">1"#, r#"Name="StopTime">0.3"#);
    fixture.edit(SYSTEM, "Integrator", "UnitDelay");
    fixture.edit(
        SYSTEM,
        r#"Name="Position">"#,
        r#"Name="SampleTime">0.1</P><P Name="Position">"#,
    );
    fixture.edit(SYSTEM, r#"Name="Inputs">+-"#, r#"Name="Inputs">++"#);
    let result = simulate(&read(&fixture));
    assert_eq!(result.frames.iter().filter(|f| f.sample_hit).count(), 4);
    for (frame, expected) in result
        .frames
        .iter()
        .filter(|f| f.sample_hit)
        .zip([0.0_f64, 1.0, 2.0, 3.0])
    {
        assert_eq!(frame.values[0].to_bits(), expected.to_bits());
    }
    for pair in result.frames.windows(2) {
        if !pair[1].sample_hit {
            assert_eq!(pair[0].values, pair[1].values);
        }
    }
}

#[test]
fn omitted_r2022b_defaults_and_inline_systems_work() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, r#"<P Name="Value">1</P>"#, "");
    fixture.edit(SYSTEM, r#"<P Name="Gain">1</P>"#, "");
    let system = fixture.parts.remove(SYSTEM).unwrap();
    fixture.parts.remove("model/_rels/document.xml.rels");
    fixture.edit(MAIN, r#"<System Ref="top"/>"#, &system);
    assert_eq!(simulate(&read(&fixture)).frames.len(), 21);
}

#[test]
fn unsupported_blocks_and_binary_assets_remain_inspectable() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, "Integrator", "FutureBlock");
    fixture.edit(
        SYSTEM,
        r#"Name="Position">"#,
        r#"Name="FutureSetting">keep me</P><P Name="Position">"#,
    );
    fixture
        .parts
        .insert("assets/unknown.bin".into(), "opaque bytes".into());
    let imported = read(&fixture);
    let issues = imported.lower().unwrap_err();
    let issue = issues
        .iter()
        .find(|i| i.code == "unsupported_block")
        .unwrap();
    assert_eq!(issue.block.as_deref(), Some("3"));
    assert_eq!(issue.part.as_deref(), Some("/graphs/top.xml"));
    assert_eq!(
        imported.document().systems[0].blocks[2].properties["FutureSetting"],
        "keep me"
    );
    assert_eq!(
        imported
            .package()
            .part("/assets/unknown.bin")
            .unwrap()
            .bytes(),
        b"opaque bytes"
    );
    assert!(
        serde_json::to_string(imported.document())
            .unwrap()
            .contains("FutureSetting")
    );
}

#[test]
fn rejects_execution_changes_instead_of_substituting_semantics() {
    let mutations = [
        (CONFIG, "ode4", "ode45", "solver"),
        (
            CONFIG,
            r#"Name="FixedStep">0.05"#,
            r#"Name="FixedStep">auto"#,
            "parameter_expression",
        ),
        (
            CONFIG,
            r#"Name="StartTime">0"#,
            r#"Name="StartTime">1"#,
            "start_time",
        ),
        (
            CONFIG,
            r#"Name="LoadInitialState">off"#,
            r#"Name="LoadInitialState">on"#,
            "configuration",
        ),
        (RELEASE, "R2022b", "R2024a", "slx_release"),
        (MAIN, "normal", "accelerator", "simulation_mode"),
        (
            SYSTEM,
            r#"Name="Gain">1"#,
            r#"Name="Gain">gain_from_workspace"#,
            "parameter_expression",
        ),
        (
            SYSTEM,
            r#"Name="Gain">1"#,
            r#"Name="Multiplication">Matrix(K*u)</P><P Name="Gain">1"#,
            "unsupported_parameter",
        ),
        (
            SYSTEM,
            r#"Name="Gain">1"#,
            r#"Name="Gain">[1 2;3 4]"#,
            "literal_shape",
        ),
        (
            SYSTEM,
            r#"Name="Value">1"#,
            r#"Name="OutDataTypeStr">single</P><P Name="Value">1"#,
            "unsupported_parameter",
        ),
        (
            SYSTEM,
            r#"Name="Inputs">+-"#,
            r#"Name="Inputs">1"#,
            "sum_inputs",
        ),
        (
            SYSTEM,
            r#"Name="Position">"#,
            r#"Name="ExternalReset">rising</P><P Name="Position">"#,
            "unsupported_parameter",
        ),
        (
            SYSTEM,
            r#"Name="Position">"#,
            r#"Name="LimitOutput">on</P><P Name="Position">"#,
            "unsupported_parameter",
        ),
        (
            SYSTEM,
            r#"Name="Name">observed"#,
            r#"Name="MustResolveToSignalObject">on"#,
            "port_feature",
        ),
        (
            SYSTEM,
            "<Branch>",
            r#"<Branch><P Name="Src">1#out:1</P>"#,
            "line_source",
        ),
    ];
    for (part, from, to, code) in mutations {
        let mut fixture = Fixture::feedback();
        fixture.edit(part, from, to);
        rejects(&fixture, code);
    }
}

#[test]
fn rejects_callbacks_custom_defaults_and_external_dependencies_without_execution() {
    let mut callback = Fixture::feedback();
    callback.edit(
        MAIN,
        "<Model>",
        r#"<Model><P Name="InitFcn">error('not executed')</P>"#,
    );
    rejects(&callback, "initialization");
    let mut defaults = Fixture::feedback();
    defaults.parts.insert(
        "settings/defaults.xml".into(),
        "<BlockDiagramDefaults><Block BlockType=\"Gain\"/></BlockDiagramDefaults>".into(),
    );
    rejects(&defaults, "custom_defaults");
    let mut external = Fixture::feedback();
    external.add_relationship(
        "external",
        "urn:test",
        "https://example.invalid/never-fetch",
        true,
    );
    rejects(&external, "external_dependency");
    let mut dictionary = Fixture::feedback();
    dictionary.parts.insert(
        "model/dictionary.xml".into(),
        "<MF0><System><Interface><Parameter/></Interface></System></MF0>".into(),
    );
    dictionary.add_relationship(
        "dictionary",
        "http://schemas.mathworks.com/simulinkModel/2016/relationships/modelDictionary",
        "model/dictionary.xml",
        false,
    );
    rejects(&dictionary, "model_dictionary");
}

#[test]
fn delay_inheritance_multiple_rates_and_noninteger_rates_are_explicitly_rejected() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, "Integrator", "UnitDelay");
    rejects(&fixture, "sample_time");
    fixture.edit(
        SYSTEM,
        r#"Name="Position">"#,
        r#"Name="SampleTime">0.075</P><P Name="Position">"#,
    );
    rejects(&fixture, "sample_time");
    fixture.edit(SYSTEM, ">0.075</P>", ">0.1</P>");
    fixture.edit(SYSTEM, "</System>", r#"<Block BlockType="UnitDelay" Name="other" SID="6"><P Name="SampleTime">0.2</P></Block></System>"#);
    rejects(&fixture, "multiple_rates");
}

#[test]
fn validates_ports_and_algebraic_loops_after_translation() {
    let mut port = Fixture::feedback();
    port.edit(SYSTEM, "2#in:2", "2#in:3");
    rejects(&port, "unknown_port");
    let mut algebraic = Fixture::feedback();
    algebraic.edit(SYSTEM, "Integrator", "Gain");
    rejects(&algebraic, "algebraic_loop");
    let mut missing = Fixture::feedback();
    missing.edit(SYSTEM, "1#out:1", "99#out:1");
    rejects(&missing, "endpoint");
}

#[test]
fn preserves_subsystems_but_does_not_flatten_their_execution() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, "</System>", r#"<Block BlockType="SubSystem" Name="child" SID="6"><System Ref="child"/></Block></System>"#);
    fixture.parts.insert(
        "graphs/child.xml".into(),
        r#"<System><Block BlockType="Constant" Name="nested" SID="6:1"/></System>"#.into(),
    );
    fixture.add_system_relationship("graphs/_rels/top.xml.rels", "child", "child.xml");
    let imported = read(&fixture);
    assert_eq!(imported.document().systems.len(), 2);
    assert_eq!(
        imported.document().systems[1].parent_block.as_deref(),
        Some("6")
    );
    rejects(&fixture, "subsystem");
    fixture.edit("graphs/_rels/top.xml.rels", "child.xml", "top.xml");
    assert_eq!(
        ImportedSlx::read(&fixture.package(), "cycle")
            .err()
            .unwrap()
            .code,
        "slx_system"
    );
}

#[test]
fn malformed_structure_is_distinct_from_unsupported_execution() {
    for (part, from, to, code) in [
        (SYSTEM, "SID=\"2\"", "SID=\"1\"", "slx_block"),
        (MAIN, "Ref=\"top\"", "Ref=\"missing\"", "slx_system"),
        (
            CONFIG,
            "</Array>",
            r#"<Object ClassName="Simulink.SolverCC"><P Name="SolverName">ode4</P></Object></Array>"#,
            "slx_configuration",
        ),
        (
            SYSTEM,
            r#"<P Name="Gain">1</P>"#,
            r#"<P Name="Gain">1</P><P Name="Gain">2</P>"#,
            "duplicate_property",
        ),
    ] {
        let mut fixture = Fixture::feedback();
        fixture.edit(part, from, to);
        assert_eq!(
            ImportedSlx::read(&fixture.package(), "invalid")
                .err()
                .unwrap()
                .code,
            code
        );
    }
}

#[test]
fn structured_parameters_and_non_grid_stop_times_cannot_be_misread() {
    for (part, from, to, code) in [
        (
            SYSTEM,
            r#"Name="Gain">1"#,
            r#"Name="Gain" Ref="unresolved">1"#,
            "property_encoding",
        ),
        (
            CONFIG,
            r#"Name="FixedStep">0.05"#,
            r#"Name="FixedStep">0.05<Object/>"#,
            "property_encoding",
        ),
        (
            CONFIG,
            r#"Name="StopTime">1"#,
            r#"Name="StopTime">0.123"#,
            "stop_time",
        ),
        (
            SYSTEM,
            r#"Name="Value">1"#,
            r#"Name="Value">[1,,2]"#,
            "literal_shape",
        ),
        (
            SYSTEM,
            r#"Name="Value">1"#,
            r#"Name="Value">[1,]"#,
            "literal_shape",
        ),
    ] {
        let mut fixture = Fixture::feedback();
        fixture.edit(part, from, to);
        // Structured configuration uses an Object with its own type information.
        if to.contains("<Object/>") {
            fixture.edit(CONFIG, "<Object/>", r#"<Object ClassName="OpenMat.Test"/>"#);
        }
        rejects(&fixture, code);
    }
}

#[test]
fn inferred_state_buffers_are_bounded_before_expansion() {
    let mut fixture = Fixture::feedback();
    let vector = format!("[{}]", vec!["1"; 140_000].join(" "));
    fixture.edit(
        SYSTEM,
        r#"Name="Value">1"#,
        &format!(r#"Name="Value">{vector}"#),
    );
    fixture.edit(
        SYSTEM,
        r#"<Block BlockType="Gain" Name="feedback" SID="4">"#,
        r#"<Block BlockType="Integrator" Name="feedback" SID="4">"#,
    );
    fixture.edit(SYSTEM, r#"<P Name="Gain">1</P>"#, "");
    rejects(&fixture, "state_limit");
}

#[test]
fn unknown_configuration_components_and_solver_extensions_block_execution() {
    let mut fixture = Fixture::feedback();
    fixture.edit(
        CONFIG,
        r#"Name="SolverName">"#,
        r#"Name="FutureSolverFlag">on</P><P Name="SolverName">"#,
    );
    rejects(&fixture, "configuration_parameter");
    let mut fixture = Fixture::feedback();
    fixture.edit(
        CONFIG,
        "</Array>",
        r#"<Object ClassName="OpenMat.UnknownConfig"/></Array>"#,
    );
    rejects(&fixture, "configuration_class");
    let mut fixture = Fixture::feedback();
    fixture.edit(
        CONFIG,
        r#"Name="SolverName">"#,
        r#"Name="DecoupledContinuousIntegration">on</P><P Name="SolverName">"#,
    );
    rejects(&fixture, "configuration");
}

#[test]
#[ignore = "requires LLVM 22; Windows CI explicitly runs this imported-model test"]
fn imported_model_executes_as_real_llvm_machine_code() {
    let mut fixture = Fixture::feedback();
    fixture.edit(SYSTEM, r#"Name="Value">1"#, r#"Name="Value">[1 2]"#);
    fixture.edit(SYSTEM, r#"Name="Gain">1"#, r#"Name="Gain">[1 2]"#);
    let imported = read(&fixture);
    let plan = compile(&imported.lower().unwrap()).unwrap();
    let library = std::path::PathBuf::from(
        std::env::var_os("OPENMAT_SIM_LLVM_LIBRARY").expect("set OPENMAT_SIM_LLVM_LIBRARY"),
    );
    let kernel = openmat_sim_llvm::LlvmKernel::compile(plan.program().clone(), &library).unwrap();
    assert!(kernel.version().starts_with("22."));
    kernel.verify_abi_guards().unwrap();
    let actual = Runner::new(plan, kernel)
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
    let expected = simulate(&imported);
    assert_eq!(actual.scopes, expected.scopes);
    assert_eq!(actual.frames.len(), expected.frames.len());
    for (actual, expected) in actual.frames.iter().zip(&expected.frames) {
        assert_eq!(actual.time.to_bits(), expected.time.to_bits());
        for (&a, &b) in actual.values.iter().zip(&expected.values) {
            assert!((a - b).abs() < 1e-12);
        }
    }
}
