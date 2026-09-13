#[allow(dead_code)]
mod support;
use openmat_sim::{CollectionLimits, Runner, compile_with_sources, numeric::ReferenceKernel};
use openmat_sim_slx::ImportedSlx;
use support::{Fixture, SYSTEM};

fn fixture(enabled: bool) -> Fixture {
    let mut f = Fixture::feedback();
    let (control, port) = if enabled {
        ("EnablePort", "enable")
    } else {
        ("TriggerPort", "trigger")
    };
    f.parts.insert(SYSTEM.into(), format!(r#"<System>
<Block BlockType="Step" Name="start" SID="1"><P Name="Time">0.2</P><P Name="Before">-1</P><P Name="After">1</P><P Name="SampleTime">0.1</P></Block>
<Block BlockType="SubSystem" Name="conditional" SID="2"><PortCounts in="0" out="1" {port}="1"/><P Name="TreatAsAtomicUnit">on</P><System>
  <Block BlockType="{control}" Name="control" SID="4"/>
  <Block BlockType="Constant" Name="value" SID="5"><P Name="Value">7</P></Block>
  <Block BlockType="Outport" Name="output" SID="6"><P Name="InitialOutput">-3</P></Block>
  <Line><P Name="Src">5#out:1</P><P Name="Dst">6#in:1</P></Line>
</System></Block>
<Block BlockType="Scope" Name="view" SID="3"/>
<Block BlockType="Scope" Name="control view" SID="7"/>
<Line><P Name="Src">1#out:1</P><Branch><P Name="Dst">2#{port}</P></Branch><Branch><P Name="Dst">7#in:1</P></Branch></Line>
<Line><P Name="Src">2#out:1</P><P Name="Dst">3#in:1</P></Line>
</System>"#));
    f
}

#[test]
fn conditional_import_preserves_control_fanout_and_round_trips_hierarchy() {
    for enabled in [false, true] {
        let imported = ImportedSlx::read(&fixture(enabled).package(), "conditionals").unwrap();
        let original = serde_json::to_value(imported.document()).unwrap();
        assert!(imported.lower_hybrid("").is_err());
        let result = imported.lower_conditional("").unwrap();
        assert_eq!(serde_json::to_value(imported.document()).unwrap(), original);
        assert_eq!(result.model.schema_version, 8);
        let port = if enabled { "enable" } else { "trigger" };
        assert!(result.model.connections.iter().any(|e| e.to.port == port));
        assert!(
            result
                .model
                .blocks
                .iter()
                .any(|b| b.parent.as_deref() == Some("slx_2"))
        );
        let model = serde_json::from_value(serde_json::to_value(result.model).unwrap()).unwrap();
        let plan = compile_with_sources(&model, &result.sources).unwrap();
        let run = Runner::new_with_update(
            plan.clone(),
            ReferenceKernel::new(plan.program().clone()),
            plan.update_program().cloned().map(ReferenceKernel::new),
        )
        .unwrap()
        .collect(CollectionLimits::default())
        .unwrap();
        let offset = run
            .scopes
            .iter()
            .find(|s| s.block == "slx_3")
            .unwrap()
            .offset;
        for f in &run.frames {
            let expected = if f.time < 0.2 { -3.0 } else { 7.0 };
            assert!((f.values[offset] - expected).abs() < f64::EPSILON);
        }
        let events: Vec<_> = run
            .frames
            .iter()
            .flat_map(|f| &f.execution_events)
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].block, "slx_2");
    }
}

#[test]
fn invalid_control_output_and_empty_lines_are_diagnosed_before_normalization() {
    for (from, to, code) in [
        (
            "<P Name=\"InitialOutput\">-3</P>",
            "<P Name=\"InitialOutput\">[]</P>",
            "conditional_initial_output",
        ),
        (
            "<Block BlockType=\"TriggerPort\" Name=\"control\" SID=\"4\"/>",
            "<Block BlockType=\"TriggerPort\" Name=\"control\" SID=\"4\"><P Name=\"ShowOutputPort\">on</P></Block>",
            "conditional_parameter",
        ),
        ("<P Name=\"Dst\">3#in:1</P>", "", "line_destination"),
    ] {
        let mut f = fixture(false);
        f.edit(SYSTEM, from, to);
        let errors = ImportedSlx::read(&f.package(), "invalid")
            .unwrap()
            .lower_conditional("")
            .err()
            .unwrap();
        assert!(errors.iter().any(|e| e.code == code), "{errors:?}");
    }
}
