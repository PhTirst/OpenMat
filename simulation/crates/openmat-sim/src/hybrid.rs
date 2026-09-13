//! Versioned control operations and pure event metadata.
use crate::{
    ModelError,
    model::SampleTime,
    numeric::{Comparison, Program},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ControlOp {
    Saturation {
        lower: Vec<f64>,
        upper: Vec<f64>,
    },
    Switch {
        criterion: SwitchCriterion,
        threshold: f64,
    },
    Relational {
        operator: Comparison,
    },
    Logical {
        operator: LogicOperator,
        inputs: usize,
    },
    Abs,
    MinMax {
        minimum: bool,
        inputs: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwitchCriterion {
    Greater,
    GreaterEqual,
    Nonzero,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicOperator {
    And,
    Or,
    Xor,
    Nand,
    Nor,
    Not,
    Nxor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    Rising,
    Falling,
    Either,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalType {
    Double,
    Logical,
}

impl ControlOp {
    #[must_use]
    pub fn input_count(&self) -> usize {
        match self {
            Self::Saturation { .. } | Self::Abs => 1,
            Self::Switch { .. } => 3,
            Self::Relational { .. } => 2,
            Self::Logical { inputs, .. } | Self::MinMax { inputs, .. } => *inputs,
        }
    }
    /// # Errors
    /// Reject malformed, non-finite or unbounded operation parameters.
    pub fn validate(&self) -> Result<(), ModelError> {
        let valid = match self {
            Self::Saturation { lower, upper } => {
                let width = lower.len().max(upper.len());
                width > 0
                    && width <= 4096
                    && !lower.is_empty()
                    && !upper.is_empty()
                    && (lower.len() == 1 || lower.len() == width)
                    && (upper.len() == 1 || upper.len() == width)
                    && (0..width).all(|i| {
                        let a = lower[i % lower.len()];
                        let b = upper[i % upper.len()];
                        a.is_finite() && b.is_finite() && a <= b
                    })
            }
            Self::Switch { threshold, .. } => threshold.is_finite(),
            Self::Logical {
                operator: LogicOperator::Not,
                inputs,
            } => *inputs == 1,
            Self::Logical { inputs, .. } => (2..=64).contains(inputs),
            Self::MinMax { inputs, .. } => (1..=64).contains(inputs),
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(ModelError::new(
                "control_parameter",
                "invalid control operation parameters or input count",
            ))
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInfo {
    pub block: String,
    pub surface: String,
    pub sample_time: SampleTime,
    pub locate: bool,
    #[serde(skip)]
    pub(crate) reset: Option<ResetTarget>,
}
#[derive(Clone, Debug)]
pub(crate) struct ResetTarget {
    pub discrete: bool,
    pub offset: usize,
    pub initial: Vec<f64>,
    pub mode: ResetMode,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPlan {
    pub signals: BTreeMap<String, SignalType>,
    pub events: Vec<EventInfo>,
    #[serde(skip)]
    pub(crate) program: Program,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRecord {
    pub block: String,
    pub surface: String,
    pub kind: &'static str,
    pub direction: i32,
}
