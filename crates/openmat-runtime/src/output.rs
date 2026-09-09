use std::{error::Error, fmt};

use openmat_graphics_model::GraphicsNotice;
use openmat_value::Value;

/// Numeric presentation selected by MATLAB's `format` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericFormat {
    Short,
    Long,
    ShortE,
    LongE,
    ShortG,
    LongG,
    ShortEng,
    LongEng,
    Bank,
    Hex,
    Rational,
}

impl NumericFormat {
    /// Returns the MATLAB option spelling used by `format` query results.
    #[must_use]
    pub const fn matlab_name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Long => "long",
            Self::ShortE => "shortE",
            Self::LongE => "longE",
            Self::ShortG => "shortG",
            Self::LongG => "longG",
            Self::ShortEng => "shortEng",
            Self::LongEng => "longEng",
            Self::Bank => "bank",
            Self::Hex => "hex",
            Self::Rational => "rational",
        }
    }
}

/// Blank-line policy selected by `format compact` or `format loose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSpacing {
    Compact,
    Loose,
}

impl LineSpacing {
    /// Returns the MATLAB option spelling used by `format` query results.
    #[must_use]
    pub const fn matlab_name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Loose => "loose",
        }
    }
}

/// Per-session command-window display settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayFormat {
    /// Numeric rendering mode.
    pub numeric: NumericFormat,
    /// Command-window blank-line policy.
    pub line_spacing: LineSpacing,
}

impl Default for DisplayFormat {
    fn default() -> Self {
        Self {
            numeric: NumericFormat::Short,
            line_spacing: LineSpacing::Loose,
        }
    }
}

/// A structured output event emitted by runtime code.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputEvent {
    /// A versioned App Designer payload, without live runtime handles.
    UiDisplay(String),
    /// A value submitted to the host's display renderer.
    Display(Value),
    /// A value submitted through MATLAB's automatic `name =` display path.
    NamedDisplay { name: String, value: Value },
    /// A committed graphics change without numerical buffers or server token.
    GraphicsNotice(GraphicsNotice),
    /// Requests that the host clear its command-window presentation only.
    CommandWindowClear,
    /// UTF-8 text written directly to the command window.
    CommandText(String),
    /// Changes formatting for subsequent display events in this ordered stream.
    DisplayFormatChanged(DisplayFormat),
}

/// A host-output failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputError {
    /// Host-provided failure detail.
    pub message: String,
}

impl OutputError {
    /// Creates an output failure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for OutputError {}

/// Host boundary for structured interpreter output.
pub trait OutputSink: Send {
    /// Accepts one output event.
    ///
    /// # Errors
    ///
    /// Returns a host-output failure when the event cannot be delivered.
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError>;
}

/// An output sink that discards all events.
#[derive(Debug, Default)]
pub struct NullOutput;

impl OutputSink for NullOutput {
    fn emit(&mut self, _event: OutputEvent) -> Result<(), OutputError> {
        Ok(())
    }
}

/// An in-memory structured output sink for embedding and tests.
#[derive(Debug, Default)]
pub struct VecOutput {
    events: Vec<OutputEvent>,
}

impl VecOutput {
    /// Creates an empty output collector.
    #[must_use]
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Returns collected events.
    #[must_use]
    pub fn events(&self) -> &[OutputEvent] {
        &self.events
    }

    /// Consumes the collector and returns its events.
    #[must_use]
    pub fn into_events(self) -> Vec<OutputEvent> {
        self.events
    }
}

impl OutputSink for VecOutput {
    fn emit(&mut self, event: OutputEvent) -> Result<(), OutputError> {
        self.events.push(event);
        Ok(())
    }
}
