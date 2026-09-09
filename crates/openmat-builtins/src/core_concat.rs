use openmat_array::{
    ArrayData, CharCodeUnit, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, DenseArray,
    IntegerArrayData, IntegerComponent, IntegerElement, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{SparseArrayData, SparseError, Value};

use crate::{
    U64_EXCLUSIVE_UPPER_BOUND, array_error, exact_real_integer_scalar, expect_max_outputs,
    type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

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
enum TargetClass {
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

struct ConcatInput<'a> {
    value: &'a Value,
    dimensions: Vec<u64>,
    axis_start: u64,
    axis_end: u64,
}

struct ConcatPlan<'a> {
    inputs: Vec<ConcatInput<'a>>,
    output_shape: Shape,
    output_dimensions: Vec<u64>,
    axis: usize,
}

trait TargetInteger: IntegerElement + Copy {
    fn convert(value: SourceComponent) -> Self;
}

pub(super) fn cat_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if arguments.is_empty() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`cat` expects at least a dimension input",
        ));
    }
    let dimension = concat_dimension(&arguments[0])?;
    concatenate("cat", dimension, &arguments[1..], context)
}

pub(super) fn horzcat_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    concatenate("horzcat", 2, arguments, context)
}

pub(super) fn vertcat_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    concatenate("vertcat", 1, arguments, context)
}

fn concatenate(
    name: &str,
    dimension: u64,
    inputs: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    if inputs.is_empty() {
        return Ok(vec![Value::empty_double()]);
    }
    if inputs.iter().any(|value| matches!(value, Value::Sparse(_))) {
        if inputs.len() == 1 {
            return Ok(vec![inputs[0].clone()]);
        }
        return concatenate_sparse(name, dimension, inputs, context).map(|value| vec![value]);
    }
    validate_inputs(name, inputs)?;
    if inputs.len() == 1 {
        return Ok(vec![inputs[0].clone()]);
    }
    let plan = concat_plan(name, dimension, inputs, context)?;
    let target = target_class(name, inputs)?;
    let output = match target {
        TargetClass::Logical => concat_logical(name, &plan, context)?,
        TargetClass::Double { complex } => concat_double(name, &plan, complex, context)?,
        TargetClass::Single { complex } => concat_single(name, &plan, complex, context)?,
        TargetClass::Char => concat_char(name, &plan, context)?,
        TargetClass::Integer(kind) => concat_integer(name, &plan, kind, context)?,
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn concatenate_sparse(
    name: &str,
    dimension: u64,
    inputs: &[Value],
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let axis = usize::try_from(dimension - 1).map_err(|_| dimension_error())?;
    let mut sparse_inputs = reserved_vec(name, inputs.len())?;
    for (index, value) in inputs.iter().enumerate() {
        check_cancelled_at(context, index)?;
        sparse_inputs.push(sparse_concat_input(name, index + 1, value, context)?);
    }
    SparseArrayData::try_concatenate_2d(&sparse_inputs, axis, Some(context.cancellation_flag()))
        .map(Value::Sparse)
        .map_err(sparse_concat_error)
}

fn sparse_concat_input(
    name: &str,
    argument: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<SparseArrayData, BuiltinError> {
    match value {
        Value::Sparse(sparse) => Ok(sparse.clone()),
        Value::Logical(value) => {
            let array = DenseArray::from_vec(
                Shape::new([1, 1]).map_err(|error| array_error(&error))?,
                vec![Logical::from(*value)],
            )
            .map(ArrayData::Logical)
            .map_err(|error| array_error(&error))?;
            sparse_from_dense(&array, context)
        }
        Value::Double(value) => {
            let array = DenseArray::from_vec(
                Shape::new([1, 1]).map_err(|error| array_error(&error))?,
                vec![*value],
            )
            .map(ArrayData::F64)
            .map_err(|error| array_error(&error))?;
            sparse_from_dense(&array, context)
        }
        Value::Complex(value) => {
            let array = DenseArray::from_vec(
                Shape::new([1, 1]).map_err(|error| array_error(&error))?,
                vec![ArrayComplex64::new(value.real, value.imaginary)],
            )
            .map(ArrayData::ComplexF64)
            .map_err(|error| array_error(&error))?;
            sparse_from_dense(&array, context)
        }
        Value::Array(
            array @ (ArrayData::Logical(_) | ArrayData::F64(_) | ArrayData::ComplexF64(_)),
        ) => sparse_from_dense(array, context),
        other => Err(type_error(
            name,
            argument,
            "sparse-compatible logical or double matrix",
            other,
        )),
    }
}

fn sparse_from_dense(
    array: &ArrayData,
    context: &BuiltinContext<'_>,
) -> Result<SparseArrayData, BuiltinError> {
    SparseArrayData::try_from_dense(array, Some(context.cancellation_flag()))
        .map_err(sparse_concat_error)
}

fn sparse_concat_error(error: SparseError) -> BuiltinError {
    match error {
        SparseError::Cancelled => BuiltinError::new(
            BuiltinErrorCategory::Cancelled,
            SparseError::Cancelled.to_string(),
        ),
        other => BuiltinError::new(BuiltinErrorCategory::Domain, other.to_string()),
    }
}

fn validate_inputs(name: &str, inputs: &[Value]) -> Result<(), BuiltinError> {
    for (index, value) in inputs.iter().enumerate() {
        if !matches!(
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
            return Err(type_error(
                name,
                index + 1,
                "numeric, logical, char, or integer array",
                value,
            ));
        }
    }
    Ok(())
}

fn target_class(name: &str, inputs: &[Value]) -> Result<TargetClass, BuiltinError> {
    let has_char = inputs
        .iter()
        .any(|value| matches!(value, Value::Array(ArrayData::Char(_))));
    let has_logical = inputs.iter().any(|value| {
        matches!(
            value,
            Value::Logical(_) | Value::Array(ArrayData::Logical(_))
        )
    });
    let complex = inputs.iter().any(Value::is_complex_numeric);
    if has_char {
        if has_logical || complex {
            return Err(conversion_error(name));
        }
        return Ok(TargetClass::Char);
    }
    if let Some(kind) = inputs.iter().find_map(integer_kind) {
        if complex {
            return Err(conversion_error(name));
        }
        return Ok(TargetClass::Integer(kind));
    }
    if inputs.iter().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        return Ok(TargetClass::Single { complex });
    }
    if inputs.iter().any(|value| {
        matches!(
            value,
            Value::Double(_)
                | Value::Complex(_)
                | Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_))
        )
    }) {
        return Ok(TargetClass::Double { complex });
    }
    Ok(TargetClass::Logical)
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

fn concat_plan<'a>(
    name: &str,
    dimension: u64,
    values: &'a [Value],
    context: &BuiltinContext<'_>,
) -> Result<ConcatPlan<'a>, BuiltinError> {
    let axis = usize::try_from(dimension - 1).map_err(|_| dimension_error())?;
    let rank = values
        .iter()
        .filter_map(Value::dimensions)
        .map(<[u64]>::len)
        .max()
        .unwrap_or(2)
        .max(axis + 1);
    let mut output_dimensions = filled_dimensions(name, rank, 1, context)?;
    let first_dimensions = values[0].dimensions().ok_or_else(|| mapping_error(name))?;
    for (index, output) in output_dimensions.iter_mut().enumerate() {
        *output = first_dimensions.get(index).copied().unwrap_or(1);
    }
    output_dimensions[axis] = 0;
    let mut inputs = reserved_vec(name, values.len())?;
    let mut axis_start = 0_u64;
    for (input_index, value) in values.iter().enumerate() {
        check_cancelled_at(context, input_index)?;
        let source = value.dimensions().ok_or_else(|| mapping_error(name))?;
        let mut dimensions = filled_dimensions(name, rank, 1, context)?;
        for (index, extent) in source.iter().copied().enumerate() {
            dimensions[index] = extent;
        }
        for (index, extent) in dimensions.iter().copied().enumerate() {
            if index != axis && extent != output_dimensions[index] {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!(
                        "input {} to `{name}` has an incompatible extent",
                        input_index + 1
                    ),
                ));
            }
        }
        let axis_end = axis_start
            .checked_add(dimensions[axis])
            .ok_or_else(|| mapping_error(name))?;
        inputs.push(ConcatInput {
            value,
            dimensions,
            axis_start,
            axis_end,
        });
        axis_start = axis_end;
    }
    output_dimensions[axis] = axis_start;
    let output_shape =
        Shape::new(output_dimensions.clone()).map_err(|error| array_error(&error))?;
    Ok(ConcatPlan {
        inputs,
        output_shape,
        output_dimensions,
        axis,
    })
}

fn concat_logical(
    name: &str,
    plan: &ConcatPlan<'_>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let values = concat_map(name, plan, context, |value, offset| match value {
        Value::Logical(value) if offset == 0 => Ok(Logical::from(*value)),
        Value::Array(ArrayData::Logical(array)) => array
            .as_slice()
            .get(offset)
            .copied()
            .ok_or_else(|| mapping_error(name)),
        _ => Err(conversion_error(name)),
    })?;
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn concat_double(
    name: &str,
    plan: &ConcatPlan<'_>,
    complex: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if complex {
        let values = concat_map(name, plan, context, |value, offset| {
            let value = numeric_element(name, value, offset)?;
            Ok(ArrayComplex64::new(
                component_f64(value.real),
                value.imaginary.map_or(0.0, component_f64),
            ))
        })?;
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let values = concat_map(name, plan, context, |value, offset| {
            let value = numeric_element(name, value, offset)?;
            if value.imaginary.is_some() {
                return Err(conversion_error(name));
            }
            Ok(component_f64(value.real))
        })?;
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn concat_single(
    name: &str,
    plan: &ConcatPlan<'_>,
    complex: bool,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    if complex {
        let values = concat_map(name, plan, context, |value, offset| {
            let value = numeric_element(name, value, offset)?;
            Ok(ArrayComplex32::new(
                component_f32(value.real),
                value.imaginary.map_or(0.0, component_f32),
            ))
        })?;
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::ComplexF32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        let values = concat_map(name, plan, context, |value, offset| {
            let value = numeric_element(name, value, offset)?;
            if value.imaginary.is_some() {
                return Err(conversion_error(name));
            }
            Ok(component_f32(value.real))
        })?;
        DenseArray::from_vec(plan.output_shape.clone(), values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    }
}

fn concat_char(
    name: &str,
    plan: &ConcatPlan<'_>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    let values = concat_map(name, plan, context, |value, offset| {
        if let Value::Array(ArrayData::Char(array)) = value {
            return array
                .as_slice()
                .get(offset)
                .copied()
                .ok_or_else(|| mapping_error(name));
        }
        let value = numeric_element(name, value, offset)?;
        if value.imaginary.is_some() {
            return Err(conversion_error(name));
        }
        Ok(CharCodeUnit::new(component_char(value.real)))
    })?;
    DenseArray::from_vec(plan.output_shape.clone(), values)
        .map(ArrayData::Char)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn concat_integer(
    name: &str,
    plan: &ConcatPlan<'_>,
    kind: IntegerKind,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    macro_rules! concatenate_integer {
        ($kind:ty) => {{
            let values = concat_map(name, plan, context, |value, offset| {
                let value = numeric_element(name, value, offset)?;
                if value.imaginary.is_some() {
                    return Err(conversion_error(name));
                }
                Ok(<$kind as TargetInteger>::convert(value.real))
            })?;
            DenseArray::from_vec(plan.output_shape.clone(), values)
                .map(IntegerArrayData::from_typed)
                .map(ArrayData::Integer)
                .map(Value::Array)
                .map_err(|error| array_error(&error))
        }};
    }
    match kind {
        IntegerKind::I8 => concatenate_integer!(i8),
        IntegerKind::U8 => concatenate_integer!(u8),
        IntegerKind::I16 => concatenate_integer!(i16),
        IntegerKind::U16 => concatenate_integer!(u16),
        IntegerKind::I32 => concatenate_integer!(i32),
        IntegerKind::U32 => concatenate_integer!(u32),
        IntegerKind::I64 => concatenate_integer!(i64),
        IntegerKind::U64 => concatenate_integer!(u64),
    }
}

fn concat_map<T, F>(
    name: &str,
    plan: &ConcatPlan<'_>,
    context: &BuiltinContext<'_>,
    mut convert: F,
) -> Result<Vec<T>, BuiltinError>
where
    F: FnMut(&Value, usize) -> Result<T, BuiltinError>,
{
    let length = usize::try_from(plan.output_shape.numel()).map_err(|_| mapping_error(name))?;
    let mut output = reserved_vec(name, length)?;
    let mut coordinates = filled_dimensions(name, plan.output_dimensions.len(), 0, context)?;
    for output_offset in 0..length {
        check_cancelled_at(context, output_offset)?;
        let mut remainder = u64::try_from(output_offset).map_err(|_| mapping_error(name))?;
        for (axis, extent) in plan.output_dimensions.iter().copied().enumerate() {
            coordinates[axis] = if extent == 0 { 0 } else { remainder % extent };
            if extent != 0 {
                remainder /= extent;
            }
        }
        let axis_coordinate = coordinates[plan.axis];
        let input = plan
            .inputs
            .iter()
            .find(|input| axis_coordinate >= input.axis_start && axis_coordinate < input.axis_end)
            .ok_or_else(|| mapping_error(name))?;
        coordinates[plan.axis] = axis_coordinate - input.axis_start;
        let input_offset = coordinates
            .iter()
            .copied()
            .zip(input.dimensions.iter().copied())
            .try_fold((0_u64, 1_u64), |(offset, stride), (coordinate, extent)| {
                let offset = coordinate
                    .checked_mul(stride)
                    .and_then(|value| offset.checked_add(value))?;
                Some((offset, stride.checked_mul(extent)?))
            })
            .map(|(offset, _)| offset)
            .ok_or_else(|| mapping_error(name))?;
        let input_offset = usize::try_from(input_offset).map_err(|_| mapping_error(name))?;
        output.push(convert(input.value, input_offset)?);
    }
    Ok(output)
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

fn concat_dimension(value: &Value) -> Result<u64, BuiltinError> {
    if matches!(value, Value::Logical(_) | Value::Complex(_)) {
        return Err(dimension_error());
    }
    if let Some(value) = exact_real_integer_scalar(value) {
        return match value {
            IntegerComponent::Signed(value) => u64::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => u64::try_from(value).ok(),
        }
        .filter(|value| *value > 0)
        .ok_or_else(dimension_error);
    }
    let value = match value {
        Value::Double(value) => Some(*value),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Some(array.as_slice()[0]),
        _ => value.as_real_single().map(f64::from),
    }
    .ok_or_else(dimension_error)?;
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(dimension_error());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn filled_dimensions(
    name: &str,
    length: usize,
    value: u64,
    context: &BuiltinContext<'_>,
) -> Result<Vec<u64>, BuiltinError> {
    let mut output = reserved_vec(name, length)?;
    while output.len() < length {
        context.check_cancelled()?;
        let target = output
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        output.resize(target, value);
    }
    Ok(output)
}

fn reserved_vec<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate {length} elements"),
        )
    })?;
    Ok(output)
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn dimension_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "the concatenation dimension must be a positive real integer scalar",
    )
}

fn conversion_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("`{name}` cannot convert all inputs to the dominant concatenation class"),
    )
}

fn mapping_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked column-major mapping"),
    )
}
