use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    LengthMismatch { x: usize, y: usize },
    InvalidLimits,
    NonFiniteInput(&'static str),
    InvalidTickTarget,
    InvalidTickSequence,
    ArithmeticOverflow(&'static str),
    DegenerateViewport,
    MissingTextMeasurement(String),
    DuplicateTextMeasurement(String),
    UnexpectedTextMeasurement(String),
    InvalidOverlay(String),
}

impl Display for LayoutError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { x, y } => {
                write!(formatter, "series x/y lengths differ: {x} versus {y}")
            }
            Self::InvalidLimits => formatter.write_str("axis limits must be finite and increasing"),
            Self::NonFiniteInput(field) => write!(formatter, "{field} must be finite"),
            Self::InvalidTickTarget => formatter.write_str("tick target must be at least two"),
            Self::InvalidTickSequence => {
                formatter.write_str("tick values must be non-NaN and strictly increasing")
            }
            Self::ArithmeticOverflow(field) => write!(formatter, "arithmetic overflow in {field}"),
            Self::DegenerateViewport => {
                formatter.write_str("viewport must have non-zero CSS width and height")
            }
            Self::MissingTextMeasurement(key) => {
                write!(formatter, "missing text measurement for {key}")
            }
            Self::DuplicateTextMeasurement(key) => {
                write!(formatter, "duplicate text measurement for {key}")
            }
            Self::UnexpectedTextMeasurement(key) => {
                write!(formatter, "unexpected text measurement for {key}")
            }
            Self::InvalidOverlay(message) => write!(formatter, "invalid overlay plan: {message}"),
        }
    }
}

impl Error for LayoutError {}
