use std::sync::atomic::AtomicBool;

use openmat_array::{ArrayData, Complex64 as ArrayComplex64, DenseArray, Logical, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinRegistry,
    BuiltinResult,
};
use openmat_value::{CooEntry, SparseArrayData, SparseError, StringValue, Value};

const CANCELLATION_INTERVAL: usize = 1_024;
const U64_EXCLUSIVE_UPPER_BOUND: f64 = 18_446_744_073_709_551_616.0;

#[cfg(test)]
const SPARSE_BUILTIN_NAMES: [&str; 8] = [
    "sparse", "full", "issparse", "nnz", "nonzeros", "spones", "speye", "spalloc",
];

/// Registers the first-tranche sparse constructor and query surface.
///
/// `find` already has a shared dense registration. The coordinator should
/// delegate sparse inputs from that implementation to
/// [`try_find_sparse_builtin`] instead of registering a second `find` name.
///
/// # Errors
///
/// Returns the registry's duplicate-name or handle-exhaustion failure.
pub fn register_sparse_builtins(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("sparse", sparse_builtin)?;
    registry.register("full", full_builtin)?;
    registry.register("issparse", issparse_builtin)?;
    registry.register("nnz", nnz_builtin)?;
    registry.register("nonzeros", nonzeros_builtin)?;
    registry.register("spones", spones_builtin)?;
    registry.register("speye", speye_builtin)?;
    registry.register("spalloc", spalloc_builtin)?;
    Ok(())
}

/// Implements `sparse(A)`, `sparse(m,n)`, and COO constructor forms.
pub fn sparse_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("sparse", context, 1)?;
    context.check_cancelled()?;
    let sparse = match arguments {
        [value] => sparse_from_value(value, context.cancellation_flag())?,
        [rows, columns] => SparseArrayData::try_spalloc(
            nonnegative_integer("sparse", 1, rows)?,
            nonnegative_integer("sparse", 2, columns)?,
            0,
            Some(context.cancellation_flag()),
        )
        .map_err(sparse_error)?,
        [rows, columns, values] => sparse_from_coo(
            rows,
            columns,
            values,
            None,
            None,
            context.cancellation_flag(),
        )?,
        [rows, columns, values, height, width] => sparse_from_coo(
            rows,
            columns,
            values,
            Some((height, width)),
            None,
            context.cancellation_flag(),
        )?,
        [rows, columns, values, height, width, nzmax] => sparse_from_coo(
            rows,
            columns,
            values,
            Some((height, width)),
            Some(nzmax),
            context.cancellation_flag(),
        )?,
        _ => {
            return Err(argument_count_error(
                "sparse",
                "1, 2, 3, 5, or 6",
                arguments.len(),
            ));
        }
    };
    one_output(context, Value::Sparse(sparse))
}

/// Implements dense materialization while leaving already-full supported values unchanged.
pub fn full_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("full", arguments, 1)?;
    expect_max_outputs("full", context, 1)?;
    context.check_cancelled()?;
    let value = match &arguments[0] {
        Value::Sparse(sparse) => sparse
            .try_to_dense(Some(context.cancellation_flag()))
            .map(Value::Array)
            .map_err(sparse_error)?,
        Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Array(
            ArrayData::F64(_)
            | ArrayData::ComplexF64(_)
            | ArrayData::Logical(_)
            | ArrayData::Char(_)
            | ArrayData::Integer(_),
        ) => arguments[0].clone(),
        other => return Err(type_error("full", 1, "numeric or logical value", other)),
    };
    one_output(context, value)
}

/// Implements the scalar logical `issparse` predicate for every dynamic value.
pub fn issparse_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("issparse", arguments, 1)?;
    expect_max_outputs("issparse", context, 1)?;
    context.check_cancelled()?;
    one_output(
        context,
        Value::Logical(matches!(arguments[0], Value::Sparse(_))),
    )
}

/// Counts numeric/logical nonzeros without counting implicit sparse zeros.
pub fn nnz_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("nnz", arguments, 1)?;
    expect_max_outputs("nnz", context, 1)?;
    context.check_cancelled()?;
    let count = nonzero_count(&arguments[0], context)?;
    #[allow(clippy::cast_precision_loss)]
    one_output(context, Value::Double(count as f64))
}

/// Returns nonzero values as a full column in column-major order.
pub fn nonzeros_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("nonzeros", arguments, 1)?;
    expect_max_outputs("nonzeros", context, 1)?;
    context.check_cancelled()?;
    let sparse = sparse_from_value(&arguments[0], context.cancellation_flag())?;
    let dense = sparse
        .try_nonzeros(Some(context.cancellation_flag()))
        .map_err(sparse_error)?;
    one_output(context, Value::Array(canonicalize_complex_dense(dense)?))
}

/// Returns sparse double ones at the nonzero pattern of a supported value.
pub fn spones_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("spones", arguments, 1)?;
    expect_max_outputs("spones", context, 1)?;
    context.check_cancelled()?;
    let sparse = sparse_from_value(&arguments[0], context.cancellation_flag())?;
    let ones = sparse
        .try_spones(Some(context.cancellation_flag()))
        .map_err(sparse_error)?;
    one_output(context, Value::Sparse(ones))
}

/// Constructs a sparse rectangular real-double identity.
pub fn speye_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("speye", arguments, 1, 2)?;
    expect_max_outputs("speye", context, 1)?;
    context.check_cancelled()?;
    let rows = nonnegative_integer("speye", 1, &arguments[0])?;
    let columns = if let Some(value) = arguments.get(1) {
        nonnegative_integer("speye", 2, value)?
    } else {
        rows
    };
    let value = SparseArrayData::try_speye(rows, columns, Some(context.cancellation_flag()))
        .map_err(sparse_error)?;
    one_output(context, Value::Sparse(value))
}

/// Constructs an empty sparse real-double matrix with reserved nonzero capacity.
pub fn spalloc_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("spalloc", arguments, 3)?;
    expect_max_outputs("spalloc", context, 1)?;
    context.check_cancelled()?;
    let rows = nonnegative_integer("spalloc", 1, &arguments[0])?;
    let columns = nonnegative_integer("spalloc", 2, &arguments[1])?;
    let nzmax = checked_usize(nonnegative_integer("spalloc", 3, &arguments[2])?)?;
    let value =
        SparseArrayData::try_spalloc(rows, columns, nzmax, Some(context.cancellation_flag()))
            .map_err(sparse_error)?;
    one_output(context, Value::Sparse(value))
}

/// Handles `find` only when its first input is sparse.
///
/// The `None` result means the existing dense implementation should continue.
pub fn try_find_sparse_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> Option<BuiltinResult> {
    matches!(arguments.first(), Some(Value::Sparse(_)))
        .then(|| find_sparse_builtin(arguments, context))
}

fn find_sparse_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("find", arguments, 1, 3)?;
    expect_max_outputs("find", context, 3)?;
    context.check_cancelled()?;
    let Value::Sparse(sparse) = &arguments[0] else {
        unreachable!("sparse find delegation validates the first argument")
    };
    let maximum = arguments.get(1).map(positive_find_count).transpose()?;
    let direction = arguments
        .get(2)
        .map_or(Ok(FindDirection::First), find_direction)?;
    let total = sparse.nnz();
    let selected = maximum.map_or(total, |maximum| maximum.min(total));
    let start = match direction {
        FindDirection::First => 0,
        FindDirection::Last => total - selected,
    };
    let row_output = sparse.shape().extent(0) == 1;
    let shape = if sparse.shape().dimensions() == [0, 0] {
        Shape::new([0, 0]).map_err(|_| find_index_error())?
    } else {
        index_shape(row_output, selected)?
    };
    let requested = context.requested_outputs();
    if requested == 0 {
        return Ok(Vec::new());
    }
    let mut outputs = reserved_values("find outputs", requested)?;
    if requested == 1 {
        outputs.push(find_linear_indices(
            sparse, start, selected, shape, context,
        )?);
        return Ok(outputs);
    }
    outputs.push(find_row_indices(
        sparse,
        start,
        selected,
        shape.clone(),
        context,
    )?);
    outputs.push(find_column_indices(
        sparse,
        start,
        selected,
        shape.clone(),
        context,
    )?);
    if requested == 3 {
        outputs.push(find_selected_values(
            sparse, start, selected, shape, context,
        )?);
    }
    Ok(outputs)
}

fn sparse_from_value(
    value: &Value,
    cancellation: &AtomicBool,
) -> Result<SparseArrayData, BuiltinError> {
    match value {
        Value::Sparse(sparse) => Ok(sparse.clone()),
        Value::Logical(value) => SparseArrayData::try_from_logical_coo(
            1,
            1,
            bool_entry(*value),
            usize::from(*value),
            Some(cancellation),
        )
        .map_err(sparse_error),
        Value::Double(value) => SparseArrayData::try_from_f64_coo(
            1,
            1,
            f64_entry(*value),
            usize::from(*value != 0.0),
            Some(cancellation),
        )
        .map_err(sparse_error),
        Value::Complex(value) => SparseArrayData::try_from_complex_f64_coo(
            1,
            1,
            complex_entry(ArrayComplex64::new(value.real, value.imaginary)),
            usize::from(!value.is_zero()),
            Some(cancellation),
        )
        .map_err(sparse_error),
        Value::Array(
            array @ (ArrayData::F64(_)
            | ArrayData::ComplexF64(_)
            | ArrayData::Logical(_)
            | ArrayData::Char(_)
            | ArrayData::Integer(_)),
        ) => SparseArrayData::try_from_dense(array, Some(cancellation)).map_err(sparse_error),
        other => Err(type_error(
            "sparse",
            1,
            "logical, double, or complex double matrix",
            other,
        )),
    }
}

fn sparse_from_coo(
    row_argument: &Value,
    column_argument: &Value,
    value_argument: &Value,
    explicit_shape: Option<(&Value, &Value)>,
    reserve_argument: Option<&Value>,
    cancellation: &AtomicBool,
) -> Result<SparseArrayData, BuiltinError> {
    let rows = RealVector::new("sparse", 1, row_argument)?;
    let columns = RealVector::new("sparse", 2, column_argument)?;
    let values = CooValues::new(value_argument)?;
    let length = broadcast_coo_length(rows.len(), columns.len(), values.len())?;
    let mut row_indices = reserved_values("sparse row indices", length)?;
    let mut column_indices = reserved_values("sparse column indices", length)?;
    let mut inferred_rows = 0_u64;
    let mut inferred_columns = 0_u64;
    for position in 0..length {
        check_cancelled(cancellation, position)?;
        let row = one_based_coo_index(rows.at(broadcast_offset(rows.len(), position))?, 1)?;
        let column =
            one_based_coo_index(columns.at(broadcast_offset(columns.len(), position))?, 2)?;
        inferred_rows = inferred_rows.max(row);
        inferred_columns = inferred_columns.max(column);
        row_indices.push(row - 1);
        column_indices.push(column - 1);
    }
    let (height, width) =
        explicit_shape.map_or(Ok((inferred_rows, inferred_columns)), |(height, width)| {
            Ok((
                nonnegative_integer("sparse", 4, height)?,
                nonnegative_integer("sparse", 5, width)?,
            ))
        })?;
    let reserve = if let Some(value) = reserve_argument {
        let reserve = checked_usize(nonnegative_integer("sparse", 6, value)?)?;
        if reserve < length {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 6 to `sparse` must be at least the expanded COO length",
            ));
        }
        reserve
    } else {
        let numel = height.checked_mul(width).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "sparse shape element count overflowed",
            )
        })?;
        length.min(usize::try_from(numel).unwrap_or(usize::MAX))
    };
    match values {
        CooValues::Logical(values) => {
            let mut entries = reserved_values("sparse logical COO", length)?;
            for position in 0..length {
                check_cancelled(cancellation, position)?;
                entries.push(CooEntry::new(
                    row_indices[position],
                    column_indices[position],
                    Logical::from(values.at(broadcast_offset(values.len(), position))?),
                ));
            }
            SparseArrayData::try_from_logical_coo(
                height,
                width,
                entries,
                reserve,
                Some(cancellation),
            )
            .map_err(sparse_error)
        }
        CooValues::F64(values) => {
            let mut entries = reserved_values("sparse double COO", length)?;
            for position in 0..length {
                check_cancelled(cancellation, position)?;
                entries.push(CooEntry::new(
                    row_indices[position],
                    column_indices[position],
                    values.at(broadcast_offset(values.len(), position))?,
                ));
            }
            SparseArrayData::try_from_f64_coo(height, width, entries, reserve, Some(cancellation))
                .map_err(sparse_error)
        }
        CooValues::Complex(values) => {
            let mut entries = reserved_values("sparse complex COO", length)?;
            for position in 0..length {
                check_cancelled(cancellation, position)?;
                entries.push(CooEntry::new(
                    row_indices[position],
                    column_indices[position],
                    values.at(broadcast_offset(values.len(), position))?,
                ));
            }
            SparseArrayData::try_from_complex_f64_coo(
                height,
                width,
                entries,
                reserve,
                Some(cancellation),
            )
            .map_err(sparse_error)
        }
    }
}

#[derive(Clone, Copy)]
enum RealVector<'a> {
    Scalar(f64),
    Array(&'a [f64]),
}

impl<'a> RealVector<'a> {
    fn new(name: &str, argument: usize, value: &'a Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Double(value) => Ok(Self::Scalar(*value)),
            Value::Array(ArrayData::F64(array)) => Ok(Self::Array(array.as_slice())),
            other => Err(type_error(name, argument, "real double indices", other)),
        }
    }

    const fn len(self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::Array(values) => values.len(),
        }
    }

    fn at(self, offset: usize) -> Result<f64, BuiltinError> {
        match self {
            Self::Scalar(value) if offset == 0 => Ok(value),
            Self::Array(values) => values.get(offset).copied().ok_or_else(|| {
                BuiltinError::new(BuiltinErrorCategory::Domain, "invalid sparse COO offset")
            }),
            Self::Scalar(_) => Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "invalid sparse scalar expansion offset",
            )),
        }
    }
}

enum CooValues<'a> {
    Logical(LogicalVector<'a>),
    F64(RealVector<'a>),
    Complex(ComplexVector<'a>),
}

impl<'a> CooValues<'a> {
    fn new(value: &'a Value) -> Result<Self, BuiltinError> {
        match value {
            Value::Logical(value) => Ok(Self::Logical(LogicalVector::Scalar(*value))),
            Value::Double(value) => Ok(Self::F64(RealVector::Scalar(*value))),
            Value::Complex(value) => Ok(Self::Complex(ComplexVector::Scalar(ArrayComplex64::new(
                value.real,
                value.imaginary,
            )))),
            Value::Array(ArrayData::Logical(array)) => {
                Ok(Self::Logical(LogicalVector::Array(array.as_slice())))
            }
            Value::Array(ArrayData::F64(array)) => {
                Ok(Self::F64(RealVector::Array(array.as_slice())))
            }
            Value::Array(ArrayData::ComplexF64(array)) => {
                Ok(Self::Complex(ComplexVector::Array(array.as_slice())))
            }
            other => Err(type_error(
                "sparse",
                3,
                "logical, double, or complex double values",
                other,
            )),
        }
    }

    const fn len(&self) -> usize {
        match self {
            Self::Logical(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::Complex(values) => values.len(),
        }
    }
}

#[derive(Clone, Copy)]
enum LogicalVector<'a> {
    Scalar(bool),
    Array(&'a [Logical]),
}

impl LogicalVector<'_> {
    const fn len(self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::Array(values) => values.len(),
        }
    }

    fn at(self, offset: usize) -> Result<bool, BuiltinError> {
        match self {
            Self::Scalar(value) if offset == 0 => Ok(value),
            Self::Array(values) => values.get(offset).map(|value| value.get()).ok_or_else(|| {
                BuiltinError::new(BuiltinErrorCategory::Domain, "invalid sparse COO offset")
            }),
            Self::Scalar(_) => Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "invalid sparse scalar expansion offset",
            )),
        }
    }
}

#[derive(Clone, Copy)]
enum ComplexVector<'a> {
    Scalar(ArrayComplex64),
    Array(&'a [ArrayComplex64]),
}

impl ComplexVector<'_> {
    const fn len(self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::Array(values) => values.len(),
        }
    }

    fn at(self, offset: usize) -> Result<ArrayComplex64, BuiltinError> {
        match self {
            Self::Scalar(value) if offset == 0 => Ok(value),
            Self::Array(values) => values.get(offset).copied().ok_or_else(|| {
                BuiltinError::new(BuiltinErrorCategory::Domain, "invalid sparse COO offset")
            }),
            Self::Scalar(_) => Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "invalid sparse scalar expansion offset",
            )),
        }
    }
}

fn broadcast_coo_length(rows: usize, columns: usize, values: usize) -> Result<usize, BuiltinError> {
    let length = rows.max(columns).max(values);
    if [rows, columns, values]
        .into_iter()
        .all(|candidate| candidate == length || (candidate == 1 && length != 0))
    {
        Ok(length)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`sparse` COO inputs must have equal lengths or be scalar",
        ))
    }
}

const fn broadcast_offset(length: usize, position: usize) -> usize {
    if length == 1 { 0 } else { position }
}

fn one_based_coo_index(value: f64, argument: usize) -> Result<u64, BuiltinError> {
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {argument} to `sparse` must contain positive integer indices"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn nonzero_count(value: &Value, context: &BuiltinContext<'_>) -> Result<usize, BuiltinError> {
    macro_rules! count_slice {
        ($slice:expr, $predicate:expr) => {{
            let mut count = 0_usize;
            for (index, value) in $slice.iter().enumerate() {
                check_context_cancelled(context, index)?;
                count += usize::from($predicate(value));
            }
            count
        }};
    }
    Ok(match value {
        Value::Sparse(sparse) => sparse.nnz(),
        Value::Logical(value) => usize::from(*value),
        Value::Double(value) => usize::from(*value != 0.0),
        Value::Complex(value) => usize::from(!value.is_zero()),
        Value::Array(ArrayData::Logical(array)) => {
            count_slice!(array.as_slice(), |value: &Logical| value.get())
        }
        Value::Array(ArrayData::F64(array)) => {
            count_slice!(array.as_slice(), |value: &f64| *value != 0.0)
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            count_slice!(array.as_slice(), |value: &ArrayComplex64| value.re != 0.0
                || value.im != 0.0)
        }
        other => return Err(type_error("nnz", 1, "numeric or logical value", other)),
    })
}

#[derive(Clone, Copy)]
enum FindDirection {
    First,
    Last,
}

fn positive_find_count(value: &Value) -> Result<usize, BuiltinError> {
    let value = value
        .as_real_number()
        .ok_or_else(|| type_error("find", 2, "positive integer scalar", value))?;
    if value == f64::INFINITY {
        return Ok(usize::MAX);
    }
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `find` must be a positive integer scalar",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = value as u64;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

fn find_direction(value: &Value) -> Result<FindDirection, BuiltinError> {
    let code_units = match value {
        Value::String(StringValue::Scalar(value)) if !value.is_missing() => {
            value.code_units().to_vec()
        }
        Value::String(StringValue::Array(value)) if value.numel() == 1 => value
            .as_slice()
            .first()
            .filter(|value| !value.is_missing())
            .map(|value| value.code_units().to_vec())
            .ok_or_else(find_direction_error)?,
        Value::Array(ArrayData::Char(value))
            if value.shape().ndims() == 2 && value.shape().extent(0) == 1 =>
        {
            value.as_slice().iter().map(|value| value.get()).collect()
        }
        other => {
            return Err(type_error(
                "find",
                3,
                "'first' or 'last' text scalar",
                other,
            ));
        }
    };
    match code_units.as_slice() {
        [102, 105, 114, 115, 116] => Ok(FindDirection::First),
        [108, 97, 115, 116] => Ok(FindDirection::Last),
        _ => Err(find_direction_error()),
    }
}

fn find_direction_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "input 3 to `find` must be 'first' or 'last'",
    )
}

fn find_linear_indices(
    sparse: &SparseArrayData,
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let rows = sparse.shape().extent(0);
    let entries = sparse_entries(sparse);
    let mut output = reserved_values("find linear indices", selected)?;
    for (position, (row, column)) in entries.skip(start).take(selected).enumerate() {
        check_context_cancelled(context, position)?;
        let index = row
            .checked_add(column.checked_mul(rows).ok_or_else(find_index_error)?)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(find_index_error)?;
        #[allow(clippy::cast_precision_loss)]
        output.push(index as f64);
    }
    dense_f64(shape, output)
}

fn find_row_indices(
    sparse: &SparseArrayData,
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    find_subscript_indices(sparse, start, selected, shape, context, |row, _| row + 1)
}

fn find_column_indices(
    sparse: &SparseArrayData,
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    find_subscript_indices(sparse, start, selected, shape, context, |_, column| {
        column + 1
    })
}

fn find_subscript_indices(
    sparse: &SparseArrayData,
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
    mut subscript: impl FnMut(u64, u64) -> u64,
) -> Result<Value, BuiltinError> {
    let mut output = reserved_values("find subscript indices", selected)?;
    for (position, (row, column)) in sparse_entries(sparse)
        .skip(start)
        .take(selected)
        .enumerate()
    {
        check_context_cancelled(context, position)?;
        #[allow(clippy::cast_precision_loss)]
        output.push(subscript(row, column) as f64);
    }
    dense_f64(shape, output)
}

fn find_selected_values(
    sparse: &SparseArrayData,
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match sparse {
        SparseArrayData::Logical(matrix) => {
            dense_selected(matrix.values(), start, selected, shape, context)
                .map(ArrayData::Logical)
                .map(Value::Array)
        }
        SparseArrayData::F64(matrix) => {
            dense_selected(matrix.values(), start, selected, shape, context)
                .map(ArrayData::F64)
                .map(Value::Array)
        }
        SparseArrayData::ComplexF64(matrix) => {
            let array = dense_selected(matrix.values(), start, selected, shape, context)?;
            canonicalize_complex_dense(ArrayData::ComplexF64(array)).map(Value::Array)
        }
    }
}

fn sparse_entries(sparse: &SparseArrayData) -> Box<dyn Iterator<Item = (u64, u64)> + '_> {
    match sparse {
        SparseArrayData::Logical(matrix) => {
            Box::new(matrix.entries().map(|(row, column, _)| (row, column)))
        }
        SparseArrayData::F64(matrix) => {
            Box::new(matrix.entries().map(|(row, column, _)| (row, column)))
        }
        SparseArrayData::ComplexF64(matrix) => {
            Box::new(matrix.entries().map(|(row, column, _)| (row, column)))
        }
    }
}

fn dense_selected<T: Clone>(
    values: &[T],
    start: usize,
    selected: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let mut output = reserved_values("find selected values", selected)?;
    for (position, value) in values.iter().skip(start).take(selected).enumerate() {
        check_context_cancelled(context, position)?;
        output.push(value.clone());
    }
    DenseArray::from_vec(shape, output).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("sparse output shape: {error}"),
        )
    })
}

fn canonicalize_complex_dense(array: ArrayData) -> Result<ArrayData, BuiltinError> {
    let ArrayData::ComplexF64(array) = array else {
        return Ok(array);
    };
    if array.as_slice().iter().any(|value| value.im != 0.0) {
        return Ok(ArrayData::ComplexF64(array));
    }
    let mut values = reserved_values("real selected values", array.as_slice().len())?;
    values.extend(array.as_slice().iter().map(|value| value.re));
    DenseArray::from_vec(array.shape().clone(), values)
        .map(ArrayData::F64)
        .map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("sparse output shape: {error}"),
            )
        })
}

fn dense_f64(shape: Shape, values: Vec<f64>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("sparse output shape: {error}"),
            )
        })
}

fn index_shape(row: bool, length: usize) -> Result<Shape, BuiltinError> {
    let length = u64::try_from(length).map_err(|_| find_index_error())?;
    Shape::new(if row { [1, length] } else { [length, 1] }).map_err(|_| find_index_error())
}

fn find_index_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`find` sparse index computation overflowed",
    )
}

fn nonnegative_integer(name: &str, argument: usize, value: &Value) -> Result<u64, BuiltinError> {
    let number = value
        .as_real_number()
        .ok_or_else(|| type_error(name, argument, "nonnegative integer scalar", value))?;
    if !number.is_finite()
        || number < 0.0
        || number.fract() != 0.0
        || number >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {argument} to `{name}` must be a nonnegative integer scalar"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u64)
}

fn bool_entry(value: bool) -> Vec<CooEntry<Logical>> {
    if value {
        vec![CooEntry::new(0, 0, Logical::TRUE)]
    } else {
        Vec::new()
    }
}

fn f64_entry(value: f64) -> Vec<CooEntry<f64>> {
    if value == 0.0 {
        Vec::new()
    } else {
        vec![CooEntry::new(0, 0, value)]
    }
}

fn complex_entry(value: ArrayComplex64) -> Vec<CooEntry<ArrayComplex64>> {
    if value.re == 0.0 && value.im == 0.0 {
        Vec::new()
    } else {
        vec![CooEntry::new(0, 0, value)]
    }
}

#[allow(clippy::unnecessary_wraps)]
fn one_output(context: &BuiltinContext<'_>, value: Value) -> BuiltinResult {
    if context.requested_outputs() == 0 {
        Ok(Vec::new())
    } else {
        Ok(vec![value])
    }
}

fn expect_argument_count(
    name: &str,
    arguments: &[Value],
    expected: usize,
) -> Result<(), BuiltinError> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(argument_count_error(
            name,
            &expected.to_string(),
            arguments.len(),
        ))
    }
}

fn expect_argument_count_range(
    name: &str,
    arguments: &[Value],
    minimum: usize,
    maximum: usize,
) -> Result<(), BuiltinError> {
    if (minimum..=maximum).contains(&arguments.len()) {
        Ok(())
    } else {
        Err(argument_count_error(
            name,
            &format!("{minimum} to {maximum}"),
            arguments.len(),
        ))
    }
}

fn expect_max_outputs(
    name: &str,
    context: &BuiltinContext<'_>,
    maximum: usize,
) -> Result<(), BuiltinError> {
    if context.requested_outputs() <= maximum {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!("`{name}` supports at most {maximum} outputs"),
        ))
    }
}

fn argument_count_error(name: &str, expected: &str, actual: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::ArgumentCount,
        format!("`{name}` expects {expected} inputs, received {actual}"),
    )
}

fn type_error(name: &str, argument: usize, expected: &str, actual: &Value) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!(
            "input {argument} to `{name}` must be {expected}, found {}",
            actual.kind()
        ),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn sparse_error(error: SparseError) -> BuiltinError {
    let category = if error == SparseError::Cancelled {
        BuiltinErrorCategory::Cancelled
    } else {
        BuiltinErrorCategory::Domain
    };
    BuiltinError::new(category, error.to_string())
}

fn checked_usize(value: u64) -> Result<usize, BuiltinError> {
    usize::try_from(value).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "sparse storage length does not fit this host",
        )
    })
}

fn reserved_values<T>(name: &'static str, capacity: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("cannot allocate {capacity} elements for {name}"),
        )
    })?;
    Ok(values)
}

fn check_context_cancelled(
    context: &BuiltinContext<'_>,
    progress: usize,
) -> Result<(), BuiltinError> {
    if progress.is_multiple_of(CANCELLATION_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn check_cancelled(cancellation: &AtomicBool, progress: usize) -> Result<(), BuiltinError> {
    use std::sync::atomic::Ordering;
    if progress.is_multiple_of(CANCELLATION_INTERVAL) && cancellation.load(Ordering::Acquire) {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Cancelled,
            "execution was cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use openmat_runtime::{CancellationToken, NullOutput};

    use super::*;

    fn invoke(
        function: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        outputs: usize,
    ) -> Result<Vec<Value>, BuiltinError> {
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context = BuiltinContext::new(outputs, &cancellation, &mut output);
        function(arguments, &mut context)
    }

    fn f64_array(shape: [u64; 2], values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(shape).unwrap(), values).unwrap(),
        ))
    }

    #[test]
    fn registration_surface_excludes_shared_find_name() {
        let mut registry = BuiltinRegistry::new();
        register_sparse_builtins(&mut registry).unwrap();
        assert_eq!(registry.len(), SPARSE_BUILTIN_NAMES.len());
        for name in SPARSE_BUILTIN_NAMES {
            assert!(registry.handle_by_name(name).is_some());
        }
        assert!(registry.handle_by_name("find").is_none());
    }

    #[test]
    fn coo_constructor_merges_duplicates_and_keeps_nan() {
        let result = invoke(
            sparse_builtin,
            &[
                f64_array([1, 5], vec![2.0, 1.0, 2.0, 3.0, 3.0]),
                f64_array([1, 5], vec![1.0, 1.0, 1.0, 2.0, 2.0]),
                f64_array(
                    [1, 5],
                    vec![4.0, 5.0, -4.0, f64::INFINITY, f64::NEG_INFINITY],
                ),
                Value::Double(3.0),
                Value::Double(2.0),
            ],
            1,
        )
        .unwrap();
        let Value::Sparse(SparseArrayData::F64(matrix)) = &result[0] else {
            panic!("real sparse result")
        };
        assert_eq!(matrix.row_indices(), &[0, 2]);
        assert!((matrix.values()[0] - 5.0).abs() < f64::EPSILON);
        assert!(matrix.values()[1].is_nan());

        let collapsed = invoke(
            sparse_builtin,
            &[
                f64_array([1, 3], vec![1.0, 1.0, 1.0]),
                f64_array([1, 3], vec![1.0, 1.0, 1.0]),
                f64_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(1.0),
                Value::Double(1.0),
            ],
            1,
        )
        .unwrap();
        assert_eq!(collapsed[0].as_sparse().unwrap().nzmax(), 1);

        let reserve_error = invoke(
            sparse_builtin,
            &[
                f64_array([1, 3], vec![1.0, 1.0, 1.0]),
                f64_array([1, 3], vec![1.0, 1.0, 1.0]),
                f64_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(1.0),
                Value::Double(1.0),
                Value::Double(2.0),
            ],
            1,
        )
        .unwrap_err();
        assert_eq!(reserve_error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn logical_full_nonzeros_spones_and_metadata_preserve_classes() {
        let logical = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([2, 2]).unwrap(),
                vec![Logical::FALSE, Logical::TRUE, Logical::TRUE, Logical::FALSE],
            )
            .unwrap(),
        ));
        let sparse = invoke(sparse_builtin, std::slice::from_ref(&logical), 1)
            .unwrap()
            .remove(0);
        assert_eq!(sparse.class_name(), "logical");
        assert_eq!(sparse.dimensions(), Some([2, 2].as_slice()));
        assert_eq!(
            invoke(issparse_builtin, std::slice::from_ref(&sparse), 1).unwrap(),
            vec![Value::Logical(true)]
        );
        assert_eq!(
            invoke(nnz_builtin, std::slice::from_ref(&sparse), 1).unwrap(),
            vec![Value::Double(2.0)]
        );
        let full = invoke(full_builtin, std::slice::from_ref(&sparse), 1).unwrap();
        assert_eq!(full, vec![logical]);
        let nonzeros = invoke(nonzeros_builtin, std::slice::from_ref(&sparse), 1).unwrap();
        assert_eq!(nonzeros[0].class_name(), "logical");
        let ones = invoke(spones_builtin, &[sparse], 1).unwrap();
        assert_eq!(ones[0].class_name(), "double");
        assert!(matches!(ones[0], Value::Sparse(_)));
    }

    #[test]
    fn rectangular_empty_speye_and_spalloc_are_checked() {
        let empty = invoke(sparse_builtin, &[Value::Double(0.0), Value::Double(3.0)], 1).unwrap();
        assert_eq!(empty[0].dimensions(), Some([0, 3].as_slice()));
        let eye = invoke(speye_builtin, &[Value::Double(2.0), Value::Double(4.0)], 1).unwrap();
        assert_eq!(eye[0].as_sparse().unwrap().nnz(), 2);
        let allocation = invoke(
            spalloc_builtin,
            &[Value::Double(4.0), Value::Double(5.0), Value::Double(9.0)],
            1,
        )
        .unwrap();
        let sparse = allocation[0].as_sparse().unwrap();
        assert_eq!(sparse.nzmax(), 9);
        assert_eq!(sparse.payload_bytes(), Some(192));
        let error = invoke(
            spalloc_builtin,
            &[Value::Double(-1.0), Value::Double(2.0), Value::Double(1.0)],
            1,
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn sparse_find_keeps_row_orientation_and_selected_value_class() {
        let sparse = invoke(
            sparse_builtin,
            &[f64_array([1, 4], vec![0.0, 2.0, 0.0, 4.0])],
            1,
        )
        .unwrap()
        .remove(0);
        let cancellation = CancellationToken::new();
        let mut output = NullOutput;
        let mut context = BuiltinContext::new(3, &cancellation, &mut output);
        let result = try_find_sparse_builtin(
            &[sparse, Value::Double(1.0), Value::from("last")],
            &mut context,
        )
        .unwrap()
        .unwrap();
        assert_eq!(result[0].dimensions(), Some([1, 1].as_slice()));
        assert_eq!(result[0].as_real_number(), Some(1.0));
        assert_eq!(result[1].as_real_number(), Some(4.0));
        assert_eq!(result[2].as_real_number(), Some(4.0));

        let empty = invoke(sparse_builtin, &[Value::Double(0.0), Value::Double(0.0)], 1)
            .unwrap()
            .remove(0);
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        let empty_find = try_find_sparse_builtin(&[empty], &mut context)
            .unwrap()
            .unwrap();
        assert_eq!(empty_find[0].dimensions(), Some([0, 0].as_slice()));

        let unlimited = invoke(
            sparse_builtin,
            &[f64_array([1, 4], vec![0.0, 2.0, 0.0, 4.0])],
            1,
        )
        .unwrap()
        .remove(0);
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        let unlimited =
            try_find_sparse_builtin(&[unlimited, Value::Double(f64::INFINITY)], &mut context)
                .unwrap()
                .unwrap();
        assert_eq!(unlimited[0].dimensions(), Some([1, 2].as_slice()));
    }

    #[test]
    fn cancellation_is_reported_before_large_sparse_work() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut output = NullOutput;
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        let error = speye_builtin(&[Value::Double(100_000.0)], &mut context).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
    }
}
