#[allow(dead_code)]
mod support;
use openmat_sim::{
    CollectionLimits, Runner, compile_with_sources, model::SampleTime, numeric::ReferenceKernel,
};
use openmat_sim_slx::ImportedSlx;
use support::{Fixture, SYSTEM};

fn fixture(kind: &str, properties: &str) -> Fixture {
    let mut f = Fixture::feedback();
    f.parts.insert(
        SYSTEM.into(),
        format!(
            r#"<System>
        <Block BlockType="Sin" Name="source" SID="1"><P Name="SampleTime">0.1</P></Block>
        <Block BlockType="{kind}" Name="filter" SID="2">{properties}</Block>
        <Block BlockType="Scope" Name="view" SID="3"/>
        <Line><P Name="Src">1#out:1</P><P Name="Dst">2#in:1</P></Line>
        <Line><P Name="Src">2#out:1</P><P Name="Dst">3#in:1</P></Line>
    </System>"#
        ),
    );
    f
}

#[test]
fn inherited_filter_and_embedded_callbacks_survive_a_model_snapshot() {
    let imported =
        ImportedSlx::read(&fixture("DiscreteTransferFcn", "").package(), "filter").unwrap();
    assert!(imported.lower_control("").is_err());
    let result = imported.lower_multirate("").unwrap();
    assert_eq!(result.model.schema_version, 5);
    assert_eq!(
        result
            .model
            .components
            .iter()
            .find(|c| c.name == "DiscreteTransferFcn")
            .unwrap()
            .sample_time,
        Some(-1.0)
    );
    assert_eq!(
        result.sampling.unwrap().blocks["slx_2"],
        SampleTime::Discrete { period: 0.1 }
    );
    let model = serde_json::from_value(serde_json::to_value(&result.model).unwrap()).unwrap();
    let plan = compile_with_sources(&model, &result.sources).unwrap();
    let update = plan.update_program().cloned().map(ReferenceKernel::new);
    let run = Runner::new_with_update(
        plan.clone(),
        ReferenceKernel::new(plan.program().clone()),
        update,
    )
    .unwrap()
    .collect(CollectionLimits::default())
    .unwrap();
    assert_eq!(
        run.frames
            .iter()
            .filter(|f| f.sample_hits.contains(&0))
            .count(),
        11
    );
}

#[test]
fn offsets_non_grid_periods_and_unsupported_modes_keep_original_block_paths() {
    for (kind, properties, code) in [
        (
            "UnitDelay",
            r#"<P Name="SampleTime">[0.1 0.05]</P>"#,
            "sample_offset",
        ),
        (
            "UnitDelay",
            r#"<P Name="SampleTime">0.075</P>"#,
            "sample_grid",
        ),
        (
            "DiscreteIntegrator",
            r#"<P Name="IntegratorMethod">Integration: Backward Euler</P>"#,
            "unsupported_parameter",
        ),
        (
            "RateTransition",
            r#"<P Name="OutPortSampleTime">0.2</P><P Name="Integrity">off</P>"#,
            "rate_transition",
        ),
        (
            "DiscreteStateSpace",
            r#"<P Name="OutDataTypeStr">single</P>"#,
            "data_type",
        ),
    ] {
        let imported = ImportedSlx::read(&fixture(kind, properties).package(), "bounded").unwrap();
        let errors = imported.lower_multirate("").err().unwrap();
        assert!(
            errors.iter().any(|e| e.code == code
                && e.block.as_deref() == Some("2")
                && e.message.contains("bounded/filter")),
            "{errors:#?}"
        );
    }
}

#[test]
fn sample_time_parameters_are_explicit_and_original_xml_is_preserved() {
    let f = fixture("ZeroOrderHold", r#"<P Name="SampleTime">Ts</P>"#);
    let imported = ImportedSlx::read(&f.package(), "parameterized").unwrap();
    assert!(imported.lower_multirate("").is_err());
    let before = serde_json::to_value(imported.document()).unwrap();
    let result = imported.lower_multirate("Ts=0.1;").unwrap();
    assert_eq!(
        result.sampling.unwrap().blocks["slx_2"],
        SampleTime::Discrete { period: 0.1 }
    );
    assert_eq!(serde_json::to_value(imported.document()).unwrap(), before);
}
