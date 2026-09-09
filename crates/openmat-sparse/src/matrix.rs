use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use openmat_array::{Complex64, Logical, Shape};

use crate::{SparseError, SparseInvariant};

const CANCELLATION_INTERVAL: usize = 1_024;

/// One zero-based coordinate/value tuple accepted by COO construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CooEntry<T> {
    /// Zero-based row.
    pub row: u64,
    /// Zero-based column.
    pub column: u64,
    /// Entry value. Duplicate coordinates are combined before zero removal.
    pub value: T,
}

impl<T> CooEntry<T> {
    /// Creates a zero-based COO entry.
    #[must_use]
    pub const fn new(row: u64, column: u64, value: T) -> Self {
        Self { row, column, value }
    }
}

/// Scalar behavior needed to normalize typed CSC storage.
pub trait CscElement: Clone + PartialEq {
    /// Combines duplicate COO coordinates using the language constructor rule.
    fn combine(&mut self, other: &Self);
    /// Returns whether this value must be removed from normalized storage.
    fn is_explicit_zero(&self) -> bool;
}

impl CscElement for f64 {
    fn combine(&mut self, other: &Self) {
        *self += *other;
    }

    fn is_explicit_zero(&self) -> bool {
        *self == 0.0
    }
}

impl CscElement for Complex64 {
    fn combine(&mut self, other: &Self) {
        *self += *other;
    }

    fn is_explicit_zero(&self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }
}

impl CscElement for Logical {
    fn combine(&mut self, other: &Self) {
        *self = Logical::from(self.get() || other.get());
    }

    fn is_explicit_zero(&self) -> bool {
        !self.get()
    }
}

#[derive(Debug, PartialEq)]
struct CscStorage<T> {
    col_offsets: Vec<u64>,
    row_indices: Vec<u64>,
    values: Vec<T>,
    reserved_nnz: usize,
}

/// Owned canonical compressed-sparse-column storage.
///
/// Rows and columns are `u64`, indices are zero based internally, and clones
/// share all three buffers. `col_offsets` is monotone, each column's row slice
/// is strictly increasing, duplicate COO coordinates have already been
/// combined, and no explicit zero is stored. This Rust type is not a stable
/// plugin or process ABI.
#[derive(Clone, Debug)]
pub struct CscMatrix<T> {
    shape: Shape,
    storage: Arc<CscStorage<T>>,
}

impl<T: PartialEq> PartialEq for CscMatrix<T> {
    fn eq(&self, other: &Self) -> bool {
        self.shape == other.shape && self.storage == other.storage
    }
}

impl<T> CscMatrix<T> {
    /// Returns the canonical two-dimensional shape.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Returns the number of rows.
    #[must_use]
    pub fn rows(&self) -> u64 {
        self.shape.extent(0)
    }

    /// Returns the number of columns.
    #[must_use]
    pub fn columns(&self) -> u64 {
        self.shape.extent(1)
    }

    /// Returns the language-visible dense element count.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.shape.numel()
    }

    /// Returns the stored nonzero count.
    #[must_use]
    pub fn nnz(&self) -> usize {
        self.storage.values.len()
    }

    /// Returns MATLAB-compatible reserved nonzero metadata.
    ///
    /// R2022b reports at least one even for an empty sparse allocation.
    #[must_use]
    pub fn nzmax(&self) -> usize {
        self.storage.reserved_nnz.max(1)
    }

    /// Borrows monotone zero-based CSC column offsets.
    #[must_use]
    pub fn col_offsets(&self) -> &[u64] {
        &self.storage.col_offsets
    }

    /// Borrows zero-based row indices, strictly increasing within each column.
    #[must_use]
    pub fn row_indices(&self) -> &[u64] {
        &self.storage.row_indices
    }

    /// Borrows stored nonzero values.
    #[must_use]
    pub fn values(&self) -> &[T] {
        &self.storage.values
    }

    /// Iterates stored entries in column-major order.
    #[must_use]
    pub fn entries(&self) -> CscEntries<'_, T> {
        CscEntries {
            matrix: self,
            offset: 0,
            column: 0,
        }
    }

    /// Returns one stored entry by zero-based storage position.
    #[must_use]
    pub fn stored_entry(&self, position: usize) -> Option<(u64, u64, &T)> {
        let row = *self.storage.row_indices.get(position)?;
        let offset = u64::try_from(position).ok()?;
        let upper = self
            .storage
            .col_offsets
            .partition_point(|column_offset| *column_offset <= offset);
        let column = upper.checked_sub(1)?;
        let column = u64::try_from(column).ok()?;
        let value = self.storage.values.get(position)?;
        Some((row, column, value))
    }

    /// Returns whether two clones share the complete CSC allocation.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    /// Returns deterministic MATLAB-style CSC payload bytes.
    ///
    /// This counts one `u64` per column offset and row index plus reserved value
    /// payload. It intentionally excludes Rust `Arc`/`Vec` headers and allocator
    /// over-allocation.
    #[must_use]
    pub fn payload_bytes(&self) -> Option<u64> {
        let offsets = u64::try_from(self.storage.col_offsets.len()).ok()?;
        let reserved = u64::try_from(self.nzmax()).ok()?;
        let index_bytes = u64::try_from(size_of::<u64>()).ok()?;
        let value_bytes = u64::try_from(size_of::<T>()).ok()?;
        offsets
            .checked_mul(index_bytes)?
            .checked_add(reserved.checked_mul(index_bytes.checked_add(value_bytes)?)?)
    }

    /// Returns the bytes addressable through the current vector capacities.
    #[must_use]
    pub fn allocated_buffer_bytes(&self) -> Option<u64> {
        let offsets = u64::try_from(self.storage.col_offsets.capacity()).ok()?;
        let rows = u64::try_from(self.storage.row_indices.capacity()).ok()?;
        let values = u64::try_from(self.storage.values.capacity()).ok()?;
        offsets
            .checked_mul(u64::try_from(size_of::<u64>()).ok()?)?
            .checked_add(rows.checked_mul(u64::try_from(size_of::<u64>()).ok()?)?)?
            .checked_add(values.checked_mul(u64::try_from(size_of::<T>()).ok()?)?)
    }

    /// Looks up a zero-based coordinate without materializing a dense value.
    #[must_use]
    pub fn get_zero_based(&self, row: u64, column: u64) -> Option<&T> {
        if row >= self.rows() || column >= self.columns() {
            return None;
        }
        let column = usize::try_from(column).ok()?;
        let start = usize::try_from(*self.storage.col_offsets.get(column)?).ok()?;
        let end = usize::try_from(*self.storage.col_offsets.get(column + 1)?).ok()?;
        let relative = self
            .storage
            .row_indices
            .get(start..end)?
            .binary_search(&row)
            .ok()?;
        self.storage.values.get(start + relative)
    }

    /// Looks up a one-based language linear index.
    ///
    /// # Errors
    ///
    /// Rejects zero, an empty target, or an index beyond `numel`.
    pub fn get_linear_one_based(&self, index: u64) -> Result<Option<&T>, SparseError> {
        if index == 0 || index > self.numel() {
            return Err(SparseError::LinearIndexOutOfBounds {
                index,
                numel: self.numel(),
            });
        }
        let offset = index - 1;
        let rows = self.rows();
        debug_assert!(
            rows > 0,
            "a valid linear index implies a nonempty row extent"
        );
        Ok(self.get_zero_based(offset % rows, offset / rows))
    }

    /// Looks up exactly two one-based language subscripts.
    ///
    /// # Errors
    ///
    /// Rejects zero or out-of-range row/column subscripts.
    pub fn get_subscripts_one_based(
        &self,
        row: u64,
        column: u64,
    ) -> Result<Option<&T>, SparseError> {
        for (dimension, (index, extent)) in [(row, self.rows()), (column, self.columns())]
            .into_iter()
            .enumerate()
        {
            if index == 0 || index > extent {
                return Err(SparseError::SubscriptOutOfBounds {
                    dimension,
                    index,
                    extent,
                });
            }
        }
        Ok(self.get_zero_based(row - 1, column - 1))
    }
}

impl<T: CscElement> CscMatrix<T> {
    /// Creates an empty CSC matrix with a fallibly reserved nonzero capacity.
    ///
    /// # Errors
    ///
    /// Returns a checked shape, host-length, allocation, or cancellation error.
    pub fn try_empty(
        rows: u64,
        columns: u64,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let shape = sparse_shape(rows, columns)?;
        check_cancelled(cancellation, 0)?;
        let column_count = checked_usize(columns)?;
        let offset_count = column_count
            .checked_add(1)
            .ok_or(SparseError::HostLengthOverflow { elements: columns })?;
        let mut col_offsets = reserved_vec("column offsets", offset_count)?;
        for index in 0..offset_count {
            check_cancelled(cancellation, index)?;
            col_offsets.push(0);
        }
        let row_indices = reserved_vec("row indices", reserved_nnz)?;
        let values = reserved_vec("values", reserved_nnz)?;
        Ok(Self {
            shape,
            storage: Arc::new(CscStorage {
                col_offsets,
                row_indices,
                values,
                reserved_nnz,
            }),
        })
    }

    /// Normalizes zero-based COO entries into owned CSC storage.
    ///
    /// Duplicate coordinates are combined before explicit-zero removal. `NaN`
    /// and infinities are nonzero and remain stored. The reserve may exceed the
    /// matrix element count, as required by `spalloc`.
    ///
    /// # Errors
    ///
    /// Returns a checked bounds, shape, allocation, or cancellation failure.
    pub fn try_from_coo(
        rows: u64,
        columns: u64,
        entries: Vec<CooEntry<T>>,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let shape = sparse_shape(rows, columns)?;
        check_cancelled(cancellation, 0)?;
        for (index, entry) in entries.iter().enumerate() {
            check_cancelled(cancellation, index)?;
            if entry.row >= rows || entry.column >= columns {
                return Err(SparseError::CoordinateOutOfBounds {
                    row: entry.row,
                    column: entry.column,
                    rows,
                    columns,
                });
            }
        }
        let mut ordered_entries = reserved_vec("COO ordering", entries.len())?;
        for (index, entry) in entries.into_iter().enumerate() {
            check_cancelled(cancellation, index)?;
            ordered_entries.push((entry, index));
        }
        ordered_entries.sort_unstable_by_key(|(entry, index)| (entry.column, entry.row, *index));
        check_cancelled(cancellation, ordered_entries.len())?;

        let working_capacity = reserved_nnz.max(ordered_entries.len());
        let mut row_indices = reserved_vec("row indices", working_capacity)?;
        let mut values = reserved_vec("values", working_capacity)?;
        let column_count = checked_usize(columns)?;
        let offset_count = column_count
            .checked_add(1)
            .ok_or(SparseError::HostLengthOverflow { elements: columns })?;
        let mut col_offsets = reserved_vec("column offsets", offset_count)?;
        col_offsets.push(0);
        let mut output_column = 0_u64;
        let mut input = 0_usize;
        while input < ordered_entries.len() {
            check_cancelled(cancellation, input)?;
            let entry = &ordered_entries[input].0;
            let coordinate = (entry.column, entry.row);
            let mut value = entry.value.clone();
            input += 1;
            while input < ordered_entries.len() {
                check_cancelled(cancellation, input)?;
                let entry = &ordered_entries[input].0;
                if (entry.column, entry.row) != coordinate {
                    break;
                }
                value.combine(&entry.value);
                input += 1;
            }
            while output_column < coordinate.0 {
                col_offsets.push(checked_u64(values.len())?);
                output_column += 1;
            }
            if !value.is_explicit_zero() {
                row_indices.push(coordinate.1);
                values.push(value);
            }
        }
        while output_column < columns {
            check_cancelled(cancellation, col_offsets.len())?;
            col_offsets.push(checked_u64(values.len())?);
            output_column += 1;
        }
        debug_assert_eq!(col_offsets.len(), offset_count);
        let normalized_reserve = reserved_nnz.max(values.len());
        Ok(Self {
            shape,
            storage: Arc::new(CscStorage {
                col_offsets,
                row_indices,
                values,
                reserved_nnz: normalized_reserve,
            }),
        })
    }

    /// Validates caller-supplied normalized CSC parts and takes ownership.
    ///
    /// # Errors
    ///
    /// Returns the exact rejected invariant, or a checked allocation/cancel
    /// failure while extending the declared reserve.
    pub fn try_from_canonical_parts(
        rows: u64,
        columns: u64,
        col_offsets: Vec<u64>,
        mut row_indices: Vec<u64>,
        mut values: Vec<T>,
        reserved_nnz: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let shape = sparse_shape(rows, columns)?;
        check_cancelled(cancellation, 0)?;
        let columns = checked_usize(columns)?;
        if col_offsets.len()
            != columns
                .checked_add(1)
                .ok_or(SparseError::InvalidCsc(SparseInvariant::ColumnOffsetLength))?
        {
            return Err(SparseError::InvalidCsc(SparseInvariant::ColumnOffsetLength));
        }
        if row_indices.len() != values.len() {
            return Err(SparseError::InvalidCsc(SparseInvariant::StorageLength));
        }
        if reserved_nnz < values.len() {
            return Err(SparseError::InvalidCsc(SparseInvariant::ReserveTooSmall));
        }
        if col_offsets.first().copied() != Some(0) {
            return Err(SparseError::InvalidCsc(SparseInvariant::FirstColumnOffset));
        }
        let nnz = checked_u64(values.len())?;
        if col_offsets.last().copied() != Some(nnz) {
            return Err(SparseError::InvalidCsc(SparseInvariant::FinalColumnOffset));
        }
        for column in 0..columns {
            check_cancelled(cancellation, column)?;
            let start = checked_usize(col_offsets[column])?;
            let end = checked_usize(col_offsets[column + 1])?;
            if start > end || end > values.len() {
                return Err(SparseError::InvalidCsc(SparseInvariant::ColumnOffsets));
            }
            let mut previous = None;
            for offset in start..end {
                check_cancelled(cancellation, offset)?;
                let row = row_indices[offset];
                if row >= rows {
                    return Err(SparseError::InvalidCsc(SparseInvariant::RowOutOfBounds));
                }
                if previous.is_some_and(|previous| row <= previous) {
                    return Err(SparseError::InvalidCsc(SparseInvariant::RowOrder));
                }
                if values[offset].is_explicit_zero() {
                    return Err(SparseError::InvalidCsc(SparseInvariant::ExplicitZero));
                }
                previous = Some(row);
            }
        }
        reserve_more("row indices", &mut row_indices, reserved_nnz)?;
        reserve_more("values", &mut values, reserved_nnz)?;
        Ok(Self {
            shape,
            storage: Arc::new(CscStorage {
                col_offsets,
                row_indices,
                values,
                reserved_nnz,
            }),
        })
    }

    /// Transposes CSC storage while applying an optional value transform.
    ///
    /// # Errors
    ///
    /// Returns a checked allocation or cancellation failure.
    pub fn try_transpose<U: CscElement>(
        &self,
        mut transform: impl FnMut(&T) -> U,
        cancellation: Option<&AtomicBool>,
    ) -> Result<CscMatrix<U>, SparseError> {
        let mut entries = reserved_vec("transpose COO", self.nnz())?;
        for (position, (row, column, value)) in self.entries().enumerate() {
            check_cancelled(cancellation, position)?;
            entries.push(CooEntry::new(column, row, transform(value)));
        }
        CscMatrix::try_from_coo(
            self.columns(),
            self.rows(),
            entries,
            self.storage.reserved_nnz,
            cancellation,
        )
    }
}

/// Iterator over canonical `(row, column, value)` tuples.
pub struct CscEntries<'a, T> {
    matrix: &'a CscMatrix<T>,
    offset: usize,
    column: usize,
}

impl<'a, T> Iterator for CscEntries<'a, T> {
    type Item = (u64, u64, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.matrix.storage.values.len() {
            return None;
        }
        while self.column + 1 < self.matrix.storage.col_offsets.len()
            && self.matrix.storage.col_offsets[self.column + 1]
                <= u64::try_from(self.offset).ok()?
        {
            self.column += 1;
        }
        let row = *self.matrix.storage.row_indices.get(self.offset)?;
        let value = self.matrix.storage.values.get(self.offset)?;
        let column = u64::try_from(self.column).ok()?;
        self.offset += 1;
        Some((row, column, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.matrix.storage.values.len().saturating_sub(self.offset);
        (remaining, Some(remaining))
    }
}

impl<T> ExactSizeIterator for CscEntries<'_, T> {}

fn sparse_shape(rows: u64, columns: u64) -> Result<Shape, SparseError> {
    Shape::new([rows, columns]).map_err(|_| SparseError::ShapeOverflow { rows, columns })
}

fn checked_usize(value: u64) -> Result<usize, SparseError> {
    usize::try_from(value).map_err(|_| SparseError::HostLengthOverflow { elements: value })
}

fn checked_u64(value: usize) -> Result<u64, SparseError> {
    u64::try_from(value).map_err(|_| SparseError::HostLengthOverflow { elements: u64::MAX })
}

fn reserved_vec<T>(storage: &'static str, capacity: usize) -> Result<Vec<T>, SparseError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| SparseError::Allocation {
            storage,
            elements: capacity,
        })?;
    Ok(values)
}

fn reserve_more<T>(
    storage: &'static str,
    values: &mut Vec<T>,
    capacity: usize,
) -> Result<(), SparseError> {
    if capacity > values.capacity() {
        values
            .try_reserve_exact(capacity - values.len())
            .map_err(|_| SparseError::Allocation {
                storage,
                elements: capacity,
            })?;
    }
    Ok(())
}

fn check_cancelled(cancellation: Option<&AtomicBool>, progress: usize) -> Result<(), SparseError> {
    if progress.is_multiple_of(CANCELLATION_INTERVAL)
        && cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
    {
        Err(SparseError::Cancelled)
    } else {
        Ok(())
    }
}
