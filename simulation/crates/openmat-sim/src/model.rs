//! Serializable authoring model, independent of editor and execution backend.
use crate::component::ComponentDefinition;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;
pub const FUNCTION_SCHEMA_VERSION: u32 = 2;
pub const COMPONENT_SCHEMA_VERSION: u32 = 3;
pub const CONTROL_SCHEMA_VERSION: u32 = 4;
pub const MULTIRATE_SCHEMA_VERSION: u32 = 5;
pub const HYBRID_SCHEMA_VERSION: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SampleTime {
    Inherited,
    Continuous,
    Constant,
    Discrete { period: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    pub schema_version: u32,
    pub name: String,
    pub settings: Settings,
    pub blocks: Vec<Block>,
    pub connections: Vec<Connection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentDefinition>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sample_times: BTreeMap<String, SampleTime>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub start_time: f64,
    pub stop_time: f64,
    pub max_step: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_time: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum BlockKind {
    Control {
        operation: crate::hybrid::ControlOp,
        #[serde(rename = "zeroCrossing")]
        zero_crossing: bool,
    },
    ResetIntegrator {
        initial: Vec<f64>,
        gain: f64,
        discrete: bool,
        reset: crate::hybrid::ResetMode,
    },
    ZeroOrderHold,
    DiscreteIntegrator {
        initial: Vec<f64>,
        gain: f64,
    },
    RateTransition {
        initial: Vec<f64>,
        deterministic: bool,
    },
    Step {
        time: f64,
        before: Vec<f64>,
        after: Vec<f64>,
    },
    Component {
        component: String,
        #[serde(default)]
        parameters: BTreeMap<String, Vec<f64>>,
    },
    Constant {
        value: Vec<f64>,
    },
    Sum {
        signs: Vec<i8>,
    },
    Gain {
        gain: Vec<f64>,
    },
    Integrator {
        initial: Vec<f64>,
    },
    UnitDelay {
        initial: Vec<f64>,
    },
    Scope,
    MFunction {
        source: String,
        entry: String,
        inputs: Vec<FunctionInput>,
        parameters: Vec<FunctionParameter>,
        #[serde(rename = "outputWidth")]
        output_width: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionInput {
    pub name: String,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionParameter {
    pub name: String,
    pub value: Vec<f64>,
}

impl BlockKind {
    pub(crate) fn inputs(&self) -> Vec<String> {
        match self {
            Self::Control { operation, .. } => (0..operation.input_count())
                .map(|i| format!("in{i}"))
                .collect(),
            Self::ResetIntegrator { .. } => vec!["in".into(), "reset".into()],
            Self::Constant { .. } | Self::Step { .. } => Vec::new(),
            Self::Sum { signs } => (0..signs.len()).map(|i| format!("in{i}")).collect(),
            Self::MFunction { inputs, .. } => {
                inputs.iter().map(|input| input.name.clone()).collect()
            }
            _ => vec!["in".into()],
        }
    }

    pub(crate) fn direct_feedthrough(&self) -> bool {
        matches!(
            self,
            Self::Sum { .. }
                | Self::Gain { .. }
                | Self::Scope
                | Self::MFunction { .. }
                | Self::Control { .. }
                | Self::ResetIntegrator { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    pub block: String,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub from: Port,
    pub to: Port,
}
