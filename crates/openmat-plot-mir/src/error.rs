use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirError {
    NonFinite(&'static str),
    Negative(&'static str),
    NonPositive(&'static str),
    InvalidRectangle(&'static str),
    InvalidColorComponent,
    InvalidFontWeight,
    InvalidCommandStream(&'static str),
    ArithmeticOverflow(&'static str),
    IndexOutOfBounds { index: u32, vertex_count: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionError {
    NonFinite(&'static str),
    NonPositive(&'static str),
    ZeroSpan(&'static str),
    Overflow(&'static str),
    InvalidViewport,
    BoxTooSmall,
}

impl Display for InteractionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(field) => write!(formatter, "{field} must be finite"),
            Self::NonPositive(field) => write!(formatter, "{field} must be positive"),
            Self::ZeroSpan(field) => write!(formatter, "{field} must have non-zero spans"),
            Self::Overflow(field) => write!(formatter, "{field} arithmetic overflowed"),
            Self::InvalidViewport => formatter.write_str("interaction viewport is degenerate"),
            Self::BoxTooSmall => formatter.write_str("box zoom must cover a non-zero CSS area"),
        }
    }
}

impl Error for InteractionError {}

impl Display for MirError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(field) => write!(formatter, "{field} must be finite"),
            Self::Negative(field) => write!(formatter, "{field} must not be negative"),
            Self::NonPositive(field) => write!(formatter, "{field} must be positive"),
            Self::InvalidRectangle(field) => {
                write!(formatter, "{field} must have ordered finite extents")
            }
            Self::InvalidColorComponent => {
                formatter.write_str("RGBA components must be finite and in 0..=1")
            }
            Self::InvalidFontWeight => formatter.write_str("font weight must be in 1..=1000"),
            Self::InvalidCommandStream(reason) => {
                write!(formatter, "invalid MIR command stream: {reason}")
            }
            Self::ArithmeticOverflow(field) => write!(formatter, "{field} arithmetic overflowed"),
            Self::IndexOutOfBounds {
                index,
                vertex_count,
            } => write!(
                formatter,
                "mesh index {index} is outside vertex count {vertex_count}"
            ),
        }
    }
}

impl Error for MirError {}
