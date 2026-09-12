use std::fmt;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelError(pub Box<Diagnostic>);

impl ModelError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self(Box::new(Diagnostic {
            code: code.into(),
            message: message.into(),
            block: None,
            port: None,
            source_path: None,
            line: None,
            column: None,
        }))
    }

    pub(crate) fn at(mut self, block: &str, port: Option<&str>) -> Self {
        self.0.block = Some(block.into());
        self.0.port = port.map(str::to_owned);
        self
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)?;
        if let Some(block) = &self.0.block {
            write!(f, " [block={block}")?;
            if let Some(port) = &self.0.port {
                write!(f, ", port={port}")?;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

impl std::error::Error for ModelError {}
