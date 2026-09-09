use std::sync::atomic::AtomicBool;

use openmat_array::{Complex64, Logical, Shape};

use crate::{CooEntry, SparseArrayData, SparseError, SparseScalarValue};

impl SparseArrayData {
    /// Gathers zero-based column-major offsets into an explicitly supplied shape.
    ///
    /// Repeated offsets and the orientation computed by the language index
    /// resolver are preserved.
    ///
    /// # Errors
    /// Returns an invalid shape, offset, allocation, overflow, or cancellation failure.
    pub fn try_gather_offsets(
        &self,
        offsets: &[usize],
        shape: &Shape,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        if shape.numel()
            != u64::try_from(offsets.len())
                .map_err(|_| SparseError::HostLengthOverflow { elements: u64::MAX })?
        {
            return Err(SparseError::ElementCountMismatch {
                source: u64::try_from(offsets.len()).unwrap_or(u64::MAX),
                destination: shape.numel(),
            });
        }
        let rows = shape.extent(0);
        match self {
            Self::Logical(_) => {
                let mut entries = try_reserved("logical gather COO", offsets.len())?;
                for (position, &offset) in offsets.iter().enumerate() {
                    check_cancelled(cancellation, position)?;
                    let value = self.element_at_offset(checked_u64(offset)?)?;
                    if matches!(value, SparseScalarValue::Logical(true)) {
                        let (row, column) = result_coordinate(rows, position)?;
                        entries.push(CooEntry::new(row, column, Logical::TRUE));
                    }
                }
                Self::try_from_logical_coo(rows, shape.extent(1), entries, 0, cancellation)
            }
            Self::F64(_) => {
                let mut entries = try_reserved("real gather COO", offsets.len())?;
                for (position, &offset) in offsets.iter().enumerate() {
                    check_cancelled(cancellation, position)?;
                    let SparseScalarValue::F64(value) =
                        self.element_at_offset(checked_u64(offset)?)?
                    else {
                        unreachable!("real sparse gather changed class")
                    };
                    if value != 0.0 {
                        let (row, column) = result_coordinate(rows, position)?;
                        entries.push(CooEntry::new(row, column, value));
                    }
                }
                Self::try_from_f64_coo(rows, shape.extent(1), entries, 0, cancellation)
            }
            Self::ComplexF64(_) => {
                let mut entries = try_reserved("complex gather COO", offsets.len())?;
                for (position, &offset) in offsets.iter().enumerate() {
                    check_cancelled(cancellation, position)?;
                    let SparseScalarValue::ComplexF64(value) =
                        self.element_at_offset(checked_u64(offset)?)?
                    else {
                        unreachable!("complex sparse gather changed class")
                    };
                    if value.re != 0.0 || value.im != 0.0 {
                        let (row, column) = result_coordinate(rows, position)?;
                        entries.push(CooEntry::new(row, column, value));
                    }
                }
                Self::try_from_complex_f64_coo(rows, shape.extent(1), entries, 0, cancellation)
            }
        }
    }

    /// Transactionally assigns scalar or conforming sparse source values at
    /// zero-based column-major destination offsets.
    ///
    /// # Errors
    /// Returns an invalid offset, size mismatch, allocation, overflow, or cancellation failure.
    pub fn try_assign_offsets(
        &self,
        offsets: &[usize],
        supplied: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let selected = checked_u64(offsets.len())?;
        if supplied.numel() != 1 && supplied.numel() != selected {
            return Err(SparseError::AssignmentSizeMismatch {
                selected,
                supplied: supplied.numel(),
            });
        }
        if offsets.is_empty() {
            return Ok(self.clone());
        }
        let mut updates = try_reserved("assignment updates", offsets.len())?;
        for (position, &offset) in offsets.iter().enumerate() {
            check_cancelled(cancellation, position)?;
            let offset_u64 = checked_u64(offset)?;
            if offset_u64 >= self.numel() {
                return Err(SparseError::LinearIndexOutOfBounds {
                    index: offset_u64.saturating_add(1),
                    numel: self.numel(),
                });
            }
            let source = if supplied.numel() == 1 {
                0
            } else {
                checked_u64(position)?
            };
            updates.push((offset_u64, position, supplied.element_at_offset(source)?));
        }
        updates.sort_unstable_by_key(|(offset, position, _)| (*offset, *position));
        let mut normalized = try_reserved("normalized assignment updates", updates.len())?;
        for update in updates {
            if normalized
                .last()
                .is_some_and(|(offset, _, _)| *offset == update.0)
            {
                if let Some(last) = normalized.last_mut() {
                    *last = update;
                }
            } else {
                normalized.push(update);
            }
        }
        let promote_complex = self.is_complex() || supplied.is_complex();
        if matches!(self, Self::Logical(_)) {
            assign_logical(self, &normalized, cancellation)
        } else if promote_complex {
            assign_complex(self, &normalized, cancellation)
        } else {
            assign_real(self, &normalized, cancellation)
        }
    }

    /// Reshapes sparse storage without changing column-major element order.
    ///
    /// # Errors
    /// Returns an element-count mismatch, allocation, overflow, or cancellation failure.
    pub fn try_reshape_2d(
        &self,
        rows: u64,
        columns: u64,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let destination = rows
            .checked_mul(columns)
            .ok_or(SparseError::ShapeOverflow { rows, columns })?;
        if destination != self.numel() {
            return Err(SparseError::ElementCountMismatch {
                source: self.numel(),
                destination,
            });
        }
        macro_rules! reshape {
            ($matrix:expr, $constructor:ident) => {{
                let mut entries = try_reserved("reshape COO", $matrix.nnz())?;
                for (position, (row, column, value)) in $matrix.entries().enumerate() {
                    check_cancelled(cancellation, position)?;
                    let linear = row
                        .checked_add(
                            column
                                .checked_mul($matrix.rows())
                                .ok_or(SparseError::OffsetOverflow)?,
                        )
                        .ok_or(SparseError::OffsetOverflow)?;
                    debug_assert!(
                        rows > 0,
                        "a nonzero entry requires nonzero destination rows"
                    );
                    entries.push(CooEntry::new(linear % rows, linear / rows, *value));
                }
                Self::$constructor(rows, columns, entries, $matrix.nzmax(), cancellation)
            }};
        }
        match self {
            Self::Logical(matrix) => reshape!(matrix, try_from_logical_coo),
            Self::F64(matrix) => reshape!(matrix, try_from_f64_coo),
            Self::ComplexF64(matrix) => reshape!(matrix, try_from_complex_f64_coo),
        }
    }

    /// Deletes a set of zero-based linear offsets and reshapes the survivors
    /// according to MATLAB's linear-deletion orientation rule.
    ///
    /// # Errors
    /// Returns an invalid offset or shape, allocation, overflow, or cancellation failure.
    pub fn try_delete_linear_offsets(
        &self,
        offsets: &[usize],
        shape: &Shape,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let removed = normalized_offsets(offsets, self.numel(), cancellation)?;
        let remaining = self
            .numel()
            .checked_sub(checked_u64(removed.len())?)
            .ok_or(SparseError::OffsetOverflow)?;
        if shape.numel() != remaining {
            return Err(SparseError::ElementCountMismatch {
                source: remaining,
                destination: shape.numel(),
            });
        }
        let old_rows = self.shape().extent(0);
        let new_rows = shape.extent(0);
        macro_rules! delete {
            ($matrix:expr, $constructor:ident) => {{
                let mut entries = try_reserved("linear deletion COO", $matrix.nnz())?;
                for (position, (row, column, value)) in $matrix.entries().enumerate() {
                    check_cancelled(cancellation, position)?;
                    let offset = row + column * old_rows;
                    match removed.binary_search(&offset) {
                        Ok(_) => {}
                        Err(before) => {
                            let shifted = offset
                                .checked_sub(checked_u64(before)?)
                                .ok_or(SparseError::OffsetOverflow)?;
                            debug_assert!(new_rows > 0, "a surviving entry requires output rows");
                            entries.push(CooEntry::new(
                                shifted % new_rows,
                                shifted / new_rows,
                                *value,
                            ));
                        }
                    }
                }
                Self::$constructor(
                    shape.extent(0),
                    shape.extent(1),
                    entries,
                    $matrix.nzmax(),
                    cancellation,
                )
            }};
        }
        match self {
            Self::Logical(matrix) => delete!(matrix, try_from_logical_coo),
            Self::F64(matrix) => delete!(matrix, try_from_f64_coo),
            Self::ComplexF64(matrix) => delete!(matrix, try_from_complex_f64_coo),
        }
    }

    /// Deletes zero-based rows while preserving every remaining column.
    ///
    /// # Errors
    /// Returns an invalid row, allocation, overflow, or cancellation failure.
    pub fn try_delete_rows(
        &self,
        rows: &[u64],
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let removed = normalized_indices(rows, self.shape().extent(0), 0, cancellation)?;
        let output_rows = self.shape().extent(0) - checked_u64(removed.len())?;
        macro_rules! delete {
            ($matrix:expr, $constructor:ident) => {{
                let mut entries = try_reserved("row deletion COO", $matrix.nnz())?;
                for (position, (row, column, value)) in $matrix.entries().enumerate() {
                    check_cancelled(cancellation, position)?;
                    match removed.binary_search(&row) {
                        Ok(_) => {}
                        Err(before) => {
                            entries.push(CooEntry::new(row - checked_u64(before)?, column, *value))
                        }
                    }
                }
                Self::$constructor(
                    output_rows,
                    $matrix.columns(),
                    entries,
                    $matrix.nzmax(),
                    cancellation,
                )
            }};
        }
        match self {
            Self::Logical(matrix) => delete!(matrix, try_from_logical_coo),
            Self::F64(matrix) => delete!(matrix, try_from_f64_coo),
            Self::ComplexF64(matrix) => delete!(matrix, try_from_complex_f64_coo),
        }
    }

    /// Deletes zero-based columns while preserving every remaining row.
    ///
    /// # Errors
    /// Returns an invalid column, allocation, overflow, or cancellation failure.
    pub fn try_delete_columns(
        &self,
        columns: &[u64],
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let removed = normalized_indices(columns, self.shape().extent(1), 1, cancellation)?;
        let output_columns = self.shape().extent(1) - checked_u64(removed.len())?;
        macro_rules! delete {
            ($matrix:expr, $constructor:ident) => {{
                let mut entries = try_reserved("column deletion COO", $matrix.nnz())?;
                for (position, (row, column, value)) in $matrix.entries().enumerate() {
                    check_cancelled(cancellation, position)?;
                    match removed.binary_search(&column) {
                        Ok(_) => {}
                        Err(before) => {
                            entries.push(CooEntry::new(row, column - checked_u64(before)?, *value))
                        }
                    }
                }
                Self::$constructor(
                    $matrix.rows(),
                    output_columns,
                    entries,
                    $matrix.nzmax(),
                    cancellation,
                )
            }};
        }
        match self {
            Self::Logical(matrix) => delete!(matrix, try_from_logical_coo),
            Self::F64(matrix) => delete!(matrix, try_from_f64_coo),
            Self::ComplexF64(matrix) => delete!(matrix, try_from_complex_f64_coo),
        }
    }

    /// Concatenates sparse values along rows (`dimension == 0`) or columns
    /// (`dimension == 1`) while preserving canonical CSC order.
    ///
    /// # Errors
    /// Returns an unsupported dimension, shape mismatch, allocation, overflow, or cancellation failure.
    pub fn try_concatenate_2d(
        inputs: &[Self],
        dimension: usize,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        if dimension > 1 {
            return Err(SparseError::UnsupportedConcatenationDimension { dimension });
        }
        let Some(first) = inputs.first() else {
            return Self::try_spalloc(0, 0, 0, cancellation);
        };
        let mut rows = first.shape().extent(0);
        let mut columns = first.shape().extent(1);
        let mut nnz = 0_usize;
        let mut complex = false;
        let mut logical = true;
        for input in inputs {
            nnz = nnz
                .checked_add(input.nnz())
                .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
            complex |= input.is_complex();
            logical &= matches!(input, Self::Logical(_));
            if dimension == 0 {
                if input.shape().extent(1) != columns {
                    return Err(SparseError::ShapeMismatch {
                        operation: "vertical concatenation",
                        lhs: first.shape().dimensions().to_vec(),
                        rhs: input.shape().dimensions().to_vec(),
                    });
                }
            } else if input.shape().extent(0) != rows {
                return Err(SparseError::ShapeMismatch {
                    operation: "horizontal concatenation",
                    lhs: first.shape().dimensions().to_vec(),
                    rhs: input.shape().dimensions().to_vec(),
                });
            }
        }
        if dimension == 0 {
            rows = inputs.iter().try_fold(0_u64, |sum, input| {
                sum.checked_add(input.shape().extent(0))
                    .ok_or(SparseError::ShapeOverflow {
                        rows: u64::MAX,
                        columns,
                    })
            })?;
        } else {
            columns = inputs.iter().try_fold(0_u64, |sum, input| {
                sum.checked_add(input.shape().extent(1))
                    .ok_or(SparseError::ShapeOverflow {
                        rows,
                        columns: u64::MAX,
                    })
            })?;
        }
        if complex {
            concatenate_complex(inputs, dimension, rows, columns, nnz, cancellation)
        } else if logical {
            concatenate_logical(inputs, dimension, rows, columns, nnz, cancellation)
        } else {
            concatenate_real(inputs, dimension, rows, columns, nnz, cancellation)
        }
    }
}

fn assign_logical(
    target: &SparseArrayData,
    updates: &[(u64, usize, SparseScalarValue)],
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let capacity = target
        .nnz()
        .checked_add(updates.len())
        .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let mut entries = try_reserved("logical assignment COO", capacity)?;
    copy_unmodified(target, updates, &mut entries, cancellation, |value| {
        Logical::from(nonzero(value))
    })?;
    for &(offset, _, value) in updates {
        if nonzero(value) {
            let (row, column) = target_coordinates(target, offset);
            entries.push(CooEntry::new(row, column, Logical::TRUE));
        }
    }
    SparseArrayData::try_from_logical_coo(
        target.shape().extent(0),
        target.shape().extent(1),
        entries,
        target.nzmax(),
        cancellation,
    )
}

fn assign_real(
    target: &SparseArrayData,
    updates: &[(u64, usize, SparseScalarValue)],
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let capacity = target
        .nnz()
        .checked_add(updates.len())
        .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let mut entries = try_reserved("real assignment COO", capacity)?;
    copy_unmodified(target, updates, &mut entries, cancellation, real_value)?;
    for &(offset, _, value) in updates {
        let value = real_value(value);
        if value != 0.0 {
            let (row, column) = target_coordinates(target, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_f64_coo(
        target.shape().extent(0),
        target.shape().extent(1),
        entries,
        target.nzmax(),
        cancellation,
    )
}

fn assign_complex(
    target: &SparseArrayData,
    updates: &[(u64, usize, SparseScalarValue)],
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let capacity = target
        .nnz()
        .checked_add(updates.len())
        .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let mut entries = try_reserved("complex assignment COO", capacity)?;
    copy_unmodified(target, updates, &mut entries, cancellation, complex_value)?;
    for &(offset, _, value) in updates {
        let value = complex_value(value);
        if value.re != 0.0 || value.im != 0.0 {
            let (row, column) = target_coordinates(target, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_complex_f64_coo(
        target.shape().extent(0),
        target.shape().extent(1),
        entries,
        target.nzmax(),
        cancellation,
    )
}

fn copy_unmodified<T>(
    target: &SparseArrayData,
    updates: &[(u64, usize, SparseScalarValue)],
    output: &mut Vec<CooEntry<T>>,
    cancellation: Option<&AtomicBool>,
    convert: impl Fn(SparseScalarValue) -> T,
) -> Result<(), SparseError> {
    let rows = target.shape().extent(0);
    for position in 0..target.nnz() {
        check_cancelled(cancellation, position)?;
        let stored = target
            .stored_entry(position)
            .expect("validated sparse storage entry");
        let offset = stored.row + stored.column * rows;
        if updates
            .binary_search_by_key(&offset, |update| update.0)
            .is_err()
        {
            output.push(CooEntry::new(
                stored.row,
                stored.column,
                convert(stored.value),
            ));
        }
    }
    Ok(())
}

fn concatenate_logical(
    inputs: &[SparseArrayData],
    dimension: usize,
    rows: u64,
    columns: u64,
    nnz: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let mut entries = try_reserved("logical concatenation COO", nnz)?;
    concatenate_entries(inputs, dimension, &mut entries, cancellation, |value| {
        Logical::from(nonzero(value))
    })?;
    SparseArrayData::try_from_logical_coo(rows, columns, entries, nnz, cancellation)
}

fn concatenate_real(
    inputs: &[SparseArrayData],
    dimension: usize,
    rows: u64,
    columns: u64,
    nnz: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let mut entries = try_reserved("real concatenation COO", nnz)?;
    concatenate_entries(inputs, dimension, &mut entries, cancellation, real_value)?;
    SparseArrayData::try_from_f64_coo(rows, columns, entries, nnz, cancellation)
}

fn concatenate_complex(
    inputs: &[SparseArrayData],
    dimension: usize,
    rows: u64,
    columns: u64,
    nnz: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let mut entries = try_reserved("complex concatenation COO", nnz)?;
    concatenate_entries(inputs, dimension, &mut entries, cancellation, complex_value)?;
    SparseArrayData::try_from_complex_f64_coo(rows, columns, entries, nnz, cancellation)
}

fn concatenate_entries<T>(
    inputs: &[SparseArrayData],
    dimension: usize,
    output: &mut Vec<CooEntry<T>>,
    cancellation: Option<&AtomicBool>,
    convert: impl Fn(SparseScalarValue) -> T,
) -> Result<(), SparseError> {
    let mut row_offset = 0_u64;
    let mut column_offset = 0_u64;
    for input in inputs {
        for position in 0..input.nnz() {
            check_cancelled(cancellation, output.len())?;
            let entry = input
                .stored_entry(position)
                .expect("validated sparse storage entry");
            output.push(CooEntry::new(
                entry.row + row_offset,
                entry.column + column_offset,
                convert(entry.value),
            ));
        }
        if dimension == 0 {
            row_offset += input.shape().extent(0);
        } else {
            column_offset += input.shape().extent(1);
        }
    }
    Ok(())
}

fn target_coordinates(target: &SparseArrayData, offset: u64) -> (u64, u64) {
    let rows = target.shape().extent(0);
    (offset % rows, offset / rows)
}

fn result_coordinate(rows: u64, position: usize) -> Result<(u64, u64), SparseError> {
    let offset = checked_u64(position)?;
    debug_assert!(rows > 0, "a gathered element requires nonzero output rows");
    Ok((offset % rows, offset / rows))
}

fn nonzero(value: SparseScalarValue) -> bool {
    match value {
        SparseScalarValue::Logical(value) => value,
        SparseScalarValue::F64(value) => value != 0.0,
        SparseScalarValue::ComplexF64(value) => value.re != 0.0 || value.im != 0.0,
    }
}

fn real_value(value: SparseScalarValue) -> f64 {
    match value {
        SparseScalarValue::Logical(value) => f64::from(value),
        SparseScalarValue::F64(value) => value,
        SparseScalarValue::ComplexF64(value) => value.re,
    }
}

fn complex_value(value: SparseScalarValue) -> Complex64 {
    match value {
        SparseScalarValue::Logical(value) => Complex64::new(f64::from(value), 0.0),
        SparseScalarValue::F64(value) => Complex64::new(value, 0.0),
        SparseScalarValue::ComplexF64(value) => value,
    }
}

fn checked_u64(value: usize) -> Result<u64, SparseError> {
    u64::try_from(value).map_err(|_| SparseError::HostLengthOverflow { elements: u64::MAX })
}

fn normalized_offsets(
    offsets: &[usize],
    extent: u64,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u64>, SparseError> {
    let mut normalized = try_reserved("deletion offsets", offsets.len())?;
    for (position, &offset) in offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let offset = checked_u64(offset)?;
        if offset >= extent {
            return Err(SparseError::LinearIndexOutOfBounds {
                index: offset.saturating_add(1),
                numel: extent,
            });
        }
        normalized.push(offset);
    }
    normalized.sort_unstable();
    normalized.dedup();
    Ok(normalized)
}

fn normalized_indices(
    indices: &[u64],
    extent: u64,
    dimension: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u64>, SparseError> {
    let mut normalized = try_reserved("deletion subscripts", indices.len())?;
    for (position, &index) in indices.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        if index >= extent {
            return Err(SparseError::SubscriptOutOfBounds {
                dimension,
                index: index.saturating_add(1),
                extent,
            });
        }
        normalized.push(index);
    }
    normalized.sort_unstable();
    normalized.dedup();
    Ok(normalized)
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

fn check_cancelled(cancellation: Option<&AtomicBool>, progress: usize) -> Result<(), SparseError> {
    use std::sync::atomic::Ordering;
    if progress.is_multiple_of(1_024)
        && cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
    {
        Err(SparseError::Cancelled)
    } else {
        Ok(())
    }
}
