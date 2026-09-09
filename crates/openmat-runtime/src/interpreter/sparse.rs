use openmat_array::{ArrayData, Complex64 as ArrayComplex64, DenseArray, Logical, Shape};
use openmat_bytecode::BinaryOperator;
use openmat_sparse_provider::FaerSparseProvider;
use openmat_value::{SparseArrayData, SparseError, Value};

use crate::{
    ArrayRuntimeError, CancellationToken, RuntimeErrorKind,
    array_ops::{self, IndexInput},
    error::array_error,
    linalg_ops::{
        SparseSolveOutcome, full_sparse_matrix_multiply, sparse_full_matrix_multiply,
        sparse_matrix_left_divide,
    },
};

pub(super) fn try_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
) -> Option<Result<SparseSolveOutcome, RuntimeErrorKind>> {
    if !matches!(lhs, Value::Sparse(_)) && !matches!(rhs, Value::Sparse(_)) {
        return None;
    }
    if operator == BinaryOperator::LeftDivide && !lhs.is_scalar() {
        let Value::Sparse(coefficients) = lhs else {
            return None;
        };
        return Some(
            sparse_matrix_left_divide(
                &FaerSparseProvider,
                coefficients,
                rhs,
                Some(cancellation.atomic_flag()),
            )
            .map_err(Into::into),
        );
    }
    let sparse_operator = match operator {
        BinaryOperator::Add => SparseOperation::Add,
        BinaryOperator::Subtract => SparseOperation::Subtract,
        BinaryOperator::ElementMultiply => SparseOperation::ElementMultiply,
        BinaryOperator::ElementDivide | BinaryOperator::ElementLeftDivide => {
            SparseOperation::ElementDivide
        }
        BinaryOperator::Multiply if lhs.is_scalar() || rhs.is_scalar() => {
            SparseOperation::ElementMultiply
        }
        BinaryOperator::Multiply => {
            return Some(without_warning(match (lhs, rhs) {
                (Value::Sparse(lhs), Value::Sparse(rhs)) => lhs
                    .try_matrix_multiply(rhs, Some(cancellation.atomic_flag()))
                    .map(Value::Sparse)
                    .map_err(|error| sparse_error(&error)),
                (Value::Sparse(lhs), rhs) => sparse_full_matrix_multiply(
                    &FaerSparseProvider,
                    lhs,
                    rhs,
                    Some(cancellation.atomic_flag()),
                )
                .map_err(Into::into),
                (lhs, Value::Sparse(rhs)) => full_sparse_matrix_multiply(
                    &FaerSparseProvider,
                    lhs,
                    rhs,
                    Some(cancellation.atomic_flag()),
                )
                .map_err(Into::into),
                _ => unreachable!("sparse dispatch requires one sparse operand"),
            }));
        }
        BinaryOperator::Divide if rhs.is_scalar() => SparseOperation::ElementDivide,
        BinaryOperator::LeftDivide if lhs.is_scalar() => SparseOperation::ElementDivide,
        _ => return None,
    };

    let swap = matches!(
        operator,
        BinaryOperator::ElementLeftDivide | BinaryOperator::LeftDivide
    );
    let (lhs, rhs) = if swap { (rhs, lhs) } else { (lhs, rhs) };
    Some(without_warning(binary_values(
        lhs,
        rhs,
        sparse_operator,
        cancellation,
    )))
}

fn without_warning(
    result: Result<Value, RuntimeErrorKind>,
) -> Result<SparseSolveOutcome, RuntimeErrorKind> {
    result.map(|value| SparseSolveOutcome {
        value,
        warning: None,
    })
}

fn binary_values(
    lhs: &Value,
    rhs: &Value,
    operator: SparseOperation,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    match (lhs, rhs) {
        (Value::Sparse(lhs), Value::Sparse(rhs)) => {
            let result = match operator {
                SparseOperation::Add => lhs.try_add_sparse(rhs, Some(cancellation.atomic_flag())),
                SparseOperation::Subtract => {
                    lhs.try_subtract_sparse(rhs, Some(cancellation.atomic_flag()))
                }
                SparseOperation::ElementMultiply => {
                    lhs.try_element_multiply_sparse(rhs, Some(cancellation.atomic_flag()))
                }
                SparseOperation::ElementDivide => {
                    lhs.try_element_divide_sparse(rhs, Some(cancellation.atomic_flag()))
                }
            };
            result
                .map(Value::Sparse)
                .map_err(|error| sparse_error(&error))
        }
        (Value::Sparse(sparse), dense) => {
            let dense = numeric_dense(dense).ok_or_else(|| invalid_operand(operator, dense))?;
            let result = match operator {
                SparseOperation::Add => sparse
                    .try_add_dense(&dense, true, Some(cancellation.atomic_flag()))
                    .map(Value::Array),
                SparseOperation::Subtract => sparse
                    .try_subtract_dense(&dense, true, Some(cancellation.atomic_flag()))
                    .map(Value::Array),
                SparseOperation::ElementMultiply => sparse
                    .try_element_multiply_dense(&dense, true, Some(cancellation.atomic_flag()))
                    .map(Value::Sparse),
                SparseOperation::ElementDivide => sparse
                    .try_element_divide_by_dense(&dense, Some(cancellation.atomic_flag()))
                    .map(Value::Sparse),
            };
            result.map_err(|error| sparse_error(&error))
        }
        (dense, Value::Sparse(sparse)) => {
            let dense = numeric_dense(dense).ok_or_else(|| invalid_operand(operator, dense))?;
            let result = match operator {
                SparseOperation::Add => sparse
                    .try_add_dense(&dense, false, Some(cancellation.atomic_flag()))
                    .map(Value::Array),
                SparseOperation::Subtract => sparse
                    .try_subtract_dense(&dense, false, Some(cancellation.atomic_flag()))
                    .map(Value::Array),
                SparseOperation::ElementMultiply => sparse
                    .try_element_multiply_dense(&dense, false, Some(cancellation.atomic_flag()))
                    .map(Value::Sparse),
                SparseOperation::ElementDivide => sparse
                    .try_dense_element_divide_by(&dense, Some(cancellation.atomic_flag()))
                    .map(Value::Array),
            };
            result.map_err(|error| sparse_error(&error))
        }
        _ => unreachable!("sparse dispatch requires one sparse operand"),
    }
}

pub(super) fn try_index(
    target: &Value,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Option<Result<Value, RuntimeErrorKind>> {
    let Value::Sparse(target) = target else {
        return None;
    };
    Some(
        array_ops::resolve_index_selection(target.shape().dimensions(), arguments, cancellation)
            .and_then(|selection| {
                target
                    .try_gather_offsets(
                        &selection.offsets,
                        &selection.shape,
                        Some(cancellation.atomic_flag()),
                    )
                    .map(Value::Sparse)
                    .map_err(|error| sparse_error(&error))
            }),
    )
}

pub(super) fn try_index_assign(
    target: &Value,
    arguments: &[IndexInput],
    supplied: &Value,
    cancellation: &CancellationToken,
) -> Option<Result<Value, RuntimeErrorKind>> {
    let Value::Sparse(target) = target else {
        return None;
    };
    if is_empty_double(supplied) {
        return Some(delete_selection(target, arguments, cancellation).map(Value::Sparse));
    }
    let supplied = match numeric_sparse(supplied, cancellation) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    Some(
        array_ops::resolve_index_selection(target.shape().dimensions(), arguments, cancellation)
            .and_then(|selection| {
                target
                    .try_assign_offsets(
                        &selection.offsets,
                        &supplied,
                        Some(cancellation.atomic_flag()),
                    )
                    .map(Value::Sparse)
                    .map_err(|error| sparse_error(&error))
            }),
    )
}

fn delete_selection(
    target: &SparseArrayData,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<SparseArrayData, RuntimeErrorKind> {
    let selection =
        array_ops::resolve_index_selection(target.shape().dimensions(), arguments, cancellation)?;
    let flag = Some(cancellation.atomic_flag());
    match arguments {
        [_] => {
            let mut removed = Vec::new();
            removed
                .try_reserve_exact(selection.offsets.len())
                .map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?;
            removed.extend_from_slice(&selection.offsets);
            removed.sort_unstable();
            removed.dedup();
            let remaining = target
                .numel()
                .checked_sub(
                    u64::try_from(removed.len())
                        .map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?,
                )
                .ok_or_else(|| array_error(ArrayRuntimeError::SizeLimit))?;
            let shape = if target.shape().extent(1) == 1 {
                Shape::new([remaining, 1])
            } else {
                Shape::new([1, remaining])
            }
            .map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?;
            target
                .try_delete_linear_offsets(&selection.offsets, &shape, flag)
                .map_err(|error| sparse_error(&error))
        }
        [IndexInput::Value(_), IndexInput::Colon] => {
            let rows = target.shape().extent(0);
            let mut selected = Vec::new();
            selected
                .try_reserve_exact(selection.offsets.len())
                .map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?;
            for offset in selection.offsets {
                selected.push(
                    u64::try_from(offset).map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?
                        % rows,
                );
            }
            target
                .try_delete_rows(&selected, flag)
                .map_err(|error| sparse_error(&error))
        }
        [IndexInput::Colon, IndexInput::Value(_)] => {
            let rows = target.shape().extent(0);
            let mut selected = Vec::new();
            selected
                .try_reserve_exact(selection.offsets.len())
                .map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?;
            for offset in selection.offsets {
                selected.push(
                    u64::try_from(offset).map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?
                        / rows,
                );
            }
            target
                .try_delete_columns(&selected, flag)
                .map_err(|error| sparse_error(&error))
        }
        _ => Err(array_error(ArrayRuntimeError::InvalidDeletionShape {
            dimensions: target.shape().dimensions().to_vec(),
            arguments: arguments.len(),
        })),
    }
}

pub(super) fn try_build_matrix(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Option<Result<Value, RuntimeErrorKind>> {
    if !rows
        .iter()
        .flatten()
        .any(|value| matches!(value, Value::Sparse(_)))
    {
        return None;
    }
    let mut horizontal = Vec::new();
    if horizontal.try_reserve_exact(rows.len()).is_err() {
        return Some(Err(array_error(ArrayRuntimeError::SizeLimit)));
    }
    for row in rows {
        let mut blocks = Vec::new();
        if blocks.try_reserve_exact(row.len()).is_err() {
            return Some(Err(array_error(ArrayRuntimeError::SizeLimit)));
        }
        for value in row {
            match numeric_sparse(value, cancellation) {
                Ok(value) => blocks.push(value),
                Err(error) => return Some(Err(error)),
            }
        }
        match SparseArrayData::try_concatenate_2d(&blocks, 1, Some(cancellation.atomic_flag())) {
            Ok(value) => horizontal.push(value),
            Err(error) => return Some(Err(sparse_error(&error))),
        }
    }
    Some(
        SparseArrayData::try_concatenate_2d(&horizontal, 0, Some(cancellation.atomic_flag()))
            .map(Value::Sparse)
            .map_err(|error| sparse_error(&error)),
    )
}

fn numeric_sparse(
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<SparseArrayData, RuntimeErrorKind> {
    match value {
        Value::Sparse(value) => Ok(value.clone()),
        other => {
            let dense = numeric_dense(other).ok_or_else(|| {
                array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "sparse numeric operation",
                    actual: other.kind(),
                })
            })?;
            SparseArrayData::try_from_dense(&dense, Some(cancellation.atomic_flag()))
                .map_err(|error| sparse_error(&error))
        }
    }
}

fn numeric_dense(value: &Value) -> Option<ArrayData> {
    let shape = Shape::new([1, 1]).ok()?;
    match value {
        Value::Logical(value) => DenseArray::from_vec(shape, vec![Logical::from(*value)])
            .ok()
            .map(ArrayData::Logical),
        Value::Double(value) => DenseArray::from_vec(shape, vec![*value])
            .ok()
            .map(ArrayData::F64),
        Value::Complex(value) => DenseArray::from_vec(
            shape,
            vec![ArrayComplex64::new(value.real, value.imaginary)],
        )
        .ok()
        .map(ArrayData::ComplexF64),
        Value::Array(
            value @ (ArrayData::Logical(_) | ArrayData::F64(_) | ArrayData::ComplexF64(_)),
        ) => Some(value.clone()),
        _ => None,
    }
}

fn is_empty_double(value: &Value) -> bool {
    matches!(value, Value::Array(ArrayData::F64(array)) if array.numel() == 0)
}

#[derive(Clone, Copy)]
enum SparseOperation {
    Add,
    Subtract,
    ElementMultiply,
    ElementDivide,
}

fn invalid_operand(operator: SparseOperation, value: &Value) -> RuntimeErrorKind {
    let operation = match operator {
        SparseOperation::Add => "sparse addition",
        SparseOperation::Subtract => "sparse subtraction",
        SparseOperation::ElementMultiply => "sparse element-wise multiplication",
        SparseOperation::ElementDivide => "sparse element-wise division",
    };
    array_error(ArrayRuntimeError::InvalidOperand {
        operation,
        actual: value.kind(),
    })
}

fn sparse_error(error: &SparseError) -> RuntimeErrorKind {
    match error {
        SparseError::Cancelled => RuntimeErrorKind::Cancelled,
        SparseError::ShapeMismatch {
            operation,
            lhs,
            rhs,
        } => array_error(ArrayRuntimeError::ShapeMismatch {
            operation,
            lhs: lhs.clone(),
            rhs: rhs.clone(),
        }),
        SparseError::AssignmentSizeMismatch { selected, supplied } => {
            array_error(ArrayRuntimeError::AssignmentSizeMismatch {
                selected: *selected,
                supplied: *supplied,
            })
        }
        SparseError::LinearIndexOutOfBounds { index, numel } => {
            array_error(ArrayRuntimeError::IndexOutOfBounds {
                argument: 0,
                index: *index,
                extent: *numel,
            })
        }
        SparseError::SubscriptOutOfBounds {
            dimension,
            index,
            extent,
        } => array_error(ArrayRuntimeError::IndexOutOfBounds {
            argument: *dimension,
            index: *index,
            extent: *extent,
        }),
        _ => array_error(ArrayRuntimeError::SizeLimit),
    }
}
