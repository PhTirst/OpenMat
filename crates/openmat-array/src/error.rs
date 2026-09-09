use std::error::Error;
use std::fmt;

/// Failures produced while constructing or indexing a dense array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrayError {
    /// A cumulative dimension product cannot be represented by the 64-bit
    /// shape model.
    SizeOverflow {
        /// Zero-based dimension at which multiplication overflowed.
        dimension: usize,
        /// Cumulative product before applying `extent`.
        partial: u64,
        /// Extent that caused the overflow.
        extent: u64,
    },
    /// A checked multiply or add used to form an offset overflowed.
    OffsetOverflow,
    /// The array has too many elements for a host `Vec` index.
    HostLengthOverflow {
        /// Number of elements requested by the shape.
        numel: u64,
    },
    /// The supplied buffer length does not match the shape.
    DataLengthMismatch {
        /// Number of elements required by the shape.
        expected: u64,
        /// Number of elements supplied by the caller.
        actual: usize,
    },
    /// At least one subscript is required.
    NoSubscripts,
    /// A one-based multidimensional subscript is outside its effective extent.
    IndexOutOfBounds {
        /// Zero-based position of the subscript in the indexing expression.
        dimension: usize,
        /// One-based index supplied by the language boundary.
        index: u64,
        /// Effective extent of this dimension.
        extent: u64,
    },
    /// A one-based linear index is outside `1..=numel`.
    LinearIndexOutOfBounds {
        /// One-based linear index supplied by the language boundary.
        index: u64,
        /// Total number of elements in the array.
        numel: u64,
    },
    /// A colon range used a zero step.
    ZeroRangeStep,
    /// Computing the number of elements in a range overflowed.
    RangeLengthOverflow,
}

impl fmt::Display for ArrayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeOverflow {
                dimension,
                partial,
                extent,
            } => write!(
                formatter,
                "shape product overflows at dimension {dimension}: {partial} * {extent}"
            ),
            Self::OffsetOverflow => formatter.write_str("array offset computation overflowed"),
            Self::HostLengthOverflow { numel } => {
                write!(formatter, "array length {numel} does not fit this host")
            }
            Self::DataLengthMismatch { expected, actual } => write!(
                formatter,
                "array shape requires {expected} elements but buffer contains {actual}"
            ),
            Self::NoSubscripts => formatter.write_str("at least one subscript is required"),
            Self::IndexOutOfBounds {
                dimension,
                index,
                extent,
            } => write!(
                formatter,
                "index {index} is outside dimension {dimension} with extent {extent}"
            ),
            Self::LinearIndexOutOfBounds { index, numel } => write!(
                formatter,
                "linear index {index} is outside an array with {numel} elements"
            ),
            Self::ZeroRangeStep => formatter.write_str("index range step cannot be zero"),
            Self::RangeLengthOverflow => {
                formatter.write_str("index range length cannot be represented")
            }
        }
    }
}

impl Error for ArrayError {}
