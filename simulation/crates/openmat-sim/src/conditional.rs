//! Bounded, sampled conditional execution. Numeric state commits remain transactional.
use crate::{ModelError, model::Port};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldReset {
    Held,
    Reset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConditionalOutput {
    pub initial: Vec<f64>,
    pub when_disabled: HoldReset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Execution {
    Enabled {
        period: f64,
        #[serde(rename = "statesWhenEnabling")]
        states_when_enabling: HoldReset,
        outputs: Vec<ConditionalOutput>,
    },
    Triggered {
        period: f64,
        edge: crate::hybrid::ResetMode,
        outputs: Vec<ConditionalOutput>,
    },
}

impl Execution {
    #[must_use]
    pub fn period(&self) -> f64 {
        match self {
            Self::Enabled { period, .. } | Self::Triggered { period, .. } => *period,
        }
    }
    #[must_use]
    pub fn outputs(&self) -> &[ConditionalOutput] {
        match self {
            Self::Enabled { outputs, .. } | Self::Triggered { outputs, .. } => outputs,
        }
    }
    #[must_use]
    pub fn control_port(&self) -> &'static str {
        match self {
            Self::Enabled { .. } => "enable",
            Self::Triggered { .. } => "trigger",
        }
    }
    pub(crate) fn validate(&self, outputs: usize) -> Result<(), ModelError> {
        if !self.period().is_finite()
            || self.period() <= 0.0
            || self.outputs().len() != outputs
            || self.outputs().iter().any(|p| {
                p.initial.is_empty()
                    || p.initial.len() > 4096
                    || p.initial.iter().any(|v| !v.is_finite())
                    || matches!(self, Self::Triggered { .. }) && p.when_disabled != HoldReset::Held
            })
        {
            return Err(ModelError::new(
                "conditional_parameter",
                "conditional execution needs a positive period and one finite initial/output policy per Outport; triggered outputs must hold",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DomainSpec {
    pub id: String,
    pub execution: Execution,
    pub control: Port,
    pub members: Vec<String>,
    pub outports: Vec<String>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DomainPlan {
    pub id: String,
    pub execution: Execution,
    pub control: (usize, usize),
    pub clock: usize,
    pub memory: usize,
    pub outputs: std::collections::BTreeMap<usize, ConditionalOutput>,
}

#[derive(Clone, Debug)]
pub(crate) struct ConditionalPlan {
    pub domains: Vec<DomainPlan>,
    pub node_domains: Vec<Option<usize>>,
    pub immediate_states: Vec<bool>,
    pub initial_cache: Vec<(usize, Vec<f64>)>,
    pub scopes: std::collections::BTreeMap<usize, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEvent {
    pub block: String,
    pub kind: &'static str,
}
