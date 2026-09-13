//! Editor-facing parameters resolved through the same bounded m evaluator as SLX.
use std::collections::BTreeMap;

use openmat_sim::model::{BlockKind, Model};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Issue, ParameterArray, Parameters};

pub const BLOCK_CATALOG: &str = include_str!("../../../blocks/common.json");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringParameters {
    pub source: String,
    pub bindings: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct ParameterResolution {
    pub model: Model,
    pub values: BTreeMap<String, BTreeMap<String, String>>,
}

fn fail(message: &str) -> Issue {
    Issue::new("block_parameter", message)
}

fn block_key(kind: &Value) -> &str {
    match kind["type"].as_str().unwrap_or("") {
        "standard" => kind["operation"]["type"].as_str().unwrap_or(""),
        "subsystem" => kind["execution"]["type"].as_str().unwrap_or(""),
        "resetIntegrator" if kind["discrete"] == true => "discreteIntegrator",
        "resetIntegrator" => "integrator",
        key => key,
    }
}

/// Resolve every binding into a fresh numerical snapshot; never trust cached values.
///
/// # Errors
/// Rejects unknown targets, unsupported options, invalid dimensions and resource limits.
///
/// # Panics
/// Only if the statically bundled descriptor catalog is malformed.
#[allow(clippy::too_many_lines, clippy::float_cmp)] // One atomic parameter transaction; sample-time sentinels require exact equality.
pub fn resolve_parameters(
    model: &Model,
    parameters: &AuthoringParameters,
) -> Result<ParameterResolution, Issue> {
    let count: usize = parameters.bindings.values().map(BTreeMap::len).sum();
    let bytes: usize = parameters
        .bindings
        .iter()
        .map(|(id, fields)| {
            id.len()
                + fields
                    .iter()
                    .map(|(key, text)| key.len() + text.len())
                    .sum::<usize>()
        })
        .sum();
    if count > 1024 || bytes > 65_536 || model.blocks.len() > 10_000 {
        return Err(fail("parameter bindings exceed 1024 entries or 64 KiB"));
    }
    let environment = Parameters::from_script(&parameters.source)?;
    let catalog: Value = serde_json::from_str(BLOCK_CATALOG).expect("bundled block descriptors");
    let mut raw = serde_json::to_value(model).map_err(|_| fail("cannot serialize model"))?;
    let indices: BTreeMap<_, _> = model
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    if indices.len() != model.blocks.len() {
        return Err(fail("duplicate block IDs"));
    }
    let mut values = BTreeMap::new();
    let mut work = 0;
    let mut total_values = 0;
    for (id, fields) in &parameters.bindings {
        let Some(&index) = indices.get(id.as_str()) else {
            return Err(fail("parameter binding references a missing block"));
        };
        let key = block_key(&raw["blocks"][index]["kind"]).to_owned();
        let descriptor = catalog["blocks"]
            .as_array()
            .expect("catalog blocks")
            .iter()
            .find(|b| b["id"] == key)
            .ok_or_else(|| fail("block does not accept common parameter bindings"))?;
        let mut resolved = BTreeMap::new();
        for (name, text) in fields {
            let outcome = (|| {
                let field = descriptor["parameters"]
                    .as_array()
                    .expect("catalog fields")
                    .iter()
                    .find(|f| f["name"] == *name)
                    .ok_or_else(|| fail("unknown block parameter"))?;
                if field["editor"] == "readonly" {
                    return Err(fail("parameter is maintained by the editor"));
                }
                if let Some(fixed) = field["fixed"].as_str() {
                    if text != fixed {
                        return Err(fail("parameter option is not supported"));
                    }
                    return Ok(text.clone());
                }
                if let Some(options) = field["options"].as_array()
                    && !options
                        .iter()
                        .any(|o| o["value"] == *text && o["supported"] == true)
                {
                    return Err(fail("parameter option is not supported"));
                }
                let path = field["path"]
                    .as_str()
                    .ok_or_else(|| fail("parameter has no binding"))?;
                if path == "sampleTime" {
                    let value = if ["inf", "[inf 0]"]
                        .iter()
                        .any(|literal| text.trim().eq_ignore_ascii_case(literal))
                    {
                        f64::INFINITY
                    } else {
                        let sample = environment
                            .evaluate_with_budget(text, &mut work)?
                            .vector()?;
                        if sample.is_empty()
                            || sample.len() > 2
                            || (sample.len() == 2 && sample[1] != 0.0)
                        {
                            return Err(fail(
                                "sample time requires a scalar or [period 0]; nonzero offsets are not supported",
                            ));
                        }
                        sample[0]
                    };
                    let sample = if value == -1.0 {
                        json!({"kind":"inherited"})
                    } else if value == 0.0 {
                        json!({"kind":"continuous"})
                    } else if value == f64::INFINITY {
                        json!({"kind":"constant"})
                    } else if value > 0.0 {
                        json!({"kind":"discrete","period":value})
                    } else {
                        return Err(fail("sample time must be -1, 0, inf or a positive period"));
                    };
                    if raw.get("sampleTimes").is_none() {
                        raw["sampleTimes"] = json!({});
                    }
                    raw["sampleTimes"][id] = sample;
                    let version = raw["schemaVersion"].as_u64().unwrap_or(1).max(5);
                    raw["schemaVersion"] = json!(version);
                    return Ok(if value.is_infinite() {
                        "inf".into()
                    } else {
                        value.to_string()
                    });
                }
                if path == "signs" || path == "operations" {
                    let normalized: String = text
                        .chars()
                        .filter(|c| !c.is_whitespace() && *c != '|')
                        .collect();
                    let minimum = if path == "signs" { 1 } else { 2 };
                    let normalized = if let Ok(n) = normalized.parse::<usize>() {
                        if !(minimum..=64).contains(&n) {
                            return Err(fail("input count is outside the supported range"));
                        }
                        if path == "signs" {
                            "+".repeat(n)
                        } else {
                            "*".repeat(n)
                        }
                    } else {
                        normalized
                    };
                    if !(minimum..=64).contains(&normalized.len())
                        || !normalized.chars().all(|c| {
                            if path == "signs" {
                                c == '+' || c == '-'
                            } else {
                                c == '*' || c == '/'
                            }
                        })
                    {
                        return Err(fail("invalid input operations"));
                    }
                    if path == "signs" {
                        raw["blocks"][index]["kind"]["signs"] = json!(
                            normalized
                                .chars()
                                .map(|c| if c == '+' { 1 } else { -1 })
                                .collect::<Vec<_>>()
                        );
                    } else {
                        raw["blocks"][index]["kind"]["operation"]["operations"] = json!(normalized);
                    }
                    return Ok(normalized);
                }
                if path == "externalReset" {
                    let kind = &mut raw["blocks"][index]["kind"];
                    let discrete = key == "discreteIntegrator";
                    if text == "none" {
                        if !discrete
                            && kind
                                .get("gain")
                                .is_some_and(|gain| gain.as_f64() != Some(1.0))
                        {
                            return Err(fail(
                                "continuous reset integrator gain must be 1 before removing reset",
                            ));
                        }
                        *kind = if discrete {
                            json!({"type":"discreteIntegrator","initial":kind["initial"],"gain":kind["gain"]})
                        } else {
                            json!({"type":"integrator","initial":kind["initial"]})
                        };
                    } else {
                        let gain = kind.get("gain").cloned().unwrap_or(json!(1));
                        *kind = json!({"type":"resetIntegrator","initial":kind["initial"],"gain":gain,"discrete":discrete,"reset":text});
                        raw["schemaVersion"] =
                            json!(raw["schemaVersion"].as_u64().unwrap_or(1).max(6));
                    }
                    return Ok(text.clone());
                }
                let is_expression = field["editor"] == "expression";
                let (value, display) = if is_expression {
                    let array = environment.evaluate_with_budget(text, &mut work)?;
                    total_values += array.values.len();
                    if total_values > 262_144 {
                        return Err(fail("resolved parameter storage exceeds its budget"));
                    }
                    (
                        convert(&array, field["shape"].as_str().unwrap_or("vector"))?,
                        array.text(),
                    )
                } else {
                    (json!(text), text.clone())
                };
                match path {
                    "initialOutput" | "outputWhenDisabled" => {
                        let block = &raw["blocks"][index];
                        let parent = block["parent"]
                            .as_str()
                            .and_then(|p| indices.get(p))
                            .copied()
                            .ok_or_else(|| fail("output policy requires a conditional parent"))?;
                        let port = block["kind"]["port"]
                            .as_u64()
                            .and_then(|n| n.checked_sub(1))
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| fail("invalid output port"))?;
                        let execution = &mut raw["blocks"][parent]["kind"]["execution"];
                        if execution.is_null()
                            || execution["outputs"]
                                .as_array()
                                .is_none_or(|p| port >= p.len())
                        {
                            return Err(fail(
                                "output policy requires a matching conditional output",
                            ));
                        }
                        if path == "outputWhenDisabled"
                            && execution["type"] == "triggered"
                            && text != "held"
                        {
                            return Err(fail("triggered outputs must hold their value"));
                        }
                        execution["outputs"][port][if path == "initialOutput" {
                            "initial"
                        } else {
                            "whenDisabled"
                        }] = value;
                    }
                    "statesWhenEnabling" | "triggerType" | "controlPeriod" => {
                        if path == "controlPeriod" && value.as_f64().is_none_or(|n| n <= 0.0) {
                            return Err(fail("control sample period must be positive"));
                        }
                        let field = match path {
                            "statesWhenEnabling" => "statesWhenEnabling",
                            "triggerType" => "edge",
                            _ => "period",
                        };
                        raw["blocks"][index]["kind"]["execution"][field] = value;
                    }
                    _ => {
                        let pointer = format!("/{}", path.replace('.', "/"));
                        let target = raw["blocks"][index]
                            .pointer_mut(&pointer)
                            .ok_or_else(|| fail("parameter target is unavailable"))?;
                        *target = value;
                    }
                }
                Ok(display)
            })();
            match outcome {
                Ok(value) => {
                    resolved.insert(name.clone(), value);
                }
                Err(mut issue) => {
                    issue.block = Some(id.clone());
                    issue.parameter = Some(name.clone());
                    return Err(issue);
                }
            }
        }
        values.insert(id.clone(), resolved);
    }
    let mut result: Model = serde_json::from_value(raw)
        .map_err(|_| fail("resolved parameters do not form a valid numerical model"))?;
    for block in &mut result.blocks {
        if let BlockKind::Standard { operation } = &mut block.kind {
            if let openmat_sim::authoring::StandardOp::StateSpace { a, initial, .. } = operation
                && initial.len() == 1
            {
                initial.resize(a.len(), initial[0]);
            }
            operation.validate().map_err(|error| {
                let mut issue = fail(&error.0.message);
                issue.block = Some(block.id.clone());
                issue
            })?;
        }
    }
    Ok(ParameterResolution {
        model: result,
        values,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Integer/positive/bounded shape values are checked before conversion.
fn convert(array: &ParameterArray, shape: &str) -> Result<Value, Issue> {
    Ok(match shape {
        "scalar" => json!(array.scalar()?),
        "count" => {
            let value = array.scalar()?;
            if value.fract() != 0.0 || !(2.0..=64.0).contains(&value) {
                return Err(fail("port count must be an integer from 2 to 64"));
            }
            json!(value as u64)
        }
        "matrix" => {
            if array.values.is_empty() || array.rows > 32 || array.columns > 32 {
                return Err(fail("matrix dimensions must be between 1 and 32"));
            }
            json!(
                (0..array.rows)
                    .map(|r| (0..array.columns)
                        .map(|c| array.values[r + c * array.rows])
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            )
        }
        "widths" => {
            let values = array.vector()?;
            if !(2..=64).contains(&values.len())
                || values
                    .iter()
                    .any(|v| v.fract() != 0.0 || !(1.0..=4096.0).contains(v))
            {
                return Err(fail("output widths require 2..64 positive integers"));
            }
            json!(values.iter().map(|v| *v as u64).collect::<Vec<_>>())
        }
        _ => json!(array.vector()?),
    })
}
