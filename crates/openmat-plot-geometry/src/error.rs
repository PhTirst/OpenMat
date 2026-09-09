use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryError {
    LengthMismatch {
        x: usize,
        y: usize,
    },
    FieldLengthMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    InvalidGridShape {
        rows: usize,
        columns: usize,
    },
    InvalidPatch(&'static str),
    DegenerateBounds(&'static str),
    InvalidCamera(&'static str),
    InvalidProjection(&'static str),
    NonFiniteCoordinate,
    DegenerateViewport,
    ArithmeticOverflow(&'static str),
    UnsupportedDashPattern,
    InvalidRoundSegments,
    InvalidSourceIndex,
    Mir(String),
}

impl Display for GeometryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { x, y } => {
                write!(formatter, "series x/y lengths differ: {x} versus {y}")
            }
            Self::FieldLengthMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "{field} requires {expected} elements but received {actual}"
            ),
            Self::InvalidGridShape { rows, columns } => {
                write!(formatter, "surface grid shape {rows}x{columns} is invalid")
            }
            Self::InvalidPatch(message) => write!(formatter, "invalid patch mesh: {message}"),
            Self::DegenerateBounds(axis) => {
                write!(
                    formatter,
                    "3D {axis} bounds must be finite and strictly increasing"
                )
            }
            Self::InvalidCamera(field) => write!(formatter, "3D camera {field} is invalid"),
            Self::InvalidProjection(field) => {
                write!(formatter, "3D projection {field} is invalid")
            }
            Self::NonFiniteCoordinate => formatter.write_str("geometry coordinate must be finite"),
            Self::DegenerateViewport => {
                formatter.write_str("geometry viewport must have non-zero CSS dimensions")
            }
            Self::ArithmeticOverflow(field) => write!(formatter, "arithmetic overflow in {field}"),
            Self::UnsupportedDashPattern => {
                formatter.write_str("dash tessellation is reserved beyond the solid v1 path")
            }
            Self::InvalidRoundSegments => {
                formatter.write_str("round tessellation requires at least one segment")
            }
            Self::InvalidSourceIndex => {
                formatter.write_str("source indices must use positive one-based values")
            }
            Self::Mir(message) => write!(formatter, "invalid MIR output: {message}"),
        }
    }
}

impl Error for GeometryError {}

impl From<openmat_plot_mir::MirError> for GeometryError {
    fn from(error: openmat_plot_mir::MirError) -> Self {
        Self::Mir(error.to_string())
    }
}
