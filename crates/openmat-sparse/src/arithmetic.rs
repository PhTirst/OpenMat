use std::sync::atomic::AtomicBool;

use openmat_array::{ArrayData, Complex64, DenseArray, Shape};

use crate::{CooEntry, SparseArrayData, SparseError};

/// Arithmetic operations whose sparse storage behavior is defined by R2022b.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseBinaryOperator {
    Add,
    Subtract,
    ElementMultiply,
    ElementDivide,
}

impl SparseBinaryOperator {
    const fn name(self) -> &'static str {
        match self {
            Self::Add => "addition",
            Self::Subtract => "subtraction",
            Self::ElementMultiply => "element-wise multiplication",
            Self::ElementDivide => "element-wise division",
        }
    }
}

/// A sparse-aware arithmetic result, including operations R2022b materializes.
#[derive(Clone, Debug, PartialEq)]
pub enum SparseBinaryResult {
    Sparse(SparseArrayData),
    Dense(ArrayData),
}

impl SparseArrayData {
    /// Adds two sparse operands and preserves CSC storage.
    ///
    /// # Errors
    /// Returns a checked shape, allocation, overflow, or cancellation failure.
    pub fn try_add_sparse(
        &self,
        rhs: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        self.try_binary_sparse(rhs, SparseBinaryOperator::Add, cancellation)
    }

    /// Subtracts two sparse operands and preserves CSC storage.
    ///
    /// # Errors
    /// Returns a checked shape, allocation, overflow, or cancellation failure.
    pub fn try_subtract_sparse(
        &self,
        rhs: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        self.try_binary_sparse(rhs, SparseBinaryOperator::Subtract, cancellation)
    }

    /// Multiplies two sparse operands element by element.
    ///
    /// # Errors
    /// Returns a checked shape, allocation, overflow, or cancellation failure.
    pub fn try_element_multiply_sparse(
        &self,
        rhs: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        self.try_binary_sparse(rhs, SparseBinaryOperator::ElementMultiply, cancellation)
    }

    /// Divides two sparse operands element by element.
    ///
    /// # Errors
    /// Returns a checked shape, allocation, overflow, or cancellation failure.
    pub fn try_element_divide_sparse(
        &self,
        rhs: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        self.try_binary_sparse(rhs, SparseBinaryOperator::ElementDivide, cancellation)
    }

    /// Adds a sparse and dense operand, returning the measured dense result.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_add_dense(
        &self,
        dense: &ArrayData,
        sparse_is_left: bool,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ArrayData, SparseError> {
        match self.try_binary_dense(
            dense,
            SparseBinaryOperator::Add,
            sparse_is_left,
            cancellation,
        )? {
            SparseBinaryResult::Dense(value) => Ok(value),
            SparseBinaryResult::Sparse(_) => unreachable!("sparse-dense addition is dense"),
        }
    }

    /// Subtracts sparse and dense operands, returning the measured dense result.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_subtract_dense(
        &self,
        dense: &ArrayData,
        sparse_is_left: bool,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ArrayData, SparseError> {
        match self.try_binary_dense(
            dense,
            SparseBinaryOperator::Subtract,
            sparse_is_left,
            cancellation,
        )? {
            SparseBinaryResult::Dense(value) => Ok(value),
            SparseBinaryResult::Sparse(_) => unreachable!("sparse-dense subtraction is dense"),
        }
    }

    /// Multiplies sparse and dense operands element by element into CSC.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_element_multiply_dense(
        &self,
        dense: &ArrayData,
        sparse_is_left: bool,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match self.try_binary_dense(
            dense,
            SparseBinaryOperator::ElementMultiply,
            sparse_is_left,
            cancellation,
        )? {
            SparseBinaryResult::Sparse(value) => Ok(value),
            SparseBinaryResult::Dense(_) => {
                unreachable!("sparse-dense element multiplication is sparse")
            }
        }
    }

    /// Divides a sparse numerator by a dense denominator into CSC.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_element_divide_by_dense(
        &self,
        dense: &ArrayData,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        match self.try_binary_dense(
            dense,
            SparseBinaryOperator::ElementDivide,
            true,
            cancellation,
        )? {
            SparseBinaryResult::Sparse(value) => Ok(value),
            SparseBinaryResult::Dense(_) => unreachable!("sparse divided by dense is sparse"),
        }
    }

    /// Divides a dense numerator by a sparse denominator into full storage.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_dense_element_divide_by(
        &self,
        dense: &ArrayData,
        cancellation: Option<&AtomicBool>,
    ) -> Result<ArrayData, SparseError> {
        match self.try_binary_dense(
            dense,
            SparseBinaryOperator::ElementDivide,
            false,
            cancellation,
        )? {
            SparseBinaryResult::Dense(value) => Ok(value),
            SparseBinaryResult::Sparse(_) => unreachable!("dense divided by sparse is dense"),
        }
    }

    /// Applies element-wise arithmetic to two sparse values.
    ///
    /// The result remains CSC even when division must explicitly store `NaN`
    /// at every implicit-zero/implicit-zero coordinate.
    ///
    /// # Errors
    /// Returns a checked shape, allocation, overflow, or cancellation failure.
    pub fn try_binary_sparse(
        &self,
        rhs: &Self,
        operator: SparseBinaryOperator,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let shape = broadcast_shape(self.shape(), rhs.shape(), operator)?;
        let complex = self.is_complex() || rhs.is_complex();
        if complex {
            sparse_complex_result(self, rhs, operator, &shape, cancellation)
        } else {
            sparse_real_result(self, rhs, operator, &shape, cancellation)
        }
    }

    /// Applies element-wise arithmetic between sparse and full storage.
    ///
    /// `sparse_is_left` preserves the observable asymmetry of division.
    ///
    /// # Errors
    /// Returns a checked type, shape, allocation, overflow, or cancellation failure.
    pub fn try_binary_dense(
        &self,
        dense: &ArrayData,
        operator: SparseBinaryOperator,
        sparse_is_left: bool,
        cancellation: Option<&AtomicBool>,
    ) -> Result<SparseBinaryResult, SparseError> {
        let dense_shape = supported_dense_shape(dense)?;
        let (lhs, rhs) = if sparse_is_left {
            (self.shape(), dense_shape)
        } else {
            (dense_shape, self.shape())
        };
        let shape = broadcast_shape(lhs, rhs, operator)?;
        let stays_sparse = matches!(operator, SparseBinaryOperator::ElementMultiply)
            || (operator == SparseBinaryOperator::ElementDivide && sparse_is_left);
        let complex = self.is_complex() || dense_is_complex(dense);
        if stays_sparse {
            let result = if complex {
                sparse_dense_complex(self, dense, operator, sparse_is_left, &shape, cancellation)?
            } else {
                sparse_dense_real(self, dense, operator, sparse_is_left, &shape, cancellation)?
            };
            Ok(SparseBinaryResult::Sparse(result))
        } else {
            let result = if complex {
                dense_complex_result(self, dense, operator, sparse_is_left, &shape, cancellation)?
            } else {
                dense_real_result(self, dense, operator, sparse_is_left, &shape, cancellation)?
            };
            Ok(SparseBinaryResult::Dense(result))
        }
    }

    /// Multiplies two two-dimensional sparse matrices and returns canonical CSC.
    ///
    /// # Errors
    /// Returns a shape mismatch, allocation, overflow, or cancellation failure.
    pub fn try_matrix_multiply(
        &self,
        rhs: &Self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Self, SparseError> {
        let rows = self.shape().extent(0);
        let inner = self.shape().extent(1);
        let rhs_inner = rhs.shape().extent(0);
        let columns = rhs.shape().extent(1);
        if inner != rhs_inner {
            return Err(SparseError::ShapeMismatch {
                operation: "matrix multiplication",
                lhs: self.shape().dimensions().to_vec(),
                rhs: rhs.shape().dimensions().to_vec(),
            });
        }
        let mut products = 0_usize;
        for (position, (k, _, _)) in rhs.numeric_entries().enumerate() {
            check_cancelled(cancellation, position)?;
            products = products
                .checked_add(self.column_nnz(k)?)
                .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
        }
        if self.is_complex() || rhs.is_complex() {
            let mut entries = try_reserved("matrix product COO", products)?;
            for (position, (k, column, rhs_value)) in rhs.numeric_entries().enumerate() {
                check_cancelled(cancellation, position)?;
                for (row, lhs_value) in self.numeric_column(k)? {
                    entries.push(CooEntry::new(
                        row,
                        column,
                        complex_mul(lhs_value, rhs_value),
                    ));
                }
            }
            Self::try_from_complex_f64_coo(rows, columns, entries, 0, cancellation)
        } else {
            let mut entries = try_reserved("matrix product COO", products)?;
            for (position, (k, column, rhs_value)) in rhs.real_entries().enumerate() {
                check_cancelled(cancellation, position)?;
                for (row, lhs_value) in self.real_column(k)? {
                    entries.push(CooEntry::new(row, column, lhs_value * rhs_value));
                }
            }
            Self::try_from_f64_coo(rows, columns, entries, 0, cancellation)
        }
    }

    fn column_nnz(&self, column: u64) -> Result<usize, SparseError> {
        macro_rules! count {
            ($matrix:expr) => {{
                let column = checked_usize(column)?;
                let start = checked_usize($matrix.col_offsets()[column])?;
                let end = checked_usize($matrix.col_offsets()[column + 1])?;
                Ok(end - start)
            }};
        }
        match self {
            Self::Logical(matrix) => count!(matrix),
            Self::F64(matrix) => count!(matrix),
            Self::ComplexF64(matrix) => count!(matrix),
        }
    }

    fn real_entries(&self) -> Box<dyn Iterator<Item = (u64, u64, f64)> + '_> {
        match self {
            Self::Logical(matrix) => Box::new(
                matrix
                    .entries()
                    .map(|(row, column, value)| (row, column, f64::from(value.get()))),
            ),
            Self::F64(matrix) => Box::new(
                matrix
                    .entries()
                    .map(|(row, column, value)| (row, column, *value)),
            ),
            Self::ComplexF64(_) => Box::new(std::iter::empty()),
        }
    }

    fn numeric_entries(&self) -> Box<dyn Iterator<Item = (u64, u64, Complex64)> + '_> {
        match self {
            Self::Logical(matrix) => Box::new(matrix.entries().map(|(row, column, value)| {
                (row, column, Complex64::new(f64::from(value.get()), 0.0))
            })),
            Self::F64(matrix) => Box::new(
                matrix
                    .entries()
                    .map(|(row, column, value)| (row, column, Complex64::new(*value, 0.0))),
            ),
            Self::ComplexF64(matrix) => Box::new(
                matrix
                    .entries()
                    .map(|(row, column, value)| (row, column, *value)),
            ),
        }
    }

    fn real_column(&self, column: u64) -> Result<Vec<(u64, f64)>, SparseError> {
        let capacity = self.column_nnz(column)?;
        let mut values = try_reserved("matrix product column", capacity)?;
        for (row, entry_column, value) in self.real_entries() {
            if entry_column == column {
                values.push((row, value));
            } else if entry_column > column {
                break;
            }
        }
        Ok(values)
    }

    fn numeric_column(&self, column: u64) -> Result<Vec<(u64, Complex64)>, SparseError> {
        let capacity = self.column_nnz(column)?;
        let mut values = try_reserved("matrix product column", capacity)?;
        for (row, entry_column, value) in self.numeric_entries() {
            if entry_column == column {
                values.push((row, value));
            } else if entry_column > column {
                break;
            }
        }
        Ok(values)
    }
}

fn sparse_real_result(
    lhs: &SparseArrayData,
    rhs: &SparseArrayData,
    operator: SparseBinaryOperator,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    if lhs.shape() == rhs.shape() && operator != SparseBinaryOperator::ElementDivide {
        return merge_sparse_real(lhs, rhs, operator, cancellation);
    }
    let capacity = checked_usize(shape.numel())?;
    let mut entries = try_reserved("arithmetic COO", capacity)?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let left = sparse_real_broadcast(lhs, shape, offset)?;
        let right = sparse_real_broadcast(rhs, shape, offset)?;
        let value = real_operation(operator, left, right);
        if value != 0.0 {
            let (row, column) = coordinates(shape, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_f64_coo(shape.extent(0), shape.extent(1), entries, 0, cancellation)
}

fn sparse_complex_result(
    lhs: &SparseArrayData,
    rhs: &SparseArrayData,
    operator: SparseBinaryOperator,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    if lhs.shape() == rhs.shape() && operator != SparseBinaryOperator::ElementDivide {
        return merge_sparse_complex(lhs, rhs, operator, cancellation);
    }
    let capacity = checked_usize(shape.numel())?;
    let mut entries = try_reserved("complex arithmetic COO", capacity)?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let left = sparse_complex_broadcast(lhs, shape, offset)?;
        let right = sparse_complex_broadcast(rhs, shape, offset)?;
        let value = complex_operation(operator, left, right);
        if value.re != 0.0 || value.im != 0.0 {
            let (row, column) = coordinates(shape, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_complex_f64_coo(
        shape.extent(0),
        shape.extent(1),
        entries,
        0,
        cancellation,
    )
}

fn merge_sparse_real(
    lhs: &SparseArrayData,
    rhs: &SparseArrayData,
    operator: SparseBinaryOperator,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let capacity = lhs
        .nnz()
        .checked_add(rhs.nnz())
        .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let mut entries = try_reserved("merged arithmetic COO", capacity)?;
    let mut left = lhs.real_entries().peekable();
    let mut right = rhs.real_entries().peekable();
    while left.peek().is_some() || right.peek().is_some() {
        check_cancelled(cancellation, entries.len())?;
        let left_key = left.peek().map(|(row, column, _)| (*column, *row));
        let right_key = right.peek().map(|(row, column, _)| (*column, *row));
        let (row, column, lhs_value, rhs_value) = match (left_key, right_key) {
            (Some(left_key), Some(right_key)) if left_key == right_key => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                let (_, _, rhs_value) = right.next().expect("peeked rhs entry");
                (row, column, lhs_value, rhs_value)
            }
            (Some(left_key), Some(right_key)) if left_key < right_key => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                (row, column, lhs_value, 0.0)
            }
            (Some(_), None) => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                (row, column, lhs_value, 0.0)
            }
            (Some(_) | None, Some(_)) => {
                let (row, column, rhs_value) = right.next().expect("peeked rhs entry");
                (row, column, 0.0, rhs_value)
            }
            (None, None) => break,
        };
        let value = real_operation(operator, lhs_value, rhs_value);
        if value != 0.0 {
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_f64_coo(
        lhs.shape().extent(0),
        lhs.shape().extent(1),
        entries,
        0,
        cancellation,
    )
}

fn merge_sparse_complex(
    lhs: &SparseArrayData,
    rhs: &SparseArrayData,
    operator: SparseBinaryOperator,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let capacity = lhs
        .nnz()
        .checked_add(rhs.nnz())
        .ok_or(SparseError::HostLengthOverflow { elements: u64::MAX })?;
    let mut entries = try_reserved("complex merged arithmetic COO", capacity)?;
    let mut left = lhs.numeric_entries().peekable();
    let mut right = rhs.numeric_entries().peekable();
    while left.peek().is_some() || right.peek().is_some() {
        check_cancelled(cancellation, entries.len())?;
        let left_key = left.peek().map(|(row, column, _)| (*column, *row));
        let right_key = right.peek().map(|(row, column, _)| (*column, *row));
        let (row, column, lhs_value, rhs_value) = match (left_key, right_key) {
            (Some(left_key), Some(right_key)) if left_key == right_key => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                let (_, _, rhs_value) = right.next().expect("peeked rhs entry");
                (row, column, lhs_value, rhs_value)
            }
            (Some(left_key), Some(right_key)) if left_key < right_key => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                (row, column, lhs_value, Complex64::ZERO)
            }
            (Some(_), None) => {
                let (row, column, lhs_value) = left.next().expect("peeked lhs entry");
                (row, column, lhs_value, Complex64::ZERO)
            }
            (Some(_) | None, Some(_)) => {
                let (row, column, rhs_value) = right.next().expect("peeked rhs entry");
                (row, column, Complex64::ZERO, rhs_value)
            }
            (None, None) => break,
        };
        let value = complex_operation(operator, lhs_value, rhs_value);
        if value.re != 0.0 || value.im != 0.0 {
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_complex_f64_coo(
        lhs.shape().extent(0),
        lhs.shape().extent(1),
        entries,
        0,
        cancellation,
    )
}

fn sparse_dense_real(
    sparse: &SparseArrayData,
    dense: &ArrayData,
    operator: SparseBinaryOperator,
    sparse_is_left: bool,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let mut entries = try_reserved("sparse-dense arithmetic COO", checked_usize(shape.numel())?)?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let sparse_value = sparse_real_broadcast(sparse, shape, offset)?;
        let dense_value = dense_real_broadcast(dense, shape, offset)?;
        let value = if sparse_is_left {
            real_operation(operator, sparse_value, dense_value)
        } else {
            real_operation(operator, dense_value, sparse_value)
        };
        if value != 0.0 {
            let (row, column) = coordinates(shape, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_f64_coo(shape.extent(0), shape.extent(1), entries, 0, cancellation)
}

fn sparse_dense_complex(
    sparse: &SparseArrayData,
    dense: &ArrayData,
    operator: SparseBinaryOperator,
    sparse_is_left: bool,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseArrayData, SparseError> {
    let mut entries = try_reserved(
        "complex sparse-dense arithmetic COO",
        checked_usize(shape.numel())?,
    )?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let sparse_value = sparse_complex_broadcast(sparse, shape, offset)?;
        let dense_value = dense_complex_broadcast(dense, shape, offset)?;
        let value = if sparse_is_left {
            complex_operation(operator, sparse_value, dense_value)
        } else {
            complex_operation(operator, dense_value, sparse_value)
        };
        if value.re != 0.0 || value.im != 0.0 {
            let (row, column) = coordinates(shape, offset);
            entries.push(CooEntry::new(row, column, value));
        }
    }
    SparseArrayData::try_from_complex_f64_coo(
        shape.extent(0),
        shape.extent(1),
        entries,
        0,
        cancellation,
    )
}

fn dense_real_result(
    sparse: &SparseArrayData,
    dense: &ArrayData,
    operator: SparseBinaryOperator,
    sparse_is_left: bool,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<ArrayData, SparseError> {
    let capacity = checked_usize(shape.numel())?;
    let mut values = try_reserved("dense arithmetic values", capacity)?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let sparse_value = sparse_real_broadcast(sparse, shape, offset)?;
        let dense_value = dense_real_broadcast(dense, shape, offset)?;
        values.push(if sparse_is_left {
            real_operation(operator, sparse_value, dense_value)
        } else {
            real_operation(operator, dense_value, sparse_value)
        });
    }
    DenseArray::from_vec(shape.clone(), values)
        .map(ArrayData::F64)
        .map_err(|_| SparseError::ShapeOverflow {
            rows: shape.extent(0),
            columns: shape.extent(1),
        })
}

fn dense_complex_result(
    sparse: &SparseArrayData,
    dense: &ArrayData,
    operator: SparseBinaryOperator,
    sparse_is_left: bool,
    shape: &Shape,
    cancellation: Option<&AtomicBool>,
) -> Result<ArrayData, SparseError> {
    let capacity = checked_usize(shape.numel())?;
    let mut values = try_reserved("complex dense arithmetic values", capacity)?;
    for offset in 0..shape.numel() {
        check_cancelled(cancellation, checked_usize(offset)?)?;
        let sparse_value = sparse_complex_broadcast(sparse, shape, offset)?;
        let dense_value = dense_complex_broadcast(dense, shape, offset)?;
        values.push(if sparse_is_left {
            complex_operation(operator, sparse_value, dense_value)
        } else {
            complex_operation(operator, dense_value, sparse_value)
        });
    }
    DenseArray::from_vec(shape.clone(), values)
        .map(ArrayData::ComplexF64)
        .map_err(|_| SparseError::ShapeOverflow {
            rows: shape.extent(0),
            columns: shape.extent(1),
        })
}

fn supported_dense_shape(dense: &ArrayData) -> Result<&Shape, SparseError> {
    let shape = match dense {
        ArrayData::Logical(value) => value.shape(),
        ArrayData::F64(value) => value.shape(),
        ArrayData::ComplexF64(value) => value.shape(),
        ArrayData::F32(_)
        | ArrayData::ComplexF32(_)
        | ArrayData::Char(_)
        | ArrayData::Integer(_) => {
            return Err(SparseError::UnsupportedDenseType);
        }
    };
    if shape.ndims() != 2 {
        return Err(SparseError::UnsupportedDenseType);
    }
    Ok(shape)
}

fn dense_is_complex(dense: &ArrayData) -> bool {
    matches!(dense, ArrayData::ComplexF64(_))
}

fn broadcast_shape(
    lhs: &Shape,
    rhs: &Shape,
    operator: SparseBinaryOperator,
) -> Result<Shape, SparseError> {
    let mut dimensions = [0_u64; 2];
    for (dimension, output) in dimensions.iter_mut().enumerate() {
        let left = lhs.extent(dimension);
        let right = rhs.extent(dimension);
        *output = if left == right {
            left
        } else if left == 1 {
            right
        } else if right == 1 {
            left
        } else {
            return Err(SparseError::ShapeMismatch {
                operation: operator.name(),
                lhs: lhs.dimensions().to_vec(),
                rhs: rhs.dimensions().to_vec(),
            });
        };
    }
    Shape::new(dimensions).map_err(|_| SparseError::ShapeOverflow {
        rows: dimensions[0],
        columns: dimensions[1],
    })
}

fn coordinates(shape: &Shape, offset: u64) -> (u64, u64) {
    let rows = shape.extent(0);
    debug_assert!(rows > 0);
    (offset % rows, offset / rows)
}

fn broadcast_offset(source: &Shape, output: &Shape, offset: u64) -> u64 {
    let (row, column) = coordinates(output, offset);
    let source_row = if source.extent(0) == 1 { 0 } else { row };
    let source_column = if source.extent(1) == 1 { 0 } else { column };
    source_row + source_column * source.extent(0)
}

fn sparse_real_broadcast(
    value: &SparseArrayData,
    output: &Shape,
    offset: u64,
) -> Result<f64, SparseError> {
    let offset = broadcast_offset(value.shape(), output, offset);
    Ok(match value.element_at_offset(offset)? {
        crate::SparseScalarValue::Logical(value) => f64::from(value),
        crate::SparseScalarValue::F64(value) => value,
        crate::SparseScalarValue::ComplexF64(_) => unreachable!("complex sparse used as real"),
    })
}

fn sparse_complex_broadcast(
    value: &SparseArrayData,
    output: &Shape,
    offset: u64,
) -> Result<Complex64, SparseError> {
    let offset = broadcast_offset(value.shape(), output, offset);
    Ok(match value.element_at_offset(offset)? {
        crate::SparseScalarValue::Logical(value) => Complex64::new(f64::from(value), 0.0),
        crate::SparseScalarValue::F64(value) => Complex64::new(value, 0.0),
        crate::SparseScalarValue::ComplexF64(value) => value,
    })
}

fn dense_real_broadcast(
    value: &ArrayData,
    output: &Shape,
    offset: u64,
) -> Result<f64, SparseError> {
    let shape = supported_dense_shape(value)?;
    let offset = checked_usize(broadcast_offset(shape, output, offset))?;
    Ok(match value {
        ArrayData::Logical(array) => f64::from(array.as_slice()[offset].get()),
        ArrayData::F64(array) => array.as_slice()[offset],
        ArrayData::ComplexF64(_) => unreachable!("complex dense used as real"),
        ArrayData::F32(_)
        | ArrayData::ComplexF32(_)
        | ArrayData::Char(_)
        | ArrayData::Integer(_) => unreachable!("validated dense type"),
    })
}

fn dense_complex_broadcast(
    value: &ArrayData,
    output: &Shape,
    offset: u64,
) -> Result<Complex64, SparseError> {
    let shape = supported_dense_shape(value)?;
    let offset = checked_usize(broadcast_offset(shape, output, offset))?;
    Ok(match value {
        ArrayData::Logical(array) => Complex64::new(f64::from(array.as_slice()[offset].get()), 0.0),
        ArrayData::F64(array) => Complex64::new(array.as_slice()[offset], 0.0),
        ArrayData::ComplexF64(array) => array.as_slice()[offset],
        ArrayData::F32(_)
        | ArrayData::ComplexF32(_)
        | ArrayData::Char(_)
        | ArrayData::Integer(_) => unreachable!("validated dense type"),
    })
}

fn real_operation(operator: SparseBinaryOperator, lhs: f64, rhs: f64) -> f64 {
    match operator {
        SparseBinaryOperator::Add => lhs + rhs,
        SparseBinaryOperator::Subtract => lhs - rhs,
        SparseBinaryOperator::ElementMultiply => lhs * rhs,
        SparseBinaryOperator::ElementDivide => lhs / rhs,
    }
}

fn complex_operation(operator: SparseBinaryOperator, lhs: Complex64, rhs: Complex64) -> Complex64 {
    match operator {
        SparseBinaryOperator::Add => Complex64::new(lhs.re + rhs.re, lhs.im + rhs.im),
        SparseBinaryOperator::Subtract => Complex64::new(lhs.re - rhs.re, lhs.im - rhs.im),
        SparseBinaryOperator::ElementMultiply => complex_mul(lhs, rhs),
        SparseBinaryOperator::ElementDivide => complex_div(lhs, rhs),
    }
}

fn complex_mul(lhs: Complex64, rhs: Complex64) -> Complex64 {
    Complex64::new(
        lhs.re.mul_add(rhs.re, -(lhs.im * rhs.im)),
        lhs.re.mul_add(rhs.im, lhs.im * rhs.re),
    )
}

fn complex_div(lhs: Complex64, rhs: Complex64) -> Complex64 {
    if rhs.re == 0.0 && rhs.im == 0.0 {
        if lhs.re == 0.0 && lhs.im == 0.0 {
            return Complex64::new(f64::NAN, 0.0);
        }
        return Complex64::new(
            if lhs.re == 0.0 { 0.0 } else { lhs.re / 0.0 },
            if lhs.im == 0.0 { 0.0 } else { lhs.im / 0.0 },
        );
    }
    if rhs.re.abs() >= rhs.im.abs() {
        let ratio = rhs.im / rhs.re;
        let denominator = rhs.re + rhs.im * ratio;
        Complex64::new(
            (lhs.re + lhs.im * ratio) / denominator,
            (lhs.im - lhs.re * ratio) / denominator,
        )
    } else {
        let ratio = rhs.re / rhs.im;
        let denominator = rhs.im + rhs.re * ratio;
        Complex64::new(
            (lhs.re * ratio + lhs.im) / denominator,
            (lhs.im * ratio - lhs.re) / denominator,
        )
    }
}

fn checked_usize(value: u64) -> Result<usize, SparseError> {
    usize::try_from(value).map_err(|_| SparseError::HostLengthOverflow { elements: value })
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
