//! Resolved, zero-offset periodic sampling metadata for schema 5.
use crate::model::SampleTime;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleClock {
    pub id: usize,
    pub period: f64,
    /// Exact multiplier of the configured base step.
    pub ticks: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingPlan {
    pub clocks: Vec<SampleClock>,
    pub blocks: BTreeMap<String, SampleTime>,
    #[serde(skip)]
    pub(crate) state_clocks: Vec<usize>,
    #[serde(skip)]
    pub(crate) cache_count: usize,
    #[serde(skip)]
    pub(crate) output_offsets: Vec<Vec<Option<usize>>>,
    #[serde(skip)]
    pub(crate) node_clocks: Vec<Option<usize>>,
    #[serde(skip)]
    pub(crate) conditional: Option<crate::conditional::ConditionalPlan>,
}

impl SamplingPlan {
    #[must_use]
    pub fn held_value_count(&self) -> usize {
        self.cache_count
    }
}
