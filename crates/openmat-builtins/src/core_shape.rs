use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, ComplexInteger, DType,
    DenseArray, IntegerArrayData, IntegerComponent, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{SparseError, StringArray, StringValue, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count,
    expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

pub(super) fn ndims_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("ndims", arguments, 1)?;
    expect_max_outputs("ndims", context, 1)?;
    context.check_cancelled()?;
    let dimensions = shape_dimensions("ndims", &arguments[0])?;
    let count = u64::try_from(dimensions.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "dimension count does not fit the runtime shape model",
        )
    })?;
    Ok(vec![dimension_value(count)])
}

pub(super) fn length_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("length", arguments, 1)?;
    expect_max_outputs("length", context, 1)?;
    context.check_cancelled()?;
    let dimensions = shape_dimensions("length", &arguments[0])?;
    let numel = arguments[0]
        .numel()
        .ok_or_else(|| type_error("length", 1, "value with shape semantics", &arguments[0]))?;
    let length = if numel == 0 {
        0
    } else {
        dimensions.iter().copied().max().unwrap_or(0)
    };
    Ok(vec![dimension_value(length)])
}

pub(super) fn isempty_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("isempty", arguments, 1)?;
    expect_max_outputs("isempty", context, 1)?;
    context.check_cancelled()?;
    let numel = arguments[0]
        .numel()
        .ok_or_else(|| type_error("isempty", 1, "value with shape semantics", &arguments[0]))?;
    Ok(vec![Value::Logical(numel == 0)])
}

pub(super) fn zeros_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    filled_array_builtin("zeros", arguments, context, false)
}

pub(super) fn ones_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    filled_array_builtin("ones", arguments, context, true)
}

pub(super) fn true_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    logical_array_builtin("true", arguments, context, true)
}

pub(super) fn false_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    logical_array_builtin("false", arguments, context, false)
}

fn logical_array_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    element: bool,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let (dimension_arguments, dtype) = constructor_storage(name, arguments, DType::Logical)?;
    if dtype != DType::Logical {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` only supports logical output storage"),
        ));
    }
    let dimensions = constructor_dimensions(name, dimension_arguments, context)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    filled_constructor_value(name, shape, DType::Logical, element, context).map(|value| vec![value])
}

fn filled_array_builtin(
    name: &str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    element: bool,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let (dimension_arguments, dtype) = constructor_storage(name, arguments, DType::F64)?;
    let dimensions = constructor_dimensions(name, dimension_arguments, context)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    filled_constructor_value(name, shape, dtype, element, context).map(|value| vec![value])
}

pub(super) fn eye_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("eye", context, 1)?;
    context.check_cancelled()?;
    let (dimension_arguments, dtype) = constructor_storage("eye", arguments, DType::F64)?;
    let dimensions = constructor_dimensions("eye", dimension_arguments, context)?;
    if dimensions.len() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`eye` supports exactly two dimensions; N-D identity arrays are not defined",
        ));
    }
    let rows = dimensions[0];
    let columns = dimensions[1];
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let mut diagonal_mask = filled_vec("eye", shape.numel(), false, context)?;
    let diagonal = rows.min(columns);
    let step = rows.checked_add(1).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`eye` diagonal stride overflowed",
        )
    })?;
    for index in 0..diagonal {
        let host_index = usize::try_from(index).map_err(|_| host_length_error("eye", diagonal))?;
        check_cancelled_at(context, host_index)?;
        let offset = index.checked_mul(step).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`eye` diagonal offset overflowed",
            )
        })?;
        let offset =
            usize::try_from(offset).map_err(|_| host_length_error("eye", shape.numel()))?;
        let slot = diagonal_mask.get_mut(offset).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`eye` diagonal offset escaped the allocated array",
            )
        })?;
        *slot = true;
    }
    context.check_cancelled()?;
    constructor_value_from_mask("eye", shape, dtype, &diagonal_mask, context)
        .map(|value| vec![value])
}

fn constructor_storage<'a>(
    name: &str,
    arguments: &'a [Value],
    default: DType,
) -> Result<(&'a [Value], DType), BuiltinError> {
    if arguments.len() >= 2 && text_equals(&arguments[arguments.len() - 2], "like") {
        let prototype = arguments.last().expect("length was checked");
        let dtype = prototype_dtype(prototype).ok_or_else(|| {
            type_error(
                name,
                arguments.len(),
                "numeric or logical prototype after 'like'",
                prototype,
            )
        })?;
        return Ok((&arguments[..arguments.len() - 2], dtype));
    }
    let Some(last) = arguments.last() else {
        return Ok((arguments, default));
    };
    let Some(class_name) = text_code_units(last) else {
        return Ok((arguments, default));
    };
    let dtype = class_dtype(&class_name).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "input {} to `{name}` is not a supported numeric class name",
                arguments.len()
            ),
        )
    })?;
    Ok((&arguments[..arguments.len() - 1], dtype))
}

fn prototype_dtype(value: &Value) -> Option<DType> {
    match value {
        Value::Logical(_) => Some(DType::Logical),
        Value::Double(_) => Some(DType::F64),
        Value::Complex(_) => Some(DType::ComplexF64),
        Value::Array(value) if value.dtype() != DType::Char => Some(value.dtype()),
        _ => None,
    }
}

fn class_dtype(class_name: &[u16]) -> Option<DType> {
    match class_name {
        [100, 111, 117, 98, 108, 101] => Some(DType::F64),
        [115, 105, 110, 103, 108, 101] => Some(DType::F32),
        [108, 111, 103, 105, 99, 97, 108] => Some(DType::Logical),
        [105, 110, 116, 56] => Some(DType::I8),
        [117, 105, 110, 116, 56] => Some(DType::U8),
        [105, 110, 116, 49, 54] => Some(DType::I16),
        [117, 105, 110, 116, 49, 54] => Some(DType::U16),
        [105, 110, 116, 51, 50] => Some(DType::I32),
        [117, 105, 110, 116, 51, 50] => Some(DType::U32),
        [105, 110, 116, 54, 52] => Some(DType::I64),
        [117, 105, 110, 116, 54, 52] => Some(DType::U64),
        _ => None,
    }
}

fn text_equals(value: &Value, expected: &str) -> bool {
    text_code_units(value).is_some_and(|value| value.iter().copied().eq(expected.encode_utf16()))
}

fn text_code_units(value: &Value) -> Option<Vec<u16>> {
    match value {
        Value::String(value) => {
            let element = value.as_scalar()?;
            (!element.is_missing()).then(|| element.code_units().to_vec())
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            Some(array.as_slice().iter().map(|value| value.get()).collect())
        }
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn filled_constructor_value(
    name: &str,
    shape: Shape,
    dtype: DType,
    element: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! dense_fill {
        ($element:expr, $wrap:expr) => {{
            let values = filled_vec(name, shape.numel(), $element, context)?;
            let array = DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?;
            Ok($wrap(array))
        }};
    }
    macro_rules! integer_fill {
        ($component:ty, $variant:ident) => {
            dense_fill!(
                if element {
                    1 as $component
                } else {
                    0 as $component
                },
                |array| Value::Array(ArrayData::Integer(IntegerArrayData::$variant(array)))
            )
        };
    }
    macro_rules! complex_integer_fill {
        ($component:ty, $variant:ident) => {
            dense_fill!(
                ComplexInteger::new(
                    if element {
                        1 as $component
                    } else {
                        0 as $component
                    },
                    0 as $component,
                ),
                |array| Value::Array(ArrayData::Integer(IntegerArrayData::$variant(array)))
            )
        };
    }
    match dtype {
        DType::F32 => dense_fill!(if element { 1.0_f32 } else { 0.0 }, |array| {
            Value::Array(ArrayData::F32(array))
        }),
        DType::ComplexF32 => dense_fill!(
            ArrayComplex32::new(if element { 1.0 } else { 0.0 }, 0.0),
            |array| Value::Array(ArrayData::ComplexF32(array))
        ),
        DType::F64 => dense_fill!(if element { 1.0_f64 } else { 0.0 }, |array| {
            Value::Array(ArrayData::F64(array))
        }),
        DType::ComplexF64 => dense_fill!(
            ArrayComplex64::new(if element { 1.0 } else { 0.0 }, 0.0),
            |array| Value::Array(ArrayData::ComplexF64(array))
        ),
        DType::Logical => dense_fill!(Logical::from(element), |array| {
            Value::Array(ArrayData::Logical(array))
        }),
        DType::Char => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` does not support char output storage"),
        )),
        DType::I8 => integer_fill!(i8, I8),
        DType::ComplexI8 => complex_integer_fill!(i8, ComplexI8),
        DType::U8 => integer_fill!(u8, U8),
        DType::ComplexU8 => complex_integer_fill!(u8, ComplexU8),
        DType::I16 => integer_fill!(i16, I16),
        DType::ComplexI16 => complex_integer_fill!(i16, ComplexI16),
        DType::U16 => integer_fill!(u16, U16),
        DType::ComplexU16 => complex_integer_fill!(u16, ComplexU16),
        DType::I32 => integer_fill!(i32, I32),
        DType::ComplexI32 => complex_integer_fill!(i32, ComplexI32),
        DType::U32 => integer_fill!(u32, U32),
        DType::ComplexU32 => complex_integer_fill!(u32, ComplexU32),
        DType::I64 => integer_fill!(i64, I64),
        DType::ComplexI64 => complex_integer_fill!(i64, ComplexI64),
        DType::U64 => integer_fill!(u64, U64),
        DType::ComplexU64 => complex_integer_fill!(u64, ComplexU64),
    }
}

#[allow(clippy::too_many_lines)]
fn constructor_value_from_mask(
    name: &str,
    shape: Shape,
    dtype: DType,
    mask: &[bool],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! dense {
        ($element:expr, $wrap:expr) => {{
            let mut values = Vec::new();
            values
                .try_reserve_exact(mask.len())
                .map_err(|_| allocation_error(name, shape.numel()))?;
            for (index, enabled) in mask.iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                values.push($element(enabled));
            }
            let array = DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))?;
            Ok($wrap(array))
        }};
    }
    macro_rules! integer {
        ($component:ty, $variant:ident) => {
            dense!(
                |enabled| if enabled {
                    1 as $component
                } else {
                    0 as $component
                },
                |array| Value::Array(ArrayData::Integer(IntegerArrayData::$variant(array)))
            )
        };
    }
    macro_rules! complex_integer {
        ($component:ty, $variant:ident) => {
            dense!(
                |enabled| ComplexInteger::new(
                    if enabled {
                        1 as $component
                    } else {
                        0 as $component
                    },
                    0 as $component,
                ),
                |array| Value::Array(ArrayData::Integer(IntegerArrayData::$variant(array)))
            )
        };
    }
    match dtype {
        DType::F32 => dense!(|enabled| if enabled { 1.0_f32 } else { 0.0 }, |array| {
            Value::Array(ArrayData::F32(array))
        }),
        DType::ComplexF32 => dense!(
            |enabled| ArrayComplex32::new(if enabled { 1.0 } else { 0.0 }, 0.0),
            |array| Value::Array(ArrayData::ComplexF32(array))
        ),
        DType::F64 => dense!(|enabled| if enabled { 1.0_f64 } else { 0.0 }, |array| {
            Value::Array(ArrayData::F64(array))
        }),
        DType::ComplexF64 => dense!(
            |enabled| ArrayComplex64::new(if enabled { 1.0 } else { 0.0 }, 0.0),
            |array| Value::Array(ArrayData::ComplexF64(array))
        ),
        DType::Logical => dense!(Logical::from, |array| {
            Value::Array(ArrayData::Logical(array))
        }),
        DType::Char => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` does not support char output storage"),
        )),
        DType::I8 => integer!(i8, I8),
        DType::ComplexI8 => complex_integer!(i8, ComplexI8),
        DType::U8 => integer!(u8, U8),
        DType::ComplexU8 => complex_integer!(u8, ComplexU8),
        DType::I16 => integer!(i16, I16),
        DType::ComplexI16 => complex_integer!(i16, ComplexI16),
        DType::U16 => integer!(u16, U16),
        DType::ComplexU16 => complex_integer!(u16, ComplexU16),
        DType::I32 => integer!(i32, I32),
        DType::ComplexI32 => complex_integer!(i32, ComplexI32),
        DType::U32 => integer!(u32, U32),
        DType::ComplexU32 => complex_integer!(u32, ComplexU32),
        DType::I64 => integer!(i64, I64),
        DType::ComplexI64 => complex_integer!(i64, ComplexI64),
        DType::U64 => integer!(u64, U64),
        DType::ComplexU64 => complex_integer!(u64, ComplexU64),
    }
}

pub(super) fn reshape_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `reshape` expects at least 2 inputs but received {}",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("reshape", context, 1)?;
    context.check_cancelled()?;
    let source_numel = reshape_numel(&arguments[0])?;
    let mut dimensions = reshape_dimensions(&arguments[1..], source_numel, context)?;
    let shape = Shape::new(dimensions.drain(..)).map_err(|error| array_error(&error))?;
    if shape.numel() != source_numel {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "`reshape` cannot change element count from {source_numel} to {}",
                shape.numel()
            ),
        ));
    }
    let result = reshape_value(&arguments[0], shape, context)?;
    context.check_cancelled()?;
    Ok(vec![result])
}

pub(super) fn constructor_dimensions(
    name: &str,
    arguments: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    match arguments {
        [] => Ok(vec![1, 1]),
        [single] => {
            let values = dimension_vector(name, 1, single, context)?;
            match values.as_slice() {
                [] => Ok(vec![0, 0]),
                [extent] => Ok(vec![*extent, *extent]),
                _ => Ok(values),
            }
        }
        _ => {
            let mut dimensions = reserved_dimensions(name, arguments.len())?;
            for (index, value) in arguments.iter().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(nonnegative_integer_dimension(name, index + 1, value)?);
            }
            Ok(dimensions)
        }
    }
}

fn reshape_dimensions(
    arguments: &[Value],
    numel: u64,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let (mut dimensions, inferred) = if arguments.len() == 1 {
        let dimensions = dimension_vector("reshape", 2, &arguments[0], context)?;
        if dimensions.len() < 2 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "the `reshape` size vector must contain at least two dimensions",
            ));
        }
        (dimensions, None)
    } else {
        let mut dimensions = reserved_dimensions("reshape", arguments.len())?;
        let mut inferred = None;
        for (index, value) in arguments.iter().enumerate() {
            check_cancelled_at(context, index)?;
            if is_empty_numeric(value) {
                if inferred.replace(index).is_some() {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "`reshape` accepts at most one inferred empty dimension",
                    ));
                }
                dimensions.push(1);
            } else {
                dimensions.push(nonnegative_integer_dimension("reshape", index + 2, value)?);
            }
        }
        (dimensions, inferred)
    };

    let mut known_product = 1_u64;
    for (index, extent) in dimensions.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        if Some(index) != inferred {
            known_product = known_product.checked_mul(extent).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`reshape` dimension product overflowed",
                )
            })?;
        }
    }
    if let Some(index) = inferred {
        if known_product == 0 || !numel.is_multiple_of(known_product) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`reshape` cannot infer an integral dimension from the requested shape",
            ));
        }
        dimensions[index] = numel / known_product;
    }
    Ok(dimensions)
}

fn reshape_value(
    value: &Value,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if value.dimensions() == Some(shape.dimensions()) {
        return Ok(value.clone());
    }
    match value {
        Value::Logical(element) => make_array(shape, [Logical::from(*element)].as_slice(), context)
            .map(|array| Value::Array(ArrayData::Logical(array))),
        Value::Double(element) => make_array(shape, [*element].as_slice(), context)
            .map(|array| Value::Array(ArrayData::F64(array))),
        Value::Complex(element) => {
            let element = ArrayComplex64::from(*element);
            make_array(shape, [element].as_slice(), context)
                .map(|array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => make_array(shape, array.as_slice(), context)
            .map(ArrayData::F32)
            .map(Value::Array),
        Value::Array(ArrayData::ComplexF32(array)) => make_array(shape, array.as_slice(), context)
            .map(ArrayData::ComplexF32)
            .map(Value::Array),
        Value::Array(ArrayData::F64(array)) => make_array(shape, array.as_slice(), context)
            .map(|array| Value::Array(ArrayData::F64(array))),
        Value::Array(ArrayData::ComplexF64(array)) => make_array(shape, array.as_slice(), context)
            .map(|array| Value::Array(ArrayData::ComplexF64(array))),
        Value::Array(ArrayData::Logical(array)) => make_array(shape, array.as_slice(), context)
            .map(|array| Value::Array(ArrayData::Logical(array))),
        Value::Array(ArrayData::Char(array)) => make_array(shape, array.as_slice(), context)
            .map(|array| Value::Array(ArrayData::Char(array))),
        Value::Array(ArrayData::Integer(integer)) => reshape_integer(integer, shape, context)
            .map(|integer| Value::Array(ArrayData::Integer(integer))),
        Value::String(StringValue::Scalar(element)) => {
            let array = StringArray::from_elements(shape, vec![element.clone()])
                .map_err(|error| array_error(&error))?;
            Ok(Value::String(StringValue::Array(array)))
        }
        Value::String(StringValue::Array(array)) => {
            let values = checked_clone_elements("reshape", array.as_slice(), context)?;
            StringArray::from_elements(shape, values)
                .map(StringValue::Array)
                .map(Value::String)
                .map_err(|error| array_error(&error))
        }
        Value::Sparse(sparse) => sparse
            .try_reshape_2d(
                shape.extent(0),
                shape.extent(1),
                Some(context.cancellation_flag()),
            )
            .map(Value::Sparse)
            .map_err(sparse_reshape_error),
        Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Nothing
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => Err(type_error(
            "reshape",
            1,
            "numeric, logical, char, integer, or string array",
            value,
        )),
    }
}

fn sparse_reshape_error(error: SparseError) -> BuiltinError {
    match error {
        SparseError::Cancelled => BuiltinError::new(
            BuiltinErrorCategory::Cancelled,
            SparseError::Cancelled.to_string(),
        ),
        other => BuiltinError::new(BuiltinErrorCategory::Domain, other.to_string()),
    }
}

fn reshape_integer(
    integer: &IntegerArrayData,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! reshape_variant {
        ($array:expr, $variant:ident) => {
            make_array(shape, $array.as_slice(), context).map(IntegerArrayData::$variant)
        };
    }
    match integer {
        IntegerArrayData::I8(array) => reshape_variant!(array, I8),
        IntegerArrayData::ComplexI8(array) => reshape_variant!(array, ComplexI8),
        IntegerArrayData::U8(array) => reshape_variant!(array, U8),
        IntegerArrayData::ComplexU8(array) => reshape_variant!(array, ComplexU8),
        IntegerArrayData::I16(array) => reshape_variant!(array, I16),
        IntegerArrayData::ComplexI16(array) => reshape_variant!(array, ComplexI16),
        IntegerArrayData::U16(array) => reshape_variant!(array, U16),
        IntegerArrayData::ComplexU16(array) => reshape_variant!(array, ComplexU16),
        IntegerArrayData::I32(array) => reshape_variant!(array, I32),
        IntegerArrayData::ComplexI32(array) => reshape_variant!(array, ComplexI32),
        IntegerArrayData::U32(array) => reshape_variant!(array, U32),
        IntegerArrayData::ComplexU32(array) => reshape_variant!(array, ComplexU32),
        IntegerArrayData::I64(array) => reshape_variant!(array, I64),
        IntegerArrayData::ComplexI64(array) => reshape_variant!(array, ComplexI64),
        IntegerArrayData::U64(array) => reshape_variant!(array, U64),
        IntegerArrayData::ComplexU64(array) => reshape_variant!(array, ComplexU64),
    }
}

fn checked_clone_elements<T: Clone>(
    name: &str,
    source: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(source.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "`{name}` cannot allocate storage for {} elements",
                source.len()
            ),
        )
    })?;
    for chunk in source.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        values.extend_from_slice(chunk);
    }
    Ok(values)
}

fn make_array<T: Clone>(
    shape: Shape,
    source: &[T],
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let mut data = Vec::new();
    data.try_reserve_exact(source.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "`reshape` cannot allocate storage for {} elements",
                source.len()
            ),
        )
    })?;
    for chunk in source.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        data.extend_from_slice(chunk);
    }
    DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))
}

fn dimension_vector(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    if let Some(value) = value.as_real_number() {
        return checked_dimension_value(name, position, value).map(|value| vec![value]);
    }
    let Value::Array(array) = value else {
        return Err(type_error(
            name,
            position,
            "real size scalar or vector",
            value,
        ));
    };
    let shape = array.shape();
    if array.numel() != 0 && (shape.ndims() != 2 || (shape.extent(0) != 1 && shape.extent(1) != 1))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a size vector"),
        ));
    }
    let dimension_count =
        usize::try_from(array.numel()).map_err(|_| host_length_error(name, array.numel()))?;
    let mut dimensions = reserved_dimensions(name, dimension_count)?;
    match array {
        ArrayData::F64(array) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_dimension_value(name, position, *value)?);
            }
        }
        ArrayData::ComplexF64(array) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                if value.im == 0.0 {
                    dimensions.push(checked_dimension_value(name, position, value.re)?);
                } else {
                    return Err(domain_dimension_error(name, position));
                }
            }
        }
        ArrayData::Logical(array) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(u64::from(value.get()));
            }
        }
        ArrayData::Integer(integer) if integer.is_complex() => {
            return Err(domain_dimension_error(name, position));
        }
        ArrayData::Integer(integer) => {
            for (index, value) in integer.elements().enumerate() {
                check_cancelled_at(context, index)?;
                dimensions.push(checked_integer_dimension_component(
                    name,
                    position,
                    value.real_component(),
                )?);
            }
        }
        ArrayData::Char(_) => return Err(domain_dimension_error(name, position)),
        ArrayData::F32(_) | ArrayData::ComplexF32(_) => {
            return Err(type_error(
                name,
                position,
                "real size scalar or vector",
                value,
            ));
        }
    }
    Ok(dimensions)
}

fn nonnegative_integer_dimension(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<u64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return checked_integer_dimension_component(name, position, value);
    }
    let Some(value) = value.as_real_number() else {
        return Err(type_error(
            name,
            position,
            "nonnegative integer size scalar",
            value,
        ));
    };
    checked_dimension_value(name, position, value)
}

fn checked_integer_dimension_component(
    name: &str,
    position: usize,
    value: IntegerComponent,
) -> Result<u64, BuiltinError> {
    match value {
        IntegerComponent::Signed(value) => {
            u64::try_from(value).map_err(|_| domain_dimension_error(name, position))
        }
        IntegerComponent::Unsigned(value) => {
            u64::try_from(value).map_err(|_| domain_dimension_error(name, position))
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn checked_dimension_value(name: &str, position: usize, value: f64) -> Result<u64, BuiltinError> {
    if !value.is_finite()
        || value < 0.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(domain_dimension_error(name, position));
    }
    Ok(value as u64)
}

fn domain_dimension_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must contain finite nonnegative integer sizes"),
    )
}

fn shape_dimensions<'value>(
    name: &str,
    value: &'value Value,
) -> Result<&'value [u64], BuiltinError> {
    value
        .dimensions()
        .ok_or_else(|| type_error(name, 1, "value with shape semantics", value))
}

fn reshape_numel(value: &Value) -> Result<u64, BuiltinError> {
    match value {
        Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Sparse(_) => value.numel().ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`reshape` could not determine the input element count",
            )
        }),
        Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Nothing
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => Err(type_error(
            "reshape",
            1,
            "numeric, logical, char, integer, or string array",
            value,
        )),
    }
}

fn is_empty_numeric(value: &Value) -> bool {
    matches!(value, Value::Array(array) if !matches!(array.dtype(), DType::F32 | DType::ComplexF32))
        && value.numel() == Some(0)
}

fn filled_vec<T: Clone>(
    name: &str,
    numel: u64,
    element: T,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let length = usize::try_from(numel).map_err(|_| host_length_error(name, numel))?;
    let mut data = Vec::new();
    data.try_reserve_exact(length)
        .map_err(|_| allocation_error(name, numel))?;
    while data.len() < length {
        context.check_cancelled()?;
        let target = data
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        data.resize(target, element.clone());
    }
    Ok(data)
}

fn reserved_dimensions(name: &str, length: usize) -> Result<Vec<u64>, BuiltinError> {
    let mut dimensions = Vec::new();
    dimensions.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate a dimension vector of length {length}"),
        )
    })?;
    Ok(dimensions)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn host_length_error(name: &str, numel: u64) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` array length {numel} does not fit this host"),
    )
}

fn allocation_error(name: &str, numel: u64) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` cannot allocate storage for {numel} elements"),
    )
}

#[allow(clippy::cast_precision_loss)]
fn dimension_value(value: u64) -> Value {
    Value::Double(value as f64)
}
