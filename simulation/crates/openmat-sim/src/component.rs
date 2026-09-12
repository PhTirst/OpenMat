//! Portable component definitions. Runtime state never lives in this metadata.
use crate::{
    ModelError,
    model::{BlockKind, FunctionInput, Model},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Callback {
    pub source: String,
    pub entry: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub value: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentDefinition {
    pub id: String,
    pub name: String,
    pub category: String,
    pub icon: String,
    pub inputs: Vec<FunctionInput>,
    pub outputs: Vec<FunctionInput>,
    pub parameters: Vec<Parameter>,
    pub continuous_states: usize,
    pub discrete_states: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialize: Option<Callback>,
    pub outputs_function: Callback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivatives: Option<Callback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Callback>,
}

impl ComponentDefinition {
    #[must_use]
    pub fn callbacks(&self) -> Vec<&Callback> {
        self.initialize
            .iter()
            .chain(std::iter::once(&self.outputs_function))
            .chain(self.derivatives.iter())
            .chain(self.update.iter())
            .collect()
    }

    /// # Errors
    /// Rejects ambiguous interfaces, incompatible states and unbounded metadata.
    pub fn validate(&self) -> Result<(), ModelError> {
        let fail = || {
            ModelError::new(
                "component_definition",
                format!("invalid component definition: {}", self.id),
            )
        };
        let valid_id = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        };
        if !valid_id(&self.id)
            || self.name.is_empty()
            || self.name.len() > 256
            || self.category.len() > 128
            || !["function", "plant", "controller", "filter", "delay"].contains(&self.icon.as_str())
            || self.inputs.len() > 64
            || self.outputs.is_empty()
            || self.outputs.len() > 64
            || self.parameters.len() > 64
            || self.continuous_states > 4096
            || self.discrete_states > 4096
            || self.continuous_states + self.discrete_states > 4096
            || self.initialize.is_some() != (self.continuous_states + self.discrete_states > 0)
            || self.derivatives.is_some() != (self.continuous_states > 0)
            || self.update.is_some() != (self.discrete_states > 0)
            || self.sample_time.is_some() != (self.discrete_states > 0)
            || self.sample_time.is_some_and(|t| !t.is_finite() || t <= 0.0)
            || self
                .callbacks()
                .iter()
                .any(|c| !crate::m_function::valid_path(&c.source) || !identifier(&c.entry))
        {
            return Err(fail());
        }
        for ports in [&self.inputs, &self.outputs] {
            let mut names = BTreeSet::new();
            if ports.iter().any(|p| {
                !identifier(&p.name) || !names.insert(&p.name) || !(1..=4096).contains(&p.width)
            }) || ports.iter().map(|p| p.width).sum::<usize>() > 4096
            {
                return Err(fail());
            }
        }
        let mut names = BTreeSet::new();
        if self.parameters.iter().any(|p| {
            !identifier(&p.name)
                || !names.insert(&p.name)
                || p.value.is_empty()
                || p.value.len() > 4096
                || p.minimum.is_some_and(|v| !v.is_finite())
                || p.maximum.is_some_and(|v| !v.is_finite())
                || p.minimum.zip(p.maximum).is_some_and(|(a, b)| a > b)
                || p.label.as_ref().is_some_and(|s| s.len() > 256)
                || p.unit.as_ref().is_some_and(|s| s.len() > 64)
                || !p.accepts(&p.value)
        }) || self.parameters.iter().map(|p| p.value.len()).sum::<usize>() > 4096
        {
            return Err(fail());
        }
        Ok(())
    }

    /// # Errors
    /// Rejects unknown overrides, shape changes and out-of-range values.
    pub fn parameter_values(
        &self,
        overrides: &BTreeMap<String, Vec<f64>>,
    ) -> Result<Vec<f64>, ModelError> {
        if overrides
            .keys()
            .any(|name| !self.parameters.iter().any(|p| &p.name == name))
        {
            return Err(ModelError::new(
                "component_parameter",
                "unknown component parameter override",
            ));
        }
        let mut values = Vec::new();
        for parameter in &self.parameters {
            let value = overrides.get(&parameter.name).unwrap_or(&parameter.value);
            if !parameter.accepts(value) {
                return Err(ModelError::new(
                    "component_parameter",
                    format!("invalid value or shape for {}", parameter.name),
                ));
            }
            values.extend(value);
        }
        Ok(values)
    }
}

impl Parameter {
    fn accepts(&self, values: &[f64]) -> bool {
        values.len() == self.value.len()
            && values.iter().all(|v| {
                v.is_finite()
                    && self.minimum.is_none_or(|m| *v >= m)
                    && self.maximum.is_none_or(|m| *v <= m)
            })
    }
}

pub(crate) fn identifier(name: &str) -> bool {
    name.len() <= 63
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[must_use]
pub fn source_references(model: &Model) -> Vec<&str> {
    let mut references = BTreeSet::new();
    for block in &model.blocks {
        match &block.kind {
            BlockKind::MFunction { source, .. } => {
                references.insert(source.as_str());
            }
            BlockKind::Component { component, .. } => {
                if let Some(definition) = model.components.iter().find(|d| &d.id == component) {
                    references.extend(
                        definition
                            .callbacks()
                            .into_iter()
                            .map(|c| c.source.as_str()),
                    );
                }
            }
            _ => {}
        }
    }
    references.into_iter().collect()
}
