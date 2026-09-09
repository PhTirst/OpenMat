use openmat_array::{ArrayData, DenseArray, IntegerComponent, Logical, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{SparseArrayData, Value};

use crate::{array_error, expect_argument_count, expect_max_outputs, type_error};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinitePredicate {
    Nan,
    Infinite,
    Finite,
}

impl FinitePredicate {
    const fn name(self) -> &'static str {
        match self {
            Self::Nan => "isnan",
            Self::Infinite => "isinf",
            Self::Finite => "isfinite",
        }
    }

    fn f64(self, real: f64, imaginary: f64) -> bool {
        match self {
            Self::Nan => real.is_nan() || imaginary.is_nan(),
            Self::Infinite => real.is_infinite() || imaginary.is_infinite(),
            Self::Finite => real.is_finite() && imaginary.is_finite(),
        }
    }

    fn f32(self, real: f32, imaginary: f32) -> bool {
        match self {
            Self::Nan => real.is_nan() || imaginary.is_nan(),
            Self::Infinite => real.is_infinite() || imaginary.is_infinite(),
            Self::Finite => real.is_finite() && imaginary.is_finite(),
        }
    }
}

pub(super) fn isnan_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    finite_predicate_builtin(FinitePredicate::Nan, arguments, context)
}

pub(super) fn isinf_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    finite_predicate_builtin(FinitePredicate::Infinite, arguments, context)
}

pub(super) fn isfinite_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    finite_predicate_builtin(FinitePredicate::Finite, arguments, context)
}

fn finite_predicate_builtin(
    predicate: FinitePredicate,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = predicate.name();
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let finite = matches!(predicate, FinitePredicate::Finite);
    let output = match &arguments[0] {
        Value::Logical(_) => Value::Logical(finite),
        Value::Double(value) => Value::Logical(predicate.f64(*value, 0.0)),
        Value::Complex(value) => Value::Logical(predicate.f64(value.real, value.imaginary)),
        Value::Array(ArrayData::F32(array)) => logical_map(
            name,
            array.shape().clone(),
            array
                .as_slice()
                .iter()
                .map(|value| predicate.f32(*value, 0.0)),
            context,
        )?,
        Value::Array(ArrayData::ComplexF32(array)) => logical_map(
            name,
            array.shape().clone(),
            array
                .as_slice()
                .iter()
                .map(|value| predicate.f32(value.re, value.im)),
            context,
        )?,
        Value::Array(ArrayData::F64(array)) => logical_map(
            name,
            array.shape().clone(),
            array
                .as_slice()
                .iter()
                .map(|value| predicate.f64(*value, 0.0)),
            context,
        )?,
        Value::Array(ArrayData::ComplexF64(array)) => logical_map(
            name,
            array.shape().clone(),
            array
                .as_slice()
                .iter()
                .map(|value| predicate.f64(value.re, value.im)),
            context,
        )?,
        Value::Array(ArrayData::Logical(array)) => {
            logical_fill(name, array.shape().clone(), finite, context)?
        }
        Value::Array(ArrayData::Char(array)) => {
            logical_fill(name, array.shape().clone(), finite, context)?
        }
        Value::Array(ArrayData::Integer(array)) => {
            logical_fill(name, array.shape().clone(), finite, context)?
        }
        value => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, or char array",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn logical_map(
    name: &str,
    shape: Shape,
    values: impl Iterator<Item = bool>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let length = host_length(name, &shape)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name, shape.numel()))?;
    for (index, value) in values.enumerate() {
        check_cancelled_at(context, index)?;
        output.push(Logical::from(value));
    }
    logical_output(shape, output)
}

fn logical_fill(
    name: &str,
    shape: Shape,
    value: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let length = host_length(name, &shape)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name, shape.numel()))?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        output.push(Logical::from(value));
    }
    logical_output(shape, output)
}

fn logical_output(shape: Shape, values: Vec<Logical>) -> Result<Value, BuiltinError> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn host_length(name: &str, shape: &Shape) -> Result<usize, BuiltinError> {
    usize::try_from(shape.numel()).map_err(|_| allocation_error(name, shape.numel()))
}

fn allocation_error(name: &str, length: u64) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` cannot allocate a logical array with {length} elements"),
    )
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

macro_rules! scalar_predicate_builtin {
    ($function:ident, $name:literal, $predicate:expr) => {
        pub(super) fn $function(
            arguments: &[Value],
            context: &mut BuiltinContext<'_>,
        ) -> BuiltinResult {
            expect_argument_count($name, arguments, 1)?;
            expect_max_outputs($name, context, 1)?;
            context.check_cancelled()?;
            Ok(vec![Value::Logical($predicate(&arguments[0]))])
        }
    };
}

scalar_predicate_builtin!(isreal_builtin, "isreal", is_real);
scalar_predicate_builtin!(isnumeric_builtin, "isnumeric", is_numeric);
scalar_predicate_builtin!(isfloat_builtin, "isfloat", is_float);
scalar_predicate_builtin!(isinteger_builtin, "isinteger", is_integer);
scalar_predicate_builtin!(islogical_builtin, "islogical", is_logical);
scalar_predicate_builtin!(ischar_builtin, "ischar", is_char);
scalar_predicate_builtin!(isstring_builtin, "isstring", is_string);
scalar_predicate_builtin!(iscell_builtin, "iscell", is_cell);
scalar_predicate_builtin!(isstruct_builtin, "isstruct", is_struct);
scalar_predicate_builtin!(isobject_builtin, "isobject", is_object);
scalar_predicate_builtin!(isscalar_builtin, "isscalar", Value::is_scalar);
scalar_predicate_builtin!(isvector_builtin, "isvector", is_vector);
scalar_predicate_builtin!(ismatrix_builtin, "ismatrix", is_matrix);
scalar_predicate_builtin!(isrow_builtin, "isrow", is_row);
scalar_predicate_builtin!(iscolumn_builtin, "iscolumn", is_column);

fn is_real(value: &Value) -> bool {
    match value {
        Value::Logical(_) | Value::Double(_) => true,
        Value::Array(value) => !value.is_complex(),
        Value::Sparse(value) => !value.is_complex(),
        _ => false,
    }
}

fn is_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Integer(_)
            )
            | Value::Sparse(SparseArrayData::F64(_) | SparseArrayData::ComplexF64(_))
    )
}

fn is_float(value: &Value) -> bool {
    matches!(
        value,
        Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
            )
            | Value::Sparse(SparseArrayData::F64(_) | SparseArrayData::ComplexF64(_))
    )
}

fn is_integer(value: &Value) -> bool {
    matches!(value, Value::Array(ArrayData::Integer(_)))
}

fn is_logical(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_)
            | Value::Sparse(SparseArrayData::Logical(_))
            | Value::Array(ArrayData::Logical(_))
    )
}

fn is_char(value: &Value) -> bool {
    matches!(value, Value::Array(ArrayData::Char(_)))
}

fn is_string(value: &Value) -> bool {
    matches!(value, Value::String(_))
}

fn is_cell(value: &Value) -> bool {
    matches!(value, Value::Cell(_))
}

fn is_struct(value: &Value) -> bool {
    matches!(value, Value::Struct(_))
}

fn is_object(value: &Value) -> bool {
    matches!(
        value,
        Value::Table(_)
            | Value::Object(_)
            | Value::ObjectArray(_)
            | Value::Graphics(_)
            | Value::GraphicsArray(_)
    )
}

fn is_vector(value: &Value) -> bool {
    value.dimensions().is_some_and(|dimensions| {
        dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1)
    })
}

fn is_matrix(value: &Value) -> bool {
    value
        .dimensions()
        .is_some_and(|dimensions| dimensions.len() <= 2)
}

fn is_row(value: &Value) -> bool {
    value
        .dimensions()
        .is_some_and(|dimensions| dimensions.len() == 2 && dimensions[0] == 1)
}

fn is_column(value: &Value) -> bool {
    value
        .dimensions()
        .is_some_and(|dimensions| dimensions.len() == 2 && dimensions[1] == 1)
}

pub(super) fn isequal_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    equality_builtin("isequal", arguments, context, false)
}

pub(super) fn isequaln_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    equality_builtin("isequaln", arguments, context, true)
}

fn equality_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    nan_equal: bool,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects at least 2 inputs but received {}",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    for (index, value) in arguments.iter().enumerate().skip(1) {
        check_cancelled_at(context, index)?;
        if !matlab_equal(&arguments[0], value, nan_equal, context)? {
            return Ok(vec![Value::Logical(false)]);
        }
    }
    context.check_cancelled()?;
    Ok(vec![Value::Logical(true)])
}

#[derive(Clone, Copy, Debug)]
enum ExactComponent {
    Float(f64),
    Integer(IntegerComponent),
}

#[derive(Clone, Copy, Debug)]
struct ExactNumeric {
    real: ExactComponent,
    imaginary: ExactComponent,
}

impl ExactNumeric {
    const fn real(value: ExactComponent) -> Self {
        Self {
            real: value,
            imaginary: ExactComponent::Integer(IntegerComponent::Unsigned(0)),
        }
    }
}

fn matlab_equal(
    left: &Value,
    right: &Value,
    nan_equal: bool,
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    if is_equality_numeric(left) || is_equality_numeric(right) {
        return numeric_values_equal(left, right, nan_equal, context);
    }
    if left.dimensions() != right.dimensions() || left.class_name() != right.class_name() {
        return Ok(false);
    }
    match (left, right) {
        (Value::String(left), Value::String(right)) => {
            let length = usize::try_from(left.numel())
                .map_err(|_| allocation_error("isequal", left.numel()))?;
            for index in 0..length {
                check_cancelled_at(context, index)?;
                if left.element(index) != right.element(index) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Cell(left), Value::Cell(right)) => {
            for (index, (left, right)) in left.values().iter().zip(right.values()).enumerate() {
                check_cancelled_at(context, index)?;
                if !matlab_equal(left, right, nan_equal, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Struct(left), Value::Struct(right)) => {
            if left.field_names() != right.field_names() {
                return Ok(false);
            }
            for field in 0..left.field_count() {
                let Some(left_values) = left.field_values(field) else {
                    return Ok(false);
                };
                let Some(right_values) = right.field_values(field) else {
                    return Ok(false);
                };
                for (index, (left, right)) in left_values.iter().zip(right_values).enumerate() {
                    check_cancelled_at(context, index)?;
                    if !matlab_equal(left, right, nan_equal, context)? {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        _ => Ok(left == right),
    }
}

fn is_equality_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Logical(_)
                    | ArrayData::Char(_)
                    | ArrayData::Integer(_)
            )
    )
}

fn numeric_values_equal(
    left: &Value,
    right: &Value,
    nan_equal: bool,
    context: &BuiltinContext<'_>,
) -> Result<bool, BuiltinError> {
    if !is_equality_numeric(left)
        || !is_equality_numeric(right)
        || left.dimensions() != right.dimensions()
    {
        return Ok(false);
    }
    let length = usize::try_from(left.numel().unwrap_or(0))
        .map_err(|_| allocation_error("isequal", left.numel().unwrap_or(0)))?;
    for index in 0..length {
        check_cancelled_at(context, index)?;
        let Some(left) = numeric_element(left, index) else {
            return Ok(false);
        };
        let Some(right) = numeric_element(right, index) else {
            return Ok(false);
        };
        if !components_equal(left.real, right.real, nan_equal)
            || !components_equal(left.imaginary, right.imaginary, nan_equal)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn numeric_element(value: &Value, index: usize) -> Option<ExactNumeric> {
    match value {
        Value::Logical(value) if index == 0 => Some(ExactNumeric::real(ExactComponent::Integer(
            IntegerComponent::Unsigned(u128::from(u8::from(*value))),
        ))),
        Value::Double(value) if index == 0 => {
            Some(ExactNumeric::real(ExactComponent::Float(*value)))
        }
        Value::Complex(value) if index == 0 => Some(ExactNumeric {
            real: ExactComponent::Float(value.real),
            imaginary: ExactComponent::Float(value.imaginary),
        }),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(index)
            .map(|value| ExactNumeric::real(ExactComponent::Float(f64::from(*value)))),
        Value::Array(ArrayData::ComplexF32(array)) => {
            array.as_slice().get(index).map(|value| ExactNumeric {
                real: ExactComponent::Float(f64::from(value.re)),
                imaginary: ExactComponent::Float(f64::from(value.im)),
            })
        }
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(index)
            .map(|value| ExactNumeric::real(ExactComponent::Float(*value))),
        Value::Array(ArrayData::ComplexF64(array)) => {
            array.as_slice().get(index).map(|value| ExactNumeric {
                real: ExactComponent::Float(value.re),
                imaginary: ExactComponent::Float(value.im),
            })
        }
        Value::Array(ArrayData::Logical(array)) => array.as_slice().get(index).map(|value| {
            ExactNumeric::real(ExactComponent::Integer(IntegerComponent::Unsigned(
                u128::from(u8::from(value.get())),
            )))
        }),
        Value::Array(ArrayData::Char(array)) => array.as_slice().get(index).map(|value| {
            ExactNumeric::real(ExactComponent::Integer(IntegerComponent::Unsigned(
                u128::from(value.get()),
            )))
        }),
        Value::Array(ArrayData::Integer(array)) => array.element(index).map(|value| ExactNumeric {
            real: ExactComponent::Integer(value.real_component()),
            imaginary: ExactComponent::Integer(
                value
                    .imaginary_component()
                    .unwrap_or(IntegerComponent::Unsigned(0)),
            ),
        }),
        _ => None,
    }
}

#[allow(clippy::float_cmp)]
fn components_equal(left: ExactComponent, right: ExactComponent, nan_equal: bool) -> bool {
    match (left, right) {
        (ExactComponent::Float(left), ExactComponent::Float(right)) => {
            left == right || nan_equal && left.is_nan() && right.is_nan()
        }
        (ExactComponent::Integer(left), ExactComponent::Integer(right)) => {
            integer_identity(left) == integer_identity(right)
        }
        (ExactComponent::Float(float), ExactComponent::Integer(integer))
        | (ExactComponent::Integer(integer), ExactComponent::Float(float)) => {
            float_integer_identity(float).is_some_and(|value| value == integer_identity(integer))
        }
    }
}

fn integer_identity(value: IntegerComponent) -> (bool, u128) {
    if let Some(value) = value.as_signed() {
        (value.is_negative(), value.unsigned_abs())
    } else {
        (false, value.as_unsigned().unwrap_or(0))
    }
}

fn float_integer_identity(value: f64) -> Option<(bool, u128)> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    if value >= 0.0 {
        if value >= 18_446_744_073_709_551_616.0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let magnitude = value as u64;
        Some((false, u128::from(magnitude)))
    } else {
        if value < -9_223_372_036_854_775_808.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let integer = value as i64;
        Some((true, u128::from(integer.unsigned_abs())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_cross_class_numeric_identity_does_not_round_large_integers() {
        assert!(components_equal(
            ExactComponent::Integer(IntegerComponent::Unsigned(1)),
            ExactComponent::Float(1.0),
            false
        ));
        assert!(!components_equal(
            ExactComponent::Integer(IntegerComponent::Unsigned(9_007_199_254_740_993)),
            ExactComponent::Float(9_007_199_254_740_992.0),
            false
        ));
    }

    #[test]
    fn nan_identity_is_opt_in() {
        assert!(!components_equal(
            ExactComponent::Float(f64::NAN),
            ExactComponent::Float(f64::NAN),
            false
        ));
        assert!(components_equal(
            ExactComponent::Float(f64::NAN),
            ExactComponent::Float(f64::NAN),
            true
        ));
    }
}
