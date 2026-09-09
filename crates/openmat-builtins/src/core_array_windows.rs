//! MATLAB R2022b array rearrangement, cumulative-extrema, and moving-window built-ins.
//!
//! The integration task owns `lib.rs`. A compile-time entry-point table keeps this reserved module
//! fully checked until that task installs the registration list reported with this commit.

use std::cmp::Ordering;

use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, IntegerComponent, IntegerElement, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{Complex64, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_argument_count,
    expect_argument_count_range, expect_max_outputs, type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

type PendingBuiltin = for<'a> fn(&[Value], &mut BuiltinContext<'a>) -> BuiltinResult;

const _: [PendingBuiltin; 11] = [
    ipermute_builtin,
    rot90_builtin,
    shiftdim_builtin,
    repelem_builtin,
    blkdiag_builtin,
    cummin_builtin,
    cummax_builtin,
    movsum_builtin,
    movmean_builtin,
    movmin_builtin,
    movmax_builtin,
];

pub(super) fn ipermute_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count("ipermute", arguments, 2)?;
    expect_max_outputs("ipermute", context, 1)?;
    context.check_cancelled()?;
    let input_dimensions = array_dimensions("ipermute", &arguments[0])?;
    let order = positive_permutation(&arguments[1], context)?;
    if order.len() < input_dimensions.len() {
        return Err(domain_error(
            "ipermute",
            "the permutation vector must include every stored input dimension",
        ));
    }
    let rank = order.len();
    let mut seen = filled_vec("ipermute", rank, false)?;
    for (index, dimension) in order.iter().copied().enumerate() {
        check_cancelled_at(context, index)?;
        let axis =
            usize::try_from(dimension.saturating_sub(1)).map_err(|_| mapping_error("ipermute"))?;
        if axis >= rank || seen[axis] {
            return Err(domain_error(
                "ipermute",
                "the permutation vector must contain each integer from one through its length",
            ));
        }
        seen[axis] = true;
    }

    let mut extended = filled_vec("ipermute", rank, 1_u64)?;
    extended[..input_dimensions.len()].copy_from_slice(input_dimensions);
    let mut output_dimensions = filled_vec("ipermute", rank, 1_u64)?;
    for (input_axis, output_axis) in order.iter().copied().enumerate() {
        output_dimensions
            [usize::try_from(output_axis - 1).map_err(|_| mapping_error("ipermute"))?] =
            extended[input_axis];
    }
    let mapping_output_dimensions = output_dimensions.clone();
    let input_strides = checked_strides("ipermute", &extended)?;
    let output_strides = checked_strides("ipermute", &mapping_output_dimensions)?;
    let shape = checked_shape("ipermute", output_dimensions)?;
    let output = reorder_value("ipermute", &arguments[0], shape, context, |offset| {
        let offset = u64::try_from(offset).map_err(|_| mapping_error("ipermute"))?;
        let mut source = 0_u64;
        for (input_axis, output_axis) in order.iter().copied().enumerate() {
            let output_axis =
                usize::try_from(output_axis - 1).map_err(|_| mapping_error("ipermute"))?;
            let coordinate =
                (offset / output_strides[output_axis]) % mapping_output_dimensions[output_axis];
            source = coordinate
                .checked_mul(input_strides[input_axis])
                .and_then(|value| source.checked_add(value))
                .ok_or_else(|| mapping_error("ipermute"))?;
        }
        usize::try_from(source).map_err(|_| mapping_error("ipermute"))
    })?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn rot90_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("rot90", arguments, 1, 2)?;
    expect_max_outputs("rot90", context, 1)?;
    context.check_cancelled()?;
    let input_dimensions = array_dimensions("rot90", &arguments[0])?;
    let turns = arguments
        .get(1)
        .map_or(Ok(1_i128), |value| integer_scalar("rot90", 2, value))?
        .rem_euclid(4);
    if turns == 0 {
        return Ok(vec![arguments[0].clone()]);
    }
    let mut output_dimensions = input_dimensions.to_vec();
    if turns % 2 == 1 {
        output_dimensions.swap(0, 1);
    }
    let shape = checked_shape("rot90", output_dimensions)?;
    let rows = input_dimensions[0];
    let columns = input_dimensions[1];
    let plane = rows
        .checked_mul(columns)
        .ok_or_else(|| mapping_error("rot90"))?;
    let output_rows = shape.extent(0);
    let output = reorder_value("rot90", &arguments[0], shape, context, |offset| {
        let offset = u64::try_from(offset).map_err(|_| mapping_error("rot90"))?;
        let plane_offset = offset % plane;
        let output_row = plane_offset % output_rows;
        let output_column = plane_offset / output_rows;
        let (input_row, input_column) = match turns {
            1 => (
                output_column,
                columns
                    .checked_sub(output_row + 1)
                    .ok_or_else(|| mapping_error("rot90"))?,
            ),
            2 => (
                rows.checked_sub(output_row + 1)
                    .ok_or_else(|| mapping_error("rot90"))?,
                columns
                    .checked_sub(output_column + 1)
                    .ok_or_else(|| mapping_error("rot90"))?,
            ),
            3 => (
                rows.checked_sub(output_column + 1)
                    .ok_or_else(|| mapping_error("rot90"))?,
                output_row,
            ),
            _ => unreachable!(),
        };
        let page = (offset / plane)
            .checked_mul(plane)
            .ok_or_else(|| mapping_error("rot90"))?;
        let source = input_column
            .checked_mul(rows)
            .and_then(|value| value.checked_add(input_row))
            .and_then(|value| page.checked_add(value))
            .ok_or_else(|| mapping_error("rot90"))?;
        usize::try_from(source).map_err(|_| mapping_error("rot90"))
    })?;
    context.check_cancelled()?;
    Ok(vec![output])
}

pub(super) fn shiftdim_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("shiftdim", arguments, 1, 2)?;
    expect_max_outputs(
        "shiftdim",
        context,
        if arguments.len() == 1 { 2 } else { 1 },
    )?;
    context.check_cancelled()?;
    let dimensions = array_dimensions("shiftdim", &arguments[0])?;
    let (output_dimensions, removed) = if let Some(value) = arguments.get(1) {
        let shift = integer_scalar("shiftdim", 2, value)?;
        if shift < 0 {
            let count =
                usize::try_from(shift.unsigned_abs()).map_err(|_| mapping_error("shiftdim"))?;
            let mut output = filled_vec("shiftdim", count, 1_u64)?;
            output.extend_from_slice(dimensions);
            (output, 0_u64)
        } else {
            let rank = dimensions.len();
            let amount = usize::try_from(shift).map_err(|_| mapping_error("shiftdim"))? % rank;
            let mut output = Vec::new();
            output
                .try_reserve_exact(rank)
                .map_err(|_| allocation_error("shiftdim", rank))?;
            output.extend_from_slice(&dimensions[amount..]);
            output.extend_from_slice(&dimensions[..amount]);
            (output, 0_u64)
        }
    } else {
        let removable = dimensions.iter().take_while(|extent| **extent == 1).count();
        let removable = if removable == dimensions.len() {
            0
        } else {
            removable
        };
        let mut output = dimensions[removable..].to_vec();
        if output.len() == 1 {
            output.push(1);
        }
        (
            output,
            u64::try_from(removable).map_err(|_| mapping_error("shiftdim"))?,
        )
    };
    let shape = checked_shape("shiftdim", output_dimensions)?;
    let output = if shape.dimensions() == dimensions {
        arguments[0].clone()
    } else {
        reorder_value("shiftdim", &arguments[0], shape, context, Ok)?
    };
    let mut outputs = vec![output];
    if arguments.len() == 1 && context.requested_outputs() > 1 {
        #[allow(clippy::cast_precision_loss)]
        outputs.push(Value::Double(removed as f64));
    }
    context.check_cancelled()?;
    Ok(outputs)
}

pub(super) fn repelem_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 2 {
        return Err(argument_count_error(
            "repelem",
            "at least an input and one repetition input are required",
        ));
    }
    expect_max_outputs("repelem", context, 1)?;
    context.check_cancelled()?;
    let dimensions = array_dimensions("repelem", &arguments[0])?;
    let repetition_inputs: Vec<&Value> = if arguments.len() == 2 {
        if !is_vector_or_scalar(dimensions) {
            return Err(domain_error(
                "repelem",
                "the two-input form requires a vector or scalar input",
            ));
        }
        vec![&arguments[1]]
    } else {
        if arguments.len() - 1 != dimensions.len() {
            return Err(argument_count_error(
                "repelem",
                "the array form requires one repetition input per stored dimension",
            ));
        }
        arguments[1..].iter().collect()
    };

    let mut lookups = Vec::new();
    lookups
        .try_reserve_exact(dimensions.len())
        .map_err(|_| allocation_error("repelem", dimensions.len()))?;
    if arguments.len() == 2 {
        let axis = vector_axis(dimensions);
        for (dimension, extent) in dimensions.iter().copied().enumerate() {
            if dimension == axis {
                let counts = repetition_counts("repelem", &arguments[1], extent, context)?;
                lookups.push(repetition_lookup("repelem", &counts, context)?);
            } else {
                lookups.push(identity_lookup("repelem", extent)?);
            }
        }
    } else {
        for (axis, (extent, repetition)) in dimensions
            .iter()
            .copied()
            .zip(repetition_inputs)
            .enumerate()
        {
            check_cancelled_at(context, axis)?;
            let counts = repetition_counts("repelem", repetition, extent, context)?;
            lookups.push(repetition_lookup("repelem", &counts, context)?);
        }
    }
    let output_dimensions = lookups
        .iter()
        .map(|lookup| u64::try_from(lookup.len()).map_err(|_| mapping_error("repelem")))
        .collect::<Result<Vec<_>, _>>()?;
    let identity = output_dimensions == dimensions
        && lookups.iter().all(|lookup| {
            lookup
                .iter()
                .copied()
                .enumerate()
                .all(|(index, source)| index == source)
        });
    if identity {
        return Ok(vec![arguments[0].clone()]);
    }
    let mapping_output_dimensions = output_dimensions.clone();
    let input_strides = checked_strides("repelem", dimensions)?;
    let shape = checked_shape("repelem", output_dimensions)?;
    let output = reorder_value("repelem", &arguments[0], shape, context, |offset| {
        let mut remainder = u64::try_from(offset).map_err(|_| mapping_error("repelem"))?;
        let mut source = 0_u64;
        for (axis, lookup) in lookups.iter().enumerate() {
            let extent = mapping_output_dimensions[axis];
            let coordinate = if extent == 0 { 0 } else { remainder % extent };
            if extent != 0 {
                remainder /= extent;
            }
            let coordinate = usize::try_from(coordinate).map_err(|_| mapping_error("repelem"))?;
            let source_coordinate = u64::try_from(
                *lookup
                    .get(coordinate)
                    .ok_or_else(|| mapping_error("repelem"))?,
            )
            .map_err(|_| mapping_error("repelem"))?;
            source = source_coordinate
                .checked_mul(input_strides[axis])
                .and_then(|value| source.checked_add(value))
                .ok_or_else(|| mapping_error("repelem"))?;
        }
        usize::try_from(source).map_err(|_| mapping_error("repelem"))
    })?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn positive_permutation(
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let values = integer_vector("ipermute", 2, value, context)?;
    values
        .into_iter()
        .map(|value| {
            u64::try_from(value)
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| domain_error("ipermute", "permutation entries must be positive"))
        })
        .collect()
}

fn repetition_counts(
    name: &str,
    value: &Value,
    extent: u64,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let values = integer_vector(name, 2, value, context)?;
    let extent = usize::try_from(extent).map_err(|_| mapping_error(name))?;
    if values.len() == 1 {
        let count = u64::try_from(values[0])
            .map_err(|_| domain_error(name, "repetition counts must be nonnegative integers"))?;
        return filled_vec(name, extent, count);
    }
    if values.len() != extent {
        return Err(domain_error(
            name,
            "a repetition vector must match the corresponding input extent",
        ));
    }
    values
        .into_iter()
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| domain_error(name, "repetition counts must be nonnegative integers"))
        })
        .collect()
}

fn repetition_lookup(
    name: &str,
    counts: &[u64],
    context: &BuiltinContext<'_>,
) -> Result<Vec<usize>, BuiltinError> {
    let total = counts.iter().try_fold(0_u64, |total, count| {
        total.checked_add(*count).ok_or_else(|| mapping_error(name))
    })?;
    let total = usize::try_from(total).map_err(|_| mapping_error(name))?;
    let mut lookup = reserved_vec(name, total)?;
    for (source, count) in counts.iter().copied().enumerate() {
        check_cancelled_at(context, source)?;
        let count = usize::try_from(count).map_err(|_| mapping_error(name))?;
        let target = lookup
            .len()
            .checked_add(count)
            .ok_or_else(|| mapping_error(name))?;
        lookup.resize(target, source);
    }
    Ok(lookup)
}

fn identity_lookup(name: &str, extent: u64) -> Result<Vec<usize>, BuiltinError> {
    let extent = usize::try_from(extent).map_err(|_| mapping_error(name))?;
    let mut output = reserved_vec(name, extent)?;
    output.extend(0..extent);
    Ok(output)
}

fn vector_axis(dimensions: &[u64]) -> usize {
    usize::from(dimensions[0] == 1)
}

fn is_vector_or_scalar(dimensions: &[u64]) -> bool {
    dimensions.len() == 2 && (dimensions[0] == 1 || dimensions[1] == 1)
}

#[derive(Clone, Copy)]
enum IntegerKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

#[derive(Clone, Copy)]
enum BlockTarget {
    Logical,
    Double { complex: bool },
    Single { complex: bool },
    Char,
    Integer(IntegerKind),
}

#[derive(Clone, Copy)]
enum SourceComponent {
    Signed(i128),
    Unsigned(u128),
    Float(f64),
}

#[derive(Clone, Copy)]
struct NumericElement {
    real: SourceComponent,
    imaginary: Option<SourceComponent>,
}

trait TargetInteger: IntegerElement + Copy {
    fn convert(value: SourceComponent) -> Self;
}

pub(super) fn blkdiag_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs("blkdiag", context, 1)?;
    context.check_cancelled()?;
    if arguments.is_empty() {
        return Ok(vec![Value::empty_double()]);
    }
    let mut total_rows = 0_u64;
    let mut total_columns = 0_u64;
    for (index, input) in arguments.iter().enumerate() {
        check_cancelled_at(context, index)?;
        let dimensions = array_dimensions("blkdiag", input)?;
        if dimensions.len() != 2 {
            return Err(domain_error(
                "blkdiag",
                "every input must be two-dimensional",
            ));
        }
        total_rows = total_rows
            .checked_add(dimensions[0])
            .ok_or_else(|| mapping_error("blkdiag"))?;
        total_columns = total_columns
            .checked_add(dimensions[1])
            .ok_or_else(|| mapping_error("blkdiag"))?;
    }
    let shape = checked_shape("blkdiag", [total_rows, total_columns])?;
    let target = block_target(arguments)?;
    let output = match target {
        BlockTarget::Logical => block_logical(arguments, shape, context)?,
        BlockTarget::Double { complex } => block_double(arguments, shape, complex, context)?,
        BlockTarget::Single { complex } => block_single(arguments, shape, complex, context)?,
        BlockTarget::Char => block_char(arguments, shape, context)?,
        BlockTarget::Integer(kind) => block_integer(arguments, shape, kind, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn block_target(inputs: &[Value]) -> Result<BlockTarget, BuiltinError> {
    let first_char = inputs
        .iter()
        .position(|value| matches!(value, Value::Array(ArrayData::Char(_))));
    let complex = inputs.iter().any(Value::is_complex_numeric);
    if let Some(first_char) = first_char {
        if complex {
            return Err(conversion_error("blkdiag"));
        }
        let logical_prefix = inputs[..first_char].iter().all(|value| {
            matches!(
                value,
                Value::Logical(_) | Value::Array(ArrayData::Logical(_))
            )
        });
        if first_char > 0 && logical_prefix {
            return Err(conversion_error("blkdiag"));
        }
        return Ok(BlockTarget::Char);
    }
    if let Some(kind) = inputs.iter().find_map(integer_kind) {
        if complex {
            return Err(conversion_error("blkdiag"));
        }
        return Ok(BlockTarget::Integer(kind));
    }
    if inputs.iter().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        return Ok(BlockTarget::Single { complex });
    }
    if inputs.iter().any(|value| {
        matches!(
            value,
            Value::Double(_)
                | Value::Complex(_)
                | Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_))
        )
    }) {
        return Ok(BlockTarget::Double { complex });
    }
    if inputs.iter().all(|value| {
        matches!(
            value,
            Value::Logical(_) | Value::Array(ArrayData::Logical(_))
        )
    }) {
        return Ok(BlockTarget::Logical);
    }
    Err(type_error(
        "blkdiag",
        1,
        "numeric, logical, char, or integer matrix",
        &inputs[0],
    ))
}

fn integer_kind(value: &Value) -> Option<IntegerKind> {
    match value {
        Value::Array(ArrayData::Integer(IntegerArrayData::I8(_))) => Some(IntegerKind::I8),
        Value::Array(ArrayData::Integer(IntegerArrayData::U8(_))) => Some(IntegerKind::U8),
        Value::Array(ArrayData::Integer(IntegerArrayData::I16(_))) => Some(IntegerKind::I16),
        Value::Array(ArrayData::Integer(IntegerArrayData::U16(_))) => Some(IntegerKind::U16),
        Value::Array(ArrayData::Integer(IntegerArrayData::I32(_))) => Some(IntegerKind::I32),
        Value::Array(ArrayData::Integer(IntegerArrayData::U32(_))) => Some(IntegerKind::U32),
        Value::Array(ArrayData::Integer(IntegerArrayData::I64(_))) => Some(IntegerKind::I64),
        Value::Array(ArrayData::Integer(IntegerArrayData::U64(_))) => Some(IntegerKind::U64),
        _ => None,
    }
}

fn block_logical(
    inputs: &[Value],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let values = block_map(
        "blkdiag",
        inputs,
        &shape,
        Logical::FALSE,
        context,
        |value, offset| match value {
            Value::Logical(value) if offset == 0 => Ok(Logical::from(*value)),
            Value::Array(ArrayData::Logical(array)) => array
                .as_slice()
                .get(offset)
                .copied()
                .ok_or_else(|| mapping_error("blkdiag")),
            _ => Err(conversion_error("blkdiag")),
        },
    )?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn block_double(
    inputs: &[Value],
    shape: Shape,
    complex: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if complex {
        let values = block_map(
            "blkdiag",
            inputs,
            &shape,
            ArrayComplex64::ZERO,
            context,
            |value, offset| {
                let value = numeric_element("blkdiag", value, offset)?;
                Ok(ArrayComplex64::new(
                    component_f64(value.real),
                    value.imaginary.map_or(0.0, component_f64),
                ))
            },
        )?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let values = block_map(
            "blkdiag",
            inputs,
            &shape,
            0.0_f64,
            context,
            |value, offset| {
                let value = numeric_element("blkdiag", value, offset)?;
                if value.imaginary.is_some() {
                    return Err(conversion_error("blkdiag"));
                }
                Ok(component_f64(value.real))
            },
        )?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn block_single(
    inputs: &[Value],
    shape: Shape,
    complex: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if complex {
        let values = block_map(
            "blkdiag",
            inputs,
            &shape,
            ArrayComplex32::ZERO,
            context,
            |value, offset| {
                let value = numeric_element("blkdiag", value, offset)?;
                Ok(ArrayComplex32::new(
                    component_f32(value.real),
                    value.imaginary.map_or(0.0, component_f32),
                ))
            },
        )?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let values = block_map(
            "blkdiag",
            inputs,
            &shape,
            0.0_f32,
            context,
            |value, offset| {
                let value = numeric_element("blkdiag", value, offset)?;
                if value.imaginary.is_some() {
                    return Err(conversion_error("blkdiag"));
                }
                Ok(component_f32(value.real))
            },
        )?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn block_char(
    inputs: &[Value],
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let values = block_map(
        "blkdiag",
        inputs,
        &shape,
        CharCodeUnit::new(0),
        context,
        |value, offset| {
            if let Value::Array(ArrayData::Char(array)) = value {
                return array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .ok_or_else(|| mapping_error("blkdiag"));
            }
            let value = numeric_element("blkdiag", value, offset)?;
            if value.imaginary.is_some() {
                return Err(conversion_error("blkdiag"));
            }
            Ok(CharCodeUnit::new(component_char(value.real)))
        },
    )?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn block_integer(
    inputs: &[Value],
    shape: Shape,
    kind: IntegerKind,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! block_variant {
        ($kind:ty) => {{
            let values = block_map(
                "blkdiag",
                inputs,
                &shape,
                0 as $kind,
                context,
                |value, offset| {
                    let value = numeric_element("blkdiag", value, offset)?;
                    if value.imaginary.is_some() {
                        return Err(conversion_error("blkdiag"));
                    }
                    Ok(<$kind as TargetInteger>::convert(value.real))
                },
            )?;
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }};
    }
    match kind {
        IntegerKind::I8 => block_variant!(i8),
        IntegerKind::U8 => block_variant!(u8),
        IntegerKind::I16 => block_variant!(i16),
        IntegerKind::U16 => block_variant!(u16),
        IntegerKind::I32 => block_variant!(i32),
        IntegerKind::U32 => block_variant!(u32),
        IntegerKind::I64 => block_variant!(i64),
        IntegerKind::U64 => block_variant!(u64),
    }
}

fn block_map<T: Clone>(
    name: &str,
    inputs: &[Value],
    shape: &Shape,
    zero: T,
    context: &BuiltinContext<'_>,
    mut convert: impl FnMut(&Value, usize) -> Result<T, BuiltinError>,
) -> Result<Vec<T>, BuiltinError> {
    let length = host_length(name, shape.numel())?;
    let mut output = filled_vec(name, length, zero)?;
    let total_rows = shape.extent(0);
    let mut row_start = 0_u64;
    let mut column_start = 0_u64;
    let mut visited = 0_usize;
    for input in inputs {
        let dimensions = array_dimensions(name, input)?;
        let rows = dimensions[0];
        let columns = dimensions[1];
        let input_length = host_length(
            name,
            rows.checked_mul(columns)
                .ok_or_else(|| mapping_error(name))?,
        )?;
        for source_offset in 0..input_length {
            check_cancelled_at(context, visited)?;
            visited = visited.saturating_add(1);
            let source_offset_u64 =
                u64::try_from(source_offset).map_err(|_| mapping_error(name))?;
            let row = if rows == 0 {
                0
            } else {
                source_offset_u64 % rows
            };
            let column = if rows == 0 {
                0
            } else {
                source_offset_u64 / rows
            };
            let target = row_start
                .checked_add(row)
                .and_then(|row| {
                    column_start
                        .checked_add(column)
                        .and_then(|column| column.checked_mul(total_rows))
                        .and_then(|column_offset| row.checked_add(column_offset))
                })
                .and_then(|offset| usize::try_from(offset).ok())
                .ok_or_else(|| mapping_error(name))?;
            *output.get_mut(target).ok_or_else(|| mapping_error(name))? =
                convert(input, source_offset)?;
        }
        row_start = row_start
            .checked_add(rows)
            .ok_or_else(|| mapping_error(name))?;
        column_start = column_start
            .checked_add(columns)
            .ok_or_else(|| mapping_error(name))?;
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Extremum {
    Minimum,
    Maximum,
}

impl Extremum {
    const fn cumulative_name(self) -> &'static str {
        match self {
            Self::Minimum => "cummin",
            Self::Maximum => "cummax",
        }
    }

    const fn moving_name(self) -> &'static str {
        match self {
            Self::Minimum => "movmin",
            Self::Maximum => "movmax",
        }
    }

    fn prefers(self, ordering: Ordering) -> bool {
        match self {
            Self::Minimum => ordering == Ordering::Less,
            Self::Maximum => ordering == Ordering::Greater,
        }
    }
}

#[derive(Clone, Copy)]
struct CumulativeOptions {
    axis: usize,
    reverse: bool,
    omit_nan: bool,
}

pub(super) fn cummin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    cumulative_extremum_builtin(Extremum::Minimum, arguments, context)
}

pub(super) fn cummax_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    cumulative_extremum_builtin(Extremum::Maximum, arguments, context)
}

fn cumulative_extremum_builtin(
    extremum: Extremum,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = extremum.cumulative_name();
    expect_argument_count_range(name, arguments, 1, 4)?;
    // MATLAB R2022b advertises nargout==1 for cummin/cummax. Octave's index output is not used.
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0])?;
    let options = cumulative_options(name, arguments, dimensions)?;
    let output = cumulative_extremum_value(extremum, &arguments[0], options, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn cumulative_options(
    name: &str,
    arguments: &[Value],
    dimensions: &[u64],
) -> Result<CumulativeOptions, BuiltinError> {
    let mut axis = default_axis(dimensions);
    let mut reverse = false;
    let mut omit_nan = true;
    let mut index = 1;
    if let Some(value) = arguments.get(index)
        && keyword(value).is_none()
    {
        axis = positive_dimension(name, index + 1, value)?;
        index += 1;
    }
    while let Some(value) = arguments.get(index) {
        match keyword(value).as_deref() {
            Some("forward") => reverse = false,
            Some("reverse") => reverse = true,
            Some("omitnan" | "omitmissing") => omit_nan = true,
            Some("includenan" | "includemissing") => omit_nan = false,
            _ => return Err(option_error(name)),
        }
        index += 1;
    }
    Ok(CumulativeOptions {
        axis: usize::try_from(axis - 1).map_err(|_| mapping_error(name))?,
        reverse,
        omit_nan,
    })
}

fn cumulative_extremum_value(
    extremum: Extremum,
    input: &Value,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = extremum.cumulative_name();
    macro_rules! cumulative_dense_value {
        ($array:expr, $missing:expr, $compare:expr, $wrap:expr) => {{
            let array =
                cumulative_dense(name, extremum, $array, options, context, $missing, $compare)?;
            Ok($wrap(array))
        }};
    }
    match input {
        Value::Double(value) => {
            if options.axis >= 2 {
                return Ok(input.clone());
            }
            let array = scalar_dense(*value)?;
            let output = cumulative_dense(
                name,
                extremum,
                &array,
                options,
                context,
                f64::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
            )?;
            Ok(Value::Double(output.as_slice()[0]))
        }
        Value::Complex(value) => {
            if options.axis >= 2 {
                return Ok(input.clone());
            }
            let array = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let output = cumulative_dense(
                name,
                extremum,
                &array,
                options,
                context,
                complex64_is_nan,
                compare_complex64,
            )?;
            Ok(Value::Complex(Complex64::from(output.as_slice()[0])))
        }
        Value::Logical(_) => Ok(input.clone()),
        Value::Array(ArrayData::F32(array)) => {
            cumulative_dense_value!(
                array,
                f32::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                |array| Value::Array(ArrayData::F32(array))
            )
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            cumulative_dense_value!(array, complex32_is_nan, compare_complex32, |array| {
                Value::Array(ArrayData::ComplexF32(array))
            })
        }
        Value::Array(ArrayData::F64(array)) => {
            cumulative_dense_value!(
                array,
                f64::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                |array| Value::Array(ArrayData::F64(array))
            )
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            cumulative_dense_value!(array, complex64_is_nan, compare_complex64, |array| {
                Value::Array(ArrayData::ComplexF64(array))
            })
        }
        Value::Array(ArrayData::Logical(array)) => cumulative_dense_value!(
            array,
            |_| false,
            |left, right| left.get().cmp(&right.get()),
            |array| Value::Array(ArrayData::Logical(array))
        ),
        Value::Array(ArrayData::Integer(array)) => {
            cumulative_integer(extremum, array, options, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            name,
            1,
            "numeric or logical array (char is not accepted by MATLAB R2022b)",
            value,
        )),
    }
}

fn cumulative_integer(
    extremum: Extremum,
    input: &IntegerArrayData,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    let name = extremum.cumulative_name();
    macro_rules! variant {
        ($array:expr) => {
            cumulative_dense(
                name,
                extremum,
                $array,
                options,
                context,
                |_| false,
                |left, right| left.cmp(&right),
            )
            .map(IntegerArrayData::from_typed)
        };
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array),
        IntegerArrayData::U8(array) => variant!(array),
        IntegerArrayData::I16(array) => variant!(array),
        IntegerArrayData::U16(array) => variant!(array),
        IntegerArrayData::I32(array) => variant!(array),
        IntegerArrayData::U32(array) => variant!(array),
        IntegerArrayData::I64(array) => variant!(array),
        IntegerArrayData::U64(array) => variant!(array),
        _ => Err(type_error(
            name,
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

fn cumulative_dense<T: Copy>(
    name: &str,
    extremum: Extremum,
    input: &DenseArray<T>,
    options: CumulativeOptions,
    context: &BuiltinContext<'_>,
    is_missing: impl Fn(T) -> bool,
    compare: impl Fn(T, T) -> Ordering,
) -> Result<DenseArray<T>, BuiltinError> {
    let shape = input.shape();
    let extent = shape.extent(options.axis);
    if extent <= 1 || input.is_empty() {
        return Ok(input.clone());
    }
    let stride = axis_stride(name, shape, options.axis)?;
    let extent = usize::try_from(extent).map_err(|_| mapping_error(name))?;
    let block = stride
        .checked_mul(extent)
        .ok_or_else(|| mapping_error(name))?;
    let mut output = clone_slice(name, input.as_slice())?;
    for base in (0..output.len()).step_by(block) {
        for inner in 0..stride {
            let mut current = None;
            for step in 0..extent {
                let axis_offset = if options.reverse {
                    extent - 1 - step
                } else {
                    step
                };
                let offset = base
                    .checked_add(inner)
                    .and_then(|value| value.checked_add(axis_offset.checked_mul(stride)?))
                    .ok_or_else(|| mapping_error(name))?;
                check_cancelled_at(context, offset)?;
                let value = *input
                    .as_slice()
                    .get(offset)
                    .ok_or_else(|| mapping_error(name))?;
                let value_missing = is_missing(value);
                current = match current {
                    None => Some(value),
                    Some(selected) if !options.omit_nan && is_missing(selected) => Some(selected),
                    Some(_) if !options.omit_nan && value_missing => Some(value),
                    Some(selected) if options.omit_nan && value_missing => Some(selected),
                    Some(selected) if is_missing(selected) => Some(value),
                    Some(selected) if extremum.prefers(compare(value, selected)) => Some(value),
                    Some(selected) => Some(selected),
                };
                *output.get_mut(offset).ok_or_else(|| mapping_error(name))? =
                    current.ok_or_else(|| mapping_error(name))?;
            }
        }
    }
    DenseArray::from_vec(shape.clone(), output).map_err(|error| array_error(&error))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MovingArithmetic {
    Sum,
    Mean,
}

impl MovingArithmetic {
    const fn name(self) -> &'static str {
        match self {
            Self::Sum => "movsum",
            Self::Mean => "movmean",
        }
    }
}

#[derive(Clone, Copy)]
struct WindowSpec {
    backward: usize,
    forward: usize,
}

#[derive(Clone)]
enum EndpointPolicy {
    Shrink,
    Discard,
    FillDefault,
    FillValue(Value),
}

#[derive(Clone)]
struct MovingOptions {
    window: WindowSpec,
    axis: usize,
    omit_nan: bool,
    endpoints: EndpointPolicy,
}

pub(super) fn movsum_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    moving_arithmetic_builtin(MovingArithmetic::Sum, arguments, context)
}

pub(super) fn movmean_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    moving_arithmetic_builtin(MovingArithmetic::Mean, arguments, context)
}

pub(super) fn movmin_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    moving_extremum_builtin(Extremum::Minimum, arguments, context)
}

pub(super) fn movmax_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    moving_extremum_builtin(Extremum::Maximum, arguments, context)
}

fn moving_arithmetic_builtin(
    operation: MovingArithmetic,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = operation.name();
    if arguments.len() < 2 {
        return Err(argument_count_error(
            name,
            "an input and window are required",
        ));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0])?;
    let options = moving_options(name, arguments, dimensions, false, context)?;
    let output = moving_arithmetic_value(operation, &arguments[0], &options, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn moving_extremum_builtin(
    extremum: Extremum,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = extremum.moving_name();
    if arguments.len() < 2 {
        return Err(argument_count_error(
            name,
            "an input and window are required",
        ));
    }
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let dimensions = numeric_dimensions(name, &arguments[0])?;
    let options = moving_options(name, arguments, dimensions, true, context)?;
    let output = moving_extremum_value(extremum, &arguments[0], &options, context)?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn moving_options(
    name: &str,
    arguments: &[Value],
    dimensions: &[u64],
    extrema: bool,
    context: &BuiltinContext<'_>,
) -> Result<MovingOptions, BuiltinError> {
    let window = window_spec(name, &arguments[1], context)?;
    let mut axis = default_axis(dimensions);
    let mut axis_seen = false;
    let mut omit_nan = extrema;
    let mut endpoints = EndpointPolicy::Shrink;
    let mut endpoints_seen = false;
    let mut index = 2;
    while let Some(value) = arguments.get(index) {
        match keyword(value).as_deref() {
            Some("omitnan" | "omitmissing") => {
                omit_nan = true;
                index += 1;
            }
            Some("includenan" | "includemissing") => {
                omit_nan = false;
                index += 1;
            }
            Some("endpoints") => {
                if endpoints_seen {
                    return Err(option_error(name));
                }
                endpoints = endpoint_policy(
                    name,
                    arguments.get(index + 1).ok_or_else(|| option_error(name))?,
                )?;
                endpoints_seen = true;
                index += 2;
            }
            Some("samplepoints") => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!(
                        "`{name}` does not yet support the R2022b SamplePoints name-value option"
                    ),
                ));
            }
            None if !axis_seen => {
                axis = positive_dimension(name, index + 1, value)?;
                axis_seen = true;
                index += 1;
            }
            Some(_) | None => return Err(option_error(name)),
        }
    }
    Ok(MovingOptions {
        window,
        axis: usize::try_from(axis - 1).map_err(|_| mapping_error(name))?,
        omit_nan,
        endpoints,
    })
}

fn window_spec(
    name: &str,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<WindowSpec, BuiltinError> {
    let values = integer_vector(name, 2, value, context)?;
    match values.as_slice() {
        [length] if *length > 0 => {
            let length = usize::try_from(*length).map_err(|_| window_error(name))?;
            let backward = length / 2;
            Ok(WindowSpec {
                backward,
                forward: length - 1 - backward,
            })
        }
        [backward, forward] if *backward >= 0 && *forward >= 0 => Ok(WindowSpec {
            backward: usize::try_from(*backward).map_err(|_| window_error(name))?,
            forward: usize::try_from(*forward).map_err(|_| window_error(name))?,
        }),
        _ => Err(window_error(name)),
    }
}

fn endpoint_policy(name: &str, value: &Value) -> Result<EndpointPolicy, BuiltinError> {
    match keyword(value).as_deref() {
        Some("shrink") => Ok(EndpointPolicy::Shrink),
        Some("discard") => Ok(EndpointPolicy::Discard),
        Some("fill") => Ok(EndpointPolicy::FillDefault),
        None if numeric_scalar_element(value).is_some() => {
            Ok(EndpointPolicy::FillValue(value.clone()))
        }
        Some(_) | None => Err(option_error(name)),
    }
}

fn moving_output_shape(
    name: &str,
    input_dimensions: &[u64],
    options: &MovingOptions,
) -> Result<Shape, BuiltinError> {
    let rank = input_dimensions.len().max(options.axis + 1);
    let mut dimensions = filled_vec(name, rank, 1_u64)?;
    dimensions[..input_dimensions.len()].copy_from_slice(input_dimensions);
    if matches!(options.endpoints, EndpointPolicy::Discard) {
        let extent = dimensions[options.axis];
        let removed = u64::try_from(options.window.backward)
            .ok()
            .and_then(|backward| {
                u64::try_from(options.window.forward)
                    .ok()
                    .and_then(|forward| backward.checked_add(forward))
            })
            .ok_or_else(|| mapping_error(name))?;
        dimensions[options.axis] = extent.saturating_sub(removed);
    }
    checked_shape(name, dimensions)
}

#[allow(clippy::too_many_lines)]
fn moving_arithmetic_value(
    operation: MovingArithmetic,
    input: &Value,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let input_dimensions = numeric_dimensions(name, input)?;
    let output_shape = moving_output_shape(name, input_dimensions, options)?;
    let double_fill = arithmetic_fill64(name, &options.endpoints)?;
    let single_fill = arithmetic_fill32(name, &options.endpoints)?;
    match input {
        Value::Double(value) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            false,
            options,
            context,
            |_| Ok(ArrayComplex64::new(*value, 0.0)),
            double_fill,
            true,
        ),
        Value::Complex(value) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            true,
            options,
            context,
            |_| Ok(ArrayComplex64::new(value.real, value.imaginary)),
            double_fill,
            true,
        ),
        Value::Array(ArrayData::F64(array)) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            false,
            options,
            context,
            |offset| {
                array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .map(|value| ArrayComplex64::new(value, 0.0))
                    .ok_or_else(|| mapping_error(name))
            },
            double_fill,
            false,
        ),
        Value::Array(ArrayData::ComplexF64(array)) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            true,
            options,
            context,
            |offset| {
                array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .ok_or_else(|| mapping_error(name))
            },
            double_fill,
            false,
        ),
        Value::Array(ArrayData::F32(array)) => moving_arithmetic32(
            operation,
            input_dimensions,
            output_shape,
            false,
            options,
            context,
            |offset| {
                array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .map(|value| ArrayComplex32::new(value, 0.0))
                    .ok_or_else(|| mapping_error(name))
            },
            single_fill,
        ),
        Value::Array(ArrayData::ComplexF32(array)) => moving_arithmetic32(
            operation,
            input_dimensions,
            output_shape,
            true,
            options,
            context,
            |offset| {
                array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .ok_or_else(|| mapping_error(name))
            },
            single_fill,
        ),
        Value::Logical(value) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            false,
            options,
            context,
            |_| Ok(ArrayComplex64::new(f64::from(*value), 0.0)),
            double_fill,
            false,
        ),
        Value::Array(ArrayData::Logical(array)) => moving_arithmetic64(
            operation,
            input_dimensions,
            output_shape,
            false,
            options,
            context,
            |offset| {
                array
                    .as_slice()
                    .get(offset)
                    .copied()
                    .map(|value| ArrayComplex64::new(f64::from(value.get()), 0.0))
                    .ok_or_else(|| mapping_error(name))
            },
            double_fill,
            false,
        ),
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            moving_integer_arithmetic(
                operation,
                array,
                output_shape,
                options,
                context,
                double_fill,
            )
        }
        value => Err(type_error(
            name,
            1,
            "real or complex floating-point, real integer, or logical array",
            value,
        )),
    }
}

fn moving_integer_arithmetic(
    operation: MovingArithmetic,
    input: &IntegerArrayData,
    output_shape: Shape,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
    fill: Option<ArrayComplex64>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let dimensions = input.shape().dimensions();
    macro_rules! variant {
        ($array:expr) => {
            moving_arithmetic64(
                operation,
                dimensions,
                output_shape,
                false,
                options,
                context,
                |offset| {
                    $array
                        .as_slice()
                        .get(offset)
                        .copied()
                        .map(|value| {
                            #[allow(clippy::cast_lossless, clippy::cast_precision_loss)]
                            let value = value as f64;
                            ArrayComplex64::new(value, 0.0)
                        })
                        .ok_or_else(|| mapping_error(name))
                },
                fill,
                false,
            )
        };
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array),
        IntegerArrayData::U8(array) => variant!(array),
        IntegerArrayData::I16(array) => variant!(array),
        IntegerArrayData::U16(array) => variant!(array),
        IntegerArrayData::I32(array) => variant!(array),
        IntegerArrayData::U32(array) => variant!(array),
        IntegerArrayData::I64(array) => variant!(array),
        IntegerArrayData::U64(array) => variant!(array),
        _ => Err(type_error(
            name,
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn moving_arithmetic64(
    operation: MovingArithmetic,
    input_dimensions: &[u64],
    output_shape: Shape,
    input_complex: bool,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
    get: impl Fn(usize) -> Result<ArrayComplex64, BuiltinError>,
    fill: Option<ArrayComplex64>,
    scalar_variant: bool,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let complex = input_complex || fill.is_some_and(|value| value.im != 0.0);
    let values = moving_line_map(
        name,
        input_dimensions,
        &output_shape,
        options,
        context,
        |range, left_pad, right_pad, stride, inner, base| {
            let mut sum = ArrayComplex64::ZERO;
            let mut count = 0_usize;
            if let Some(fill) = fill {
                accumulate64(&mut sum, &mut count, fill, left_pad, options.omit_nan)?;
            }
            for axis_index in range {
                let offset = base
                    .checked_add(inner)
                    .and_then(|value| value.checked_add(axis_index.checked_mul(stride)?))
                    .ok_or_else(|| mapping_error(name))?;
                accumulate64(&mut sum, &mut count, get(offset)?, 1, options.omit_nan)?;
            }
            if let Some(fill) = fill {
                accumulate64(&mut sum, &mut count, fill, right_pad, options.omit_nan)?;
            }
            if operation == MovingArithmetic::Mean {
                if count == 0 {
                    return Ok(ArrayComplex64::new(f64::NAN, 0.0));
                }
                #[allow(clippy::cast_precision_loss)]
                let count = count as f64;
                sum.re /= count;
                sum.im /= count;
            }
            Ok(sum)
        },
    )?;
    if scalar_variant && output_shape.numel() == 1 {
        let value = values.first().copied().ok_or_else(|| mapping_error(name))?;
        return Ok(if complex {
            Value::Complex(Complex64::from(value))
        } else {
            Value::Double(value.re)
        });
    }
    if complex {
        DenseArray::from_vec(output_shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let values = map_owned(name, values, |value| value.re)?;
        DenseArray::from_vec(output_shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

#[allow(clippy::too_many_arguments)]
fn moving_arithmetic32(
    operation: MovingArithmetic,
    input_dimensions: &[u64],
    output_shape: Shape,
    input_complex: bool,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
    get: impl Fn(usize) -> Result<ArrayComplex32, BuiltinError>,
    fill: Option<ArrayComplex32>,
) -> Result<Value, BuiltinError> {
    let name = operation.name();
    let complex = input_complex || fill.is_some_and(|value| value.im != 0.0);
    let values = moving_line_map(
        name,
        input_dimensions,
        &output_shape,
        options,
        context,
        |range, left_pad, right_pad, stride, inner, base| {
            let mut sum = ArrayComplex32::ZERO;
            let mut count = 0_usize;
            if let Some(fill) = fill {
                accumulate32(&mut sum, &mut count, fill, left_pad, options.omit_nan)?;
            }
            for axis_index in range {
                let offset = base
                    .checked_add(inner)
                    .and_then(|value| value.checked_add(axis_index.checked_mul(stride)?))
                    .ok_or_else(|| mapping_error(name))?;
                accumulate32(&mut sum, &mut count, get(offset)?, 1, options.omit_nan)?;
            }
            if let Some(fill) = fill {
                accumulate32(&mut sum, &mut count, fill, right_pad, options.omit_nan)?;
            }
            if operation == MovingArithmetic::Mean {
                if count == 0 {
                    return Ok(ArrayComplex32::new(f32::NAN, 0.0));
                }
                #[allow(clippy::cast_precision_loss)]
                let count = count as f32;
                sum.re /= count;
                sum.im /= count;
            }
            Ok(sum)
        },
    )?;
    let output = if complex {
        ArrayData::ComplexF32(
            DenseArray::from_vec(output_shape, values).map_err(|error| array_error(&error))?,
        )
    } else {
        ArrayData::F32(
            DenseArray::from_vec(output_shape, map_owned(name, values, |value| value.re)?)
                .map_err(|error| array_error(&error))?,
        )
    };
    Ok(Value::Array(output))
}

fn accumulate64(
    sum: &mut ArrayComplex64,
    count: &mut usize,
    value: ArrayComplex64,
    multiplicity: usize,
    omit_nan: bool,
) -> Result<(), BuiltinError> {
    if multiplicity == 0 || omit_nan && complex64_is_nan(value) {
        return Ok(());
    }
    *count = count
        .checked_add(multiplicity)
        .ok_or_else(|| mapping_error("moving window"))?;
    #[allow(clippy::cast_precision_loss)]
    let multiplicity = multiplicity as f64;
    sum.re += value.re * multiplicity;
    sum.im += value.im * multiplicity;
    Ok(())
}

fn accumulate32(
    sum: &mut ArrayComplex32,
    count: &mut usize,
    value: ArrayComplex32,
    multiplicity: usize,
    omit_nan: bool,
) -> Result<(), BuiltinError> {
    if multiplicity == 0 || omit_nan && complex32_is_nan(value) {
        return Ok(());
    }
    *count = count
        .checked_add(multiplicity)
        .ok_or_else(|| mapping_error("moving window"))?;
    #[allow(clippy::cast_precision_loss)]
    let multiplicity = multiplicity as f32;
    sum.re += value.re * multiplicity;
    sum.im += value.im * multiplicity;
    Ok(())
}

fn arithmetic_fill64(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<ArrayComplex64>, BuiltinError> {
    match endpoints {
        EndpointPolicy::Shrink | EndpointPolicy::Discard => Ok(None),
        EndpointPolicy::FillDefault => Ok(Some(ArrayComplex64::new(f64::NAN, 0.0))),
        EndpointPolicy::FillValue(value) => numeric_scalar_complex64(name, value).map(Some),
    }
}

fn arithmetic_fill32(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<ArrayComplex32>, BuiltinError> {
    arithmetic_fill64(name, endpoints).map(|value| {
        value.map(|value| {
            #[allow(clippy::cast_possible_truncation)]
            let real = value.re as f32;
            #[allow(clippy::cast_possible_truncation)]
            let imaginary = value.im as f32;
            ArrayComplex32::new(real, imaginary)
        })
    })
}

fn moving_line_map<T>(
    name: &str,
    input_dimensions: &[u64],
    output_shape: &Shape,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
    mut evaluate: impl FnMut(
        std::ops::Range<usize>,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> Result<T, BuiltinError>,
) -> Result<Vec<T>, BuiltinError> {
    let input_extent = usize::try_from(input_dimensions.get(options.axis).copied().unwrap_or(1))
        .map_err(|_| mapping_error(name))?;
    let output_extent =
        usize::try_from(output_shape.extent(options.axis)).map_err(|_| mapping_error(name))?;
    let stride = input_dimensions
        .iter()
        .take(options.axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error(name))?;
    let input_block = stride
        .checked_mul(input_extent)
        .ok_or_else(|| mapping_error(name))?;
    let input_length = input_dimensions
        .iter()
        .try_fold(1_u64, |length, extent| length.checked_mul(*extent))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error(name))?;
    let outer = if input_block == 0 {
        output_shape
            .dimensions()
            .iter()
            .skip(options.axis + 1)
            .try_fold(1_u64, |count, extent| count.checked_mul(*extent))
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| mapping_error(name))?
    } else {
        input_length / input_block
    };
    let output_length = host_length(name, output_shape.numel())?;
    let mut output = reserved_vec(name, output_length)?;
    let mut visited = 0_usize;
    for outer_index in 0..outer {
        let base = outer_index
            .checked_mul(input_block)
            .ok_or_else(|| mapping_error(name))?;
        for output_axis in 0..output_extent {
            let center = if matches!(options.endpoints, EndpointPolicy::Discard) {
                output_axis
                    .checked_add(options.window.backward)
                    .ok_or_else(|| mapping_error(name))?
            } else {
                output_axis
            };
            let start = center.saturating_sub(options.window.backward);
            let end = center
                .checked_add(options.window.forward)
                .and_then(|value| value.checked_add(1))
                .unwrap_or(usize::MAX)
                .min(input_extent);
            let left_pad = options.window.backward.saturating_sub(center);
            let right_pad = center
                .checked_add(options.window.forward)
                .and_then(|value| value.checked_add(1))
                .map_or(usize::MAX, |end| end.saturating_sub(input_extent));
            for inner in 0..stride {
                check_cancelled_at(context, visited)?;
                visited = visited.saturating_add(1);
                output.push(evaluate(
                    start..end,
                    left_pad,
                    right_pad,
                    stride,
                    inner,
                    base,
                )?);
            }
        }
    }
    if output.len() != output_length {
        return Err(mapping_error(name));
    }
    Ok(output)
}

#[allow(clippy::too_many_lines)]
fn moving_extremum_value(
    extremum: Extremum,
    input: &Value,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let name = extremum.moving_name();
    let dimensions = numeric_dimensions(name, input)?;
    let output_shape = moving_output_shape(name, dimensions, options)?;
    macro_rules! moving_dense_value {
        ($array:expr, $fill:expr, $missing:expr, $compare:expr, $wrap:expr) => {{
            let values = moving_extremum_dense(
                name,
                extremum,
                $array,
                &output_shape,
                options,
                context,
                $fill,
                $missing,
                $compare,
            )?;
            DenseArray::from_vec(output_shape, values)
                .map($wrap)
                .map_err(|error| array_error(&error))
        }};
    }
    match input {
        Value::Double(value) => {
            let array = scalar_dense(*value)?;
            let fill = extrema_fill64(name, &options.endpoints)?;
            let values = moving_extremum_dense(
                name,
                extremum,
                &array,
                &output_shape,
                options,
                context,
                fill,
                f64::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
            )?;
            if output_shape.numel() == 1 {
                Ok(Value::Double(values[0]))
            } else {
                DenseArray::from_vec(output_shape, values)
                    .map(ArrayData::F64)
                    .map(Value::Array)
                    .map_err(|error| array_error(&error))
            }
        }
        Value::Complex(value) => {
            let array = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let fill = extrema_fill_complex64(name, &options.endpoints)?;
            let values = moving_extremum_dense(
                name,
                extremum,
                &array,
                &output_shape,
                options,
                context,
                fill,
                complex64_is_nan,
                compare_complex64,
            )?;
            if output_shape.numel() == 1 {
                Ok(Value::Complex(Complex64::from(values[0])))
            } else {
                DenseArray::from_vec(output_shape, values)
                    .map(ArrayData::ComplexF64)
                    .map(Value::Array)
                    .map_err(|error| array_error(&error))
            }
        }
        Value::Logical(value) => {
            let array = scalar_dense(Logical::from(*value))?;
            let fill = extrema_fill_logical(name, &options.endpoints)?;
            let values = moving_extremum_dense(
                name,
                extremum,
                &array,
                &output_shape,
                options,
                context,
                fill,
                |_| false,
                |left, right| left.get().cmp(&right.get()),
            )?;
            if output_shape.numel() == 1 {
                Ok(Value::Logical(values[0].get()))
            } else {
                DenseArray::from_vec(output_shape, values)
                    .map(ArrayData::Logical)
                    .map(Value::Array)
                    .map_err(|error| array_error(&error))
            }
        }
        Value::Array(ArrayData::F32(array)) => {
            let fill = extrema_fill32(name, &options.endpoints)?;
            moving_dense_value!(
                array,
                fill,
                f32::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                |array| Value::Array(ArrayData::F32(array))
            )
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            let fill = extrema_fill_complex32(name, &options.endpoints)?;
            moving_dense_value!(array, fill, complex32_is_nan, compare_complex32, |array| {
                Value::Array(ArrayData::ComplexF32(array))
            })
        }
        Value::Array(ArrayData::F64(array)) => {
            let fill = extrema_fill64(name, &options.endpoints)?;
            moving_dense_value!(
                array,
                fill,
                f64::is_nan,
                |left, right| left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                |array| Value::Array(ArrayData::F64(array))
            )
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let fill = extrema_fill_complex64(name, &options.endpoints)?;
            moving_dense_value!(array, fill, complex64_is_nan, compare_complex64, |array| {
                Value::Array(ArrayData::ComplexF64(array))
            })
        }
        Value::Array(ArrayData::Logical(array)) => {
            let fill = extrema_fill_logical(name, &options.endpoints)?;
            moving_dense_value!(
                array,
                fill,
                |_| false,
                |left, right| left.get().cmp(&right.get()),
                |array| Value::Array(ArrayData::Logical(array))
            )
        }
        Value::Array(ArrayData::Integer(array)) => {
            moving_integer_extremum(extremum, array, output_shape, options, context)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            name,
            1,
            "numeric or logical array (char is not accepted by MATLAB R2022b)",
            value,
        )),
    }
}

fn moving_integer_extremum(
    extremum: Extremum,
    input: &IntegerArrayData,
    output_shape: Shape,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    let name = extremum.moving_name();
    macro_rules! variant {
        ($array:expr, $kind:ty) => {{
            let fill = extrema_fill_integer::<$kind>(name, &options.endpoints)?;
            let values = moving_extremum_dense(
                name,
                extremum,
                $array,
                &output_shape,
                options,
                context,
                fill,
                |_| false,
                |left, right| left.cmp(&right),
            )?;
            DenseArray::from_vec(output_shape, values)
                .map(IntegerArrayData::from_typed)
                .map_err(|error| array_error(&error))
        }};
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array, i8),
        IntegerArrayData::U8(array) => variant!(array, u8),
        IntegerArrayData::I16(array) => variant!(array, i16),
        IntegerArrayData::U16(array) => variant!(array, u16),
        IntegerArrayData::I32(array) => variant!(array, i32),
        IntegerArrayData::U32(array) => variant!(array, u32),
        IntegerArrayData::I64(array) => variant!(array, i64),
        IntegerArrayData::U64(array) => variant!(array, u64),
        _ => Err(type_error(
            name,
            1,
            "real integer array",
            &Value::Array(ArrayData::Integer(input.clone())),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn moving_extremum_dense<T: Copy>(
    name: &str,
    extremum: Extremum,
    input: &DenseArray<T>,
    output_shape: &Shape,
    options: &MovingOptions,
    context: &BuiltinContext<'_>,
    fill: Option<T>,
    is_missing: impl Fn(T) -> bool,
    compare: impl Fn(T, T) -> Ordering,
) -> Result<Vec<T>, BuiltinError> {
    moving_line_map(
        name,
        input.shape().dimensions(),
        output_shape,
        options,
        context,
        |range, left_pad, right_pad, stride, inner, base| {
            let mut selected = None;
            let mut fallback = None;
            let mut consider = |value: T| {
                if is_missing(value) {
                    if options.omit_nan {
                        fallback.get_or_insert(value);
                    } else if selected.is_none_or(|current| !is_missing(current)) {
                        selected = Some(value);
                    }
                    return;
                }
                if selected.is_none_or(|current| {
                    !is_missing(current) && extremum.prefers(compare(value, current))
                }) {
                    selected = Some(value);
                }
            };
            if left_pad > 0
                && let Some(fill) = fill
            {
                consider(fill);
            }
            for axis_index in range {
                let offset = base
                    .checked_add(inner)
                    .and_then(|value| value.checked_add(axis_index.checked_mul(stride)?))
                    .ok_or_else(|| mapping_error(name))?;
                consider(
                    *input
                        .as_slice()
                        .get(offset)
                        .ok_or_else(|| mapping_error(name))?,
                );
            }
            if right_pad > 0
                && let Some(fill) = fill
            {
                consider(fill);
            }
            selected.or(fallback).ok_or_else(|| mapping_error(name))
        },
    )
}

fn extrema_fill64(name: &str, endpoints: &EndpointPolicy) -> Result<Option<f64>, BuiltinError> {
    match endpoints {
        EndpointPolicy::Shrink | EndpointPolicy::Discard | EndpointPolicy::FillDefault => Ok(None),
        EndpointPolicy::FillValue(value) => {
            let value = numeric_scalar_complex64(name, value)?;
            if value.im == 0.0 {
                Ok(Some(value.re))
            } else {
                Err(conversion_error(name))
            }
        }
    }
}

fn extrema_fill32(name: &str, endpoints: &EndpointPolicy) -> Result<Option<f32>, BuiltinError> {
    extrema_fill64(name, endpoints).map(|value| {
        value.map(|value| {
            #[allow(clippy::cast_possible_truncation)]
            let value = value as f32;
            value
        })
    })
}

fn extrema_fill_complex64(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<ArrayComplex64>, BuiltinError> {
    match endpoints {
        EndpointPolicy::Shrink | EndpointPolicy::Discard | EndpointPolicy::FillDefault => Ok(None),
        EndpointPolicy::FillValue(value) => numeric_scalar_complex64(name, value).map(Some),
    }
}

fn extrema_fill_complex32(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<ArrayComplex32>, BuiltinError> {
    extrema_fill_complex64(name, endpoints).map(|value| {
        value.map(|value| {
            #[allow(clippy::cast_possible_truncation)]
            let real = value.re as f32;
            #[allow(clippy::cast_possible_truncation)]
            let imaginary = value.im as f32;
            ArrayComplex32::new(real, imaginary)
        })
    })
}

fn extrema_fill_logical(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<Logical>, BuiltinError> {
    match endpoints {
        EndpointPolicy::Shrink | EndpointPolicy::Discard | EndpointPolicy::FillDefault => Ok(None),
        EndpointPolicy::FillValue(value) => {
            let value = numeric_scalar_complex64(name, value)?;
            if value.im != 0.0 {
                return Err(conversion_error(name));
            }
            Ok(Some(Logical::from(value.re != 0.0)))
        }
    }
}

fn extrema_fill_integer<T: TargetInteger>(
    name: &str,
    endpoints: &EndpointPolicy,
) -> Result<Option<T>, BuiltinError> {
    match endpoints {
        EndpointPolicy::Shrink | EndpointPolicy::Discard | EndpointPolicy::FillDefault => Ok(None),
        EndpointPolicy::FillValue(value) => {
            let value = numeric_scalar_element(value).ok_or_else(|| conversion_error(name))?;
            if value.imaginary.is_some() {
                return Err(conversion_error(name));
            }
            Ok(Some(T::convert(value.real)))
        }
    }
}

fn reorder_value(
    name: &str,
    input: &Value,
    shape: Shape,
    context: &BuiltinContext<'_>,
    source_offset: impl Fn(usize) -> Result<usize, BuiltinError>,
) -> Result<Value, BuiltinError> {
    macro_rules! reorder_dense_value {
        ($array:expr, $wrap:expr) => {{
            let array = reorder_dense(name, $array, shape, context, &source_offset)?;
            Ok($wrap(array))
        }};
    }
    if shape.numel() == 1 && input.is_scalar() {
        return Ok(input.clone());
    }
    match input {
        Value::Logical(value) => {
            let array = scalar_dense(Logical::from(*value))?;
            reorder_dense_value!(&array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Double(value) => {
            let array = scalar_dense(*value)?;
            reorder_dense_value!(&array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Complex(value) => {
            let array = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            reorder_dense_value!(&array, |array| Value::Array(ArrayData::ComplexF64(array)))
        }
        Value::Array(ArrayData::F32(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::F32(array)))
        }
        Value::Array(ArrayData::ComplexF32(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::ComplexF32(array)))
        }
        Value::Array(ArrayData::F64(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::F64(array)))
        }
        Value::Array(ArrayData::ComplexF64(array)) => reorder_dense_value!(array, |array| {
            Value::Array(ArrayData::ComplexF64(array))
        }),
        Value::Array(ArrayData::Logical(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::Logical(array)))
        }
        Value::Array(ArrayData::Char(array)) => {
            reorder_dense_value!(array, |array| Value::Array(ArrayData::Char(array)))
        }
        Value::Array(ArrayData::Integer(array)) => {
            reorder_integer(name, array, shape, context, &source_offset)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        value => Err(type_error(
            name,
            1,
            "double, single, complex, integer, logical, or char array",
            value,
        )),
    }
}

fn reorder_dense<T: Copy>(
    name: &str,
    input: &DenseArray<T>,
    shape: Shape,
    context: &BuiltinContext<'_>,
    source_offset: &impl Fn(usize) -> Result<usize, BuiltinError>,
) -> Result<DenseArray<T>, BuiltinError> {
    let length = host_length(name, shape.numel())?;
    let mut output = reserved_vec(name, length)?;
    for offset in 0..length {
        check_cancelled_at(context, offset)?;
        let source = source_offset(offset)?;
        output.push(
            *input
                .as_slice()
                .get(source)
                .ok_or_else(|| mapping_error(name))?,
        );
    }
    DenseArray::from_vec(shape, output).map_err(|error| array_error(&error))
}

fn reorder_integer(
    name: &str,
    input: &IntegerArrayData,
    shape: Shape,
    context: &BuiltinContext<'_>,
    source_offset: &impl Fn(usize) -> Result<usize, BuiltinError>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! variant {
        ($array:expr) => {
            reorder_dense(name, $array, shape, context, source_offset)
                .map(IntegerArrayData::from_typed)
        };
    }
    match input {
        IntegerArrayData::I8(array) => variant!(array),
        IntegerArrayData::ComplexI8(array) => variant!(array),
        IntegerArrayData::U8(array) => variant!(array),
        IntegerArrayData::ComplexU8(array) => variant!(array),
        IntegerArrayData::I16(array) => variant!(array),
        IntegerArrayData::ComplexI16(array) => variant!(array),
        IntegerArrayData::U16(array) => variant!(array),
        IntegerArrayData::ComplexU16(array) => variant!(array),
        IntegerArrayData::I32(array) => variant!(array),
        IntegerArrayData::ComplexI32(array) => variant!(array),
        IntegerArrayData::U32(array) => variant!(array),
        IntegerArrayData::ComplexU32(array) => variant!(array),
        IntegerArrayData::I64(array) => variant!(array),
        IntegerArrayData::ComplexI64(array) => variant!(array),
        IntegerArrayData::U64(array) => variant!(array),
        IntegerArrayData::ComplexU64(array) => variant!(array),
    }
}

fn scalar_dense<T>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(checked_shape("scalar", [1, 1])?, vec![value])
        .map_err(|error| array_error(&error))
}

fn checked_strides(name: &str, dimensions: &[u64]) -> Result<Vec<u64>, BuiltinError> {
    let mut strides = reserved_vec(name, dimensions.len())?;
    let mut stride = 1_u64;
    for extent in dimensions.iter().copied() {
        strides.push(stride);
        stride = stride
            .checked_mul(extent)
            .ok_or_else(|| mapping_error(name))?;
    }
    Ok(strides)
}

fn integer_vector(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<Vec<i128>, BuiltinError> {
    let dimensions = value
        .dimensions()
        .ok_or_else(|| type_error(name, position, "real integer vector", value))?;
    if !is_vector_or_scalar(dimensions) && value.numel() != Some(0) {
        return Err(type_error(name, position, "real integer vector", value));
    }
    let mut output = Vec::new();
    let parse_float = |value: f64| -> Result<i128, BuiltinError> {
        if !value.is_finite()
            || value.fract() != 0.0
            || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
        {
            return Err(domain_error(
                name,
                "integer inputs must contain finite exact integers",
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        let value = i128::from(value as i64);
        Ok(value)
    };
    match value {
        Value::Double(value) => output.push(parse_float(*value)?),
        Value::Array(ArrayData::F64(array)) => {
            output
                .try_reserve_exact(array.as_slice().len())
                .map_err(|_| allocation_error(name, array.as_slice().len()))?;
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(parse_float(value)?);
            }
        }
        Value::Array(ArrayData::F32(array)) => {
            output
                .try_reserve_exact(array.as_slice().len())
                .map_err(|_| allocation_error(name, array.as_slice().len()))?;
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(parse_float(f64::from(value))?);
            }
        }
        Value::Array(ArrayData::Integer(array)) if !array.is_complex() => {
            let length = host_length(name, array.shape().numel())?;
            output
                .try_reserve_exact(length)
                .map_err(|_| allocation_error(name, length))?;
            for (index, value) in array.elements().enumerate() {
                check_cancelled_at(context, index)?;
                output.push(match value.real_component() {
                    IntegerComponent::Signed(value) => value,
                    IntegerComponent::Unsigned(value) => i128::try_from(value).map_err(|_| {
                        domain_error(name, "integer input exceeds the signed range")
                    })?,
                });
            }
        }
        _ => return Err(type_error(name, position, "real integer vector", value)),
    }
    Ok(output)
}

fn integer_scalar(name: &str, position: usize, value: &Value) -> Result<i128, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => Ok(value),
            IntegerComponent::Unsigned(value) => i128::try_from(value)
                .map_err(|_| domain_error(name, "integer scalar is outside the supported range")),
        };
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(|| type_error(name, position, "real integer scalar", value))?;
    if !value.is_finite()
        || value.fract() != 0.0
        || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
    {
        return Err(domain_error(
            name,
            "the input must be a finite real integer scalar",
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(i128::from(value as i64))
}

fn positive_dimension(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        }
        .filter(|value| *value > 0)
        .ok_or_else(|| domain_error(name, "the dimension must be a positive integer"));
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            array.as_slice().first().copied()
        }
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(|| type_error(name, position, "positive integer dimension", value))?;
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(domain_error(
            name,
            "the dimension must be a positive integer",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn numeric_dimensions<'a>(name: &str, value: &'a Value) -> Result<&'a [u64], BuiltinError> {
    if matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::Logical(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Integer(_)
            )
    ) {
        return value.dimensions().ok_or_else(|| mapping_error(name));
    }
    Err(type_error(name, 1, "numeric or logical array", value))
}

fn array_dimensions<'a>(name: &str, value: &'a Value) -> Result<&'a [u64], BuiltinError> {
    if matches!(
        value,
        Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(
                ArrayData::F32(_)
                    | ArrayData::ComplexF32(_)
                    | ArrayData::Logical(_)
                    | ArrayData::F64(_)
                    | ArrayData::ComplexF64(_)
                    | ArrayData::Char(_)
                    | ArrayData::Integer(_)
            )
    ) {
        return value.dimensions().ok_or_else(|| mapping_error(name));
    }
    Err(type_error(
        name,
        1,
        "double, single, complex, integer, logical, or char array",
        value,
    ))
}

fn numeric_element(
    name: &str,
    value: &Value,
    offset: usize,
) -> Result<NumericElement, BuiltinError> {
    let real = |value| NumericElement {
        real: value,
        imaginary: None,
    };
    match value {
        Value::Logical(value) if offset == 0 => {
            Ok(real(SourceComponent::Unsigned(u128::from(*value))))
        }
        Value::Double(value) if offset == 0 => Ok(real(SourceComponent::Float(*value))),
        Value::Complex(value) if offset == 0 => Ok(NumericElement {
            real: SourceComponent::Float(value.real),
            imaginary: Some(SourceComponent::Float(value.imaginary)),
        }),
        Value::Array(ArrayData::F32(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| real(SourceComponent::Float(f64::from(*value))))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| NumericElement {
                real: SourceComponent::Float(f64::from(value.re)),
                imaginary: Some(SourceComponent::Float(f64::from(value.im))),
            })
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| real(SourceComponent::Unsigned(u128::from(value.get()))))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::F64(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| real(SourceComponent::Float(*value)))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| NumericElement {
                real: SourceComponent::Float(value.re),
                imaginary: Some(SourceComponent::Float(value.im)),
            })
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::Char(array)) => array
            .as_slice()
            .get(offset)
            .map(|value| real(SourceComponent::Unsigned(u128::from(value.get()))))
            .ok_or_else(|| mapping_error(name)),
        Value::Array(ArrayData::Integer(array)) => array
            .element(offset)
            .map(|value| NumericElement {
                real: source_integer(value.real_component()),
                imaginary: value.imaginary_component().map(source_integer),
            })
            .ok_or_else(|| mapping_error(name)),
        _ => Err(conversion_error(name)),
    }
}

fn numeric_scalar_element(value: &Value) -> Option<NumericElement> {
    if value.numel() != Some(1) {
        return None;
    }
    numeric_element("numeric scalar", value, 0).ok()
}

fn numeric_scalar_complex64(name: &str, value: &Value) -> Result<ArrayComplex64, BuiltinError> {
    let value = numeric_scalar_element(value).ok_or_else(|| conversion_error(name))?;
    Ok(ArrayComplex64::new(
        component_f64(value.real),
        value.imaginary.map_or(0.0, component_f64),
    ))
}

const fn source_integer(value: IntegerComponent) -> SourceComponent {
    match value {
        IntegerComponent::Signed(value) => SourceComponent::Signed(value),
        IntegerComponent::Unsigned(value) => SourceComponent::Unsigned(value),
    }
}

#[allow(clippy::cast_precision_loss)]
fn component_f64(value: SourceComponent) -> f64 {
    match value {
        SourceComponent::Signed(value) => value as f64,
        SourceComponent::Unsigned(value) => value as f64,
        SourceComponent::Float(value) => value,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn component_f32(value: SourceComponent) -> f32 {
    component_f64(value) as f32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn component_char(value: SourceComponent) -> u16 {
    match value {
        SourceComponent::Signed(value) => {
            u16::try_from(value).unwrap_or(if value < 0 { 0 } else { u16::MAX })
        }
        SourceComponent::Unsigned(value) => u16::try_from(value).unwrap_or(u16::MAX),
        SourceComponent::Float(value) if value.is_nan() || value <= 0.0 => 0,
        SourceComponent::Float(value) if !value.is_finite() || value >= f64::from(u16::MAX) => {
            u16::MAX
        }
        SourceComponent::Float(value) => value.trunc() as u16,
    }
}

macro_rules! impl_target_signed {
    ($kind:ty) => {
        impl TargetInteger for $kind {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            fn convert(value: SourceComponent) -> Self {
                let minimum = i128::from(<$kind>::MIN);
                let maximum = i128::from(<$kind>::MAX);
                let value = match value {
                    SourceComponent::Signed(value) => value.clamp(minimum, maximum),
                    SourceComponent::Unsigned(value) => {
                        i128::try_from(value).unwrap_or(maximum).min(maximum)
                    }
                    SourceComponent::Float(value) if value.is_nan() => 0,
                    SourceComponent::Float(value) if value <= minimum as f64 => minimum,
                    SourceComponent::Float(value) if value >= maximum as f64 => maximum,
                    SourceComponent::Float(value) => value.round() as i128,
                };
                value as Self
            }
        }
    };
}

macro_rules! impl_target_unsigned {
    ($kind:ty) => {
        impl TargetInteger for $kind {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            fn convert(value: SourceComponent) -> Self {
                let maximum = u128::from(<$kind>::MAX);
                let value = match value {
                    SourceComponent::Signed(value) => {
                        u128::try_from(value).unwrap_or(0).min(maximum)
                    }
                    SourceComponent::Unsigned(value) => value.min(maximum),
                    SourceComponent::Float(value) if value.is_nan() || value <= 0.0 => 0,
                    SourceComponent::Float(value) if value >= maximum as f64 => maximum,
                    SourceComponent::Float(value) => value.round() as u128,
                };
                value as Self
            }
        }
    };
}

impl_target_signed!(i8);
impl_target_unsigned!(u8);
impl_target_signed!(i16);
impl_target_unsigned!(u16);
impl_target_signed!(i32);
impl_target_unsigned!(u32);
impl_target_signed!(i64);
impl_target_unsigned!(u64);

fn compare_complex64(left: ArrayComplex64, right: ArrayComplex64) -> Ordering {
    left.re
        .hypot(left.im)
        .partial_cmp(&right.re.hypot(right.im))
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.im
                .atan2(left.re)
                .partial_cmp(&right.im.atan2(right.re))
                .unwrap_or(Ordering::Equal)
        })
}

fn compare_complex32(left: ArrayComplex32, right: ArrayComplex32) -> Ordering {
    left.re
        .hypot(left.im)
        .partial_cmp(&right.re.hypot(right.im))
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.im
                .atan2(left.re)
                .partial_cmp(&right.im.atan2(right.re))
                .unwrap_or(Ordering::Equal)
        })
}

const fn complex64_is_nan(value: ArrayComplex64) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

const fn complex32_is_nan(value: ArrayComplex32) -> bool {
    value.re.is_nan() || value.im.is_nan()
}

fn keyword(value: &Value) -> Option<String> {
    let units: Vec<u16> = match value {
        Value::String(value) => value.as_scalar()?.code_units().to_vec(),
        Value::Array(ArrayData::Char(array))
            if array.shape().ndims() == 2 && array.shape().extent(0) == 1 =>
        {
            array.as_slice().iter().map(|value| value.get()).collect()
        }
        _ => return None,
    };
    String::from_utf16(&units)
        .ok()
        .map(|value| value.to_ascii_lowercase())
}

fn axis_stride(name: &str, shape: &Shape, axis: usize) -> Result<usize, BuiltinError> {
    shape
        .dimensions()
        .iter()
        .take(axis)
        .try_fold(1_u64, |stride, extent| stride.checked_mul(*extent))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| mapping_error(name))
}

fn default_axis(dimensions: &[u64]) -> u64 {
    dimensions
        .iter()
        .position(|extent| *extent != 1)
        .and_then(|axis| u64::try_from(axis + 1).ok())
        .unwrap_or(1)
}

fn checked_shape(
    name: &str,
    dimensions: impl IntoIterator<Item = u64>,
) -> Result<Shape, BuiltinError> {
    Shape::new(dimensions).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` computed an invalid or overflowing shape: {error}"),
        )
    })
}

fn host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| mapping_error(name))
}

fn reserved_vec<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| allocation_error(name, length))?;
    Ok(output)
}

fn filled_vec<T: Clone>(name: &str, length: usize, value: T) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec(name, length)?;
    output.resize(length, value);
    Ok(output)
}

fn clone_slice<T: Copy>(name: &str, values: &[T]) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_vec(name, values.len())?;
    output.extend_from_slice(values);
    Ok(output)
}

fn map_owned<T, U>(
    name: &str,
    values: Vec<T>,
    mut map: impl FnMut(T) -> U,
) -> Result<Vec<U>, BuiltinError> {
    let mut output = reserved_vec(name, values.len())?;
    output.extend(values.into_iter().map(&mut map));
    Ok(output)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
    }
    Ok(())
}

fn argument_count_error(name: &str, detail: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::ArgumentCount,
        format!("invalid argument count for `{name}`: {detail}"),
    )
}

fn domain_error(name: &str, detail: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("invalid `{name}` request: {detail}"),
    )
}

fn option_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` received an unsupported or malformed option"),
    )
}

fn window_error(name: &str) -> BuiltinError {
    domain_error(
        name,
        "the window must be a positive scalar or a nonnegative [nb nf] pair",
    )
}

fn conversion_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("`{name}` cannot convert all inputs to the MATLAB-selected output class"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked column-major mapping"),
    )
}

fn allocation_error(name: &str, length: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` cannot allocate {length} result elements"),
    )
}

#[cfg(test)]
#[path = "core_array_windows_tests.rs"]
mod focused_tests;

#[cfg(test)]
mod tests {
    use openmat_array::{ArrayData, CharCodeUnit, DenseArray, IntegerArrayData, Shape};
    use openmat_runtime::{BuiltinContext, BuiltinErrorCategory, CancellationToken, VecOutput};
    use openmat_value::Value;

    use super::*;

    fn invoke(
        function: fn(&[Value], &mut BuiltinContext<'_>) -> BuiltinResult,
        arguments: &[Value],
        outputs: usize,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        let mut sink = VecOutput::new();
        let mut context = BuiltinContext::new(outputs, &cancellation, &mut sink);
        function(arguments, &mut context)
    }

    fn real_array(dimensions: impl IntoIterator<Item = u64>, values: Vec<f64>) -> Value {
        Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        ))
    }

    fn char_row(value: &str) -> Value {
        let values = value
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(
                Shape::new([1, u64::try_from(values.len()).unwrap()]).unwrap(),
                values,
            )
            .unwrap(),
        ))
    }

    fn uint8_row(values: Vec<u8>) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            DenseArray::from_vec(
                Shape::new([1, u64::try_from(values.len()).unwrap()]).unwrap(),
                values,
            )
            .unwrap(),
        )))
    }

    fn only(result: BuiltinResult) -> Value {
        let mut values = result.unwrap();
        assert_eq!(values.len(), 1);
        values.remove(0)
    }

    #[test]
    fn rearrangement_matches_column_major_nd_mappings_and_identity_cow() {
        let input = real_array([2, 3, 2], (1..=12).map(f64::from).collect());
        let order = real_array([1, 3], vec![3.0, 1.0, 2.0]);
        let output = only(invoke(ipermute_builtin, &[input.clone(), order], 1));
        let Value::Array(ArrayData::F64(array)) = output else {
            panic!("ipermute output class");
        };
        assert_eq!(array.shape().dimensions(), &[3, 2, 2]);
        assert_eq!(
            array.as_slice(),
            &[
                1.0, 3.0, 5.0, 7.0, 9.0, 11.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0
            ]
        );

        let rotated = only(invoke(rot90_builtin, std::slice::from_ref(&input), 1));
        let Value::Array(ArrayData::F64(rotated)) = rotated else {
            panic!("rot90 output class");
        };
        assert_eq!(rotated.shape().dimensions(), &[3, 2, 2]);
        assert_eq!(
            rotated.as_slice(),
            &[
                5.0, 3.0, 1.0, 6.0, 4.0, 2.0, 11.0, 9.0, 7.0, 12.0, 10.0, 8.0
            ]
        );

        let identity = only(invoke(
            rot90_builtin,
            &[input.clone(), Value::Double(4.0)],
            1,
        ));
        assert!(identity.shares_array_storage_with(&input));
    }

    #[test]
    fn shiftdim_and_repelem_cover_default_explicit_empty_and_classes() {
        let source = real_array([1, 1, 2, 3], (1..=6).map(f64::from).collect());
        let shifted = invoke(shiftdim_builtin, std::slice::from_ref(&source), 2).unwrap();
        assert_eq!(shifted.len(), 2);
        assert_eq!(shifted[0].dimensions(), Some([2, 3].as_slice()));
        assert_eq!(shifted[1], Value::Double(2.0));
        let inserted = only(invoke(
            shiftdim_builtin,
            &[shifted[0].clone(), Value::Double(-2.0)],
            1,
        ));
        assert_eq!(inserted.dimensions(), Some([1, 1, 2, 3].as_slice()));

        let repeated = only(invoke(
            repelem_builtin,
            &[
                uint8_row(vec![1, 2, 3]),
                real_array([1, 3], vec![1.0, 0.0, 2.0]),
            ],
            1,
        ));
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = repeated else {
            panic!("repelem must retain uint8");
        };
        assert_eq!(array.shape().dimensions(), &[1, 3]);
        assert_eq!(array.as_slice(), &[1, 3, 3]);

        let empty = only(invoke(
            repelem_builtin,
            &[real_array([1, 0], Vec::new()), Value::Double(3.0)],
            1,
        ));
        assert_eq!(empty.dimensions(), Some([1, 0].as_slice()));
    }

    #[test]
    fn blkdiag_uses_r2022b_mixed_class_dominance_and_checked_shape() {
        let mixed = only(invoke(
            blkdiag_builtin,
            &[Value::Double(1.0), uint8_row(vec![2])],
            1,
        ));
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = mixed else {
            panic!("integer input must dominate double");
        };
        assert_eq!(array.shape().dimensions(), &[2, 2]);
        assert_eq!(array.as_slice(), &[1, 0, 0, 2]);

        let chars = only(invoke(
            blkdiag_builtin,
            &[char_row("ab"), Value::Logical(true)],
            1,
        ));
        let Value::Array(ArrayData::Char(chars)) = chars else {
            panic!("leading char must dominate logical");
        };
        assert_eq!(chars.shape().dimensions(), &[2, 3]);
        assert_eq!(
            chars
                .as_slice()
                .iter()
                .map(|value| value.get())
                .collect::<Vec<_>>(),
            [97, 0, 98, 0, 0, 1]
        );
        let error = invoke(blkdiag_builtin, &[Value::Logical(true), char_row("a")], 1).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Type);
    }

    #[test]
    fn cumulative_extrema_match_nan_direction_complex_and_output_arity() {
        let values = real_array([1, 5], vec![3.0, f64::NAN, 2.0, f64::NAN, 1.0]);
        let output = only(invoke(cummin_builtin, std::slice::from_ref(&values), 1));
        let Value::Array(ArrayData::F64(output)) = output else {
            panic!("cummin output class");
        };
        assert_eq!(output.as_slice(), &[3.0, 3.0, 2.0, 2.0, 1.0]);

        let included = only(invoke(
            cummin_builtin,
            &[values, Value::from("includenan")],
            1,
        ));
        let Value::Array(ArrayData::F64(included)) = included else {
            panic!("cummin included output class");
        };
        assert_eq!(included.as_slice()[0].to_bits(), 3.0_f64.to_bits());
        assert!(included.as_slice()[1..].iter().all(|value| value.is_nan()));

        let reverse = only(invoke(
            cummax_builtin,
            &[
                real_array([1, 5], vec![1.0, 3.0, 3.0, 2.0, 3.0]),
                Value::from("reverse"),
            ],
            1,
        ));
        let Value::Array(ArrayData::F64(reverse)) = reverse else {
            panic!("cummax reverse class");
        };
        assert_eq!(reverse.as_slice(), &[3.0; 5]);
        let error = invoke(cummin_builtin, &[Value::Double(1.0)], 2).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::ArgumentCount);
    }

    #[test]
    fn moving_windows_cover_alignment_endpoints_nanflags_and_native_extrema() {
        let values = real_array([1, 6], (1..=6).map(f64::from).collect());
        let sum = only(invoke(
            movsum_builtin,
            &[values.clone(), Value::Double(4.0)],
            1,
        ));
        let Value::Array(ArrayData::F64(sum)) = sum else {
            panic!("movsum class");
        };
        assert_eq!(sum.as_slice(), &[3.0, 6.0, 10.0, 14.0, 18.0, 15.0]);

        let discard = only(invoke(
            movsum_builtin,
            &[
                values.clone(),
                Value::Double(3.0),
                Value::from("Endpoints"),
                Value::from("discard"),
            ],
            1,
        ));
        let Value::Array(ArrayData::F64(discard)) = discard else {
            panic!("discard class");
        };
        assert_eq!(discard.shape().dimensions(), &[1, 4]);
        assert_eq!(discard.as_slice(), &[6.0, 9.0, 12.0, 15.0]);

        let fill = only(invoke(
            movmean_builtin,
            &[
                values,
                Value::Double(4.0),
                Value::from("Endpoints"),
                Value::Double(0.0),
            ],
            1,
        ));
        let Value::Array(ArrayData::F64(fill)) = fill else {
            panic!("custom fill class");
        };
        assert_eq!(fill.as_slice(), &[0.75, 1.5, 2.5, 3.5, 4.5, 3.75]);

        let native = only(invoke(
            movmin_builtin,
            &[uint8_row(vec![3, 1, 2]), Value::Double(3.0)],
            1,
        ));
        let Value::Array(ArrayData::Integer(IntegerArrayData::U8(native))) = native else {
            panic!("movmin must retain uint8");
        };
        assert_eq!(native.as_slice(), &[1, 1, 1]);
    }

    #[test]
    fn moving_windows_reject_char_samplepoints_and_observe_cancellation() {
        let char_error =
            invoke(movsum_builtin, &[char_row("abc"), Value::Double(3.0)], 1).unwrap_err();
        assert_eq!(char_error.category, BuiltinErrorCategory::Type);
        let sample_error = invoke(
            movsum_builtin,
            &[
                real_array([1, 3], vec![1.0, 2.0, 3.0]),
                Value::Double(3.0),
                Value::from("SamplePoints"),
                real_array([1, 3], vec![1.0, 2.0, 3.0]),
            ],
            1,
        )
        .unwrap_err();
        assert_eq!(sample_error.category, BuiltinErrorCategory::Domain);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut sink = VecOutput::new();
        let mut context = BuiltinContext::new(1, &cancellation, &mut sink);
        let error = movmax_builtin(
            &[real_array([1, 3], vec![1.0, 2.0, 3.0]), Value::Double(3.0)],
            &mut context,
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
    }
}
