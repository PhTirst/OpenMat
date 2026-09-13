use crate::{
    Block, Document, Issue, Parameters, blocks,
    lower::{runtime_id, value},
};
use openmat_sim::{
    CompiledModel,
    hybrid::{ControlOp, LogicOperator, ResetMode, SignalType, SwitchCriterion},
    model::{BlockKind, Model},
    numeric::Comparison,
};

#[allow(clippy::too_many_lines)] // One exhaustive mapping keeps accepted block parameters beside their numerical meaning.
pub(super) fn adapt(block: &mut Block, p: &Parameters) -> Result<Option<BlockKind>, Issue> {
    let original = block.clone();
    let reset = ["Integrator", "DiscreteIntegrator"].contains(&block.block_type.as_str())
        && value(&block.properties, "ExternalReset", "none") != "none";
    if !reset
        && ![
            "Saturate",
            "Switch",
            "RelationalOperator",
            "Logic",
            "Abs",
            "MinMax",
        ]
        .contains(&block.block_type.as_str())
    {
        return Ok(None);
    }
    let properties = &original.properties;
    let scalar = |key, default| {
        p.evaluate(value(properties, key, default))
            .and_then(|v| v.scalar())
            .map_err(|e| e.at(&original, Some(key)))
    };
    let vector = |key, default| {
        p.evaluate(value(properties, key, default))
            .and_then(|v| v.vector())
            .map_err(|e| e.at(&original, Some(key)))
    };
    let integer = |key, default| -> Result<usize, Issue> {
        let n = scalar(key, default)?;
        if n.fract() != 0.0 || !(1.0..=64.0).contains(&n) {
            return Err(Issue::new(
                "control_parameter",
                "input count must be an integer in 1..64",
            )
            .at(&original, Some(key)));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(n as usize)
    };
    let invalid = |key| {
        Issue::new(
            "unsupported_parameter",
            format!("unsupported {} {key}", original.block_type),
        )
        .at(&original, Some(key))
    };
    let operation = match original.block_type.as_str() {
        "Saturate" => Some(ControlOp::Saturation {
            lower: vector("LowerLimit", "-0.5")?,
            upper: vector("UpperLimit", "0.5")?,
        }),
        "Switch" => Some(ControlOp::Switch {
            criterion: match value(properties, "Criteria", "u2 >= Threshold") {
                "u2 >= Threshold" => SwitchCriterion::GreaterEqual,
                "u2 > Threshold" => SwitchCriterion::Greater,
                "u2 ~= 0" => SwitchCriterion::Nonzero,
                _ => return Err(invalid("Criteria")),
            },
            threshold: scalar("Threshold", "0")?,
        }),
        "RelationalOperator" => Some(ControlOp::Relational {
            operator: match value(properties, "Operator", ">=") {
                "==" => Comparison::Equal,
                "~=" => Comparison::NotEqual,
                "<" => Comparison::Less,
                "<=" => Comparison::LessEqual,
                ">" => Comparison::Greater,
                ">=" => Comparison::GreaterEqual,
                _ => return Err(invalid("Operator")),
            },
        }),
        "Logic" => Some(ControlOp::Logical {
            operator: match value(properties, "Operator", "AND") {
                "AND" => LogicOperator::And,
                "OR" => LogicOperator::Or,
                "XOR" => LogicOperator::Xor,
                "NAND" => LogicOperator::Nand,
                "NOR" => LogicOperator::Nor,
                "NOT" => LogicOperator::Not,
                "NXOR" => LogicOperator::Nxor,
                _ => return Err(invalid("Operator")),
            },
            inputs: if value(properties, "Operator", "AND") == "NOT" {
                1
            } else {
                integer("Inputs", "2")?
            },
        }),
        "Abs" => Some(ControlOp::Abs),
        "MinMax" => Some(ControlOp::MinMax {
            minimum: match value(properties, "Function", "min") {
                "min" => true,
                "max" => false,
                _ => return Err(invalid("Function")),
            },
            inputs: integer("Inputs", "1")?,
        }),
        _ => None,
    };
    let zero_crossing = match value(properties, "ZeroCross", "on") {
        "on" => true,
        "off" => false,
        _ => return Err(invalid("ZeroCross")),
    };
    let kind = if let Some(operation) = operation {
        operation
            .validate()
            .map_err(|e| Issue::new(&e.0.code, e.0.message).at(&original, None))?;
        BlockKind::Control {
            operation,
            zero_crossing,
        }
    } else {
        for (key, expected) in [
            ("InitialConditionSource", "internal"),
            ("LimitOutput", "off"),
            ("ShowSaturationPort", "off"),
            ("ShowStatePort", "off"),
            ("WrapState", "off"),
            ("InitialConditionSetting", "Output"),
        ] {
            if value(properties, key, expected) != expected {
                return Err(invalid(key));
            }
        }
        let discrete = original.block_type == "DiscreteIntegrator";
        if !discrete && (!zero_crossing || properties.contains_key("SampleTime")) {
            return Err(invalid("ZeroCross/SampleTime"));
        }
        if discrete
            && value(properties, "IntegratorMethod", "Integration: Forward Euler")
                != "Integration: Forward Euler"
        {
            return Err(invalid("IntegratorMethod"));
        }
        BlockKind::ResetIntegrator {
            initial: vector("InitialCondition", "0")?,
            gain: if discrete {
                scalar("gainval", "1")?
            } else {
                1.0
            },
            discrete,
            reset: match value(properties, "ExternalReset", "none") {
                "rising" => ResetMode::Rising,
                "falling" => ResetMode::Falling,
                "either" => ResetMode::Either,
                _ => return Err(invalid("ExternalReset")),
            },
        }
    };
    consume_properties(block, &original, &kind, reset)?;
    let rate = if matches!(
        kind,
        BlockKind::ResetIntegrator {
            discrete: false,
            ..
        }
    ) {
        "0"
    } else {
        value(properties, "SampleTime", if reset { "1" } else { "-1" })
    };
    block.properties.insert("SampleTime".into(), rate.into());
    block.properties.insert("Gain".into(), "1".into());
    block.block_type = "Gain".into();
    Ok(Some(kind))
}

pub(super) fn check_types(
    document: &Document,
    model: &Model,
    plan: &CompiledModel,
) -> Result<(), Issue> {
    let types = &plan.events().expect("hybrid events").signals;
    for b in document.systems.iter().flat_map(|s| &s.blocks) {
        if b.block_type == "Logic" && value(&b.properties, "AllPortsSameDT", "on") == "on" {
            let id = runtime_id(&b.sid)?;
            for edge in model.connections.iter().filter(|e| e.to.block == id) {
                if types[&edge.from.block] != SignalType::Logical {
                    return Err(Issue::new("signal_type","Logic with AllPortsSameDT=on requires logical inputs; use comparison outputs or an explicit conversion").at(b,Some("AllPortsSameDT")));
                }
            }
        }
    }
    Ok(())
}

fn consume_properties(
    block: &mut Block,
    original: &Block,
    kind: &BlockKind,
    reset: bool,
) -> Result<(), Issue> {
    let logical = matches!(
        kind,
        BlockKind::Control {
            operation: ControlOp::Logical { .. } | ControlOp::Relational { .. },
            ..
        }
    );
    // Consume only validated execution parameters. Remaining metadata is checked
    // by the existing structural validator after conversion to a neutral proxy.
    for (key, text) in &original.properties {
        let accepted = match key.as_str() {
            "UpperLimit" | "LowerLimit" => original.block_type == "Saturate",
            "Threshold" | "Criteria" | "AllowDiffInputSizes" => {
                original.block_type == "Switch" && (key != "AllowDiffInputSizes" || text == "off")
            }
            "Operator" => ["Logic", "RelationalOperator"].contains(&original.block_type.as_str()),
            "Function" => original.block_type == "MinMax",
            "Inputs" => ["Logic", "MinMax"].contains(&original.block_type.as_str()),
            "ZeroCross" | "SampleTime" => true,
            "OutDataTypeStr" => {
                if logical {
                    [
                        "boolean",
                        "Inherit: Logical (see Configuration Parameters: Optimization)",
                    ]
                    .contains(&text.as_str())
                } else {
                    [
                        "double",
                        "Inherit: Same as input",
                        "Inherit: Inherit via internal rule",
                    ]
                    .contains(&text.as_str())
                }
            }
            "InputSameDT" | "AllPortsSameDT" | "LinearizeAsGain" | "SaturateOnIntegerOverflow" => {
                ["on", "off"].contains(&text.as_str())
            }
            "IconShape" => ["rectangular", "distinctive"].contains(&text.as_str()),
            "LockScale" => text == "off",
            "RndMeth" => [
                "Floor",
                "Nearest",
                "Zero",
                "Ceiling",
                "Convergent",
                "Round",
                "Simplest",
            ]
            .contains(&text.as_str()),
            "ExternalReset"
            | "InitialCondition"
            | "InitialConditionSource"
            | "InitialConditionSetting"
            | "LimitOutput"
            | "ShowSaturationPort"
            | "ShowStatePort"
            | "WrapState"
            | "IntegratorMethod"
            | "gainval"
            | "UpperSaturationLimit"
            | "LowerSaturationLimit"
            | "IgnoreLimit"
            | "WrappedStateUpperValue"
            | "WrappedStateLowerValue" => reset,
            "ICPrevOutput" | "ICPrevScaledInput" => reset && text == "DiscIntNeverNeededParam",
            "ContinuousStateAttributes" => reset && text == "''",
            "AbsoluteTolerance" => reset && text == "auto",
            "StateMustResolveToSignalObject" => reset && text == "off",
            "StateStorageClass" => reset && text == "Auto",
            "RTWStateStorageTypeQualifier" => reset && text.is_empty(),
            _ => false,
        };
        if accepted {
            block.properties.remove(key);
        } else if !blocks::parameter_supported(original, key, text) {
            return Err(Issue::new(
                "unsupported_parameter",
                format!("unsupported {} {key}", original.block_type),
            )
            .at(original, Some(key)));
        }
    }
    Ok(())
}
