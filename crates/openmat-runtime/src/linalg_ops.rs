use std::{
    borrow::Cow,
    error::Error,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use openmat_array::{ArrayData, Complex32, Complex64, DType, DenseArray, Shape};
use openmat_linalg::{
    CscMatrixRef, LinalgError, LinalgProvider, RectangularSolveRequest, SolveRequest,
    SparseCancellation, SparseError, SparseMetrics, SparseNumericFactor, SparseProvider,
    SparseSolveResult, matrix_dimensions, matrix_multiply_complex32, matrix_multiply_complex64,
    matrix_multiply_f32, matrix_multiply_f64, solve_underdetermined_basic_complex32,
    solve_underdetermined_basic_complex64, solve_underdetermined_basic_f32,
    solve_underdetermined_basic_f64,
};
use openmat_value::{
    CscMatrix, SparseArrayData, SparseError as SparseValueError, Value, ValueKind,
};

/// A structured failure at the dynamic-value linear-algebra boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeLinalgError {
    /// Provider-independent validation or provider execution failed.
    Linalg(LinalgError),
    /// A dynamic operand is not a real or complex binary64 matrix.
    DoubleMatrixRequired {
        /// Runtime operation receiving the operand.
        operation: &'static str,
        /// Position or role of the rejected operand.
        operand: &'static str,
        /// Coarse dynamic value category.
        actual: ValueKind,
        /// Exact dense element type, when the value has one.
        dtype: Option<DType>,
    },
    /// Matrix division received a value outside the real/complex double or
    /// single dense numeric boundary.
    NumericMatrixRequired {
        /// Runtime operation receiving the operand.
        operation: &'static str,
        /// Position or role of the rejected operand.
        operand: &'static str,
        /// Coarse dynamic value category.
        actual: ValueKind,
        /// Exact dense element type, when the value has one.
        dtype: Option<DType>,
    },
    /// Matrix division does not define a mixed single/double result class yet.
    MixedPrecisionMatrixDivision {
        /// Runtime matrix-division operation.
        operation: &'static str,
        /// Storage type of the left operand, when represented separately from
        /// its coarse value kind.
        left_dtype: Option<DType>,
        /// Storage type of the right operand.
        right_dtype: Option<DType>,
    },
}

impl fmt::Display for RuntimeLinalgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linalg(error) => error.fmt(formatter),
            Self::DoubleMatrixRequired {
                operation,
                operand,
                actual,
                dtype,
            } => write!(
                formatter,
                "{operation} {operand} must be a real or complex double matrix, received {actual} with storage {dtype:?}"
            ),
            Self::NumericMatrixRequired {
                operation,
                operand,
                actual,
                dtype,
            } => write!(
                formatter,
                "{operation} {operand} must be a real or complex double or single matrix, received {actual} with storage {dtype:?}"
            ),
            Self::MixedPrecisionMatrixDivision {
                operation,
                left_dtype,
                right_dtype,
            } => write!(
                formatter,
                "{operation} does not support mixed single/double operands ({left_dtype:?} and {right_dtype:?})"
            ),
        }
    }
}

impl Error for RuntimeLinalgError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Linalg(error) => Some(error),
            Self::DoubleMatrixRequired { .. }
            | Self::NumericMatrixRequired { .. }
            | Self::MixedPrecisionMatrixDivision { .. } => None,
        }
    }
}

impl From<LinalgError> for RuntimeLinalgError {
    fn from(error: LinalgError) -> Self {
        Self::Linalg(error)
    }
}

/// Warning-bearing success metadata for sparse matrix left division.
///
/// Providers continue to report singularity and deficient rank as typed
/// failures. The runtime converts only those two states into MATLAB-compatible
/// fallback values and carries this category to the interpreter for session
/// warning handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SparseSolveWarning {
    /// A square sparse factorization encountered a zero pivot.
    Singular { pivot: u64 },
    /// A tall sparse least-squares factorization measured deficient rank.
    RankDeficient { rank: u64, required_rank: u64 },
}

impl SparseSolveWarning {
    /// Stable OpenMat-owned warning identifier.
    pub(crate) const fn identifier(self) -> &'static str {
        match self {
            Self::Singular { .. } => "OpenMat:Sparse:SingularMatrix",
            Self::RankDeficient { .. } => "OpenMat:Sparse:RankDeficientMatrix",
        }
    }

    /// User-visible warning text owned by `OpenMat` rather than a provider.
    pub(crate) fn message(self) -> String {
        match self {
            Self::Singular { pivot } => format!(
                "sparse coefficient matrix has a zero pivot at position {pivot}; returning singular fallback values"
            ),
            Self::RankDeficient {
                rank,
                required_rank,
            } => format!(
                "sparse coefficient matrix has numerical rank {rank} below {required_rank}; returning a basic least-squares solution"
            ),
        }
    }
}

/// Successful sparse solve value plus optional warning metadata.
pub(crate) struct SparseSolveOutcome {
    pub(crate) value: Value,
    pub(crate) warning: Option<SparseSolveWarning>,
}

struct SparseDenseSolve<T> {
    value: DenseArray<T>,
    warning: Option<SparseSolveWarning>,
}

/// Multiplies two dynamic real or complex matrices of one precision with
/// `provider`.
///
/// Scalar-optimized double values are treated as `1x1` matrices. If either
/// operand is complex, the other operand is promoted within the operand
/// precision by copying each real component and supplying a zero imaginary
/// component. Inputs remain borrowed and immutable, and the returned
/// dense-array value owns result storage allocated independently from both
/// inputs.
///
/// # Errors
///
/// Returns [`RuntimeLinalgError::DoubleMatrixRequired`] for unsupported dynamic
/// values. Matrix validation and provider failures remain structured as
/// [`RuntimeLinalgError::Linalg`].
pub fn linalg_matrix_multiply(
    provider: &dyn LinalgProvider,
    left: &Value,
    right: &Value,
) -> Result<Value, RuntimeLinalgError> {
    if matches!(
        left,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) && matches!(
        right,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) {
        let left = SingleMatrix::from_value("matrix multiplication", "left operand", left)?;
        let right = SingleMatrix::from_value("matrix multiplication", "right operand", right)?;
        left.validate_rank("left matrix-multiply operand")?;
        right.validate_rank("right matrix-multiply operand")?;

        return match (left, right) {
            (SingleMatrix::Real(left), SingleMatrix::Real(right)) => Ok(Value::Array(
                ArrayData::F32(matrix_multiply_f32(provider, left, right)?),
            )),
            (left, right) => {
                let left = left.into_complex(None)?;
                let right = right.into_complex(None)?;
                Ok(Value::Array(ArrayData::ComplexF32(
                    matrix_multiply_complex32(provider, left.as_ref(), right.as_ref())?,
                )))
            }
        };
    }

    let left = DoubleMatrix::from_value("matrix multiplication", "left operand", left)?;
    let right = DoubleMatrix::from_value("matrix multiplication", "right operand", right)?;
    left.validate_rank("left matrix-multiply operand")?;
    right.validate_rank("right matrix-multiply operand")?;

    let result = match (left, right) {
        (DoubleMatrix::Real(left), DoubleMatrix::Real(right)) => {
            let output = matrix_multiply_f64(provider, left.as_ref(), right.as_ref())?;
            ArrayData::F64(output)
        }
        (left, right) => {
            let left = left.into_complex(None)?;
            let right = right.into_complex(None)?;
            let output = matrix_multiply_complex64(provider, left.as_ref(), right.as_ref())?;
            ArrayData::ComplexF64(output)
        }
    };
    Ok(Value::Array(result))
}

/// Multiplies a non-scalar sparse matrix by a full double matrix.
///
/// Each full right-hand-side column is applied through the sparse provider's
/// `SpMV` boundary. Real operands remain real; mixing either side with complex
/// double promotes the other side without mutating it. The result is always an
/// independently owned full matrix.
pub(crate) fn sparse_full_matrix_multiply<P: SparseProvider>(
    provider: &P,
    sparse: &SparseArrayData,
    full: &Value,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    const OPERATION: &str = "sparse-by-full matrix multiplication";
    check_cancellation(cancellation, OPERATION)?;
    let full = DoubleMatrix::from_value(OPERATION, "right operand", full)?;
    full.validate_rank("sparse-by-full right operand")?;
    let (full_rows, full_columns) = full.dimensions("sparse-by-full right operand")?;
    let sparse_rows = sparse.shape().extent(0);
    let sparse_columns = sparse.shape().extent(1);
    if sparse_columns != full_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "sparse-by-full inner dimensions",
            left: sparse_columns,
            right: full_rows,
        }
        .into());
    }

    let complex = sparse.is_complex() || matches!(&full, DoubleMatrix::Complex(_));
    if sparse_rows == 0 || sparse_columns == 0 || full_columns == 0 {
        let result = if complex {
            ArrayData::ComplexF64(zero_matrix(
                sparse_rows,
                full_columns,
                Complex64::ZERO,
                "sparse-by-full empty complex result",
                cancellation,
            )?)
        } else {
            ArrayData::F64(zero_matrix(
                sparse_rows,
                full_columns,
                0.0,
                "sparse-by-full empty real result",
                cancellation,
            )?)
        };
        return Ok(Value::Array(result));
    }

    if complex {
        let full = full.into_complex(cancellation)?;
        let result = match sparse {
            SparseArrayData::ComplexF64(matrix) => sparse_dense_columns_complex64(
                provider,
                sparse_view(matrix)?,
                full.as_ref(),
                cancellation,
            )?,
            SparseArrayData::F64(matrix) => {
                let values = promote_sparse_values(matrix.values(), cancellation)?;
                let matrix = CscMatrixRef::new(
                    matrix.rows(),
                    matrix.columns(),
                    matrix.col_offsets(),
                    matrix.row_indices(),
                    &values,
                )
                .map_err(sparse_provider_error)?;
                sparse_dense_columns_complex64(provider, matrix, full.as_ref(), cancellation)?
            }
            SparseArrayData::Logical(matrix) => {
                let values = promote_sparse_logical_complex(matrix.values(), cancellation)?;
                let matrix = CscMatrixRef::new(
                    matrix.rows(),
                    matrix.columns(),
                    matrix.col_offsets(),
                    matrix.row_indices(),
                    &values,
                )
                .map_err(sparse_provider_error)?;
                sparse_dense_columns_complex64(provider, matrix, full.as_ref(), cancellation)?
            }
        };
        Ok(Value::Array(ArrayData::ComplexF64(result)))
    } else {
        let DoubleMatrix::Real(full) = full else {
            unreachable!("complex full storage was selected above")
        };
        let result = match sparse {
            SparseArrayData::F64(matrix) => sparse_dense_columns_f64(
                provider,
                sparse_view(matrix)?,
                full.as_ref(),
                cancellation,
            )?,
            SparseArrayData::Logical(matrix) => {
                let values = promote_sparse_logical_real(matrix.values(), cancellation)?;
                let matrix = CscMatrixRef::new(
                    matrix.rows(),
                    matrix.columns(),
                    matrix.col_offsets(),
                    matrix.row_indices(),
                    &values,
                )
                .map_err(sparse_provider_error)?;
                sparse_dense_columns_f64(provider, matrix, full.as_ref(), cancellation)?
            }
            SparseArrayData::ComplexF64(_) => {
                unreachable!("complex sparse storage was selected above")
            }
        };
        Ok(Value::Array(ArrayData::F64(result)))
    }
}

/// Multiplies a full double matrix by a non-scalar sparse matrix.
///
/// This applies the exact transpose/conjugate-transpose relation
/// `(S' * F')'`, reusing the provider-backed sparse-by-full path and preserving
/// full result storage for real, complex, multi-column, and empty shapes.
pub(crate) fn full_sparse_matrix_multiply<P: SparseProvider>(
    provider: &P,
    full: &Value,
    sparse: &SparseArrayData,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    const OPERATION: &str = "full-by-sparse matrix multiplication";
    check_cancellation(cancellation, OPERATION)?;
    let full_matrix = DoubleMatrix::from_value(OPERATION, "left operand", full)?;
    full_matrix.validate_rank("full-by-sparse left operand")?;
    let (_, full_columns) = full_matrix.dimensions("full-by-sparse left operand")?;
    if full_columns != sparse.shape().extent(0) {
        return Err(LinalgError::DimensionMismatch {
            operation: "full-by-sparse inner dimensions",
            left: full_columns,
            right: sparse.shape().extent(0),
        }
        .into());
    }

    let complex = sparse.is_complex() || matches!(&full_matrix, DoubleMatrix::Complex(_));
    let sparse_transpose = sparse
        .try_transpose(complex, cancellation)
        .map_err(sparse_value_linalg_error)?;
    if complex {
        let full = full_matrix.into_complex(cancellation)?;
        let full_transpose = conjugate_transpose_complex(full.as_ref(), cancellation)?;
        let transposed = sparse_full_matrix_multiply(
            provider,
            &sparse_transpose,
            &Value::Array(ArrayData::ComplexF64(full_transpose)),
            cancellation,
        )?;
        let Value::Array(ArrayData::ComplexF64(transposed)) = transposed else {
            unreachable!("complex transpose relation returns complex full storage")
        };
        Ok(Value::Array(ArrayData::ComplexF64(
            conjugate_transpose_complex(&transposed, cancellation)?,
        )))
    } else {
        let DoubleMatrix::Real(full) = full_matrix else {
            unreachable!("complex full storage was selected above")
        };
        let full_transpose = transpose_real(full.as_ref(), cancellation)?;
        let transposed = sparse_full_matrix_multiply(
            provider,
            &sparse_transpose,
            &Value::Array(ArrayData::F64(full_transpose)),
            cancellation,
        )?;
        let Value::Array(ArrayData::F64(transposed)) = transposed else {
            unreachable!("real transpose relation returns real full storage")
        };
        Ok(Value::Array(ArrayData::F64(transpose_real(
            &transposed,
            cancellation,
        )?)))
    }
}

/// Solves `coefficients * X = right_hand_side` with `provider`.
///
/// Scalar-optimized double values are treated as `1x1` matrices. Mixed real
/// and complex operands are promoted to complex binary64 without changing any
/// input storage. `cancellation`, when present, is passed through to the
/// provider's existing cooperative-cancellation contract; no global provider
/// or cancellation state is installed.
///
/// # Errors
///
/// Returns [`RuntimeLinalgError::DoubleMatrixRequired`] for unsupported dynamic
/// values. Shape, singularity, cancellation, allocation, and provider failures
/// remain structured as [`RuntimeLinalgError::Linalg`].
pub fn linalg_square_solve(
    provider: &dyn LinalgProvider,
    coefficients: &Value,
    right_hand_side: &Value,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    let coefficients =
        DoubleMatrix::from_value("square linear solve", "coefficient matrix", coefficients)?;
    let right_hand_side = DoubleMatrix::from_value(
        "square linear solve",
        "right-hand-side matrix",
        right_hand_side,
    )?;
    coefficients.validate_rank("linear solve coefficient matrix")?;
    right_hand_side.validate_rank("linear solve right-hand side")?;

    let result = match (coefficients, right_hand_side) {
        (DoubleMatrix::Real(coefficients), DoubleMatrix::Real(right_hand_side)) => {
            let request = with_cancellation(
                SolveRequest::new(coefficients.as_ref(), right_hand_side.as_ref()),
                cancellation,
            );
            ArrayData::F64(provider.solve_f64(request)?)
        }
        (coefficients, right_hand_side) => {
            let coefficients = coefficients.into_complex(cancellation)?;
            let right_hand_side = right_hand_side.into_complex(cancellation)?;
            let request = with_cancellation(
                SolveRequest::new(coefficients.as_ref(), right_hand_side.as_ref()),
                cancellation,
            );
            ArrayData::ComplexF64(provider.solve_complex64(request)?)
        }
    };
    Ok(Value::Array(result))
}

/// Computes MATLAB-style matrix left division for dense double or single
/// numeric matrices.
///
/// Shape selects square solve, overdetermined least squares, or the
/// column-pivoted underdetermined basic-solution helper. Binary32 inputs remain
/// binary32 throughout promotion, factorization, solve, and result storage.
///
/// # Errors
///
/// Returns a structured numeric-class, shape, cancellation, allocation,
/// rank/singularity, or provider error.
pub fn linalg_matrix_left_divide(
    provider: &dyn LinalgProvider,
    coefficients: &Value,
    right_hand_side: &Value,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    match division_precision(
        "matrix left division",
        "coefficient matrix",
        coefficients,
        "right-hand-side matrix",
        right_hand_side,
    )? {
        MatrixPrecision::Double => left_divide_double(
            provider,
            DoubleMatrix::from_value("matrix left division", "coefficient matrix", coefficients)?,
            DoubleMatrix::from_value(
                "matrix left division",
                "right-hand-side matrix",
                right_hand_side,
            )?,
            cancellation,
        ),
        MatrixPrecision::Single => left_divide_single(
            provider,
            SingleMatrix::from_value("matrix left division", "coefficient matrix", coefficients)?,
            SingleMatrix::from_value(
                "matrix left division",
                "right-hand-side matrix",
                right_hand_side,
            )?,
            cancellation,
        ),
    }
}

/// Computes sparse-double matrix left division through an in-process sparse provider.
///
/// Square systems use reusable LU, tall systems use sparse QR least squares,
/// and wide full-row-rank systems use the provider-neutral sparse basic-solve
/// contract. Results are independently owned dense double or complex-double
/// matrices.
pub(crate) fn sparse_matrix_left_divide<P: SparseProvider>(
    provider: &P,
    coefficients: &SparseArrayData,
    right_hand_side: &Value,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseSolveOutcome, RuntimeLinalgError> {
    check_cancellation(cancellation, "sparse matrix left division")?;
    if matches!(coefficients, SparseArrayData::Logical(_)) {
        return Err(RuntimeLinalgError::NumericMatrixRequired {
            operation: "sparse matrix left division",
            operand: "coefficient matrix",
            actual: ValueKind::Sparse,
            dtype: Some(coefficients.dtype()),
        });
    }
    let right_hand_side = DoubleMatrix::from_value(
        "sparse matrix left division",
        "right-hand-side matrix",
        right_hand_side,
    )?;
    right_hand_side.validate_rank("sparse matrix left division right-hand side")?;
    let (right_rows, right_columns) =
        right_hand_side.dimensions("sparse matrix left division right-hand side")?;
    let rows = coefficients.shape().extent(0);
    let columns = coefficients.shape().extent(1);
    if rows != right_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "sparse matrix left division coefficient and right-hand-side rows",
            left: rows,
            right: right_rows,
        }
        .into());
    }
    let complex = coefficients.is_complex() || matches!(&right_hand_side, DoubleMatrix::Complex(_));
    if rows == 0 || columns == 0 || right_columns == 0 {
        let result = if complex {
            ArrayData::ComplexF64(zero_matrix(
                columns,
                right_columns,
                Complex64::ZERO,
                "sparse matrix left division empty complex result",
                cancellation,
            )?)
        } else {
            ArrayData::F64(zero_matrix(
                columns,
                right_columns,
                0.0,
                "sparse matrix left division empty real result",
                cancellation,
            )?)
        };
        return Ok(SparseSolveOutcome {
            value: Value::Array(result),
            warning: None,
        });
    }
    let result = match (coefficients, right_hand_side) {
        (SparseArrayData::F64(coefficients), DoubleMatrix::Real(right_hand_side)) => {
            solve_sparse_f64(
                provider,
                coefficients,
                right_hand_side.as_ref(),
                cancellation,
            )
            .map(|result| (ArrayData::F64(result.value), result.warning))
        }
        (SparseArrayData::ComplexF64(coefficients), right_hand_side) => {
            let right_hand_side = right_hand_side.into_complex(cancellation)?;
            solve_sparse_complex64(
                provider,
                sparse_view(coefficients)?,
                right_hand_side.as_ref(),
                cancellation,
            )
            .map(|result| (ArrayData::ComplexF64(result.value), result.warning))
        }
        (SparseArrayData::F64(coefficients), right_hand_side) => {
            let right_hand_side = right_hand_side.into_complex(cancellation)?;
            let values = promote_sparse_values(coefficients.values(), cancellation)?;
            let view = CscMatrixRef::new(
                coefficients.rows(),
                coefficients.columns(),
                coefficients.col_offsets(),
                coefficients.row_indices(),
                &values,
            )
            .map_err(sparse_provider_error)?;
            solve_sparse_complex64(provider, view, right_hand_side.as_ref(), cancellation)
                .map(|result| (ArrayData::ComplexF64(result.value), result.warning))
        }
        (SparseArrayData::Logical(_), _) => unreachable!("logical sparse rejected above"),
    }
    .map_err(RuntimeLinalgError::from)?;
    Ok(SparseSolveOutcome {
        value: Value::Array(result.0),
        warning: result.1,
    })
}

fn solve_sparse_f64<P: SparseProvider>(
    provider: &P,
    coefficients: &CscMatrix<f64>,
    right_hand_side: &DenseArray<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseDenseSolve<f64>, LinalgError> {
    let view = sparse_view(coefficients)?;
    let sparse_cancellation = sparse_cancellation(cancellation);
    let (result, warning) = match coefficients.rows().cmp(&coefficients.columns()) {
        std::cmp::Ordering::Equal => {
            let provider_result = provider
                .analyze_lu(view.pattern(), sparse_cancellation)
                .and_then(|symbolic| provider.factor_lu_f64(&symbolic, view, sparse_cancellation))
                .and_then(|factor| {
                    factor.solve(
                        right_hand_side.as_slice(),
                        right_hand_side.shape().extent(1),
                        sparse_cancellation,
                    )
                });
            match provider_result {
                Ok(result) => (result, None),
                Err(SparseError::Singular { pivot, .. }) => (
                    singular_sparse_fallback(view, right_hand_side, cancellation)?,
                    Some(SparseSolveWarning::Singular { pivot }),
                ),
                Err(error) => return Err(sparse_provider_error(error)),
            }
        }
        std::cmp::Ordering::Greater => match provider.least_squares_f64(
            view,
            right_hand_side.as_slice(),
            right_hand_side.shape().extent(1),
            sparse_cancellation,
        ) {
            Ok(result) => (result, None),
            Err(SparseError::RankDeficient {
                rank,
                required_rank,
                ..
            }) => (
                rank_deficient_sparse_fallback(view, right_hand_side, cancellation)?,
                Some(SparseSolveWarning::RankDeficient {
                    rank,
                    required_rank,
                }),
            ),
            Err(error) => return Err(sparse_provider_error(error)),
        },
        std::cmp::Ordering::Less => (
            provider
                .underdetermined_basic_f64(
                    view,
                    right_hand_side.as_slice(),
                    right_hand_side.shape().extent(1),
                    sparse_cancellation,
                )
                .map_err(sparse_provider_error)?,
            None,
        ),
    };
    let value = dense_sparse_result(
        result,
        coefficients.columns(),
        right_hand_side.shape().extent(1),
    )?;
    Ok(SparseDenseSolve { value, warning })
}

fn solve_sparse_complex64<P: SparseProvider>(
    provider: &P,
    coefficients: CscMatrixRef<'_, Complex64>,
    right_hand_side: &DenseArray<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseDenseSolve<Complex64>, LinalgError> {
    let sparse_cancellation = sparse_cancellation(cancellation);
    let pattern = coefficients.pattern();
    let (result, warning) = match pattern.rows().cmp(&pattern.columns()) {
        std::cmp::Ordering::Equal => {
            let provider_result = provider
                .analyze_lu(pattern, sparse_cancellation)
                .and_then(|symbolic| {
                    provider.factor_lu_complex64(&symbolic, coefficients, sparse_cancellation)
                })
                .and_then(|factor| {
                    factor.solve(
                        right_hand_side.as_slice(),
                        right_hand_side.shape().extent(1),
                        sparse_cancellation,
                    )
                });
            match provider_result {
                Ok(result) => (result, None),
                Err(SparseError::Singular { pivot, .. }) => (
                    singular_sparse_fallback(coefficients, right_hand_side, cancellation)?,
                    Some(SparseSolveWarning::Singular { pivot }),
                ),
                Err(error) => return Err(sparse_provider_error(error)),
            }
        }
        std::cmp::Ordering::Greater => match provider.least_squares_complex64(
            coefficients,
            right_hand_side.as_slice(),
            right_hand_side.shape().extent(1),
            sparse_cancellation,
        ) {
            Ok(result) => (result, None),
            Err(SparseError::RankDeficient {
                rank,
                required_rank,
                ..
            }) => (
                rank_deficient_sparse_fallback(coefficients, right_hand_side, cancellation)?,
                Some(SparseSolveWarning::RankDeficient {
                    rank,
                    required_rank,
                }),
            ),
            Err(error) => return Err(sparse_provider_error(error)),
        },
        std::cmp::Ordering::Less => (
            provider
                .underdetermined_basic_complex64(
                    coefficients,
                    right_hand_side.as_slice(),
                    right_hand_side.shape().extent(1),
                    sparse_cancellation,
                )
                .map_err(sparse_provider_error)?,
            None,
        ),
    };
    let value = dense_sparse_result(
        result,
        coefficients.pattern().columns(),
        right_hand_side.shape().extent(1),
    )?;
    Ok(SparseDenseSolve { value, warning })
}

fn singular_sparse_fallback<T: SparseFallbackScalar>(
    coefficients: CscMatrixRef<'_, T>,
    right_hand_side: &DenseArray<T>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseSolveResult<T>, LinalgError> {
    const OPERATION: &str = "singular sparse square fallback";
    let pattern = coefficients.pattern();
    let order = checked_host_index(pattern.rows(), OPERATION)?;
    let right_columns = checked_host_index(right_hand_side.shape().extent(1), OPERATION)?;
    let matrix_len = checked_matrix_length(OPERATION, pattern.rows(), pattern.columns())?;
    let mut factors = filled_buffer(matrix_len, T::zero(), OPERATION)?;
    for column in 0..order {
        check_cancellation(cancellation, OPERATION)?;
        let start = checked_host_index(pattern.col_offsets()[column], OPERATION)?;
        let end = checked_host_index(pattern.col_offsets()[column + 1], OPERATION)?;
        for index in start..end {
            let row = checked_host_index(pattern.row_indices()[index], OPERATION)?;
            factors[row + column * order] = coefficients.values()[index];
        }
    }
    let mut transformed_rhs = reserved_buffer(right_hand_side.as_slice().len(), OPERATION)?;
    transformed_rhs.extend_from_slice(right_hand_side.as_slice());

    // Continue past exact zero pivots rather than rejecting the factor. This
    // leaves the later triangular division to produce the NaN/Inf payload that
    // sparse R2022b exposes while still preserving solvable structural pivots.
    for pivot_column in 0..order {
        check_cancellation(cancellation, OPERATION)?;
        let mut pivot_row = pivot_column;
        let mut pivot_norm = factors[pivot_column + pivot_column * order].norm_squared();
        for row in (pivot_column + 1)..order {
            let candidate = factors[row + pivot_column * order].norm_squared();
            if candidate > pivot_norm {
                pivot_norm = candidate;
                pivot_row = row;
            }
        }
        if pivot_norm == 0.0 || pivot_norm.is_nan() {
            continue;
        }
        if pivot_row != pivot_column {
            for column in 0..order {
                factors.swap(pivot_column + column * order, pivot_row + column * order);
            }
            for rhs_column in 0..right_columns {
                transformed_rhs.swap(
                    pivot_column + rhs_column * order,
                    pivot_row + rhs_column * order,
                );
            }
        }
        let diagonal = factors[pivot_column + pivot_column * order];
        for row in (pivot_column + 1)..order {
            let multiplier = factors[row + pivot_column * order].divide(diagonal);
            factors[row + pivot_column * order] = multiplier;
            for column in (pivot_column + 1)..order {
                let index = row + column * order;
                factors[index] = factors[index]
                    .subtract(multiplier.multiply(factors[pivot_column + column * order]));
            }
            for rhs_column in 0..right_columns {
                let index = row + rhs_column * order;
                transformed_rhs[index] = transformed_rhs[index].subtract(
                    multiplier.multiply(transformed_rhs[pivot_column + rhs_column * order]),
                );
            }
        }
    }

    let mut solution = filled_buffer(
        checked_matrix_length(
            OPERATION,
            pattern.columns(),
            right_hand_side.shape().extent(1),
        )?,
        T::zero(),
        OPERATION,
    )?;
    for rhs_column in 0..right_columns {
        for row in (0..order).rev() {
            check_cancellation(cancellation, OPERATION)?;
            let mut residual = transformed_rhs[row + rhs_column * order];
            for column in (row + 1)..order {
                residual = residual.subtract(
                    factors[row + column * order].multiply(solution[column + rhs_column * order]),
                );
            }
            solution[row + rhs_column * order] = residual.divide(factors[row + row * order]);
        }
    }
    Ok(SparseSolveResult {
        rows: pattern.columns(),
        columns: right_hand_side.shape().extent(1),
        values: solution,
        metrics: SparseMetrics::default(),
    })
}

fn rank_deficient_sparse_fallback<T: SparseFallbackScalar>(
    coefficients: CscMatrixRef<'_, T>,
    right_hand_side: &DenseArray<T>,
    cancellation: Option<&AtomicBool>,
) -> Result<SparseSolveResult<T>, LinalgError> {
    const OPERATION: &str = "rank-deficient sparse tall fallback";
    let pattern = coefficients.pattern();
    let rows = checked_host_index(pattern.rows(), OPERATION)?;
    let columns = checked_host_index(pattern.columns(), OPERATION)?;
    let right_columns = checked_host_index(right_hand_side.shape().extent(1), OPERATION)?;
    let matrix_norm = coefficients
        .values()
        .iter()
        .map(|value| value.norm_squared())
        .sum::<f64>()
        .sqrt();
    let dimension_scale = f64::from(u32::try_from(rows.max(columns)).unwrap_or(u32::MAX));
    let tolerance = f64::EPSILON * dimension_scale * matrix_norm.max(1.0);

    let mut selected_columns = reserved_buffer(columns, OPERATION)?;
    let mut basis: Vec<Vec<T>> = reserved_buffer(columns, OPERATION)?;
    let mut triangular_columns: Vec<Vec<T>> = reserved_buffer(columns, OPERATION)?;
    for column in 0..columns {
        check_cancellation(cancellation, OPERATION)?;
        let mut candidate = filled_buffer(rows, T::zero(), OPERATION)?;
        let start = checked_host_index(pattern.col_offsets()[column], OPERATION)?;
        let end = checked_host_index(pattern.col_offsets()[column + 1], OPERATION)?;
        for index in start..end {
            let row = checked_host_index(pattern.row_indices()[index], OPERATION)?;
            candidate[row] = coefficients.values()[index];
        }
        let mut projections = filled_buffer(basis.len(), T::zero(), OPERATION)?;
        for _ in 0..2 {
            for (previous, q) in basis.iter().enumerate() {
                let projection = dot_conjugate(q, &candidate);
                projections[previous] = projections[previous].add(projection);
                for row in 0..rows {
                    candidate[row] = candidate[row].subtract(q[row].multiply(projection));
                }
            }
        }
        let norm = candidate
            .iter()
            .map(|value| value.norm_squared())
            .sum::<f64>()
            .sqrt();
        if !norm.is_finite() || norm <= tolerance {
            continue;
        }
        projections.push(T::from_real(norm));
        for value in &mut candidate {
            *value = value.scale(1.0 / norm);
        }
        selected_columns.push(column);
        basis.push(candidate);
        triangular_columns.push(projections);
    }

    let rank = selected_columns.len();
    let mut projected_rhs = filled_buffer(
        rank.checked_mul(right_columns)
            .ok_or(LinalgError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            })?,
        T::zero(),
        OPERATION,
    )?;
    for rhs_column in 0..right_columns {
        let start = rhs_column * rows;
        let rhs = &right_hand_side.as_slice()[start..start + rows];
        for (basis_column, q) in basis.iter().enumerate() {
            projected_rhs[basis_column + rhs_column * rank] = dot_conjugate(q, rhs);
        }
    }

    let mut solution = filled_buffer(
        checked_matrix_length(
            OPERATION,
            pattern.columns(),
            right_hand_side.shape().extent(1),
        )?,
        T::zero(),
        OPERATION,
    )?;
    for rhs_column in 0..right_columns {
        for selected in (0..rank).rev() {
            check_cancellation(cancellation, OPERATION)?;
            let mut residual = projected_rhs[selected + rhs_column * rank];
            for upper in (selected + 1)..rank {
                residual = residual.subtract(
                    triangular_columns[upper][selected]
                        .multiply(solution[selected_columns[upper] + rhs_column * columns]),
                );
            }
            solution[selected_columns[selected] + rhs_column * columns] =
                residual.divide(triangular_columns[selected][selected]);
        }
    }
    Ok(SparseSolveResult {
        rows: pattern.columns(),
        columns: right_hand_side.shape().extent(1),
        values: solution,
        metrics: SparseMetrics::default(),
    })
}

fn dot_conjugate<T: SparseFallbackScalar>(left: &[T], right: &[T]) -> T {
    left.iter()
        .zip(right)
        .fold(T::zero(), |sum, (&left, &right)| {
            sum.add(left.conjugate().multiply(right))
        })
}

fn filled_buffer<T: Copy>(
    length: usize,
    value: T,
    operation: &'static str,
) -> Result<Vec<T>, LinalgError> {
    let mut values = reserved_buffer(length, operation)?;
    values.resize(length, value);
    Ok(values)
}

trait SparseFallbackScalar: Copy {
    fn zero() -> Self;
    fn from_real(value: f64) -> Self;
    fn norm_squared(self) -> f64;
    fn conjugate(self) -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn scale(self, factor: f64) -> Self;
}

impl SparseFallbackScalar for f64 {
    fn zero() -> Self {
        0.0
    }

    fn from_real(value: f64) -> Self {
        value
    }

    fn norm_squared(self) -> f64 {
        self * self
    }

    fn conjugate(self) -> Self {
        self
    }

    fn add(self, right: Self) -> Self {
        self + right
    }

    fn subtract(self, right: Self) -> Self {
        self - right
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        self / right
    }

    fn scale(self, factor: f64) -> Self {
        self * factor
    }
}

impl SparseFallbackScalar for Complex64 {
    fn zero() -> Self {
        Self::ZERO
    }

    fn from_real(value: f64) -> Self {
        Self::new(value, 0.0)
    }

    fn norm_squared(self) -> f64 {
        self.re.mul_add(self.re, self.im * self.im)
    }

    fn conjugate(self) -> Self {
        self.conjugate()
    }

    fn add(self, right: Self) -> Self {
        Self::new(self.re + right.re, self.im + right.im)
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }

    fn multiply(self, right: Self) -> Self {
        self * right
    }

    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.im + right.re * ratio;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

fn sparse_dense_columns_f64<P: SparseProvider>(
    provider: &P,
    sparse: CscMatrixRef<'_, f64>,
    full: &DenseArray<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<f64>, LinalgError> {
    const OPERATION: &str = "sparse-by-full real matrix multiplication";
    let rows = sparse.pattern().rows();
    let columns = full.shape().extent(1);
    let output_len = checked_matrix_length(OPERATION, rows, columns)?;
    let input_rows = checked_host_index(sparse.pattern().columns(), OPERATION)?;
    let output_rows = checked_host_index(rows, OPERATION)?;
    let mut values = reserved_buffer(output_len, OPERATION)?;
    let cancellation = sparse_cancellation(cancellation);
    for column in 0..checked_host_index(columns, OPERATION)? {
        cancellation
            .checkpoint(OPERATION, openmat_linalg::SparsePhase::Execution)
            .map_err(sparse_provider_error)?;
        let start = column
            .checked_mul(input_rows)
            .ok_or(LinalgError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            })?;
        let end = start
            .checked_add(input_rows)
            .ok_or(LinalgError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            })?;
        let (column_values, _) = provider
            .spmv_f64(sparse, &full.as_slice()[start..end], cancellation)
            .map_err(sparse_provider_error)?;
        if column_values.len() != output_rows {
            return Err(LinalgError::ProviderFailure {
                provider: provider.name(),
                operation: OPERATION,
                detail: format!(
                    "expected {output_rows} output rows, received {}",
                    column_values.len()
                ),
            });
        }
        values.extend(column_values);
    }
    DenseArray::from_vec(Shape::new([rows, columns])?, values).map_err(Into::into)
}

fn sparse_dense_columns_complex64<P: SparseProvider>(
    provider: &P,
    sparse: CscMatrixRef<'_, Complex64>,
    full: &DenseArray<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    const OPERATION: &str = "sparse-by-full complex matrix multiplication";
    let rows = sparse.pattern().rows();
    let columns = full.shape().extent(1);
    let output_len = checked_matrix_length(OPERATION, rows, columns)?;
    let input_rows = checked_host_index(sparse.pattern().columns(), OPERATION)?;
    let output_rows = checked_host_index(rows, OPERATION)?;
    let mut values = reserved_buffer(output_len, OPERATION)?;
    let cancellation = sparse_cancellation(cancellation);
    for column in 0..checked_host_index(columns, OPERATION)? {
        cancellation
            .checkpoint(OPERATION, openmat_linalg::SparsePhase::Execution)
            .map_err(sparse_provider_error)?;
        let start = column
            .checked_mul(input_rows)
            .ok_or(LinalgError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            })?;
        let end = start
            .checked_add(input_rows)
            .ok_or(LinalgError::AllocationFailure {
                operation: OPERATION,
                elements: u64::MAX,
            })?;
        let (column_values, _) = provider
            .spmv_complex64(sparse, &full.as_slice()[start..end], cancellation)
            .map_err(sparse_provider_error)?;
        if column_values.len() != output_rows {
            return Err(LinalgError::ProviderFailure {
                provider: provider.name(),
                operation: OPERATION,
                detail: format!(
                    "expected {output_rows} output rows, received {}",
                    column_values.len()
                ),
            });
        }
        values.extend(column_values);
    }
    DenseArray::from_vec(Shape::new([rows, columns])?, values).map_err(Into::into)
}

fn checked_matrix_length(
    operation: &'static str,
    rows: u64,
    columns: u64,
) -> Result<usize, LinalgError> {
    let elements = rows
        .checked_mul(columns)
        .ok_or(LinalgError::AllocationFailure {
            operation,
            elements: u64::MAX,
        })?;
    checked_host_index(elements, operation)
}

fn sparse_view<T>(matrix: &CscMatrix<T>) -> Result<CscMatrixRef<'_, T>, LinalgError> {
    CscMatrixRef::new(
        matrix.rows(),
        matrix.columns(),
        matrix.col_offsets(),
        matrix.row_indices(),
        matrix.values(),
    )
    .map_err(sparse_provider_error)
}

fn dense_sparse_result<T>(
    result: SparseSolveResult<T>,
    expected_rows: u64,
    expected_columns: u64,
) -> Result<DenseArray<T>, LinalgError> {
    if result.rows != expected_rows || result.columns != expected_columns {
        return Err(LinalgError::ProviderFailure {
            provider: "sparse provider",
            operation: "sparse solve result validation",
            detail: format!(
                "expected {expected_rows}x{expected_columns}, received {}x{}",
                result.rows, result.columns
            ),
        });
    }
    DenseArray::from_vec(Shape::new([result.rows, result.columns])?, result.values)
        .map_err(Into::into)
}

fn promote_sparse_values(
    values: &[f64],
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<Complex64>, LinalgError> {
    let mut promoted = reserved_buffer(values.len(), "sparse complex promotion")?;
    for &value in values {
        check_cancellation(cancellation, "sparse complex promotion")?;
        promoted.push(Complex64::new(value, 0.0));
    }
    Ok(promoted)
}

fn promote_sparse_logical_real(
    values: &[openmat_array::Logical],
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<f64>, LinalgError> {
    let mut promoted = reserved_buffer(values.len(), "sparse logical-to-real promotion")?;
    for &value in values {
        check_cancellation(cancellation, "sparse logical-to-real promotion")?;
        promoted.push(if value.get() { 1.0 } else { 0.0 });
    }
    Ok(promoted)
}

fn promote_sparse_logical_complex(
    values: &[openmat_array::Logical],
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<Complex64>, LinalgError> {
    let mut promoted = reserved_buffer(values.len(), "sparse logical-to-complex promotion")?;
    for &value in values {
        check_cancellation(cancellation, "sparse logical-to-complex promotion")?;
        promoted.push(Complex64::new(if value.get() { 1.0 } else { 0.0 }, 0.0));
    }
    Ok(promoted)
}

fn sparse_cancellation(cancellation: Option<&AtomicBool>) -> SparseCancellation<'_> {
    cancellation.map_or_else(SparseCancellation::never, SparseCancellation::from_atomic)
}

fn sparse_provider_error(error: SparseError) -> LinalgError {
    match error {
        SparseError::DimensionMismatch {
            operation,
            expected,
            actual,
        } => LinalgError::DimensionMismatch {
            operation,
            left: expected,
            right: actual,
        },
        SparseError::SquareMatrixRequired { rows, columns } => LinalgError::SquareMatrixRequired {
            operand: "sparse coefficient matrix",
            rows,
            columns,
        },
        SparseError::Cancelled { operation, .. } => LinalgError::Cancelled { operation },
        SparseError::Singular { provider, pivot } => LinalgError::SingularMatrix {
            provider,
            operation: "sparse LU factorization",
            pivot,
        },
        SparseError::RankDeficient {
            provider,
            rank,
            required_rank,
        } => LinalgError::RankDeficient {
            provider,
            operation: "sparse QR least-squares factorization",
            deficient_diagonal: rank.saturating_add(1),
            required_rank,
        },
        SparseError::AllocationFailure {
            operation,
            elements,
        } => LinalgError::AllocationFailure {
            operation,
            elements,
        },
        SparseError::ProviderFailure {
            provider,
            operation,
            detail,
        } => LinalgError::ProviderFailure {
            provider,
            operation,
            detail,
        },
        other => LinalgError::ProviderFailure {
            provider: "sparse provider",
            operation: "sparse matrix left division",
            detail: other.to_string(),
        },
    }
}

fn sparse_value_linalg_error(error: SparseValueError) -> LinalgError {
    match error {
        SparseValueError::Cancelled => LinalgError::Cancelled {
            operation: "sparse transpose for matrix multiplication",
        },
        SparseValueError::HostLengthOverflow { elements } => LinalgError::AllocationFailure {
            operation: "sparse transpose for matrix multiplication",
            elements,
        },
        SparseValueError::ShapeOverflow { rows, columns } => LinalgError::AllocationFailure {
            operation: "sparse transpose for matrix multiplication",
            elements: rows.saturating_mul(columns),
        },
        SparseValueError::Allocation { elements, .. } => LinalgError::AllocationFailure {
            operation: "sparse transpose for matrix multiplication",
            elements: u64::try_from(elements).unwrap_or(u64::MAX),
        },
        other => LinalgError::ProviderFailure {
            provider: "openmat sparse value",
            operation: "sparse transpose for matrix multiplication",
            detail: other.to_string(),
        },
    }
}

/// Computes matrix right division through `(right' \\ left')'`.
///
/// Every apostrophe is a conjugate transpose. The original operands are
/// validated and borrowed immutably, and all transposed and result storage is
/// independently owned.
///
/// # Errors
///
/// Returns the same structured failures as [`linalg_matrix_left_divide`].
pub fn linalg_matrix_right_divide(
    provider: &dyn LinalgProvider,
    left: &Value,
    right: &Value,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    match division_precision(
        "matrix right division",
        "left matrix",
        left,
        "right matrix",
        right,
    )? {
        MatrixPrecision::Double => right_divide_double(
            provider,
            DoubleMatrix::from_value("matrix right division", "left matrix", left)?,
            DoubleMatrix::from_value("matrix right division", "right matrix", right)?,
            cancellation,
        ),
        MatrixPrecision::Single => right_divide_single(
            provider,
            SingleMatrix::from_value("matrix right division", "left matrix", left)?,
            SingleMatrix::from_value("matrix right division", "right matrix", right)?,
            cancellation,
        ),
    }
}

fn with_cancellation<'array, T>(
    request: SolveRequest<'array, T>,
    cancellation: Option<&'array AtomicBool>,
) -> SolveRequest<'array, T> {
    match cancellation {
        Some(flag) => request.with_cancellation_flag(flag),
        None => request,
    }
}

enum DoubleMatrix<'array> {
    Real(Cow<'array, DenseArray<f64>>),
    Complex(Cow<'array, DenseArray<Complex64>>),
}

impl<'array> DoubleMatrix<'array> {
    fn from_value(
        operation: &'static str,
        operand: &'static str,
        value: &'array Value,
    ) -> Result<Self, RuntimeLinalgError> {
        match value {
            Value::Double(value) => Ok(Self::Real(Cow::Owned(scalar_array(*value)?))),
            Value::Complex(value) => Ok(Self::Complex(Cow::Owned(scalar_array((*value).into())?))),
            Value::Array(ArrayData::F64(array)) => Ok(Self::Real(Cow::Borrowed(array))),
            Value::Array(ArrayData::ComplexF64(array)) => Ok(Self::Complex(Cow::Borrowed(array))),
            _ => Err(RuntimeLinalgError::DoubleMatrixRequired {
                operation,
                operand,
                actual: value.kind(),
                dtype: value.dtype(),
            }),
        }
    }

    fn validate_rank(&self, operand: &'static str) -> Result<(), LinalgError> {
        match self {
            Self::Real(array) => matrix_dimensions(operand, array.as_ref()).map(|_| ()),
            Self::Complex(array) => matrix_dimensions(operand, array.as_ref()).map(|_| ()),
        }
    }

    fn into_complex(
        self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Cow<'array, DenseArray<Complex64>>, LinalgError> {
        match self {
            Self::Real(array) => Ok(Cow::Owned(promote_real(array.as_ref(), cancellation)?)),
            Self::Complex(array) => Ok(array),
        }
    }

    fn dimensions(&self, operand: &'static str) -> Result<(u64, u64), LinalgError> {
        match self {
            Self::Real(array) => matrix_dimensions(operand, array.as_ref()),
            Self::Complex(array) => matrix_dimensions(operand, array.as_ref()),
        }
    }
}

#[derive(Clone, Copy)]
enum SingleMatrix<'array> {
    Real(&'array DenseArray<f32>),
    Complex(&'array DenseArray<Complex32>),
}

impl<'array> SingleMatrix<'array> {
    fn from_value(
        operation: &'static str,
        operand: &'static str,
        value: &'array Value,
    ) -> Result<Self, RuntimeLinalgError> {
        match value {
            Value::Array(ArrayData::F32(array)) => Ok(Self::Real(array)),
            Value::Array(ArrayData::ComplexF32(array)) => Ok(Self::Complex(array)),
            _ => Err(numeric_matrix_required(operation, operand, value)),
        }
    }

    fn dimensions(&self, operand: &'static str) -> Result<(u64, u64), LinalgError> {
        match self {
            Self::Real(array) => matrix_dimensions(operand, array),
            Self::Complex(array) => matrix_dimensions(operand, array),
        }
    }

    fn validate_rank(&self, operand: &'static str) -> Result<(), LinalgError> {
        self.dimensions(operand).map(|_| ())
    }

    fn into_complex(
        self,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Cow<'array, DenseArray<Complex32>>, LinalgError> {
        match self {
            Self::Real(array) => Ok(Cow::Owned(promote_single_real(array, cancellation)?)),
            Self::Complex(array) => Ok(Cow::Borrowed(array)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatrixPrecision {
    Double,
    Single,
}

fn division_precision(
    operation: &'static str,
    left_operand: &'static str,
    left: &Value,
    right_operand: &'static str,
    right: &Value,
) -> Result<MatrixPrecision, RuntimeLinalgError> {
    let left_precision = value_precision(left)
        .ok_or_else(|| numeric_matrix_required(operation, left_operand, left))?;
    let right_precision = value_precision(right)
        .ok_or_else(|| numeric_matrix_required(operation, right_operand, right))?;
    if left_precision != right_precision {
        return Err(RuntimeLinalgError::MixedPrecisionMatrixDivision {
            operation,
            left_dtype: left.dtype(),
            right_dtype: right.dtype(),
        });
    }
    Ok(left_precision)
}

fn value_precision(value: &Value) -> Option<MatrixPrecision> {
    match value {
        Value::Double(_)
        | Value::Complex(_)
        | Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_)) => {
            Some(MatrixPrecision::Double)
        }
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_)) => Some(MatrixPrecision::Single),
        _ => None,
    }
}

fn numeric_matrix_required(
    operation: &'static str,
    operand: &'static str,
    value: &Value,
) -> RuntimeLinalgError {
    RuntimeLinalgError::NumericMatrixRequired {
        operation,
        operand,
        actual: value.kind(),
        dtype: value.dtype(),
    }
}

fn left_divide_double(
    provider: &dyn LinalgProvider,
    coefficients: DoubleMatrix<'_>,
    right_hand_side: DoubleMatrix<'_>,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    let result = match (coefficients, right_hand_side) {
        (DoubleMatrix::Real(coefficients), DoubleMatrix::Real(right_hand_side)) => {
            ArrayData::F64(solve_matrix_f64(
                provider,
                coefficients.as_ref(),
                right_hand_side.as_ref(),
                cancellation,
            )?)
        }
        (coefficients, right_hand_side) => {
            let coefficients = coefficients.into_complex(cancellation)?;
            let right_hand_side = right_hand_side.into_complex(cancellation)?;
            ArrayData::ComplexF64(solve_matrix_complex64(
                provider,
                coefficients.as_ref(),
                right_hand_side.as_ref(),
                cancellation,
            )?)
        }
    };
    Ok(Value::Array(result))
}

fn left_divide_single(
    provider: &dyn LinalgProvider,
    coefficients: SingleMatrix<'_>,
    right_hand_side: SingleMatrix<'_>,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    match (coefficients, right_hand_side) {
        (SingleMatrix::Real(coefficients), SingleMatrix::Real(right_hand_side)) => {
            Ok(Value::Array(ArrayData::F32(solve_matrix_f32(
                provider,
                coefficients,
                right_hand_side,
                cancellation,
            )?)))
        }
        (coefficients, right_hand_side) => {
            let coefficients = coefficients.into_complex(cancellation)?;
            let right_hand_side = right_hand_side.into_complex(cancellation)?;
            Ok(Value::Array(ArrayData::ComplexF32(solve_matrix_complex32(
                provider,
                coefficients.as_ref(),
                right_hand_side.as_ref(),
                cancellation,
            )?)))
        }
    }
}

fn right_divide_double(
    provider: &dyn LinalgProvider,
    left: DoubleMatrix<'_>,
    right: DoubleMatrix<'_>,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    validate_right_division_dimensions(&left, &right)?;
    match (left, right) {
        (DoubleMatrix::Real(left), DoubleMatrix::Real(right)) => {
            let right_transpose = transpose_real(right.as_ref(), cancellation)?;
            let left_transpose = transpose_real(left.as_ref(), cancellation)?;
            let transposed_result =
                solve_matrix_f64(provider, &right_transpose, &left_transpose, cancellation)?;
            Ok(Value::Array(ArrayData::F64(transpose_real(
                &transposed_result,
                cancellation,
            )?)))
        }
        (left, right) => {
            let left = left.into_complex(cancellation)?;
            let right = right.into_complex(cancellation)?;
            let right_transpose = conjugate_transpose_complex(right.as_ref(), cancellation)?;
            let left_transpose = conjugate_transpose_complex(left.as_ref(), cancellation)?;
            let transposed_result =
                solve_matrix_complex64(provider, &right_transpose, &left_transpose, cancellation)?;
            Ok(Value::Array(ArrayData::ComplexF64(
                conjugate_transpose_complex(&transposed_result, cancellation)?,
            )))
        }
    }
}

fn right_divide_single(
    provider: &dyn LinalgProvider,
    left: SingleMatrix<'_>,
    right: SingleMatrix<'_>,
    cancellation: Option<&AtomicBool>,
) -> Result<Value, RuntimeLinalgError> {
    validate_right_division_dimensions_single(&left, &right)?;
    match (left, right) {
        (SingleMatrix::Real(left), SingleMatrix::Real(right)) => {
            let right_transpose = transpose_real(right, cancellation)?;
            let left_transpose = transpose_real(left, cancellation)?;
            let transposed_result =
                solve_matrix_f32(provider, &right_transpose, &left_transpose, cancellation)?;
            let result = transpose_real(&transposed_result, cancellation)?;
            Ok(Value::Array(ArrayData::F32(result)))
        }
        (left, right) => {
            let left = left.into_complex(cancellation)?;
            let right = right.into_complex(cancellation)?;
            let right_transpose = conjugate_transpose_complex32(right.as_ref(), cancellation)?;
            let left_transpose = conjugate_transpose_complex32(left.as_ref(), cancellation)?;
            let transposed_result =
                solve_matrix_complex32(provider, &right_transpose, &left_transpose, cancellation)?;
            let result = conjugate_transpose_complex32(&transposed_result, cancellation)?;
            Ok(Value::Array(ArrayData::ComplexF32(result)))
        }
    }
}

fn solve_matrix_f32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f32>,
    right_hand_side: &DenseArray<f32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<f32>, LinalgError> {
    let (rows, columns, right_hand_sides) =
        validate_left_division_dimensions(coefficients, right_hand_side)?;
    check_cancellation(cancellation, "matrix left division")?;
    if rows == 0 || columns == 0 || right_hand_sides == 0 {
        return zero_matrix(
            columns,
            right_hand_sides,
            0.0,
            "matrix left division empty real result",
            cancellation,
        );
    }
    match rows.cmp(&columns) {
        std::cmp::Ordering::Equal => provider.solve_f32(with_cancellation(
            SolveRequest::new(coefficients, right_hand_side),
            cancellation,
        )),
        std::cmp::Ordering::Greater => {
            provider.solve_rectangular_f32(with_rectangular_cancellation(
                RectangularSolveRequest::new(coefficients, right_hand_side),
                cancellation,
            ))
        }
        std::cmp::Ordering::Less => {
            solve_underdetermined_basic_f32(provider, coefficients, right_hand_side, cancellation)
        }
    }
}

fn solve_matrix_complex32(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex32>,
    right_hand_side: &DenseArray<Complex32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    let (rows, columns, right_hand_sides) =
        validate_left_division_dimensions(coefficients, right_hand_side)?;
    check_cancellation(cancellation, "matrix left division")?;
    if rows == 0 || columns == 0 || right_hand_sides == 0 {
        return zero_matrix(
            columns,
            right_hand_sides,
            Complex32::ZERO,
            "matrix left division empty complex result",
            cancellation,
        );
    }
    match rows.cmp(&columns) {
        std::cmp::Ordering::Equal => provider.solve_complex32(with_cancellation(
            SolveRequest::new(coefficients, right_hand_side),
            cancellation,
        )),
        std::cmp::Ordering::Greater => {
            provider.solve_rectangular_complex32(with_rectangular_cancellation(
                RectangularSolveRequest::new(coefficients, right_hand_side),
                cancellation,
            ))
        }
        std::cmp::Ordering::Less => solve_underdetermined_basic_complex32(
            provider,
            coefficients,
            right_hand_side,
            cancellation,
        ),
    }
}

fn solve_matrix_f64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<f64>,
    right_hand_side: &DenseArray<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<f64>, LinalgError> {
    let (rows, columns, right_hand_sides) =
        validate_left_division_dimensions(coefficients, right_hand_side)?;
    check_cancellation(cancellation, "matrix left division")?;
    if rows == 0 || columns == 0 || right_hand_sides == 0 {
        return zero_matrix(
            columns,
            right_hand_sides,
            0.0,
            "matrix left division empty real result",
            cancellation,
        );
    }
    match rows.cmp(&columns) {
        std::cmp::Ordering::Equal => provider.solve_f64(with_cancellation(
            SolveRequest::new(coefficients, right_hand_side),
            cancellation,
        )),
        std::cmp::Ordering::Greater => {
            provider.solve_rectangular_f64(with_rectangular_cancellation(
                RectangularSolveRequest::new(coefficients, right_hand_side),
                cancellation,
            ))
        }
        std::cmp::Ordering::Less => {
            solve_underdetermined_basic_f64(provider, coefficients, right_hand_side, cancellation)
        }
    }
}

fn solve_matrix_complex64(
    provider: &dyn LinalgProvider,
    coefficients: &DenseArray<Complex64>,
    right_hand_side: &DenseArray<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    let (rows, columns, right_hand_sides) =
        validate_left_division_dimensions(coefficients, right_hand_side)?;
    check_cancellation(cancellation, "matrix left division")?;
    if rows == 0 || columns == 0 || right_hand_sides == 0 {
        return zero_matrix(
            columns,
            right_hand_sides,
            Complex64::ZERO,
            "matrix left division empty complex result",
            cancellation,
        );
    }
    match rows.cmp(&columns) {
        std::cmp::Ordering::Equal => provider.solve_complex64(with_cancellation(
            SolveRequest::new(coefficients, right_hand_side),
            cancellation,
        )),
        std::cmp::Ordering::Greater => {
            provider.solve_rectangular_complex64(with_rectangular_cancellation(
                RectangularSolveRequest::new(coefficients, right_hand_side),
                cancellation,
            ))
        }
        std::cmp::Ordering::Less => solve_underdetermined_basic_complex64(
            provider,
            coefficients,
            right_hand_side,
            cancellation,
        ),
    }
}

fn validate_left_division_dimensions<T, U>(
    coefficients: &DenseArray<T>,
    right_hand_side: &DenseArray<U>,
) -> Result<(u64, u64, u64), LinalgError> {
    let (rows, columns) =
        matrix_dimensions("matrix left division coefficient matrix", coefficients)?;
    let (right_rows, right_hand_sides) =
        matrix_dimensions("matrix left division right-hand side", right_hand_side)?;
    if rows != right_rows {
        return Err(LinalgError::DimensionMismatch {
            operation: "matrix left division coefficient and right-hand-side rows",
            left: rows,
            right: right_rows,
        });
    }
    Ok((rows, columns, right_hand_sides))
}

fn validate_right_division_dimensions(
    left: &DoubleMatrix<'_>,
    right: &DoubleMatrix<'_>,
) -> Result<(), LinalgError> {
    let (_, left_columns) = left.dimensions("matrix right division left matrix")?;
    let (_, right_columns) = right.dimensions("matrix right division right matrix")?;
    validate_right_columns(left_columns, right_columns)
}

fn validate_right_division_dimensions_single(
    left: &SingleMatrix<'_>,
    right: &SingleMatrix<'_>,
) -> Result<(), LinalgError> {
    let (_, left_columns) = left.dimensions("matrix right division left matrix")?;
    let (_, right_columns) = right.dimensions("matrix right division right matrix")?;
    validate_right_columns(left_columns, right_columns)
}

fn validate_right_columns(left: u64, right: u64) -> Result<(), LinalgError> {
    if left == right {
        Ok(())
    } else {
        Err(LinalgError::DimensionMismatch {
            operation: "matrix right division operand columns",
            left,
            right,
        })
    }
}

fn with_rectangular_cancellation<'array, T>(
    request: RectangularSolveRequest<'array, T>,
    cancellation: Option<&'array AtomicBool>,
) -> RectangularSolveRequest<'array, T> {
    match cancellation {
        Some(flag) => request.with_cancellation_flag(flag),
        None => request,
    }
}

fn zero_matrix<T: Copy>(
    rows: u64,
    columns: u64,
    zero: T,
    operation: &'static str,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<T>, LinalgError> {
    check_cancellation(cancellation, operation)?;
    let result = DenseArray::from_elem(Shape::new([rows, columns])?, zero)?;
    check_cancellation(cancellation, operation)?;
    Ok(result)
}

fn transpose_real<T: Copy>(
    array: &DenseArray<T>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<T>, LinalgError> {
    transpose_map(
        array,
        "matrix right division transpose",
        cancellation,
        |value| value,
    )
}

fn conjugate_transpose_complex(
    array: &DenseArray<Complex64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    transpose_map(
        array,
        "matrix right division conjugate transpose",
        cancellation,
        Complex64::conjugate,
    )
}

fn conjugate_transpose_complex32(
    array: &DenseArray<Complex32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    transpose_map(
        array,
        "matrix right division conjugate transpose",
        cancellation,
        Complex32::conjugate,
    )
}

fn transpose_map<T: Copy, U>(
    array: &DenseArray<T>,
    operation: &'static str,
    cancellation: Option<&AtomicBool>,
    mut map: impl FnMut(T) -> U,
) -> Result<DenseArray<U>, LinalgError> {
    let (rows, columns) = matrix_dimensions(operation, array)?;
    check_cancellation(cancellation, operation)?;
    let mut output = reserved_buffer(array.as_slice().len(), operation)?;
    for output_column in 0..rows {
        for output_row in 0..columns {
            check_cancellation(cancellation, operation)?;
            let source = checked_host_index(output_column + output_row * rows, operation)?;
            output.push(map(array.as_slice()[source]));
        }
    }
    check_cancellation(cancellation, operation)?;
    DenseArray::from_vec(Shape::new([columns, rows])?, output).map_err(Into::into)
}

fn map_array<T: Copy, U>(
    array: &DenseArray<T>,
    operation: &'static str,
    cancellation: Option<&AtomicBool>,
    mut map: impl FnMut(T) -> U,
) -> Result<DenseArray<U>, LinalgError> {
    check_cancellation(cancellation, operation)?;
    let mut values = reserved_buffer(array.as_slice().len(), operation)?;
    for &value in array.as_slice() {
        check_cancellation(cancellation, operation)?;
        values.push(map(value));
    }
    check_cancellation(cancellation, operation)?;
    DenseArray::from_vec(array.shape().clone(), values).map_err(Into::into)
}

fn reserved_buffer<T>(length: usize, operation: &'static str) -> Result<Vec<T>, LinalgError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| LinalgError::AllocationFailure {
            operation,
            elements: u64::try_from(length).unwrap_or(u64::MAX),
        })?;
    Ok(values)
}

fn checked_host_index(index: u64, operation: &'static str) -> Result<usize, LinalgError> {
    usize::try_from(index).map_err(|_| LinalgError::AllocationFailure {
        operation,
        elements: index,
    })
}

fn check_cancellation(
    cancellation: Option<&AtomicBool>,
    operation: &'static str,
) -> Result<(), LinalgError> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(LinalgError::Cancelled { operation })
    } else {
        Ok(())
    }
}

fn scalar_array<T>(value: T) -> Result<DenseArray<T>, LinalgError> {
    DenseArray::from_vec(Shape::new([1, 1])?, vec![value]).map_err(Into::into)
}

fn promote_real(
    array: &DenseArray<f64>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex64>, LinalgError> {
    map_array(
        array,
        "runtime real-to-complex matrix promotion",
        cancellation,
        |value| Complex64::new(value, 0.0),
    )
}

fn promote_single_real(
    array: &DenseArray<f32>,
    cancellation: Option<&AtomicBool>,
) -> Result<DenseArray<Complex32>, LinalgError> {
    map_array(
        array,
        "runtime single real-to-complex matrix promotion",
        cancellation,
        |value| Complex32::new(value, 0.0),
    )
}
