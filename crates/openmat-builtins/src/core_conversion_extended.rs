//! Extended numeric and container conversion built-ins.
use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, ComplexInteger, DenseArray,
    IntegerArrayData, IntegerComponent, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{CellArray, Value};

use crate::{
    array_error, expect_argument_count, expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_INTERVAL: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversionClass {
    Logical,
    Char,
    Double,
    Single,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

impl ConversionClass {
    fn parse(name: &[u16]) -> Option<Self> {
        if ascii_utf16_eq_ignore_case(name, "logical") {
            Some(Self::Logical)
        } else if ascii_utf16_eq_ignore_case(name, "char") {
            Some(Self::Char)
        } else if ascii_utf16_eq_ignore_case(name, "double") {
            Some(Self::Double)
        } else if ascii_utf16_eq_ignore_case(name, "single") {
            Some(Self::Single)
        } else if ascii_utf16_eq_ignore_case(name, "int8") {
            Some(Self::I8)
        } else if ascii_utf16_eq_ignore_case(name, "uint8") {
            Some(Self::U8)
        } else if ascii_utf16_eq_ignore_case(name, "int16") {
            Some(Self::I16)
        } else if ascii_utf16_eq_ignore_case(name, "uint16") {
            Some(Self::U16)
        } else if ascii_utf16_eq_ignore_case(name, "int32") {
            Some(Self::I32)
        } else if ascii_utf16_eq_ignore_case(name, "uint32") {
            Some(Self::U32)
        } else if ascii_utf16_eq_ignore_case(name, "int64") {
            Some(Self::I64)
        } else if ascii_utf16_eq_ignore_case(name, "uint64") {
            Some(Self::U64)
        } else {
            None
        }
    }

    const fn class_name(self) -> &'static str {
        match self {
            Self::Logical => "logical",
            Self::Char => "char",
            Self::Double => "double",
            Self::Single => "single",
            Self::I8 => "int8",
            Self::U8 => "uint8",
            Self::I16 => "int16",
            Self::U16 => "uint16",
            Self::I32 => "int32",
            Self::U32 => "uint32",
            Self::I64 => "int64",
            Self::U64 => "uint64",
        }
    }

    const fn typecast_width(self) -> Option<usize> {
        match self {
            Self::I8 | Self::U8 => Some(1),
            Self::I16 | Self::U16 => Some(2),
            Self::Single | Self::I32 | Self::U32 => Some(4),
            Self::Double | Self::I64 | Self::U64 => Some(8),
            Self::Logical | Self::Char => None,
        }
    }
}

/// Implements MATLAB R2022b `cast`, including the three-input `'like'` form.
pub(super) fn cast_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("cast", arguments, 2, 3)?;
    expect_max_outputs("cast", context, 1)?;
    context.check_cancelled()?;

    let (class, force_complex) = if arguments.len() == 3 {
        let token = text_scalar("cast", 2, &arguments[1])?;
        if !ascii_utf16_eq_ignore_case(&token, "like") {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "with three inputs, input 2 to `cast` must be 'like'",
            ));
        }
        let prototype = cast_prototype(&arguments[2])?;
        (prototype.0, prototype.1)
    } else {
        let class_name = text_scalar("cast", 2, &arguments[1])?;
        let class = ConversionClass::parse(&class_name).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 2 to `cast` names an unsupported target class",
            )
        })?;
        (class, false)
    };

    if arguments[0].class_name() == class.class_name()
        && (!force_complex || arguments[0].is_complex_numeric())
    {
        return Ok(vec![context.language_copy(&arguments[0])?]);
    }
    if arguments[0].is_complex_numeric()
        && matches!(class, ConversionClass::Logical | ConversionClass::Char)
    {
        return Err(type_error(
            "cast",
            1,
            "real input for a logical or char target",
            &arguments[0],
        ));
    }

    let converted = match class {
        ConversionClass::Logical => crate::core_numeric::logical_builtin(&arguments[..1], context)?,
        ConversionClass::Char => crate::core_numeric::char_builtin(&arguments[..1], context)?,
        ConversionClass::Double => crate::core_numeric::double_builtin(&arguments[..1], context)?,
        ConversionClass::Single => crate::core_numeric::single_builtin(&arguments[..1], context)?,
        ConversionClass::I8 => crate::core_numeric::int8_builtin(&arguments[..1], context)?,
        ConversionClass::U8 => crate::core_numeric::uint8_builtin(&arguments[..1], context)?,
        ConversionClass::I16 => crate::core_numeric::int16_builtin(&arguments[..1], context)?,
        ConversionClass::U16 => crate::core_numeric::uint16_builtin(&arguments[..1], context)?,
        ConversionClass::I32 => crate::core_numeric::int32_builtin(&arguments[..1], context)?,
        ConversionClass::U32 => crate::core_numeric::uint32_builtin(&arguments[..1], context)?,
        ConversionClass::I64 => crate::core_numeric::int64_builtin(&arguments[..1], context)?,
        ConversionClass::U64 => crate::core_numeric::uint64_builtin(&arguments[..1], context)?,
    };
    let mut value = converted
        .into_iter()
        .next()
        .expect("numeric conversion returns one value");
    if force_complex && !value.is_complex_numeric() {
        value = promote_to_complex(value)?;
    }
    context.check_cancelled()?;
    Ok(vec![value])
}

fn cast_prototype(value: &Value) -> Result<(ConversionClass, bool), BuiltinError> {
    let class = match value.class_name() {
        "logical" => ConversionClass::Logical,
        "char" => ConversionClass::Char,
        "double" => ConversionClass::Double,
        "single" => ConversionClass::Single,
        "int8" => ConversionClass::I8,
        "uint8" => ConversionClass::U8,
        "int16" => ConversionClass::I16,
        "uint16" => ConversionClass::U16,
        "int32" => ConversionClass::I32,
        "uint32" => ConversionClass::U32,
        "int64" => ConversionClass::I64,
        "uint64" => ConversionClass::U64,
        _ => {
            return Err(type_error(
                "cast",
                3,
                "numeric, logical, or char prototype",
                value,
            ));
        }
    };
    Ok((class, value.is_complex_numeric()))
}

fn promote_to_complex(value: Value) -> Result<Value, BuiltinError> {
    macro_rules! promote_integer {
        ($array:expr, $variant:ident) => {{
            let shape = $array.shape().clone();
            let values = $array
                .as_slice()
                .iter()
                .copied()
                .map(|value| ComplexInteger::new(value, Default::default()))
                .collect();
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::$variant)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }};
    }

    match value {
        Value::Double(value) => Ok(Value::Complex(openmat_value::Complex64::new(value, 0.0))),
        Value::Array(ArrayData::F64(array)) => {
            let shape = array.shape().clone();
            let values = array
                .as_slice()
                .iter()
                .copied()
                .map(|value| ArrayComplex64::new(value, 0.0))
                .collect();
            DenseArray::from_vec(shape, values)
                .map(ArrayData::ComplexF64)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }
        Value::Array(ArrayData::F32(array)) => {
            let shape = array.shape().clone();
            let values = array
                .as_slice()
                .iter()
                .copied()
                .map(Complex32::from)
                .collect();
            DenseArray::from_vec(shape, values)
                .map(ArrayData::ComplexF32)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }
        Value::Array(ArrayData::Integer(integer)) => match integer {
            IntegerArrayData::I8(array) => promote_integer!(array, ComplexI8),
            IntegerArrayData::U8(array) => promote_integer!(array, ComplexU8),
            IntegerArrayData::I16(array) => promote_integer!(array, ComplexI16),
            IntegerArrayData::U16(array) => promote_integer!(array, ComplexU16),
            IntegerArrayData::I32(array) => promote_integer!(array, ComplexI32),
            IntegerArrayData::U32(array) => promote_integer!(array, ComplexU32),
            IntegerArrayData::I64(array) => promote_integer!(array, ComplexI64),
            IntegerArrayData::U64(array) => promote_integer!(array, ComplexU64),
            complex @ (IntegerArrayData::ComplexI8(_)
            | IntegerArrayData::ComplexU8(_)
            | IntegerArrayData::ComplexI16(_)
            | IntegerArrayData::ComplexU16(_)
            | IntegerArrayData::ComplexI32(_)
            | IntegerArrayData::ComplexU32(_)
            | IntegerArrayData::ComplexI64(_)
            | IntegerArrayData::ComplexU64(_)) => Ok(Value::Array(ArrayData::Integer(complex))),
        },
        already @ (Value::Complex(_)
        | Value::Array(ArrayData::ComplexF32(_) | ArrayData::ComplexF64(_))) => Ok(already),
        other => Err(type_error(
            "cast",
            3,
            "prototype whose class can store complex values",
            &other,
        )),
    }
}

/// Implements MATLAB R2022b `typecast` with native-endian, explicitly packed bytes.
pub(super) fn typecast_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("typecast", arguments, 2)?;
    expect_max_outputs("typecast", context, 1)?;
    context.check_cancelled()?;

    let class_name = text_scalar("typecast", 2, &arguments[1])?;
    let class = ConversionClass::parse(&class_name).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `typecast` names an unsupported target class",
        )
    })?;
    let width = class.typecast_width().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`typecast` supports only floating-point and fixed-width integer targets",
        )
    })?;
    let input_shape = real_typecast_shape(&arguments[0])?;
    if input_shape.ndims() != 2
        || (!input_shape.is_empty() && input_shape.extent(0) != 1 && input_shape.extent(1) != 1)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 1 to `typecast` must be a real two-dimensional vector or empty matrix",
        ));
    }

    let bytes = typecast_input_bytes(&arguments[0], context)?;
    if !bytes.len().is_multiple_of(width) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!(
                "input 1 to `typecast` has {} bytes, which is not divisible by the {}-byte `{}` target",
                bytes.len(),
                width,
                class.class_name()
            ),
        ));
    }
    let output_count = bytes.len() / width;
    let output_shape = if input_shape.is_empty() {
        input_shape.clone()
    } else if input_shape.extent(0) == 1 {
        Shape::new([1, host_length_u64("typecast", output_count)?])
            .map_err(|error| array_error(&error))?
    } else {
        Shape::new([host_length_u64("typecast", output_count)?, 1])
            .map_err(|error| array_error(&error))?
    };
    let value = decode_typecast(class, output_shape, &bytes)?;
    context.check_cancelled()?;
    Ok(vec![value])
}

fn real_typecast_shape(value: &Value) -> Result<&Shape, BuiltinError> {
    match value {
        Value::Array(ArrayData::F64(array)) => Ok(array.shape()),
        Value::Array(ArrayData::F32(array)) => Ok(array.shape()),
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => Ok(integer.shape()),
        Value::Double(_) => Ok(scalar_shape()),
        _ => Err(type_error(
            "typecast",
            1,
            "real floating-point or fixed-width integer vector",
            value,
        )),
    }
}

fn scalar_shape() -> &'static Shape {
    static SHAPE: std::sync::OnceLock<Shape> = std::sync::OnceLock::new();
    SHAPE.get_or_init(|| Shape::new([1, 1]).expect("the scalar shape is valid"))
}

fn typecast_input_bytes(
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u8>, BuiltinError> {
    macro_rules! extend_bytes {
        ($values:expr, $width:expr) => {{
            let capacity = $values.len().checked_mul($width).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "`typecast` byte length overflowed",
                )
            })?;
            let mut bytes = reserved_bytes("typecast", capacity)?;
            for (index, value) in $values.iter().enumerate() {
                check_cancelled_at(context, index)?;
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
            bytes
        }};
    }

    let bytes = match value {
        Value::Double(value) => value.to_ne_bytes().to_vec(),
        Value::Array(ArrayData::F64(array)) => extend_bytes!(array.as_slice(), 8),
        Value::Array(ArrayData::F32(array)) => extend_bytes!(array.as_slice(), 4),
        Value::Array(ArrayData::Integer(integer)) => match integer {
            IntegerArrayData::I8(array) => {
                let mut bytes = reserved_bytes("typecast", array.as_slice().len())?;
                for (index, value) in array.as_slice().iter().enumerate() {
                    check_cancelled_at(context, index)?;
                    bytes.push(value.to_ne_bytes()[0]);
                }
                bytes
            }
            IntegerArrayData::U8(array) => {
                let mut bytes = reserved_bytes("typecast", array.as_slice().len())?;
                for (index, value) in array.as_slice().iter().enumerate() {
                    check_cancelled_at(context, index)?;
                    bytes.push(*value);
                }
                bytes
            }
            IntegerArrayData::I16(array) => extend_bytes!(array.as_slice(), 2),
            IntegerArrayData::U16(array) => extend_bytes!(array.as_slice(), 2),
            IntegerArrayData::I32(array) => extend_bytes!(array.as_slice(), 4),
            IntegerArrayData::U32(array) => extend_bytes!(array.as_slice(), 4),
            IntegerArrayData::I64(array) => extend_bytes!(array.as_slice(), 8),
            IntegerArrayData::U64(array) => extend_bytes!(array.as_slice(), 8),
            IntegerArrayData::ComplexI8(_)
            | IntegerArrayData::ComplexU8(_)
            | IntegerArrayData::ComplexI16(_)
            | IntegerArrayData::ComplexU16(_)
            | IntegerArrayData::ComplexI32(_)
            | IntegerArrayData::ComplexU32(_)
            | IntegerArrayData::ComplexI64(_)
            | IntegerArrayData::ComplexU64(_) => unreachable!("complex input was rejected"),
        },
        _ => unreachable!("typecast input class was validated"),
    };
    Ok(bytes)
}

fn decode_typecast(
    class: ConversionClass,
    shape: Shape,
    bytes: &[u8],
) -> Result<Value, BuiltinError> {
    macro_rules! dense_value {
        ($values:expr, $wrap:expr) => {{
            DenseArray::from_vec(shape, $values)
                .map($wrap)
                .map_err(|error| array_error(&error))
        }};
    }
    macro_rules! decode2 {
        ($type:ty) => {
            bytes
                .chunks_exact(2)
                .map(|chunk| <$type>::from_ne_bytes([chunk[0], chunk[1]]))
                .collect()
        };
    }
    macro_rules! decode4 {
        ($type:ty) => {
            bytes
                .chunks_exact(4)
                .map(|chunk| <$type>::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect()
        };
    }
    macro_rules! decode8 {
        ($type:ty) => {
            bytes
                .chunks_exact(8)
                .map(|chunk| {
                    <$type>::from_ne_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                        chunk[7],
                    ])
                })
                .collect()
        };
    }

    match class {
        ConversionClass::I8 => dense_value!(
            bytes
                .iter()
                .map(|value| i8::from_ne_bytes([*value]))
                .collect(),
            |array| Value::Array(ArrayData::Integer(IntegerArrayData::I8(array)))
        ),
        ConversionClass::U8 => dense_value!(bytes.to_vec(), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::U8(array))
        )),
        ConversionClass::I16 => dense_value!(decode2!(i16), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::I16(array))
        )),
        ConversionClass::U16 => dense_value!(decode2!(u16), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::U16(array))
        )),
        ConversionClass::I32 => dense_value!(decode4!(i32), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::I32(array))
        )),
        ConversionClass::U32 => dense_value!(decode4!(u32), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::U32(array))
        )),
        ConversionClass::I64 => dense_value!(decode8!(i64), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::I64(array))
        )),
        ConversionClass::U64 => dense_value!(decode8!(u64), |array| Value::Array(
            ArrayData::Integer(IntegerArrayData::U64(array))
        )),
        ConversionClass::Single => {
            dense_value!(decode4!(f32), |array| Value::Array(ArrayData::F32(array)))
        }
        ConversionClass::Double => {
            dense_value!(decode8!(f64), |array| Value::Array(ArrayData::F64(array)))
        }
        ConversionClass::Logical | ConversionClass::Char => {
            unreachable!("unsupported typecast targets were rejected")
        }
    }
}

#[derive(Debug)]
struct GroupingPlan {
    source_dimensions: Vec<u64>,
    cell_shape: Shape,
    block_shape: Shape,
    grouped_dimensions: Vec<usize>,
    block_positions: Vec<usize>,
}

/// Implements MATLAB R2022b `num2cell`, including ordered dimension grouping.
pub(super) fn num2cell_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("num2cell", arguments, 1, 2)?;
    expect_max_outputs("num2cell", context, 1)?;
    context.check_cancelled()?;
    let source_shape = numeric_array_shape("num2cell", &arguments[0])?;
    let dimensions = if arguments.len() == 2 {
        let dimensions = positive_dimension_vector("num2cell", 2, &arguments[1])?;
        if dimensions.is_empty() {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 2 to `num2cell` must contain at least one dimension",
            ));
        }
        dimensions
    } else {
        Vec::new()
    };
    let plan = grouping_plan(source_shape, &dimensions)?;
    let cell_count = host_numel("num2cell", &plan.cell_shape)?;
    let mut cells = reserved_values("num2cell", cell_count)?;
    for cell_offset in 0..cell_count {
        check_cancelled_at(context, cell_offset)?;
        cells.push(grouped_numeric_block(&arguments[0], &plan, cell_offset)?);
    }
    let cell = CellArray::from_values(plan.cell_shape, cells)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    context.check_cancelled()?;
    Ok(vec![Value::Cell(cell)])
}

fn grouping_plan(source_shape: &Shape, dimensions: &[u64]) -> Result<GroupingPlan, BuiltinError> {
    let mut grouped_dimensions = Vec::with_capacity(dimensions.len());
    for &dimension in dimensions {
        let dimension = usize::try_from(dimension - 1).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "a `num2cell` grouping dimension does not fit this host",
            )
        })?;
        if grouped_dimensions.contains(&dimension) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 2 to `num2cell` must not repeat a dimension",
            ));
        }
        grouped_dimensions.push(dimension);
    }
    let rank = grouped_dimensions
        .iter()
        .copied()
        .max()
        .map_or(source_shape.ndims(), |maximum| {
            source_shape.ndims().max(maximum.saturating_add(1))
        });
    let mut source_dimensions = source_shape.dimensions().to_vec();
    source_dimensions.resize(rank, 1);
    let mut cell_dimensions = source_dimensions.clone();
    for &dimension in &grouped_dimensions {
        cell_dimensions[dimension] = 1;
    }
    let mut block_positions = grouped_dimensions.clone();
    block_positions.sort_unstable();
    let mut block_dimensions = vec![1; rank];
    for (&block_position, &source_dimension) in
        block_positions.iter().zip(grouped_dimensions.iter())
    {
        block_dimensions[block_position] = source_dimensions[source_dimension];
    }
    let cell_shape = Shape::new(cell_dimensions).map_err(|error| array_error(&error))?;
    let block_shape = Shape::new(block_dimensions).map_err(|error| array_error(&error))?;
    Ok(GroupingPlan {
        source_dimensions,
        cell_shape,
        block_shape,
        grouped_dimensions,
        block_positions,
    })
}

#[allow(clippy::too_many_lines)]
fn grouped_numeric_block(
    value: &Value,
    plan: &GroupingPlan,
    cell_offset: usize,
) -> Result<Value, BuiltinError> {
    macro_rules! gather_array {
        ($array:expr, $wrap:expr) => {{
            Ok($wrap(gather_grouped_dense(
                $array.as_slice(),
                plan,
                cell_offset,
            )?))
        }};
    }
    match value {
        Value::Double(value) => Ok(Value::Array(ArrayData::F64(gather_grouped_dense(
            std::slice::from_ref(value),
            plan,
            cell_offset,
        )?))),
        Value::Complex(value) => {
            let value = ArrayComplex64::from(*value);
            Ok(Value::Array(ArrayData::ComplexF64(gather_grouped_dense(
                std::slice::from_ref(&value),
                plan,
                cell_offset,
            )?)))
        }
        Value::Logical(value) => {
            let value = Logical::from(*value);
            Ok(Value::Array(ArrayData::Logical(gather_grouped_dense(
                std::slice::from_ref(&value),
                plan,
                cell_offset,
            )?)))
        }
        Value::Array(ArrayData::F32(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::F32(array)))
        }
        Value::Array(ArrayData::ComplexF32(array)) => gather_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::F64(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => gather_array!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            gather_array!(array, |array| { Value::Array(ArrayData::Logical(array)) })
        }
        Value::Array(ArrayData::Char(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(integer)) => match integer {
            IntegerArrayData::I8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I8(array))
            )),
            IntegerArrayData::ComplexI8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI8(array))
            )),
            IntegerArrayData::U8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U8(array))
            )),
            IntegerArrayData::ComplexU8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU8(array))
            )),
            IntegerArrayData::I16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I16(array))
            )),
            IntegerArrayData::ComplexI16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI16(array))
            )),
            IntegerArrayData::U16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U16(array))
            )),
            IntegerArrayData::ComplexU16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU16(array))
            )),
            IntegerArrayData::I32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I32(array))
            )),
            IntegerArrayData::ComplexI32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI32(array))
            )),
            IntegerArrayData::U32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U32(array))
            )),
            IntegerArrayData::ComplexU32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU32(array))
            )),
            IntegerArrayData::I64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I64(array))
            )),
            IntegerArrayData::ComplexI64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI64(array))
            )),
            IntegerArrayData::U64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U64(array))
            )),
            IntegerArrayData::ComplexU64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU64(array))
            )),
        },
        _ => Err(type_error(
            "num2cell",
            1,
            "numeric, logical, or char array",
            value,
        )),
    }
}

fn gather_grouped_dense<T: Clone>(
    source: &[T],
    plan: &GroupingPlan,
    cell_offset: usize,
) -> Result<DenseArray<T>, BuiltinError> {
    let cell_subscripts = subscripts_from_offset(
        host_length_u64("num2cell", cell_offset)?,
        plan.cell_shape.dimensions(),
    );
    let block_count = host_numel("num2cell", &plan.block_shape)?;
    let mut output = reserved_elements("num2cell", block_count)?;
    for block_offset in 0..block_count {
        let block_subscripts = subscripts_from_offset(
            host_length_u64("num2cell", block_offset)?,
            plan.block_shape.dimensions(),
        );
        let mut source_subscripts = cell_subscripts.clone();
        source_subscripts.resize(plan.source_dimensions.len(), 0);
        for (&block_position, &source_dimension) in plan
            .block_positions
            .iter()
            .zip(plan.grouped_dimensions.iter())
        {
            source_subscripts[source_dimension] =
                block_subscripts.get(block_position).copied().unwrap_or(0);
        }
        let source_offset = offset_from_subscripts(&source_subscripts, &plan.source_dimensions)?;
        output.push(
            source
                .get(source_offset)
                .ok_or_else(|| internal_offset_error("num2cell"))?
                .clone(),
        );
    }
    DenseArray::from_vec(plan.block_shape.clone(), output).map_err(|error| array_error(&error))
}

#[derive(Debug)]
struct MatrixPartitionPlan {
    source_dimensions: Vec<u64>,
    cell_shape: Shape,
    partitions: Vec<Vec<u64>>,
    starts: Vec<Vec<u64>>,
}

/// Implements MATLAB R2022b `mat2cell` with column-major N-D block extraction.
pub(super) fn mat2cell_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `mat2cell` expects at least 2 inputs but received {}",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("mat2cell", context, 1)?;
    context.check_cancelled()?;
    let source_shape = numeric_array_shape("mat2cell", &arguments[0])?;
    let supplied = arguments.len() - 1;
    if supplied != 1 && supplied != source_shape.ndims() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "`mat2cell` requires either one partition vector or {} vectors for this input",
                source_shape.ndims()
            ),
        ));
    }
    let mut partitions = Vec::with_capacity(source_shape.ndims());
    for dimension in 0..source_shape.ndims() {
        let partition = if dimension < supplied {
            nonnegative_integer_vector("mat2cell", dimension + 2, &arguments[dimension + 1])?
        } else {
            vec![source_shape.extent(dimension)]
        };
        let sum = partition.iter().try_fold(0_u64, |sum, extent| {
            sum.checked_add(*extent).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "a `mat2cell` partition sum overflowed",
                )
            })
        })?;
        if sum != source_shape.extent(dimension) {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!(
                    "partition vector {} to `mat2cell` sums to {sum}, expected {}",
                    dimension + 1,
                    source_shape.extent(dimension)
                ),
            ));
        }
        partitions.push(partition);
    }
    let plan = matrix_partition_plan(source_shape, partitions)?;
    let cell_count = host_numel("mat2cell", &plan.cell_shape)?;
    let mut cells = reserved_values("mat2cell", cell_count)?;
    for cell_offset in 0..cell_count {
        check_cancelled_at(context, cell_offset)?;
        cells.push(matrix_partition_block(&arguments[0], &plan, cell_offset)?);
    }
    let cell = CellArray::from_values(plan.cell_shape, cells)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string()))?;
    context.check_cancelled()?;
    Ok(vec![Value::Cell(cell)])
}

fn matrix_partition_plan(
    source_shape: &Shape,
    partitions: Vec<Vec<u64>>,
) -> Result<MatrixPartitionPlan, BuiltinError> {
    let mut starts = Vec::with_capacity(partitions.len());
    let mut cell_dimensions = Vec::with_capacity(partitions.len());
    for partition in &partitions {
        cell_dimensions.push(host_length_u64("mat2cell", partition.len())?);
        let mut next = 0_u64;
        let mut dimension_starts = Vec::with_capacity(partition.len());
        for extent in partition {
            dimension_starts.push(next);
            next = next.checked_add(*extent).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "a `mat2cell` block start overflowed",
                )
            })?;
        }
        starts.push(dimension_starts);
    }
    let cell_shape = Shape::new(cell_dimensions).map_err(|error| array_error(&error))?;
    Ok(MatrixPartitionPlan {
        source_dimensions: source_shape.dimensions().to_vec(),
        cell_shape,
        partitions,
        starts,
    })
}

fn matrix_partition_block(
    value: &Value,
    plan: &MatrixPartitionPlan,
    cell_offset: usize,
) -> Result<Value, BuiltinError> {
    let cell_subscripts = subscripts_from_offset(
        host_length_u64("mat2cell", cell_offset)?,
        plan.cell_shape.dimensions(),
    );
    let mut block_dimensions = Vec::with_capacity(plan.partitions.len());
    let mut starts = Vec::with_capacity(plan.partitions.len());
    for (dimension, subscript) in cell_subscripts
        .iter()
        .copied()
        .enumerate()
        .take(plan.partitions.len())
    {
        let coordinate =
            usize::try_from(subscript).map_err(|_| internal_offset_error("mat2cell"))?;
        block_dimensions.push(plan.partitions[dimension][coordinate]);
        starts.push(plan.starts[dimension][coordinate]);
    }
    let block_shape = Shape::new(block_dimensions).map_err(|error| array_error(&error))?;
    numeric_block(value, &plan.source_dimensions, &block_shape, &starts)
}

#[allow(clippy::too_many_lines)]
fn numeric_block(
    value: &Value,
    source_dimensions: &[u64],
    block_shape: &Shape,
    starts: &[u64],
) -> Result<Value, BuiltinError> {
    macro_rules! gather_array {
        ($array:expr, $wrap:expr) => {{
            Ok($wrap(gather_dense_block(
                $array.as_slice(),
                source_dimensions,
                block_shape,
                starts,
            )?))
        }};
    }
    match value {
        Value::Double(value) => Ok(Value::Array(ArrayData::F64(gather_dense_block(
            std::slice::from_ref(value),
            source_dimensions,
            block_shape,
            starts,
        )?))),
        Value::Complex(value) => {
            let value = ArrayComplex64::from(*value);
            Ok(Value::Array(ArrayData::ComplexF64(gather_dense_block(
                std::slice::from_ref(&value),
                source_dimensions,
                block_shape,
                starts,
            )?)))
        }
        Value::Logical(value) => {
            let value = Logical::from(*value);
            Ok(Value::Array(ArrayData::Logical(gather_dense_block(
                std::slice::from_ref(&value),
                source_dimensions,
                block_shape,
                starts,
            )?)))
        }
        Value::Array(ArrayData::F32(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::F32(array)))
        }
        Value::Array(ArrayData::ComplexF32(array)) => gather_array!(array, |array| {
            Value::Array(ArrayData::ComplexF32(array))
        }),
        Value::Array(ArrayData::F64(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => gather_array!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            gather_array!(array, |array| { Value::Array(ArrayData::Logical(array)) })
        }
        Value::Array(ArrayData::Char(array)) => {
            gather_array!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(integer)) => match integer {
            IntegerArrayData::I8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I8(array))
            )),
            IntegerArrayData::ComplexI8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI8(array))
            )),
            IntegerArrayData::U8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U8(array))
            )),
            IntegerArrayData::ComplexU8(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU8(array))
            )),
            IntegerArrayData::I16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I16(array))
            )),
            IntegerArrayData::ComplexI16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI16(array))
            )),
            IntegerArrayData::U16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U16(array))
            )),
            IntegerArrayData::ComplexU16(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU16(array))
            )),
            IntegerArrayData::I32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I32(array))
            )),
            IntegerArrayData::ComplexI32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI32(array))
            )),
            IntegerArrayData::U32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U32(array))
            )),
            IntegerArrayData::ComplexU32(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU32(array))
            )),
            IntegerArrayData::I64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::I64(array))
            )),
            IntegerArrayData::ComplexI64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexI64(array))
            )),
            IntegerArrayData::U64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::U64(array))
            )),
            IntegerArrayData::ComplexU64(array) => gather_array!(array, |array| Value::Array(
                ArrayData::Integer(IntegerArrayData::ComplexU64(array))
            )),
        },
        _ => Err(type_error(
            "mat2cell",
            1,
            "numeric, logical, or char array",
            value,
        )),
    }
}

fn gather_dense_block<T: Clone>(
    source: &[T],
    source_dimensions: &[u64],
    block_shape: &Shape,
    starts: &[u64],
) -> Result<DenseArray<T>, BuiltinError> {
    let count = host_numel("mat2cell", block_shape)?;
    let mut output = reserved_elements("mat2cell", count)?;
    for output_offset in 0..count {
        let local = subscripts_from_offset(
            host_length_u64("mat2cell", output_offset)?,
            block_shape.dimensions(),
        );
        let source_subscripts = starts
            .iter()
            .enumerate()
            .map(|(dimension, start)| start + local.get(dimension).copied().unwrap_or(0))
            .collect::<Vec<_>>();
        let source_offset = offset_from_subscripts(&source_subscripts, source_dimensions)?;
        output.push(
            source
                .get(source_offset)
                .ok_or_else(|| internal_offset_error("mat2cell"))?
                .clone(),
        );
    }
    DenseArray::from_vec(block_shape.clone(), output).map_err(|error| array_error(&error))
}

#[derive(Debug)]
struct CellTilingPlan {
    cell_dimensions: Vec<u64>,
    block_dimensions: Vec<Vec<u64>>,
    starts: Vec<Vec<u64>>,
    output_shape: Shape,
}

/// Implements MATLAB R2022b `cell2mat` for homogeneous numeric, logical, and char blocks.
#[allow(clippy::too_many_lines)]
pub(super) fn cell2mat_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("cell2mat", arguments, 1)?;
    expect_max_outputs("cell2mat", context, 1)?;
    context.check_cancelled()?;
    let Value::Cell(cell) = &arguments[0] else {
        return Err(type_error("cell2mat", 1, "cell array", &arguments[0]));
    };
    if cell.is_empty() {
        return Ok(vec![Value::empty_double()]);
    }
    let first = cell
        .value_at_offset(0)
        .expect("a nonempty cell has a first value");
    let class = first.class_name();
    if !matches!(
        class,
        "double"
            | "single"
            | "logical"
            | "char"
            | "int8"
            | "uint8"
            | "int16"
            | "uint16"
            | "int32"
            | "uint32"
            | "int64"
            | "uint64"
    ) {
        return Err(type_error(
            "cell2mat",
            1,
            "cell array containing numeric, logical, or char blocks",
            first,
        ));
    }
    if cell
        .values()
        .iter()
        .any(|value| value.class_name() != class)
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "input 1 to `cell2mat` contains mixed data classes",
        ));
    }
    let force_complex = cell.values().iter().any(Value::is_complex_numeric);
    let plan = cell_tiling_plan(cell)?;

    macro_rules! integer_output {
        ($real:ident, $complex:ident, $type:ty) => {{
            if force_complex {
                let array = tile_cell_blocks(cell, &plan, context, |value| match value {
                    Value::Array(ArrayData::Integer(IntegerArrayData::$real(array))) => Ok(array
                        .as_slice()
                        .iter()
                        .copied()
                        .map(|value| ComplexInteger::new(value, 0 as $type))
                        .collect()),
                    Value::Array(ArrayData::Integer(IntegerArrayData::$complex(array))) => {
                        Ok(array.as_slice().to_vec())
                    }
                    _ => Err(mixed_cell_error()),
                })?;
                Value::Array(ArrayData::Integer(IntegerArrayData::$complex(array)))
            } else {
                let array = tile_cell_blocks(cell, &plan, context, |value| match value {
                    Value::Array(ArrayData::Integer(IntegerArrayData::$real(array))) => {
                        Ok(array.as_slice().to_vec())
                    }
                    _ => Err(mixed_cell_error()),
                })?;
                Value::Array(ArrayData::Integer(IntegerArrayData::$real(array)))
            }
        }};
    }

    let output = match class {
        "double" if force_complex => Value::Array(ArrayData::ComplexF64(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Double(value) => Ok(vec![ArrayComplex64::new(*value, 0.0)]),
                Value::Complex(value) => Ok(vec![ArrayComplex64::from(*value)]),
                Value::Array(ArrayData::F64(array)) => Ok(array
                    .as_slice()
                    .iter()
                    .copied()
                    .map(|value| ArrayComplex64::new(value, 0.0))
                    .collect()),
                Value::Array(ArrayData::ComplexF64(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "double" => Value::Array(ArrayData::F64(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Double(value) => Ok(vec![*value]),
                Value::Array(ArrayData::F64(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "single" if force_complex => Value::Array(ArrayData::ComplexF32(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Array(ArrayData::F32(array)) => Ok(array
                    .as_slice()
                    .iter()
                    .copied()
                    .map(|value| Complex32::new(value, 0.0))
                    .collect()),
                Value::Array(ArrayData::ComplexF32(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "single" => Value::Array(ArrayData::F32(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Array(ArrayData::F32(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "logical" => Value::Array(ArrayData::Logical(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Logical(value) => Ok(vec![Logical::from(*value)]),
                Value::Array(ArrayData::Logical(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "char" => Value::Array(ArrayData::Char(tile_cell_blocks(
            cell,
            &plan,
            context,
            |value| match value {
                Value::Array(ArrayData::Char(array)) => Ok(array.as_slice().to_vec()),
                _ => Err(mixed_cell_error()),
            },
        )?)),
        "int8" => integer_output!(I8, ComplexI8, i8),
        "uint8" => integer_output!(U8, ComplexU8, u8),
        "int16" => integer_output!(I16, ComplexI16, i16),
        "uint16" => integer_output!(U16, ComplexU16, u16),
        "int32" => integer_output!(I32, ComplexI32, i32),
        "uint32" => integer_output!(U32, ComplexU32, u32),
        "int64" => integer_output!(I64, ComplexI64, i64),
        "uint64" => integer_output!(U64, ComplexU64, u64),
        _ => unreachable!("cell class was validated"),
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn cell_tiling_plan(cell: &CellArray) -> Result<CellTilingPlan, BuiltinError> {
    let rank = cell
        .values()
        .iter()
        .try_fold(cell.shape().ndims(), |rank, value| {
            value.dimensions().map_or_else(
                || Err(mixed_cell_error()),
                |dimensions| Ok(rank.max(dimensions.len())),
            )
        })?;
    let mut cell_dimensions = cell.shape().dimensions().to_vec();
    cell_dimensions.resize(rank, 1);
    let partition_lengths = cell_dimensions
        .iter()
        .map(|extent| {
            usize::try_from(*extent).map_err(|_| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "a `cell2mat` cell dimension does not fit this host",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut partitions = partition_lengths
        .iter()
        .map(|length| vec![None; *length])
        .collect::<Vec<Vec<Option<u64>>>>();
    let mut block_dimensions = Vec::with_capacity(cell.values().len());
    for (offset, value) in cell.values().iter().enumerate() {
        let coordinates =
            subscripts_from_offset(host_length_u64("cell2mat", offset)?, &cell_dimensions);
        let mut dimensions = value.dimensions().ok_or_else(mixed_cell_error)?.to_vec();
        dimensions.resize(rank, 1);
        for dimension in 0..rank {
            let coordinate = usize::try_from(coordinates[dimension])
                .map_err(|_| internal_offset_error("cell2mat"))?;
            match partitions[dimension][coordinate] {
                Some(expected) if expected != dimensions[dimension] => {
                    return Err(BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "input 1 to `cell2mat` has incompatible block dimensions",
                    ));
                }
                Some(_) => {}
                None => partitions[dimension][coordinate] = Some(dimensions[dimension]),
            }
        }
        block_dimensions.push(dimensions);
    }
    let partitions = partitions
        .into_iter()
        .map(|dimension| {
            dimension
                .into_iter()
                .map(|extent| extent.ok_or_else(|| internal_offset_error("cell2mat")))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut starts = Vec::with_capacity(rank);
    let mut output_dimensions = Vec::with_capacity(rank);
    for partition in &partitions {
        let mut next = 0_u64;
        let mut dimension_starts = Vec::with_capacity(partition.len());
        for extent in partition {
            dimension_starts.push(next);
            next = next.checked_add(*extent).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "a `cell2mat` output dimension overflowed",
                )
            })?;
        }
        starts.push(dimension_starts);
        output_dimensions.push(next);
    }
    let output_shape = Shape::new(output_dimensions).map_err(|error| array_error(&error))?;
    Ok(CellTilingPlan {
        cell_dimensions,
        block_dimensions,
        starts,
        output_shape,
    })
}

fn tile_cell_blocks<T: Clone + Default>(
    cell: &CellArray,
    plan: &CellTilingPlan,
    context: &BuiltinContext<'_>,
    extract: impl Fn(&Value) -> Result<Vec<T>, BuiltinError>,
) -> Result<DenseArray<T>, BuiltinError> {
    let output_count = host_numel("cell2mat", &plan.output_shape)?;
    let mut output = reserved_elements("cell2mat", output_count)?;
    output.resize(output_count, T::default());
    for (cell_offset, value) in cell.values().iter().enumerate() {
        check_cancelled_at(context, cell_offset)?;
        let coordinates = subscripts_from_offset(
            host_length_u64("cell2mat", cell_offset)?,
            &plan.cell_dimensions,
        );
        let block_dimensions = &plan.block_dimensions[cell_offset];
        let block_shape =
            Shape::new(block_dimensions.clone()).map_err(|error| array_error(&error))?;
        let values = extract(value)?;
        if values.len() != host_numel("cell2mat", &block_shape)? {
            return Err(internal_offset_error("cell2mat"));
        }
        for (block_offset, value) in values.into_iter().enumerate() {
            let local = subscripts_from_offset(
                host_length_u64("cell2mat", block_offset)?,
                block_dimensions,
            );
            let destination = (0..plan.cell_dimensions.len())
                .map(|dimension| {
                    let coordinate = usize::try_from(coordinates[dimension])
                        .map_err(|_| internal_offset_error("cell2mat"))?;
                    plan.starts[dimension][coordinate]
                        .checked_add(local[dimension])
                        .ok_or_else(|| internal_offset_error("cell2mat"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let destination = offset_from_subscripts(&destination, plan.output_shape.dimensions())?;
            *output
                .get_mut(destination)
                .ok_or_else(|| internal_offset_error("cell2mat"))? = value;
        }
    }
    DenseArray::from_vec(plan.output_shape.clone(), output).map_err(|error| array_error(&error))
}

fn mixed_cell_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        "input 1 to `cell2mat` contains mixed or unsupported data",
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecimalValue {
    Negative(i128),
    Nonnegative(u128),
}

impl DecimalValue {
    const fn is_negative(self) -> bool {
        matches!(self, Self::Negative(_))
    }
}

/// Implements MATLAB R2022b `dec2hex` with exact fixed-width integer handling.
pub(super) fn dec2hex_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    decimal_text_builtin("dec2hex", 16, arguments, context)
}

/// Implements MATLAB R2022b `dec2bin` with exact fixed-width integer handling.
pub(super) fn dec2bin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    decimal_text_builtin("dec2bin", 2, arguments, context)
}

fn decimal_text_builtin(
    name: &str,
    radix: u32,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range(name, arguments, 1, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let values = decimal_values(name, radix, &arguments[0], context)?;
    let minimum_width = if arguments.len() == 2 {
        decimal_minimum_width(name, radix, &arguments[1])?
    } else {
        0
    };
    if values.is_empty() {
        return Ok(vec![empty_char_array()?]);
    }
    let (natural_width, signed_bits) = decimal_output_width(name, radix, &values)?;
    let width = natural_width.max(minimum_width);
    let mut rows = Vec::with_capacity(values.len());
    for (index, value) in values.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        rows.push(format_decimal_value(radix, value, signed_bits, width)?);
    }
    context.check_cancelled()?;
    Ok(vec![char_rows(&rows, width)?])
}

fn decimal_values(
    name: &str,
    radix: u32,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<DecimalValue>, BuiltinError> {
    let mut values = Vec::new();
    let mut push_float = |value: f64| -> Result<(), BuiltinError> {
        values.push(float_decimal_value(name, radix, value)?);
        Ok(())
    };
    match value {
        Value::Double(value) => push_float(*value)?,
        Value::Logical(value) => values.push(DecimalValue::Nonnegative(u128::from(*value))),
        Value::Array(ArrayData::F32(array)) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                push_float(f64::from(*value))?;
            }
        }
        Value::Array(ArrayData::F64(array)) => {
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled_at(context, index)?;
                push_float(*value)?;
            }
        }
        Value::Array(ArrayData::Logical(array)) => values.extend(
            array
                .as_slice()
                .iter()
                .map(|value| DecimalValue::Nonnegative(u128::from(value.get()))),
        ),
        Value::Array(ArrayData::Char(array)) => values.extend(
            array
                .as_slice()
                .iter()
                .map(|value| DecimalValue::Nonnegative(u128::from(value.get()))),
        ),
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => {
            values.extend(
                integer
                    .elements()
                    .map(|element| match element.real_component() {
                        IntegerComponent::Signed(value) if value < 0 => {
                            DecimalValue::Negative(value)
                        }
                        IntegerComponent::Signed(value) => DecimalValue::Nonnegative(
                            u128::try_from(value).expect("a nonnegative signed integer fits u128"),
                        ),
                        IntegerComponent::Unsigned(value) => DecimalValue::Nonnegative(value),
                    }),
            );
        }
        Value::Complex(_)
        | Value::Array(
            ArrayData::ComplexF32(_) | ArrayData::ComplexF64(_) | ArrayData::Integer(_),
        ) => {
            return Err(decimal_real_error(name, value));
        }
        _ => {
            return Err(type_error(
                name,
                1,
                "real numeric, logical, char, or integer array",
                value,
            ));
        }
    }
    Ok(values)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn float_decimal_value(name: &str, radix: u32, value: f64) -> Result<DecimalValue, BuiltinError> {
    if !value.is_finite() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 1 to `{name}` must contain finite values"),
        ));
    }
    let value = if radix == 16 {
        if value.fract() != 0.0 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 1 to `dec2hex` must contain integer-valued elements",
            ));
        }
        value
    } else if value < 0.0 {
        value.round()
    } else {
        value.floor()
    };
    if value < 0.0 {
        if value < i64::MIN as f64 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input 1 to `{name}` contains a value below int64 range"),
            ));
        }
        Ok(DecimalValue::Negative(i128::from(value as i64)))
    } else {
        const U64_EXCLUSIVE_UPPER_BOUND: f64 = 18_446_744_073_709_551_616.0;
        if value >= U64_EXCLUSIVE_UPPER_BOUND {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input 1 to `{name}` contains a value above uint64 range"),
            ));
        }
        Ok(DecimalValue::Nonnegative(u128::from(value as u64)))
    }
}

fn decimal_real_error(name: &str, value: &Value) -> BuiltinError {
    type_error(
        name,
        1,
        "real numeric, logical, char, or integer array",
        value,
    )
}

fn decimal_minimum_width(name: &str, radix: u32, value: &Value) -> Result<usize, BuiltinError> {
    if radix == 2 {
        let number = value
            .as_real_number()
            .ok_or_else(|| type_error(name, 2, "finite nonnegative scalar width", value))?;
        if !number.is_finite() || number < 0.0 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 2 to `dec2bin` must be a finite nonnegative scalar",
            ));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return usize::try_from(number.ceil() as u128).map_err(|_| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "input 2 to `dec2bin` is too large",
            )
        });
    }
    let widths = nonnegative_integer_vector(name, 2, value)?;
    if widths.len() != 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `dec2hex` must be a scalar integer width",
        ));
    }
    usize::try_from(widths[0]).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "input 2 to `dec2hex` is too large",
        )
    })
}

fn decimal_output_width(
    name: &str,
    radix: u32,
    values: &[DecimalValue],
) -> Result<(usize, Option<u32>), BuiltinError> {
    if values.iter().any(|value| value.is_negative()) {
        let minimum = values.iter().filter_map(|value| match value {
            DecimalValue::Negative(value) => Some(*value),
            DecimalValue::Nonnegative(_) => None,
        });
        let minimum = minimum.min().expect("a negative value exists");
        let maximum = values
            .iter()
            .filter_map(|value| match value {
                DecimalValue::Negative(_) => None,
                DecimalValue::Nonnegative(value) => Some(*value),
            })
            .max()
            .unwrap_or(0);
        let bits = [8_u32, 16, 32, 64]
            .into_iter()
            .find(|bits| signed_range_contains(*bits, minimum, maximum))
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input 1 to `{name}` cannot fit one signed 64-bit range"),
                )
            })?;
        let width = if radix == 16 {
            usize::try_from(bits / 4).expect("fixed bit widths fit usize")
        } else {
            usize::try_from(bits).expect("fixed bit widths fit usize")
        };
        Ok((width, Some(bits)))
    } else {
        let maximum = values
            .iter()
            .map(|value| match value {
                DecimalValue::Nonnegative(value) => *value,
                DecimalValue::Negative(_) => unreachable!(),
            })
            .max()
            .unwrap_or(0);
        Ok((digit_width(maximum, radix), None))
    }
}

fn signed_range_contains(bits: u32, minimum: i128, maximum: u128) -> bool {
    let negative_limit = -(1_i128 << (bits - 1));
    let positive_limit = (1_u128 << (bits - 1)) - 1;
    minimum >= negative_limit && maximum <= positive_limit
}

fn digit_width(mut value: u128, radix: u32) -> usize {
    let mut width = 1;
    while value >= u128::from(radix) {
        value /= u128::from(radix);
        width += 1;
    }
    width
}

fn format_decimal_value(
    radix: u32,
    value: DecimalValue,
    signed_bits: Option<u32>,
    width: usize,
) -> Result<Vec<u16>, BuiltinError> {
    let (digits, padding) = match value {
        DecimalValue::Nonnegative(value) => (
            if radix == 16 {
                format!("{value:X}")
            } else {
                format!("{value:b}")
            },
            '0',
        ),
        DecimalValue::Negative(value) => {
            let bits = signed_bits.expect("negative output has a signed width");
            let encoded = if bits == 64 {
                let signed = i64::try_from(value).expect("the signed 64-bit range was validated");
                u128::from(u64::from_ne_bytes(signed.to_ne_bytes()))
            } else {
                u128::try_from((1_i128 << bits) + value)
                    .expect("value was checked against the signed width")
            };
            (
                if radix == 16 {
                    format!("{encoded:X}")
                } else {
                    format!("{encoded:b}")
                },
                if radix == 16 { 'F' } else { '1' },
            )
        }
    };
    if digits.len() > width {
        return Err(internal_offset_error("decimal text conversion"));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(width).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("decimal text conversion cannot allocate a width of {width}"),
        )
    })?;
    output.extend(std::iter::repeat_n(
        u16::try_from(u32::from(padding)).expect("ASCII padding fits u16"),
        width - digits.len(),
    ));
    output.extend(digits.encode_utf16());
    Ok(output)
}

fn char_rows(rows: &[Vec<u16>], width: usize) -> Result<Value, BuiltinError> {
    let shape = Shape::new([
        host_length_u64("radix conversion", rows.len())?,
        host_length_u64("radix conversion", width)?,
    ])
    .map_err(|error| array_error(&error))?;
    let count = host_numel("radix conversion", &shape)?;
    let mut values = reserved_elements("radix conversion", count)?;
    for column in 0..width {
        for row in rows {
            values.push(CharCodeUnit::new(row[column]));
        }
    }
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn empty_char_array() -> Result<Value, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([0, 0]).map_err(|error| array_error(&error))?,
        Vec::new(),
    )
    .map(ArrayData::Char)
    .map(Value::Array)
    .map_err(|error| array_error(&error))
}

#[derive(Debug)]
struct RadixTextInput {
    texts: Vec<Vec<u16>>,
    output_shape: Shape,
}

/// Implements MATLAB R2022b `hex2dec` for char, string, and cellstr inputs.
pub(super) fn hex2dec_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    radix_decimal_builtin("hex2dec", 16, arguments, context)
}

/// Implements MATLAB R2022b `bin2dec` for char, string, and cellstr inputs.
pub(super) fn bin2dec_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    radix_decimal_builtin("bin2dec", 2, arguments, context)
}

fn radix_decimal_builtin(
    name: &str,
    radix: u32,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let input = radix_text_input(name, &arguments[0])?;
    let mut values = reserved_elements(name, input.texts.len())?;
    for (index, text) in input.texts.iter().enumerate() {
        check_cancelled_at(context, index)?;
        values.push(parse_radix_text(name, radix, text)?);
    }
    let array =
        DenseArray::from_vec(input.output_shape, values).map_err(|error| array_error(&error))?;
    context.check_cancelled()?;
    Ok(vec![Value::Array(ArrayData::F64(array))])
}

fn radix_text_input(name: &str, value: &Value) -> Result<RadixTextInput, BuiltinError> {
    match value {
        Value::Array(ArrayData::Char(array)) => {
            if array.shape().ndims() != 2 {
                return Err(type_error(name, 1, "two-dimensional char array", value));
            }
            let rows = usize::try_from(array.shape().extent(0))
                .map_err(|_| internal_offset_error(name))?;
            let columns = usize::try_from(array.shape().extent(1))
                .map_err(|_| internal_offset_error(name))?;
            let mut texts = Vec::with_capacity(rows);
            for row in 0..rows {
                let mut text = Vec::with_capacity(columns);
                for column in 0..columns {
                    text.push(array.as_slice()[row + column * rows].get());
                }
                texts.push(text);
            }
            let output_shape = if texts.is_empty() {
                Shape::new([0, 0])
            } else {
                Shape::new([host_length_u64(name, rows)?, 1])
            }
            .map_err(|error| array_error(&error))?;
            Ok(RadixTextInput {
                texts,
                output_shape,
            })
        }
        Value::String(string) => {
            let mut texts = Vec::with_capacity(
                usize::try_from(string.numel()).map_err(|_| internal_offset_error(name))?,
            );
            for offset in 0..string.numel() {
                let offset = usize::try_from(offset).map_err(|_| internal_offset_error(name))?;
                let element = string
                    .element(offset)
                    .ok_or_else(|| internal_offset_error(name))?;
                if element.is_missing() {
                    return Err(type_error(name, 1, "nonmissing string array", value));
                }
                texts.push(element.code_units().to_vec());
            }
            let scalar_empty = texts.len() == 1 && texts[0].is_empty();
            let output_shape = if texts.is_empty() || scalar_empty {
                texts.clear();
                Shape::new([0, 0])
            } else {
                Shape::new(string.dimensions().iter().copied())
            }
            .map_err(|error| array_error(&error))?;
            Ok(RadixTextInput {
                texts,
                output_shape,
            })
        }
        Value::Cell(cell) => {
            let mut texts = Vec::with_capacity(cell.values().len());
            for item in cell.values() {
                let Value::Array(ArrayData::Char(array)) = item else {
                    return Err(type_error(name, 1, "cell array of char rows", value));
                };
                if array.shape().dimensions() == [0, 0] {
                    texts.push(Vec::new());
                } else if array.shape().ndims() == 2 && array.shape().extent(0) == 1 {
                    texts.push(array.as_slice().iter().map(|value| value.get()).collect());
                } else {
                    return Err(type_error(name, 1, "cell array of char rows", value));
                }
            }
            let scalar_empty = texts.len() == 1 && texts[0].is_empty();
            let output_shape = if texts.is_empty() || scalar_empty {
                texts.clear();
                Shape::new([0, 0])
            } else {
                Shape::new([host_length_u64(name, texts.len())?, 1])
            }
            .map_err(|error| array_error(&error))?;
            Ok(RadixTextInput {
                texts,
                output_shape,
            })
        }
        _ => Err(type_error(
            name,
            1,
            "char array, string array, or cell array of char rows",
            value,
        )),
    }
}

#[allow(clippy::cast_precision_loss)]
fn parse_radix_text(name: &str, radix: u32, text: &[u16]) -> Result<f64, BuiltinError> {
    let mut units = if radix == 2 {
        text.iter()
            .copied()
            .filter(|unit| !ascii_whitespace(*unit))
            .collect::<Vec<_>>()
    } else {
        let first = text
            .iter()
            .position(|unit| !ascii_whitespace(*unit))
            .unwrap_or(text.len());
        let end = text
            .iter()
            .rposition(|unit| !ascii_whitespace(*unit))
            .map_or(first, |last| last + 1);
        text[first..end].to_vec()
    };
    if units.len() >= 2
        && units[0] == u16::from(b'0')
        && ((radix == 16 && matches!(units[1], 88 | 120))
            || (radix == 2 && matches!(units[1], 66 | 98)))
    {
        units.drain(..2);
    }
    let maximum_digits = if radix == 16 { 16 } else { 64 };
    if units.len() > maximum_digits {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 1 to `{name}` contains more than {maximum_digits} digits"),
        ));
    }
    let mut value = 0_u64;
    for unit in units {
        let digit = ascii_digit(unit, radix).ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input 1 to `{name}` contains an invalid digit"),
            )
        })?;
        value = value
            .checked_mul(u64::from(radix))
            .and_then(|value| value.checked_add(u64::from(digit)))
            .ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!("input 1 to `{name}` exceeds 64 bits"),
                )
            })?;
    }
    Ok(value as f64)
}

const fn ascii_whitespace(value: u16) -> bool {
    matches!(value, 9..=13 | 32)
}

fn ascii_digit(value: u16, radix: u32) -> Option<u32> {
    let digit = match value {
        48..=57 => u32::from(value - 48),
        65..=70 => u32::from(value - 65) + 10,
        97..=102 => u32::from(value - 97) + 10,
        _ => return None,
    };
    if digit < radix { Some(digit) } else { None }
}

fn numeric_array_shape<'a>(name: &str, value: &'a Value) -> Result<&'a Shape, BuiltinError> {
    match value {
        Value::Double(_) | Value::Complex(_) | Value::Logical(_) => Ok(scalar_shape()),
        Value::Array(array) => Ok(array.shape()),
        _ => Err(type_error(
            name,
            1,
            "numeric, logical, or char array",
            value,
        )),
    }
}

fn positive_dimension_vector(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<u64>, BuiltinError> {
    let dimensions = nonnegative_integer_vector(name, position, value)?;
    if dimensions.contains(&0) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must contain positive dimensions"),
        ));
    }
    Ok(dimensions)
}

fn nonnegative_integer_vector(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<Vec<u64>, BuiltinError> {
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, position, "real nonnegative integer vector", value))?;
    if dimensions.len() != 2
        || (value.numel() != Some(0) && dimensions[0] != 1 && dimensions[1] != 1)
    {
        return Err(type_error(
            name,
            position,
            "real nonnegative integer vector",
            value,
        ));
    }
    let domain_error = || {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must contain finite nonnegative integers"),
        )
    };
    match value {
        Value::Double(value) => Ok(vec![float_dimension(*value).ok_or_else(domain_error)?]),
        Value::Logical(value) => Ok(vec![u64::from(*value)]),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .iter()
            .map(|value| float_dimension(f64::from(*value)).ok_or_else(&domain_error))
            .collect(),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .iter()
            .map(|value| float_dimension(*value).ok_or_else(&domain_error))
            .collect(),
        Value::Array(ArrayData::Logical(array)) => Ok(array
            .as_slice()
            .iter()
            .map(|value| u64::from(value.get()))
            .collect()),
        Value::Array(ArrayData::Integer(integer)) if !integer.is_complex() => integer
            .elements()
            .map(|element| match element.real_component() {
                IntegerComponent::Signed(value) => u64::try_from(value).map_err(|_| domain_error()),
                IntegerComponent::Unsigned(value) => {
                    u64::try_from(value).map_err(|_| domain_error())
                }
            })
            .collect(),
        _ => Err(type_error(
            name,
            position,
            "real nonnegative integer vector",
            value,
        )),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn float_dimension(value: f64) -> Option<u64> {
    if value.is_finite()
        && value >= 0.0
        && value.fract() == 0.0
        && value < 18_446_744_073_709_551_616.0
    {
        Some(value as u64)
    } else {
        None
    }
}

fn subscripts_from_offset(mut offset: u64, dimensions: &[u64]) -> Vec<u64> {
    dimensions
        .iter()
        .map(|extent| {
            let subscript = if *extent == 0 { 0 } else { offset % *extent };
            if *extent != 0 {
                offset /= *extent;
            }
            subscript
        })
        .collect()
}

fn offset_from_subscripts(subscripts: &[u64], dimensions: &[u64]) -> Result<usize, BuiltinError> {
    let mut stride = 1_u64;
    let mut offset = 0_u64;
    for (dimension, extent) in dimensions.iter().copied().enumerate() {
        let subscript = subscripts.get(dimension).copied().unwrap_or(0);
        let contribution = subscript
            .checked_mul(stride)
            .ok_or_else(|| internal_offset_error("conversion"))?;
        offset = offset
            .checked_add(contribution)
            .ok_or_else(|| internal_offset_error("conversion"))?;
        stride = stride
            .checked_mul(extent)
            .ok_or_else(|| internal_offset_error("conversion"))?;
    }
    usize::try_from(offset).map_err(|_| internal_offset_error("conversion"))
}

fn host_numel(name: &str, shape: &Shape) -> Result<usize, BuiltinError> {
    usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` array length does not fit this host"),
        )
    })
}

fn reserved_values(name: &str, capacity: usize) -> Result<Vec<Value>, BuiltinError> {
    reserved_elements(name, capacity)
}

fn reserved_elements<T>(name: &str, capacity: usize) -> Result<Vec<T>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {capacity} elements"),
        )
    })?;
    Ok(values)
}

fn internal_offset_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` encountered an internal checked-offset failure"),
    )
}

fn text_scalar(name: &str, position: usize, value: &Value) -> Result<Vec<u16>, BuiltinError> {
    match value {
        Value::String(string) => {
            let element = string
                .as_scalar()
                .ok_or_else(|| type_error(name, position, "nonmissing text scalar", value))?;
            if element.is_missing() {
                return Err(type_error(name, position, "nonmissing text scalar", value));
            }
            Ok(element.code_units().to_vec())
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            Ok(array.as_slice().iter().map(|value| value.get()).collect())
        }
        _ => Err(type_error(name, position, "text scalar", value)),
    }
}

fn ascii_utf16_eq_ignore_case(value: &[u16], expected: &str) -> bool {
    value.len() == expected.len()
        && value
            .iter()
            .zip(expected.bytes())
            .all(|(actual, expected)| {
                u8::try_from(*actual).is_ok_and(|actual| actual.eq_ignore_ascii_case(&expected))
            })
}

fn host_length_u64(name: &str, value: usize) -> Result<u64, BuiltinError> {
    u64::try_from(value).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` output length does not fit the shape model"),
        )
    })
}

fn reserved_bytes(name: &str, capacity: usize) -> Result<Vec<u8>, BuiltinError> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate {capacity} bytes"),
        )
    })?;
    Ok(bytes)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use openmat_array::IntegerElement;
    use openmat_runtime::{CancellationToken, VecOutput};
    use openmat_value::{Complex64, StringArray, StringElement, StringValue};

    use super::*;

    type Builtin = for<'a> fn(&[Value], &mut BuiltinContext<'a>) -> BuiltinResult;

    fn invoke(function: Builtin, arguments: &[Value]) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut output = VecOutput::new();
        let mut context = BuiltinContext::new(1, &cancellation, &mut output);
        function(arguments, &mut context)
    }

    fn shape(dimensions: impl IntoIterator<Item = u64>) -> Shape {
        Shape::new(dimensions).unwrap()
    }

    fn f64_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(shape(dimensions), values).unwrap(),
        ))
    }

    fn integer_array<T: IntegerElement>(
        dimensions: impl IntoIterator<Item = u64>,
        values: Vec<T>,
    ) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            DenseArray::from_vec(shape(dimensions), values).unwrap(),
        )))
    }

    fn char_array(dimensions: impl IntoIterator<Item = u64>, values: &[u16]) -> Value {
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                shape(dimensions),
                values.iter().copied().map(CharCodeUnit::new).collect(),
            )
            .unwrap(),
        ))
    }

    fn char_rows_value(rows: &[&str]) -> Value {
        let row_count = rows.len();
        let column_count = rows.first().map_or(0, |row| row.encode_utf16().count());
        assert!(
            rows.iter()
                .all(|row| row.encode_utf16().count() == column_count)
        );
        let encoded = rows
            .iter()
            .map(|row| row.encode_utf16().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut values = Vec::with_capacity(row_count * column_count);
        for column in 0..column_count {
            for row in &encoded {
                values.push(row[column]);
            }
        }
        char_array(
            [
                u64::try_from(row_count).unwrap(),
                u64::try_from(column_count).unwrap(),
            ],
            &values,
        )
    }

    fn char_rows(value: &Value) -> Vec<String> {
        let Value::Array(ArrayData::Char(array)) = value else {
            panic!("expected char array");
        };
        let rows = usize::try_from(array.shape().extent(0)).unwrap();
        let columns = usize::try_from(array.shape().extent(1)).unwrap();
        (0..rows)
            .map(|row| {
                String::from_utf16(
                    &(0..columns)
                        .map(|column| array.as_slice()[row + column * rows].get())
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn cast_covers_named_and_like_classes_complexity_and_cow() {
        let original = integer_array([1, 2], vec![1_u64, u64::MAX]);
        let same = invoke(cast_builtin, &[original.clone(), Value::from("uint64")])
            .unwrap()
            .remove(0);
        assert_eq!(same.class_name(), "uint64");
        assert!(original.shares_storage_with(&same));

        let prototype = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
            DenseArray::from_vec(shape([1, 1]), vec![ComplexInteger::new(0_i8, 1_i8)]).unwrap(),
        )));
        let complex = invoke(
            cast_builtin,
            &[
                f64_array([1, 2], vec![1.0, 2.0]),
                Value::from("LIKE"),
                prototype,
            ],
        )
        .unwrap()
        .remove(0);
        let Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(array))) = complex else {
            panic!("like prototype should force complex int8 storage");
        };
        assert_eq!(
            array.as_slice(),
            [ComplexInteger::new(1, 0), ComplexInteger::new(2, 0)]
        );

        for class in [
            "logical", "char", "double", "single", "int8", "uint8", "int16", "uint16", "int32",
            "uint32", "int64", "uint64",
        ] {
            let result = invoke(cast_builtin, &[Value::Double(1.0), Value::from(class)]).unwrap();
            assert_eq!(result[0].class_name(), class);
        }
        let error = invoke(
            cast_builtin,
            &[
                Value::Complex(Complex64::new(1.0, 2.0)),
                Value::from("logical"),
            ],
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Type);
    }

    #[test]
    fn typecast_uses_native_bytes_and_r2022b_vector_orientation() {
        let input = integer_array([1, 2], vec![1_u16, 256_u16]);
        let result = invoke(typecast_builtin, &[input, Value::from("uint8")])
            .unwrap()
            .remove(0);
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = result else {
            panic!("uint8 typecast output");
        };
        assert_eq!(array.shape().dimensions(), &[1, 4]);
        let expected = [1_u16, 256_u16]
            .into_iter()
            .flat_map(u16::to_ne_bytes)
            .collect::<Vec<_>>();
        assert_eq!(array.as_slice(), expected);

        let column = integer_array([2, 1], vec![1_u16, 256_u16]);
        let result = invoke(typecast_builtin, &[column, Value::from("uint8")])
            .unwrap()
            .remove(0);
        assert_eq!(result.dimensions(), Some([4, 1].as_slice()));

        let empty = integer_array::<u8>([0, 3], Vec::new());
        let result = invoke(typecast_builtin, &[empty, Value::from("uint64")])
            .unwrap()
            .remove(0);
        assert_eq!(result.dimensions(), Some([0, 3].as_slice()));

        let nd = integer_array([1, 1, 3], vec![1_u8, 2, 3]);
        assert_eq!(
            invoke(typecast_builtin, &[nd, Value::from("uint8")])
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Domain
        );
        assert_eq!(
            invoke(
                typecast_builtin,
                &[Value::Logical(true), Value::from("uint8")]
            )
            .unwrap_err()
            .category,
            BuiltinErrorCategory::Type
        );
        assert_eq!(
            invoke(
                typecast_builtin,
                &[
                    integer_array([1, 3], vec![1_u8, 2, 3]),
                    Value::from("uint16")
                ]
            )
            .unwrap_err()
            .category,
            BuiltinErrorCategory::Domain
        );
    }

    #[test]
    fn num2cell_honors_ordered_group_dimensions_and_empty_nd_shape() {
        let source = f64_array([2, 3, 4], (1..=24).map(f64::from).collect());
        let dimension_order = f64_array([1, 2], vec![3.0, 1.0]);
        let result = invoke(num2cell_builtin, &[source, dimension_order])
            .unwrap()
            .remove(0);
        let Value::Cell(cell) = result else {
            panic!("num2cell output");
        };
        assert_eq!(cell.shape().dimensions(), &[1, 3]);
        let Value::Array(ArrayData::F64(first)) = &cell.values()[0] else {
            panic!("double cell block");
        };
        assert_eq!(first.shape().dimensions(), &[4, 1, 2]);
        assert_eq!(
            first.as_slice(),
            [1.0, 7.0, 13.0, 19.0, 2.0, 8.0, 14.0, 20.0]
        );

        let empty = f64_array([0, 3, 2], Vec::new());
        let result = invoke(num2cell_builtin, &[empty]).unwrap().remove(0);
        assert_eq!(result.dimensions(), Some([0, 3, 2].as_slice()));
    }

    #[test]
    fn mat2cell_and_cell2mat_round_trip_nd_column_major_blocks() {
        let source = f64_array([4, 3, 2], (1..=24).map(f64::from).collect());
        let cells = invoke(
            mat2cell_builtin,
            &[
                source.clone(),
                f64_array([1, 2], vec![1.0, 3.0]),
                f64_array([1, 2], vec![2.0, 1.0]),
                f64_array([1, 2], vec![1.0, 1.0]),
            ],
        )
        .unwrap()
        .remove(0);
        assert_eq!(cells.dimensions(), Some([2, 2, 2].as_slice()));
        let rebuilt = invoke(cell2mat_builtin, std::slice::from_ref(&cells))
            .unwrap()
            .remove(0);
        assert_eq!(rebuilt, source);

        let Value::Cell(original_cell) = cells else {
            unreachable!();
        };
        let mut incompatible_values = original_cell.values().to_vec();
        incompatible_values[1] = f64_array([1, 1], vec![9.0]);
        let incompatible = Value::Cell(
            CellArray::from_values(original_cell.shape().clone(), incompatible_values).unwrap(),
        );
        let snapshot = incompatible.clone();
        let error = invoke(cell2mat_builtin, std::slice::from_ref(&incompatible)).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);
        assert!(snapshot.shares_storage_with(&incompatible));
    }

    #[test]
    fn cell2mat_promotes_real_blocks_to_complex_without_changing_integer_width() {
        let cell = Value::Cell(
            CellArray::from_values(
                shape([1, 2]),
                vec![
                    integer_array([1, 1], vec![1_i8]),
                    Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
                        DenseArray::from_vec(shape([1, 1]), vec![ComplexInteger::new(2_i8, 3_i8)])
                            .unwrap(),
                    ))),
                ],
            )
            .unwrap(),
        );
        let result = invoke(cell2mat_builtin, &[cell]).unwrap().remove(0);
        let Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(array))) = result else {
            panic!("complex int8 output");
        };
        assert_eq!(array.shape().dimensions(), &[1, 2]);
        assert_eq!(
            array.as_slice(),
            [ComplexInteger::new(1, 0), ComplexInteger::new(2, 3)]
        );

        let mixed = Value::Cell(
            CellArray::from_values(
                shape([1, 2]),
                vec![
                    integer_array([1, 1], vec![1_i8]),
                    integer_array([1, 1], vec![2_u8]),
                ],
            )
            .unwrap(),
        );
        assert_eq!(
            invoke(cell2mat_builtin, &[mixed]).unwrap_err().category,
            BuiltinErrorCategory::Type
        );
    }

    #[test]
    fn num2cell_cell2mat_preserves_every_dense_numeric_storage_class() {
        fn round_trip(value: &Value) {
            let cells = invoke(num2cell_builtin, std::slice::from_ref(value))
                .unwrap()
                .remove(0);
            let rebuilt = invoke(cell2mat_builtin, &[cells]).unwrap().remove(0);
            assert_eq!(&rebuilt, value);
        }

        round_trip(&f64_array([2, 2], vec![1.0, 2.0, 3.0, 4.0]));
        round_trip(&Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                shape([1, 2]),
                vec![ArrayComplex64::new(1.0, 2.0), ArrayComplex64::new(3.0, 4.0)],
            )
            .unwrap(),
        )));
        round_trip(&Value::Array(ArrayData::F32(
            DenseArray::from_vec(shape([1, 2]), vec![1.25_f32, -2.5]).unwrap(),
        )));
        round_trip(&Value::Array(ArrayData::ComplexF32(
            DenseArray::from_vec(
                shape([1, 2]),
                vec![Complex32::new(1.0, 2.0), Complex32::new(3.0, 4.0)],
            )
            .unwrap(),
        )));
        round_trip(&Value::Array(ArrayData::Logical(
            DenseArray::from_vec(shape([1, 2]), vec![Logical::TRUE, Logical::FALSE]).unwrap(),
        )));
        round_trip(&char_array([1, 2], &[0xd83d, 0xde00]));
        round_trip(&integer_array([1, 2], vec![i8::MIN, i8::MAX]));
        round_trip(&integer_array([1, 2], vec![u8::MIN, u8::MAX]));
        round_trip(&integer_array([1, 2], vec![i16::MIN, i16::MAX]));
        round_trip(&integer_array([1, 2], vec![u16::MIN, u16::MAX]));
        round_trip(&integer_array([1, 2], vec![i32::MIN, i32::MAX]));
        round_trip(&integer_array([1, 2], vec![u32::MIN, u32::MAX]));
        round_trip(&integer_array([1, 2], vec![i64::MIN, i64::MAX]));
        round_trip(&integer_array([1, 2], vec![u64::MIN, u64::MAX]));
    }

    #[test]
    fn decimal_text_conversions_preserve_uint64_and_signed_padding() {
        let hexadecimal = invoke(dec2hex_builtin, &[integer_array([1, 1], vec![u64::MAX])])
            .unwrap()
            .remove(0);
        assert_eq!(char_rows(&hexadecimal), ["FFFFFFFFFFFFFFFF"]);

        let signed = invoke(
            dec2hex_builtin,
            &[
                integer_array([1, 2], vec![-1_i16, 256_i16]),
                Value::Double(4.0),
            ],
        )
        .unwrap()
        .remove(0);
        assert_eq!(char_rows(&signed), ["FFFF", "0100"]);

        let binary = invoke(
            dec2bin_builtin,
            &[
                integer_array([1, 2], vec![-1_i8, 1_i8]),
                Value::Double(12.0),
            ],
        )
        .unwrap()
        .remove(0);
        assert_eq!(char_rows(&binary), ["111111111111", "000000000001"]);

        let ordered = invoke(
            dec2hex_builtin,
            &[integer_array([2, 2], vec![1_u16, 2, 3, 4])],
        )
        .unwrap()
        .remove(0);
        assert_eq!(char_rows(&ordered), ["1", "2", "3", "4"]);

        assert_eq!(
            invoke(dec2bin_builtin, &[Value::Complex(Complex64::new(1.0, 0.0))])
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Type
        );
    }

    #[test]
    fn radix_text_inputs_cover_char_string_cellstr_prefixes_and_shapes() {
        let characters = char_rows_value(&["0F", "10"]);
        let result = invoke(hex2dec_builtin, &[characters]).unwrap().remove(0);
        let Value::Array(ArrayData::F64(array)) = result else {
            panic!("double output");
        };
        assert_eq!(array.shape().dimensions(), &[2, 1]);
        assert_eq!(array.as_slice(), [15.0, 16.0]);

        let strings = Value::String(StringValue::Array(
            StringArray::from_elements(
                shape([2, 2]),
                ["0xF", "A", "10", "ff"]
                    .into_iter()
                    .map(StringElement::from)
                    .collect(),
            )
            .unwrap(),
        ));
        let result = invoke(hex2dec_builtin, &[strings]).unwrap().remove(0);
        assert_eq!(result.dimensions(), Some([2, 2].as_slice()));
        let Value::Array(ArrayData::F64(array)) = result else {
            unreachable!();
        };
        assert_eq!(array.as_slice(), [15.0, 10.0, 16.0, 255.0]);

        let cellstr = Value::Cell(
            CellArray::from_values(
                shape([1, 2]),
                vec![char_rows_value(&["1 0"]), char_rows_value(&["0b11"])],
            )
            .unwrap(),
        );
        let result = invoke(bin2dec_builtin, &[cellstr]).unwrap().remove(0);
        assert_eq!(result.dimensions(), Some([2, 1].as_slice()));
        let Value::Array(ArrayData::F64(array)) = result else {
            unreachable!();
        };
        assert_eq!(array.as_slice(), [2.0, 3.0]);

        assert_eq!(
            invoke(hex2dec_builtin, &[char_rows_value(&["GG"])])
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Domain
        );
        assert_eq!(
            invoke(bin2dec_builtin, &[Value::Double(1.0)])
                .unwrap_err()
                .category,
            BuiltinErrorCategory::Type
        );
    }
}
