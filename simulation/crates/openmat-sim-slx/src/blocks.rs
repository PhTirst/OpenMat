use openmat_sim::model::{Block as RuntimeBlock, BlockKind, Position};

use crate::lower::{runtime_id, value};
use crate::{Block, Issue, literal};

pub(crate) fn translate(block: &Block) -> Result<RuntimeBlock, Issue> {
    translate_inner(block).map_err(|error| error.at(block, None))
}
fn translate_inner(block: &Block) -> Result<RuntimeBlock, Issue> {
    let p = &block.properties;
    check_parameters(block)?;
    let kind = match block.block_type.as_str() {
        "Constant" => {
            let values = literal::vector(value(p, "Value", "1"))?;
            if values.len() > 1 && value(p, "VectorParams1D", "on") != "on" {
                return Err(Issue::new(
                    "signal_shape",
                    "Constant matrix outputs are unsupported",
                ));
            }
            BlockKind::Constant { value: values }
        }
        "Gain" => BlockKind::Gain {
            gain: literal::vector(value(p, "Gain", "1"))?,
        },
        "Sum" => {
            let inputs = value(p, "Inputs", "++").replace('|', "");
            let signs = if let Ok(count) = inputs.parse::<usize>() {
                if !(2..=64).contains(&count) {
                    return Err(Issue::new(
                        "sum_inputs",
                        "Sum requires 2..64 input ports in SLX v0",
                    ));
                }
                vec![1; count]
            } else {
                let signs: Vec<_> = inputs
                    .bytes()
                    .map(|b| match b {
                        b'+' => 1,
                        b'-' => -1,
                        _ => 0,
                    })
                    .collect();
                if signs.len() < 2 || signs.len() > 64 || signs.contains(&0) {
                    return Err(Issue::new(
                        "sum_inputs",
                        "single-input reduction or invalid Sum configuration",
                    ));
                }
                signs
            };
            BlockKind::Sum { signs }
        }
        "Integrator" => BlockKind::Integrator {
            initial: literal::vector(value(p, "InitialCondition", "0"))?,
        },
        "UnitDelay" => BlockKind::UnitDelay {
            initial: literal::vector(value(p, "InitialCondition", "0"))?,
        },
        "Scope" => BlockKind::Scope,
        other => {
            return Err(Issue::new(
                "unsupported_block",
                format!("unsupported block type {other}"),
            ));
        }
    };
    check_ports(block, &kind)?;
    let position = p
        .get("Position")
        .map(|text| {
            let values = literal::vector(text)?;
            if values.len() != 4 {
                return Err(Issue::new(
                    "position",
                    "expected a four-number block rectangle",
                ));
            }
            Ok(Position {
                x: values[0],
                y: values[1],
            })
        })
        .transpose()?;
    Ok(RuntimeBlock {
        id: runtime_id(&block.sid)?,
        kind,
        position,
    })
}

fn check_parameters(block: &Block) -> Result<(), Issue> {
    if ![
        "Constant",
        "Gain",
        "Sum",
        "Integrator",
        "UnitDelay",
        "Scope",
    ]
    .contains(&block.block_type.as_str())
    {
        return Err(Issue::new(
            "unsupported_block",
            format!("unsupported block type {}", block.block_type),
        ));
    }
    if block
        .attributes
        .keys()
        .any(|k| !["BlockType", "Name", "SID"].contains(&k.as_str()))
        || block
            .extra_elements
            .iter()
            .any(|e| !["PortCounts", "PortProperties"].contains(&e.as_str()))
    {
        return Err(Issue::new(
            "block_feature",
            "mask, subsystem or additional block structure is unsupported",
        ));
    }
    for port in &block.port_properties {
        if !port.extra_elements.is_empty()
            || port
                .attributes
                .keys()
                .any(|k| !["Type", "Index"].contains(&k.as_str()))
            || port.properties.keys().any(|k| k != "Name")
        {
            return Err(Issue::new(
                "port_feature",
                "only display names are supported in port properties",
            ));
        }
    }
    for (key, text) in &block.properties {
        let text = text.trim();
        let valid = parameter_supported(block, key, text);
        if !valid {
            return Err(Issue::new(
                "unsupported_parameter",
                format!("unsupported {} parameter {key}={text}", block.block_type),
            )
            .at(block, Some(key)));
        }
    }
    Ok(())
}

fn check_ports(block: &Block, kind: &BlockKind) -> Result<(), Issue> {
    let expected_inputs = match kind {
        BlockKind::Constant { .. } => 0,
        BlockKind::Sum { signs } => signs.len(),
        _ => 1,
    };
    for (port, count) in &block.port_counts {
        let expected = match port.as_str() {
            "in" => expected_inputs,
            "out" => usize::from(!matches!(kind, BlockKind::Scope)),
            _ => {
                return Err(Issue::new(
                    "port_kind",
                    "state, enable, trigger and control ports are unsupported",
                ));
            }
        };
        if count.parse::<usize>().ok() != Some(expected) {
            return Err(Issue::new(
                "port_count",
                "port count does not match supported block configuration",
            ));
        }
    }
    Ok(())
}

fn parameter_supported(block: &Block, key: &str, text: &str) -> bool {
    match key {
        "Position"
        | "ZOrder"
        | "NamePlacement"
        | "ShowName"
        | "HideAutomaticName"
        | "Orientation"
        | "BlockMirror"
        | "ForegroundColor"
        | "BackgroundColor"
        | "FontName"
        | "FontSize"
        | "FontWeight"
        | "FontAngle"
        | "Description"
        | "Tag"
        | "AttributesFormatString"
        | "StateName" => true,
        "Value" => block.block_type == "Constant",
        "Gain" => block.block_type == "Gain",
        "Inputs" => block.block_type == "Sum",
        "InitialCondition" => ["Integrator", "UnitDelay"].contains(&block.block_type.as_str()),
        "Multiplication" => block.block_type == "Gain" && text == "Element-wise(K.*u)",
        "VectorParams1D" => block.block_type == "Constant" && ["on", "off"].contains(&text),
        "SampleTime" => match block.block_type.as_str() {
            "Constant" => text == "inf",
            "UnitDelay" => true,
            "Gain" | "Sum" => text == "-1",
            _ => false,
        },
        "OutDataTypeStr" => {
            text == "double"
                || match block.block_type.as_str() {
                    "Constant" => text == "Inherit: Inherit from 'Constant value'",
                    "Gain" | "Sum" | "UnitDelay" => text == "Inherit: Same as input",
                    _ => false,
                }
        }
        "ParamDataTypeStr" => {
            block.block_type == "Gain" && ["double", "Inherit: Inherit from 'Gain'"].contains(&text)
        }
        "AccumDataTypeStr" => {
            block.block_type == "Sum"
                && ["double", "Inherit: Inherit via internal rule"].contains(&text)
        }
        "OutMin" | "OutMax" => text == "[]",
        "RndMeth" | "SaturateOnIntegerOverflow" => {
            ["Gain", "Sum", "Constant"].contains(&block.block_type.as_str())
        }
        "ExternalReset" => block.block_type == "Integrator" && text == "none",
        "InitialConditionSource" => block.block_type == "Integrator" && text == "internal",
        "LimitOutput" | "WrapState" | "ShowSaturationPort" | "ShowStatePort" => {
            block.block_type == "Integrator" && text == "off"
        }
        "AbsoluteTolerance" => block.block_type == "Integrator" && text == "auto",
        "InputProcessing" => {
            block.block_type == "UnitDelay" && text == "Elements as channels (sample based)"
        }
        "HasFrameUpgradeWarning" => {
            block.block_type == "UnitDelay" && ["on", "off"].contains(&text)
        }
        "NumInputPorts" => block.block_type == "Scope" && text == "1",
        "Floating" => block.block_type == "Scope" && text == "off",
        _ => false,
    }
}
