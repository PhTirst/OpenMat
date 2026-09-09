use std::{error::Error, fmt};

/// The canonical CSC invariant rejected by checked construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseInvariant {
    /// `col_offsets` does not have exactly `columns + 1` entries.
    ColumnOffsetLength,
    /// The first column offset is not zero.
    FirstColumnOffset,
    /// Column offsets decrease or point past `nnz`.
    ColumnOffsets,
    /// The final column offset is not exactly `nnz`.
    FinalColumnOffset,
    /// Row indices and stored values have different lengths.
    StorageLength,
    /// A row index is outside the matrix.
    RowOutOfBounds,
    /// Row indices within one column are not strictly increasing.
    RowOrder,
    /// A stored value is an explicit zero.
    ExplicitZero,
    /// The declared reserve is smaller than the stored-entry count.
    ReserveTooSmall,
}

/// A checked CSC construction, conversion, or indexing failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SparseError {
    /// The two-dimensional language shape overflows the `u64` shape model.
    ShapeOverflow { rows: u64, columns: u64 },
    /// A host vector length or vector byte size cannot be represented.
    HostLengthOverflow { elements: u64 },
    /// A fallible allocation request was rejected.
    Allocation {
        storage: &'static str,
        elements: usize,
    },
    /// A zero-based COO coordinate is outside the declared shape.
    CoordinateOutOfBounds {
        row: u64,
        column: u64,
        rows: u64,
        columns: u64,
    },
    /// A one-based language linear index is invalid.
    LinearIndexOutOfBounds { index: u64, numel: u64 },
    /// A one-based language subscript is invalid.
    SubscriptOutOfBounds {
        dimension: usize,
        index: u64,
        extent: u64,
    },
    /// Checked offset arithmetic overflowed.
    OffsetOverflow,
    /// Caller-supplied CSC parts are not normalized.
    InvalidCsc(SparseInvariant),
    /// A dense array has no sparse representation in the first tranche.
    UnsupportedDenseType,
    /// Arithmetic operands cannot be expanded to one common two-dimensional shape.
    ShapeMismatch {
        operation: &'static str,
        lhs: Vec<u64>,
        rhs: Vec<u64>,
    },
    /// A structural operation would change the language-visible element count.
    ElementCountMismatch { source: u64, destination: u64 },
    /// Indexed assignment has neither a scalar source nor one source per destination.
    AssignmentSizeMismatch { selected: u64, supplied: u64 },
    /// A concatenation request used an unsupported dimension.
    UnsupportedConcatenationDimension { dimension: usize },
    /// Cooperative cancellation was observed.
    Cancelled,
}

impl fmt::Display for SparseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeOverflow { rows, columns } => {
                write!(
                    formatter,
                    "sparse shape {rows}x{columns} overflows the shape model"
                )
            }
            Self::HostLengthOverflow { elements } => {
                write!(
                    formatter,
                    "sparse storage length {elements} does not fit this host"
                )
            }
            Self::Allocation { storage, elements } => {
                write!(
                    formatter,
                    "cannot allocate {elements} elements for sparse {storage}"
                )
            }
            Self::CoordinateOutOfBounds {
                row,
                column,
                rows,
                columns,
            } => write!(
                formatter,
                "zero-based sparse coordinate ({row}, {column}) is outside {rows}x{columns}"
            ),
            Self::LinearIndexOutOfBounds { index, numel } => write!(
                formatter,
                "one-based sparse linear index {index} is outside 1..={numel}"
            ),
            Self::SubscriptOutOfBounds {
                dimension,
                index,
                extent,
            } => write!(
                formatter,
                "one-based sparse subscript {index} is outside dimension {dimension} with extent {extent}"
            ),
            Self::OffsetOverflow => formatter.write_str("sparse offset arithmetic overflowed"),
            Self::InvalidCsc(invariant) => {
                write!(formatter, "invalid normalized CSC storage: {invariant:?}")
            }
            Self::UnsupportedDenseType => formatter.write_str(
                "only logical, double, and complex double dense arrays can become sparse",
            ),
            Self::ShapeMismatch {
                operation,
                lhs,
                rhs,
            } => write!(
                formatter,
                "sparse {operation} shape mismatch: {lhs:?} and {rhs:?}"
            ),
            Self::ElementCountMismatch {
                source,
                destination,
            } => write!(
                formatter,
                "sparse reshape cannot change element count from {source} to {destination}"
            ),
            Self::AssignmentSizeMismatch { selected, supplied } => write!(
                formatter,
                "sparse indexed assignment selected {selected} elements but supplied {supplied}"
            ),
            Self::UnsupportedConcatenationDimension { dimension } => write!(
                formatter,
                "sparse concatenation dimension {dimension} is unsupported"
            ),
            Self::Cancelled => formatter.write_str("sparse operation was cancelled"),
        }
    }
}

impl Error for SparseError {}
