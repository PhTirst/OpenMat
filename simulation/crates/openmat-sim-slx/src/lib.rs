//! Import a documented executable subset of Simulink SLX model documents.
#![forbid(unsafe_code)]

mod blocks;
mod compatibility;
mod configuration;
mod control;
mod control_blocks;
mod control_emit;
mod control_graph;
mod document;
mod literal;
mod lower;
mod parameters;

use openmat_opc::{Limits, Package};
use openmat_sim::model::Model;
use serde::Serialize;
use std::fmt;

pub use configuration::ConfigurationObject;
pub use control::ControlModel;
pub use document::{Block, Document, Line, PortProperties, Properties, Source, System};
pub use parameters::{MAX_PARAMETER_TEXT, ParameterArray, Parameters};

#[derive(Clone, Debug, Serialize)]
pub struct Issue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}
impl Issue {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            block: None,
            parameter: None,
            part: None,
        }
    }
    pub(crate) fn at(mut self, block: &Block, parameter: Option<&str>) -> Self {
        self.block = Some(block.sid.clone());
        self.part = Some(block.source.part.clone());
        if parameter.is_some() {
            self.parameter = parameter.map(str::to_owned);
        }
        self
    }
}
impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Issue {}
impl From<openmat_opc::Error> for Issue {
    fn from(error: openmat_opc::Error) -> Self {
        Self {
            code: error.code.into(),
            message: error.message,
            part: error.part,
            block: None,
            parameter: None,
        }
    }
}

pub struct ImportedSlx {
    document: Document,
    package: Package,
}
impl ImportedSlx {
    /// Parse structure and preserve the original package, without executing callbacks.
    ///
    /// # Errors
    /// Rejects invalid packages or ambiguous/unbounded model structure.
    pub fn read(bytes: &[u8], name: &str) -> Result<Self, Issue> {
        let package = Package::read(bytes, Limits::default())?;
        let document = document::read(&package, name)?;
        Ok(Self { document, package })
    }
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
    /// Original parts remain available for unsupported properties and binary assets.
    #[must_use]
    pub fn package(&self) -> &Package {
        &self.package
    }
    /// Translate only a validated execution profile; loading alone does not imply compatibility.
    ///
    /// # Errors
    /// Reports unsupported versions, dependencies, parameters, connectivity or execution semantics.
    pub fn lower(&self) -> Result<Model, Vec<Issue>> {
        lower::lower(&self.document, &self.package)
    }
}
