use std::sync::atomic::AtomicBool;

use openmat_array::{ArrayData, Complex64, DType, DenseArray, Logical, Shape};

use crate::{CooEntry, CscMatrix, SparseError};

const CANCELLATION_INTERVAL: usize = 1_024;

/// One logical, real-double, or complex-double sparse element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SparseScalarValue {
    /// Logical element.
    Logical(bool),
    /// Real double element.
    F64(f64),
    /// Complex double element.
    ComplexF64(Complex64),
}

/// One normalized stored CSC entry for display and observation adapters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SparseStoredEntry {
    /// Zero-based row. Language presentation adds one.
    pub row: u64,
    /// Zero-based column. Language presentation adds one.
    pub column: u64,
    /// Stored nonzero value.
    pub value: SparseScalarValue,
}

/// The three first-tranche MATLAB sparse storage classes.
#[derive(Clone, Debug, PartialEq)]
pub enum SparseArrayData {
    /// Sparse logical values.
    Logical(CscMatrix<Logical>),
    /// Sparse real double values.
    F64(CscMatrix<f64>),
    /// Sparse complex double values.
    ComplexF64(CscMatrix<Complex64>),
}

impl SparseArrayData {
    /// Builds normalized sparse logical CSC from zero-based COO entries.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions or coordinates, allocation failure,
    /// arithmetic overflow, or cancellation.
    pub fn try_from_logical_coo(
        rows: u64,
        columns: u64,
        entries: Vec<CooEntry<Logical>>,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        CscMatrix::try_from_coo(rows, columns, entries, reserved_nnz, cancellation)
            .map(Self::Logical)
    }

    /// Builds normalized sparse real-double CSC from zero-based COO entries.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions or coordinates, allocation failure,
    /// arithmetic overflow, or cancellation.
    pub fn try_from_f64_coo(
        rows: u64,
        columns: u64,
        entries: Vec<CooEntry<f64>>,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        CscMatrix::try_from_coo(rows, columns, entries, reserved_nnz, cancellation).map(Self::F64)
    }

    /// Builds normalized complex COO and canonicalizes an all-real result.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions or coordinates, allocation failure,
    /// arithmetic overflow, or cancellation.
    pub fn try_from_complex_f64_coo(
        rows: u64,
        columns: u64,
        entries: Vec<CooEntry<Complex64>>,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let complex = CscMatrix::try_from_coo(rows, columns, entries, reserved_nnz, cancellation)?;
        let mut all_real = true;
        for (position, value) in complex.values().iter().enumerate() {
            check_cancelled(cancellation, position)?;
            all_real &= value.im == 0.0;
        }
        if all_real {
            let mut entries = try_reserved("real canonicalization COO", complex.nnz())?;
            for (position, (row, column, value)) in complex.entries().enumerate() {
                check_cancelled(cancellation, position)?;
                entries.push(CooEntry::new(row, column, value.re));
            }
            CscMatrix::try_from_coo(rows, columns, entries, reserved_nnz, cancellation)
                .map(Self::F64)
        } else {
            Ok(Self::ComplexF64(complex))
        }
    }

    /// Creates an empty real-double sparse allocation for `spalloc`.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested dimensions or capacity do not fit,
    /// allocation fails, or cancellation is requested.
    pub fn try_spalloc(
        rows: u64,
        columns: u64,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        CscMatrix::try_empty(rows, columns, reserved_nnz, cancellation).map(Self::F64)
    }

    /// Creates a sparse real-double rectangular identity matrix.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested dimensions do not fit, allocation fails,
    /// or cancellation is requested.
    pub fn try_speye(
        rows: u64,
        columns: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let diagonal = rows.min(columns);
        let capacity = usize::try_from(diagonal)
            .map_err(|_| SparseError::HostLengthOverflow { elements: diagonal })?;
        let mut entries = try_reserved("identity COO", capacity)?;
        for index in 0..diagonal {
            check_cancelled(cancellation, checked_usize(index)?)?;
            entries.push(CooEntry::new(index, index, 1.0));
        }
        Self::try_from_f64_coo(rows, columns, entries, capacity, cancellation)
    }

    /// Converts supported dense storage into normalized sparse CSC.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported dense storage, allocation failure, arithmetic
    /// overflow, or cancellation.
    pub fn try_from_dense(
        dense: &ArrayData,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match dense {
            ArrayData::Logical(array) => dense_to_coo(array, |value| value.get(), cancellation)
                .and_then(|entries| {
                    Self::try_from_logical_coo(
                        array.shape().extent(0),
                        array.shape().extent(1),
                        entries,
                        0,
                        cancellation,
                    )
                }),
            ArrayData::F64(array) => dense_to_coo(array, |value| *value != 0.0, cancellation)
                .and_then(|entries| {
                    Self::try_from_f64_coo(
                        array.shape().extent(0),
                        array.shape().extent(1),
                        entries,
                        0,
                        cancellation,
                    )
                }),
            ArrayData::ComplexF64(array) => dense_to_coo(
                array,
                |value| value.re != 0.0 || value.im != 0.0,
                cancellation,
            )
            .and_then(|entries| {
                Self::try_from_complex_f64_coo(
                    array.shape().extent(0),
                    array.shape().extent(1),
                    entries,
                    0,
                    cancellation,
                )
            }),
            ArrayData::F32(_)
            | ArrayData::ComplexF32(_)
            | ArrayData::Char(_)
            | ArrayData::Integer(_) => Err(SparseError::UnsupportedDenseType),
        }
    }

    /// Returns the exact storage type.
    #[must_use]
    pub const fn dtype(&self) -> DType {
        match self {
            Self::Logical(_) => DType::Logical,
            Self::F64(_) => DType::F64,
            Self::ComplexF64(_) => DType::ComplexF64,
        }
    }

    /// Returns the MATLAB-compatible class name.
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        self.dtype().class_name()
    }

    /// Returns whether complex-double storage is present.
    #[must_use]
    pub const fn is_complex(&self) -> bool {
        matches!(self, Self::ComplexF64(_))
    }

    /// Returns the canonical two-dimensional shape.
    #[must_use]
    pub fn shape(&self) -> &Shape {
        match self {
            Self::Logical(matrix) => matrix.shape(),
            Self::F64(matrix) => matrix.shape(),
            Self::ComplexF64(matrix) => matrix.shape(),
        }
    }

    /// Returns the language-visible dense element count.
    #[must_use]
    pub fn numel(&self) -> u64 {
        self.shape().numel()
    }

    /// Returns the stored nonzero count.
    #[must_use]
    pub fn nnz(&self) -> usize {
        match self {
            Self::Logical(matrix) => matrix.nnz(),
            Self::F64(matrix) => matrix.nnz(),
            Self::ComplexF64(matrix) => matrix.nnz(),
        }
    }

    /// Returns MATLAB-compatible reserved nonzero metadata.
    #[must_use]
    pub fn nzmax(&self) -> usize {
        match self {
            Self::Logical(matrix) => matrix.nzmax(),
            Self::F64(matrix) => matrix.nzmax(),
            Self::ComplexF64(matrix) => matrix.nzmax(),
        }
    }

    /// Returns deterministic MATLAB-style CSC payload bytes.
    #[must_use]
    pub fn payload_bytes(&self) -> Option<u64> {
        match self {
            Self::Logical(matrix) => matrix.payload_bytes(),
            Self::F64(matrix) => matrix.payload_bytes(),
            Self::ComplexF64(matrix) => matrix.payload_bytes(),
        }
    }

    /// Returns a normalized stored entry by zero-based storage position.
    ///
    /// This allocation-free adapter is intended for basic display and protocol
    /// observation without exposing any third-party sparse container.
    #[must_use]
    pub fn stored_entry(&self, position: usize) -> Option<SparseStoredEntry> {
        let (row, column, value) = match self {
            Self::Logical(matrix) => {
                let (row, column, value) = matrix.stored_entry(position)?;
                (row, column, SparseScalarValue::Logical(value.get()))
            }
            Self::F64(matrix) => {
                let (row, column, value) = matrix.stored_entry(position)?;
                (row, column, SparseScalarValue::F64(*value))
            }
            Self::ComplexF64(matrix) => {
                let (row, column, value) = matrix.stored_entry(position)?;
                (row, column, SparseScalarValue::ComplexF64(*value))
            }
        };
        Some(SparseStoredEntry { row, column, value })
    }

    /// Returns one zero-based column-major element, including implicit zero.
    ///
    /// # Errors
    ///
    /// Rejects an offset outside `0..numel`.
    pub fn element_at_offset(&self, offset: u64) -> Result<SparseScalarValue, SparseError> {
        if offset >= self.numel() {
            return Err(SparseError::LinearIndexOutOfBounds {
                index: offset.saturating_add(1),
                numel: self.numel(),
            });
        }
        let rows = self.shape().extent(0);
        debug_assert!(rows > 0, "a valid sparse offset implies nonzero rows");
        let row = offset % rows;
        let column = offset / rows;
        Ok(match self {
            Self::Logical(matrix) => SparseScalarValue::Logical(
                matrix
                    .get_zero_based(row, column)
                    .is_some_and(|value| value.get()),
            ),
            Self::F64(matrix) => {
                SparseScalarValue::F64(matrix.get_zero_based(row, column).copied().unwrap_or(0.0))
            }
            Self::ComplexF64(matrix) => SparseScalarValue::ComplexF64(
                matrix
                    .get_zero_based(row, column)
                    .copied()
                    .unwrap_or(Complex64::ZERO),
            ),
        })
    }

    /// Returns whether two values share their complete CSC allocation.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Logical(left), Self::Logical(right)) => left.shares_storage_with(right),
            (Self::F64(left), Self::F64(right)) => left.shares_storage_with(right),
            (Self::ComplexF64(left), Self::ComplexF64(right)) => left.shares_storage_with(right),
            _ => false,
        }
    }

    /// Borrows sparse logical CSC storage.
    #[must_use]
    pub const fn as_logical(&self) -> Option<&CscMatrix<Logical>> {
        match self {
            Self::Logical(matrix) => Some(matrix),
            Self::F64(_) | Self::ComplexF64(_) => None,
        }
    }

    /// Borrows sparse real-double CSC storage.
    #[must_use]
    pub const fn as_f64(&self) -> Option<&CscMatrix<f64>> {
        match self {
            Self::F64(matrix) => Some(matrix),
            Self::Logical(_) | Self::ComplexF64(_) => None,
        }
    }

    /// Borrows sparse complex-double CSC storage.
    #[must_use]
    pub const fn as_complex_f64(&self) -> Option<&CscMatrix<Complex64>> {
        match self {
            Self::ComplexF64(matrix) => Some(matrix),
            Self::Logical(_) | Self::F64(_) => None,
        }
    }

    /// Materializes a dense column-major array of the same class and shape.
    ///
    /// # Errors
    ///
    /// Returns an error when the dense allocation does not fit, allocation fails, or
    /// cancellation is requested.
    pub fn try_to_dense(
        &self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ArrayData, SparseError> {
        match self {
            Self::Logical(matrix) => {
                materialize(matrix, Logical::FALSE, cancellation).map(ArrayData::Logical)
            }
            Self::F64(matrix) => materialize(matrix, 0.0, cancellation).map(ArrayData::F64),
            Self::ComplexF64(matrix) => {
                materialize(matrix, Complex64::ZERO, cancellation).map(ArrayData::ComplexF64)
            }
        }
    }

    /// Returns stored values as a dense `nnz x 1` column.
    ///
    /// # Errors
    ///
    /// Returns an error when the result allocation does not fit, allocation fails, or
    /// cancellation is requested.
    pub fn try_nonzeros(
        &self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ArrayData, SparseError> {
        match self {
            Self::Logical(matrix) => {
                dense_column(matrix.values(), cancellation).map(ArrayData::Logical)
            }
            Self::F64(matrix) => dense_column(matrix.values(), cancellation).map(ArrayData::F64),
            Self::ComplexF64(matrix) => {
                dense_column(matrix.values(), cancellation).map(ArrayData::ComplexF64)
            }
        }
    }

    /// Returns sparse double ones at every stored coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error when allocation fails, arithmetic overflows, or cancellation
    /// is requested.
    pub fn try_spones(&self, cancellation: Option<&AtomicBool>) -> Result<Self, SparseError> {
        macro_rules! ones {
            ($matrix:expr) => {{
                let mut entries = try_reserved("spones COO", $matrix.nnz())?;
                for (position, (row, column, _)) in $matrix.entries().enumerate() {
                    check_cancelled(cancellation, position)?;
                    entries.push(CooEntry::new(row, column, 1.0));
                }
                Self::try_from_f64_coo(
                    $matrix.rows(),
                    $matrix.columns(),
                    entries,
                    $matrix.nnz(),
                    cancellation,
                )
            }};
        }
        match self {
            Self::Logical(matrix) => ones!(matrix),
            Self::F64(matrix) => ones!(matrix),
            Self::ComplexF64(matrix) => ones!(matrix),
        }
    }

    /// Returns transpose or conjugate transpose while preserving sparse form.
    ///
    /// # Errors
    ///
    /// Returns an error when allocation fails, arithmetic overflows, or cancellation
    /// is requested.
    pub fn try_transpose(
        &self,
        conjugate: bool,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match self {
            Self::Logical(matrix) => matrix
                .try_transpose(|value| *value, cancellation)
                .map(Self::Logical),
            Self::F64(matrix) => matrix
                .try_transpose(|value| *value, cancellation)
                .map(Self::F64),
            Self::ComplexF64(matrix) => {
                let matrix = matrix.try_transpose(
                    |value| if conjugate { value.conjugate() } else { *value },
                    cancellation,
                )?;
                Ok(Self::ComplexF64(matrix))
            }
        }
    }

    /// Returns sparse `1x1` storage for one one-based linear index.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid index, allocation failure, arithmetic overflow,
    /// or cancellation.
    pub fn try_index_linear_one_based(
        &self,
        index: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match self {
            Self::Logical(matrix) => {
                scalar_logical(matrix.get_linear_one_based(index)?.copied(), cancellation)
            }
            Self::F64(matrix) => {
                scalar_f64(matrix.get_linear_one_based(index)?.copied(), cancellation)
            }
            Self::ComplexF64(matrix) => {
                scalar_complex(matrix.get_linear_one_based(index)?.copied(), cancellation)
            }
        }
    }

    /// Returns sparse `1x1` storage for exactly two one-based subscripts.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid subscripts, allocation failure, arithmetic
    /// overflow, or cancellation.
    pub fn try_index_subscripts_one_based(
        &self,
        row: u64,
        column: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match self {
            Self::Logical(matrix) => scalar_logical(
                matrix.get_subscripts_one_based(row, column)?.copied(),
                cancellation,
            ),
            Self::F64(matrix) => scalar_f64(
                matrix.get_subscripts_one_based(row, column)?.copied(),
                cancellation,
            ),
            Self::ComplexF64(matrix) => scalar_complex(
                matrix.get_subscripts_one_based(row, column)?.copied(),
                cancellation,
            ),
        }
    }
}

fn scalar_logical(
    value: Option<Logical>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let entries = value
        .filter(|value| value.get())
        .map_or_else(Vec::new, |value| vec![CooEntry::new(0, 0, value)]);
    SparseArrayData::try_from_logical_coo(1, 1, entries, 1, cancellation)
}

fn scalar_f64(
    value: Option<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let entries = value
        .filter(|value| *value != 0.0)
        .map_or_else(Vec::new, |value| vec![CooEntry::new(0, 0, value)]);
    SparseArrayData::try_from_f64_coo(1, 1, entries, 1, cancellation)
}

fn scalar_complex(
    value: Option<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let entries = value
        .filter(|value| value.re != 0.0 || value.im != 0.0)
        .map_or_else(Vec::new, |value| vec![CooEntry::new(0, 0, value)]);
    SparseArrayData::try_from_complex_f64_coo(1, 1, entries, 1, cancellation)
}

fn dense_to_coo<T: Clone>(
    array: &DenseArray<T>,
    mut nonzero: impl FnMut(&T) -> bool,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<CooEntry<T>>, SparseError> {
    if array.shape().ndims() != 2 {
        return Err(SparseError::UnsupportedDenseType);
    }
    let mut count = 0_usize;
    for (offset, value) in array.as_slice().iter().enumerate() {
        check_cancelled(cancellation, offset)?;
        count += usize::from(nonzero(value));
    }
    let mut entries = try_reserved("dense conversion COO", count)?;
    let rows = array.shape().extent(0);
    for (offset, value) in array.as_slice().iter().enumerate() {
        check_cancelled(cancellation, offset)?;
        if nonzero(value) {
            let offset = u64::try_from(offset)
                .map_err(|_| SparseError::HostLengthOverflow { elements: u64::MAX })?;
            entries.push(CooEntry::new(offset % rows, offset / rows, value.clone()));
        }
    }
    Ok(entries)
}

fn materialize<T: Clone>(
    matrix: &CscMatrix<T>,
    zero: T,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<T>, SparseError> {
    let length = checked_usize(matrix.numel())?;
    let mut values = try_reserved("full values", length)?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        values.push(zero.clone());
    }
    let rows = matrix.rows();
    for (position, (row, column, value)) in matrix.entries().enumerate() {
        check_cancelled(cancellation, position)?;
        let offset = row
            .checked_add(
                column
                    .checked_mul(rows)
                    .ok_or(SparseError::OffsetOverflow)?,
            )
            .ok_or(SparseError::OffsetOverflow)?;
        values[checked_usize(offset)?] = value.clone();
    }
    DenseArray::from_vec(matrix.shape().clone(), values).map_err(|_| {
        SparseError::HostLengthOverflow {
            elements: matrix.numel(),
        }
    })
}

fn dense_column<T: Clone>(
    source: &[T],
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<T>, SparseError> {
    let mut values = try_reserved("nonzeros values", source.len())?;
    for (index, value) in source.iter().enumerate() {
        check_cancelled(cancellation, index)?;
        values.push(value.clone());
    }
    let length = u64::try_from(source.len())
        .map_err(|_| SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let shape = Shape::new([length, 1]).map_err(|_| SparseError::ShapeOverflow {
        rows: length,
        columns: 1,
    })?;
    DenseArray::from_vec(shape, values)
        .map_err(|_| SparseError::HostLengthOverflow { elements: length })
}

fn try_reserved<T>(storage: &'static str, capacity: usize) -> Result<Vec<T>, SparseError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| SparseError::Allocation {
            storage,
            elements: capacity,
        })?;
    Ok(values)
}

fn checked_usize(value: u64) -> Result<usize, SparseError> {
    usize::try_from(value).map_err(|_| SparseError::HostLengthOverflow { elements: value })
}

fn check_cancelled(cancellation: Option<&AtomicBool>, progress: usize) -> Result<(), SparseError> {
    use std::sync::atomic::Ordering;
    if progress.is_multiple_of(CANCELLATION_INTERVAL)
        && cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
    {
        Err(SparseError::Cancelled)
    } else {
        Ok(())
    }
}
