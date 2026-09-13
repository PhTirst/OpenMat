#[allow(dead_code)]
mod support;
use openmat_sim::{CollectionLimits, Runner, compile_with_sources, numeric::ReferenceKernel};
use openmat_sim_slx::ImportedSlx;
use support::{Fixture, SYSTEM};

fn fixture(kind: &str, parameters: &str) -> Fixture {
    let mut f = Fixture::feedback();
    f.parts.insert(
        SYSTEM.into(),
        format!(
            r#"<System>
    <Block BlockType="Constant" Name="input" SID="1"><P Name="Value">[2 -3 1]</P></Block>
    <Block BlockType="{kind}" Name="operation" SID="2">{parameters}</Block>
    <Block BlockType="Scope" Name="scope" SID="3"/>
    <Line><P Name="Src">1#out:1</P><P Name="Dst">2#in:1</P></Line>
    <Line><P Name="Src">2#out:1</P><P Name="Dst">3#in:1</P></Line>
    </System>"#
        ),
    );
    f
}

#[test]
fn imports_native_controls_without_generated_callbacks_and_preserves_old_profiles() {
    for (kind, params, expected) in [
        ("MinMax", "", vec![-3.0]),
        (
            "Saturate",
            r#"<P Name="LowerLimit">-1</P><P Name="UpperLimit">1</P>"#,
            vec![1.0, -1.0, 1.0],
        ),
        ("Abs", "", vec![2.0, 3.0, 1.0]),
    ] {
        let imported = ImportedSlx::read(&fixture(kind, params).package(), "hybrid").unwrap();
        assert!(imported.lower_multirate("").is_err());
        let lowered = imported.lower_hybrid("").unwrap();
        assert!(lowered.sources.is_empty());
        assert_eq!(lowered.model.schema_version, 6);
        assert!(lowered.event_plan.is_some());
        let plan = compile_with_sources(&lowered.model, &lowered.sources).unwrap();
        let update = plan.update_program().cloned().map(ReferenceKernel::new);
        let result = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            update,
        )
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        assert_eq!(result.frames[0].values, expected);
    }
}

#[test]
fn rejects_unsupported_semantics_with_original_block_identity() {
    for (kind, params) in [
        ("Saturate", r#"<P Name="OutDataTypeStr">int16</P>"#),
        ("Abs", r#"<P Name="UnknownBehavior">on</P>"#),
        ("Integrator", r#"<P Name="ExternalReset">level</P>"#),
        (
            "Integrator",
            r#"<P Name="ExternalReset">rising</P><P Name="InitialConditionSource">external</P>"#,
        ),
    ] {
        let imported = ImportedSlx::read(&fixture(kind, params).package(), "hybrid").unwrap();
        let errors = imported.lower_hybrid("").err().unwrap();
        assert!(errors.iter().any(|e| e.block.as_deref() == Some("2")));
        assert!(errors.iter().any(|e| { e.message.contains("operation") }));
    }
}
