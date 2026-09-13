use crate::{Block, Issue, Parameters, lower::value};
use openmat_sim::model::SampleTime;

pub(super) enum AdaptKind {
    Same,
    Hold,
    Transition {
        initial: Vec<f64>,
        deterministic: bool,
    },
    Integrator {
        gain: f64,
    },
    StateSpace {
        initial: Option<Vec<f64>>,
    },
}
pub(super) struct Adaptation {
    pub rate: Option<SampleTime>,
    pub kind: AdaptKind,
    pub name: String,
}

fn scalar(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<f64, Issue> {
    p.evaluate(value(&block.properties, key, default))
        .and_then(|a| a.scalar())
        .map_err(|e| e.at(block, Some(key)))
}
fn vector(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<Vec<f64>, Issue> {
    p.evaluate(value(&block.properties, key, default))
        .and_then(|a| a.vector())
        .map_err(|e| e.at(block, Some(key)))
}
fn require(block: &Block, key: &str, expected: &str) -> Result<(), Issue> {
    if value(&block.properties, key, expected) != expected {
        return Err(Issue::new(
            "unsupported_parameter",
            format!(
                "{} requires {key}={expected} in multirate-v1",
                block.block_type
            ),
        )
        .at(block, Some(key)));
    }
    Ok(())
}
fn rate(block: &Block, p: &Parameters, key: &str, default: &str) -> Result<SampleTime, Issue> {
    let text = value(&block.properties, key, default).trim();
    if ["inf", "Inf", "[inf 0]", "[Inf 0]"].contains(&text) {
        return Ok(SampleTime::Constant);
    }
    let values = vector(block, p, key, default)?;
    if values.len() > 2 || values.len() == 2 && values[1] != 0.0 {
        return Err(Issue::new(
            "sample_offset",
            "only zero-offset scalar or [period 0] sample times are supported",
        )
        .at(block, Some(key)));
    }
    match values[0] {
        -1.0 => Ok(SampleTime::Inherited),
        0.0 => Ok(SampleTime::Continuous),
        period if period > 0.0 => Ok(SampleTime::Discrete { period }),
        _ => Err(Issue::new("sample_time", "unsupported sample-time kind").at(block, Some(key))),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn block(block: &mut Block, p: &Parameters) -> Result<Adaptation, Issue> {
    // Logging labels do not alter signal evaluation; preserve them in the original document.
    for port in &mut block.port_properties {
        if port
            .properties
            .get("DataLoggingNameMode")
            .is_some_and(|m| !["Custom", "SignalName"].contains(&m.as_str()))
        {
            return Err(Issue::new(
                "port_feature",
                "unsupported signal logging name mode",
            ));
        }
        port.properties.remove("DataLoggingNameMode");
        port.properties.remove("DataLoggingName");
    }
    let name = block.block_type.clone();
    let sample = match name.as_str() {
        "Constant" => Some(rate(block, p, "SampleTime", "inf")?),
        "Gain"
        | "Sum"
        | "Bias"
        | "Product"
        | "Sin"
        | "Step"
        | "UnitDelay"
        | "DiscreteTransferFcn" => Some(rate(block, p, "SampleTime", "-1")?),
        "DiscreteStateSpace" | "DiscreteIntegrator" | "ZeroOrderHold" => {
            Some(rate(block, p, "SampleTime", "1")?)
        }
        "RateTransition" => Some(rate(block, p, "OutPortSampleTime", "-1")?),
        _ => None,
    };
    let kind = match name.as_str() {
        "ZeroOrderHold" => {
            block.block_type = "Gain".into();
            block.properties.remove("SampleTime");
            block.properties.insert("Gain".into(), "1".into());
            AdaptKind::Hold
        }
        "RateTransition" => {
            require(block, "OutPortSampleTimeOpt", "Specify")?;
            let integrity = value(&block.properties, "Integrity", "on");
            let deterministic = value(&block.properties, "Deterministic", "on");
            if !["on", "off"].contains(&integrity) || integrity != deterministic {
                return Err(Issue::new("rate_transition", "initial support requires Integrity and Deterministic to be both on or both off").at(block, None));
            }
            let deterministic = deterministic == "on";
            let initial = vector(block, p, "InitialCondition", "0")?;
            for key in [
                "OutPortSampleTimeOpt",
                "OutPortSampleTime",
                "Integrity",
                "Deterministic",
                "InitialCondition",
            ] {
                block.properties.remove(key);
            }
            block.block_type = "Gain".into();
            block.properties.insert("Gain".into(), "1".into());
            AdaptKind::Transition {
                initial,
                deterministic,
            }
        }
        "DiscreteIntegrator" => {
            for (key, expected) in [
                ("IntegratorMethod", "Integration: Forward Euler"),
                ("InitialConditionSetting", "Output"),
                ("InitialConditionSource", "internal"),
                ("ExternalReset", "none"),
                ("LimitOutput", "off"),
                ("ShowSaturationPort", "off"),
                ("ShowStatePort", "off"),
                ("IgnoreLimit", "off"),
            ] {
                require(block, key, expected)?;
            }
            let gain = scalar(block, p, "gainval", "1")?;
            for key in [
                "SampleTime",
                "gainval",
                "IntegratorMethod",
                "InitialConditionSetting",
                "IgnoreLimit",
                "ICPrevOutput",
                "ICPrevScaledInput",
                "UpperSaturationLimit",
                "LowerSaturationLimit",
            ] {
                block.properties.remove(key);
            }
            real_metadata(block)?;
            block.block_type = "Integrator".into();
            AdaptKind::Integrator { gain }
        }
        "DiscreteStateSpace" => {
            block.properties.remove("SampleTime");
            real_metadata(block)?;
            block.block_type = "StateSpace".into();
            AdaptKind::StateSpace { initial: None }
        }
        "DiscreteTransferFcn" => {
            for (key, expected) in [
                ("NumeratorSource", "Dialog"),
                ("DenominatorSource", "Dialog"),
                ("InitialStatesSource", "Dialog"),
                ("InputProcessing", "Elements as channels (sample based)"),
                ("ExternalReset", "None"),
                ("FilterStructure", "Direct form II"),
                ("a0EqualsOne", "off"),
                ("InputPortMap", "u0"),
                ("InitialDenominatorStates", "0"),
            ] {
                require(block, key, expected)?;
            }
            let initial = vector(block, p, "InitialStates", "0")?;
            // R2022b built-in DiscreteTransferFcn differs from continuous TransferFcn.
            let denominator = value(&block.properties, "Denominator", "[1 0.5]").to_owned();
            for key in [
                "SampleTime",
                "NumeratorSource",
                "DenominatorSource",
                "InitialStatesSource",
                "InitialStates",
                "InputProcessing",
                "ExternalReset",
                "FilterStructure",
                "a0EqualsOne",
                "InputPortMap",
                "InitialDenominatorStates",
            ] {
                block.properties.remove(key);
            }
            real_metadata(block)?;
            block.properties.insert("Denominator".into(), denominator);
            block.block_type = "TransferFcn".into();
            AdaptKind::StateSpace {
                initial: Some(initial),
            }
        }
        _ => AdaptKind::Same,
    };
    match name.as_str() {
        "Constant" => {
            block.properties.insert("SampleTime".into(), "inf".into());
        }
        "Gain" | "Sum" | "Bias" | "Product" => {
            block.properties.insert("SampleTime".into(), "-1".into());
        }
        "UnitDelay" => {
            block.properties.insert("SampleTime".into(), "1".into());
        }
        "Sin" | "Step" => {
            block.properties.insert("SampleTime".into(), "0".into());
        }
        _ => {}
    }
    Ok(Adaptation {
        name,
        rate: sample,
        kind,
    })
}

fn real_metadata(block: &mut Block) -> Result<(), Issue> {
    for key in [
        "OutDataTypeStr",
        "StateDataTypeStr",
        "MultiplicandDataTypeStr",
        "NumCoefDataTypeStr",
        "DenCoefDataTypeStr",
        "NumProductDataTypeStr",
        "DenProductDataTypeStr",
        "NumAccumDataTypeStr",
        "DenAccumDataTypeStr",
    ] {
        if let Some(text) = block.properties.get(key) {
            if ![
                "double",
                "Inherit: Same as input",
                "Inherit: Inherit via internal rule",
            ]
            .contains(&text.as_str())
            {
                return Err(Issue::new(
                    "data_type",
                    "multirate-v1 only supports real double signals",
                )
                .at(block, Some(key)));
            }
            block.properties.remove(key);
        }
    }
    for key in ["NumCoefMin", "NumCoefMax", "DenCoefMin", "DenCoefMax"] {
        require(block, key, "[]")?;
        block.properties.remove(key);
    }
    for (key, expected) in [
        ("LockScale", "off"),
        ("StateMustResolveToSignalObject", "off"),
        ("StateStorageClass", "Auto"),
        ("RTWStateStorageTypeQualifier", ""),
        ("StateSignalObject", "[]"),
    ] {
        require(block, key, expected)?;
        block.properties.remove(key);
    }
    // Rounding and saturation selectors have no effect for the accepted f64 types.
    for key in ["RndMeth", "SaturateOnIntegerOverflow"] {
        block.properties.remove(key);
    }
    Ok(())
}
