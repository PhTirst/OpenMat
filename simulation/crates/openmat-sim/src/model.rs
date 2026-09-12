//! Serializable authoring model, independent of editor and execution backend.
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    pub schema_version: u32,
    pub name: String,
    pub settings: Settings,
    pub blocks: Vec<Block>,
    pub connections: Vec<Connection>,
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
    Constant { value: Vec<f64> },
    Sum { signs: Vec<i8> },
    Gain { gain: Vec<f64> },
    Integrator { initial: Vec<f64> },
    UnitDelay { initial: Vec<f64> },
    Scope,
}

impl BlockKind {
    pub(crate) fn inputs(&self) -> Vec<String> {
        match self {
            Self::Constant { .. } => Vec::new(),
            Self::Sum { signs } => (0..signs.len()).map(|i| format!("in{i}")).collect(),
            _ => vec!["in".into()],
        }
    }

    pub(crate) fn direct_feedthrough(&self) -> bool {
        matches!(self, Self::Sum { .. } | Self::Gain { .. } | Self::Scope)
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
