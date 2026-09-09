use openmat_array::{
    ArrayData, ArrayElement, CharCodeUnit, Complex32, Complex64 as ArrayComplex64, ComplexInteger,
    DenseArray, IntegerArrayData, IntegerComponent, IntegerElement, IntegerElementValue, Logical,
    Shape,
};
use openmat_bytecode::BinaryOperator;
use openmat_linalg::{
    LinalgProvider, MatrixFunctionRequest, matrix_power_complex32, matrix_power_complex64,
    matrix_power_f32, matrix_power_f64,
};
use openmat_value::{
    Complex64, ConditionEvaluation, SparseArrayData, SparseError, StringArray, StringElement,
    StringValue, Value,
};

use crate::{
    ArrayRuntimeError, CancellationToken, IndexErrorKind, RangeErrorKind, RuntimeErrorKind,
    error::array_error, linalg_matrix_left_divide, linalg_matrix_multiply,
    linalg_matrix_right_divide, linalg_ops::RuntimeLinalgError,
};

const SCALAR_DIMENSIONS: [u64; 2] = [1, 1];
const CANCELLATION_INTERVAL: usize = 1_024;
const I64_MIN_AS_F64: f64 = -9_223_372_036_854_775_808.0;
const U64_EXCLUSIVE_LIMIT_AS_F64: f64 = 18_446_744_073_709_551_616.0;

macro_rules! dispatch_integer_array {
    ($value:expr, |$array:ident| $body:expr) => {
        match $value {
            IntegerArrayData::I8($array) => $body,
            IntegerArrayData::ComplexI8($array) => $body,
            IntegerArrayData::U8($array) => $body,
            IntegerArrayData::ComplexU8($array) => $body,
            IntegerArrayData::I16($array) => $body,
            IntegerArrayData::ComplexI16($array) => $body,
            IntegerArrayData::U16($array) => $body,
            IntegerArrayData::ComplexU16($array) => $body,
            IntegerArrayData::I32($array) => $body,
            IntegerArrayData::ComplexI32($array) => $body,
            IntegerArrayData::U32($array) => $body,
            IntegerArrayData::ComplexU32($array) => $body,
            IntegerArrayData::I64($array) => $body,
            IntegerArrayData::ComplexI64($array) => $body,
            IntegerArrayData::U64($array) => $body,
            IntegerArrayData::ComplexU64($array) => $body,
        }
    };
}

#[derive(Clone)]
pub(crate) enum IndexInput {
    Value(Value),
    Colon,
}

enum NumericView<'a> {
    LogicalScalar(bool),
    RealScalar(f64),
    ComplexScalar(Complex64),
    LogicalArray(&'a DenseArray<Logical>),
    RealArray(&'a DenseArray<f64>),
    ComplexArray(&'a DenseArray<ArrayComplex64>),
}

struct BroadcastPlan {
    shape: Shape,
    lhs_strides: Vec<u64>,
    rhs_strides: Vec<u64>,
}

impl BroadcastPlan {
    fn new(
        operator: BinaryOperator,
        lhs_dimensions: &[u64],
        rhs_dimensions: &[u64],
    ) -> Result<Self, RuntimeErrorKind> {
        let rank = lhs_dimensions.len().max(rhs_dimensions.len());
        let mut dimensions = Vec::with_capacity(rank);
        let mut lhs_strides = Vec::with_capacity(rank);
        let mut rhs_strides = Vec::with_capacity(rank);
        let mut lhs_stride = 1_u64;
        let mut rhs_stride = 1_u64;

        for dimension in 0..rank {
            let lhs_extent = lhs_dimensions.get(dimension).copied().unwrap_or(1);
            let rhs_extent = rhs_dimensions.get(dimension).copied().unwrap_or(1);
            let output_extent = if lhs_extent == rhs_extent {
                lhs_extent
            } else if lhs_extent == 1 {
                rhs_extent
            } else if rhs_extent == 1 {
                lhs_extent
            } else {
                return Err(array_error(ArrayRuntimeError::ShapeMismatch {
                    operation: operator_name(operator),
                    lhs: lhs_dimensions.to_vec(),
                    rhs: rhs_dimensions.to_vec(),
                }));
            };

            dimensions.push(output_extent);
            lhs_strides.push(if lhs_extent == 1 { 0 } else { lhs_stride });
            rhs_strides.push(if rhs_extent == 1 { 0 } else { rhs_stride });
            lhs_stride = lhs_stride
                .checked_mul(lhs_extent)
                .ok_or_else(array_size_error)?;
            rhs_stride = rhs_stride
                .checked_mul(rhs_extent)
                .ok_or_else(array_size_error)?;
        }

        let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
        Ok(Self {
            shape,
            lhs_strides,
            rhs_strides,
        })
    }

    fn offsets(&self, output_offset: usize) -> Result<(usize, usize), RuntimeErrorKind> {
        let mut remainder = u64::try_from(output_offset).map_err(|_| array_size_error())?;
        let mut lhs_offset = 0_u64;
        let mut rhs_offset = 0_u64;
        for (dimension, extent) in self.shape.dimensions().iter().copied().enumerate() {
            debug_assert!(extent != 0, "empty broadcasts never map an output element");
            let coordinate = remainder % extent;
            remainder /= extent;
            lhs_offset = lhs_offset
                .checked_add(
                    coordinate
                        .checked_mul(self.lhs_strides[dimension])
                        .ok_or_else(array_size_error)?,
                )
                .ok_or_else(array_size_error)?;
            rhs_offset = rhs_offset
                .checked_add(
                    coordinate
                        .checked_mul(self.rhs_strides[dimension])
                        .ok_or_else(array_size_error)?,
                )
                .ok_or_else(array_size_error)?;
        }
        Ok((checked_usize(lhs_offset)?, checked_usize(rhs_offset)?))
    }
}

enum ArrayView<'a> {
    Numeric(NumericView<'a>),
    SingleReal(&'a DenseArray<f32>),
    SingleComplex(&'a DenseArray<Complex32>),
    Char(&'a DenseArray<CharCodeUnit>),
    Integer(&'a IntegerArrayData),
    String(&'a StringValue),
}

impl<'a> ArrayView<'a> {
    fn new(value: &'a Value, operation: &'static str) -> Result<Self, RuntimeErrorKind> {
        match value {
            Value::Array(ArrayData::F32(value)) => Ok(Self::SingleReal(value)),
            Value::Array(ArrayData::ComplexF32(value)) => Ok(Self::SingleComplex(value)),
            Value::Array(ArrayData::Char(array)) => Ok(Self::Char(array)),
            Value::Array(ArrayData::Integer(array)) => Ok(Self::Integer(array)),
            Value::String(value) => Ok(Self::String(value)),
            _ => NumericView::new(value, operation).map(Self::Numeric),
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::Numeric(value) => value.dimensions(),
            Self::SingleReal(value) => value.shape().dimensions(),
            Self::SingleComplex(value) => value.shape().dimensions(),
            Self::Char(value) => value.shape().dimensions(),
            Self::Integer(value) => value.shape().dimensions(),
            Self::String(value) => value.dimensions(),
        }
    }
}

impl<'a> NumericView<'a> {
    fn new(value: &'a Value, operation: &'static str) -> Result<Self, RuntimeErrorKind> {
        match value {
            Value::Logical(value) => Ok(Self::LogicalScalar(*value)),
            Value::Double(value) => Ok(Self::RealScalar(*value)),
            Value::Complex(value) => Ok(Self::ComplexScalar(*value)),
            Value::Array(ArrayData::Logical(array)) => Ok(Self::LogicalArray(array)),
            Value::Array(ArrayData::F64(array)) => Ok(Self::RealArray(array)),
            Value::Array(ArrayData::ComplexF64(array)) => Ok(Self::ComplexArray(array)),
            other => Err(array_error(ArrayRuntimeError::InvalidOperand {
                operation,
                actual: other.kind(),
            })),
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::LogicalScalar(_) | Self::RealScalar(_) | Self::ComplexScalar(_) => {
                &SCALAR_DIMENSIONS
            }
            Self::LogicalArray(array) => array.shape().dimensions(),
            Self::RealArray(array) => array.shape().dimensions(),
            Self::ComplexArray(array) => array.shape().dimensions(),
        }
    }

    fn numel(&self) -> u64 {
        match self {
            Self::LogicalScalar(_) | Self::RealScalar(_) | Self::ComplexScalar(_) => 1,
            Self::LogicalArray(array) => array.numel(),
            Self::RealArray(array) => array.numel(),
            Self::ComplexArray(array) => array.numel(),
        }
    }

    fn is_complex(&self) -> bool {
        matches!(self, Self::ComplexScalar(_) | Self::ComplexArray(_))
    }

    fn is_logical(&self) -> bool {
        matches!(self, Self::LogicalScalar(_) | Self::LogicalArray(_))
    }

    fn complex_at(&self, offset: usize) -> Option<Complex64> {
        match self {
            Self::LogicalScalar(value) if offset == 0 => {
                Some(Complex64::new(f64::from(*value), 0.0))
            }
            Self::RealScalar(value) if offset == 0 => Some(Complex64::new(*value, 0.0)),
            Self::ComplexScalar(value) if offset == 0 => Some(*value),
            Self::LogicalArray(array) => array
                .as_slice()
                .get(offset)
                .map(|value| Complex64::new(f64::from(value.get()), 0.0)),
            Self::RealArray(array) => array
                .as_slice()
                .get(offset)
                .map(|value| Complex64::new(*value, 0.0)),
            Self::ComplexArray(array) => array
                .as_slice()
                .get(offset)
                .map(|value| Complex64::new(value.re, value.im)),
            _ => None,
        }
    }

    fn real_at(&self, offset: usize) -> Option<f64> {
        let value = self.complex_at(offset)?;
        (value.imaginary == 0.0).then_some(value.real)
    }

    fn logical_at(&self, offset: usize) -> Option<bool> {
        match self {
            Self::LogicalScalar(value) if offset == 0 => Some(*value),
            Self::LogicalArray(array) => array.as_slice().get(offset).map(|value| value.get()),
            _ => None,
        }
    }
}

enum SingleMatrixView<'a> {
    SingleReal(&'a DenseArray<f32>),
    SingleComplex(&'a DenseArray<Complex32>),
    Numeric(NumericView<'a>),
}

impl<'a> SingleMatrixView<'a> {
    fn new(value: &'a Value) -> Result<Self, RuntimeErrorKind> {
        match value {
            Value::Array(ArrayData::F32(value)) => Ok(Self::SingleReal(value)),
            Value::Array(ArrayData::ComplexF32(value)) => Ok(Self::SingleComplex(value)),
            _ => NumericView::new(value, "single matrix concatenation").map(Self::Numeric),
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::SingleReal(value) => value.shape().dimensions(),
            Self::SingleComplex(value) => value.shape().dimensions(),
            Self::Numeric(value) => value.dimensions(),
        }
    }

    fn is_complex(&self) -> bool {
        match self {
            Self::SingleReal(_) => false,
            Self::SingleComplex(_) => true,
            // R2022b converts an empty complex double operand to an empty real
            // single during mixed concatenation. Exact complex-single storage
            // retains its complexness even when empty.
            Self::Numeric(value) => value.is_complex() && value.numel() != 0,
        }
    }

    fn complex_at(&self, offset: usize) -> Option<Complex32> {
        match self {
            Self::SingleReal(array) => array.as_slice().get(offset).copied().map(Complex32::from),
            Self::SingleComplex(array) => array.as_slice().get(offset).copied(),
            Self::Numeric(value) => value.complex_at(offset).map(|value| {
                Complex32::new(
                    single_from_double(value.real),
                    single_from_double(value.imaginary),
                )
            }),
        }
    }

    fn real_at(&self, offset: usize) -> Option<f32> {
        match self {
            Self::SingleReal(array) => array.as_slice().get(offset).copied(),
            Self::SingleComplex(array) => array
                .as_slice()
                .get(offset)
                .filter(|value| value.im == 0.0)
                .map(|value| value.re),
            Self::Numeric(value) => value.real_at(offset).map(single_from_double),
        }
    }
}

enum SingleBinaryView<'a> {
    SingleReal(&'a DenseArray<f32>),
    SingleComplex(&'a DenseArray<Complex32>),
    Numeric(NumericView<'a>),
}

impl<'a> SingleBinaryView<'a> {
    fn new(value: &'a Value) -> Option<Self> {
        match value {
            Value::Array(ArrayData::F32(value)) => Some(Self::SingleReal(value)),
            Value::Array(ArrayData::ComplexF32(value)) => Some(Self::SingleComplex(value)),
            Value::Logical(_)
            | Value::Double(_)
            | Value::Complex(_)
            | Value::Array(ArrayData::Logical(_) | ArrayData::F64(_) | ArrayData::ComplexF64(_)) => {
                NumericView::new(value, "single element-wise binary operation")
                    .ok()
                    .map(Self::Numeric)
            }
            _ => None,
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::SingleReal(value) => value.shape().dimensions(),
            Self::SingleComplex(value) => value.shape().dimensions(),
            Self::Numeric(value) => value.dimensions(),
        }
    }

    fn numel(&self) -> u64 {
        match self {
            Self::SingleReal(value) => value.numel(),
            Self::SingleComplex(value) => value.numel(),
            Self::Numeric(value) => value.numel(),
        }
    }

    fn is_complex(&self) -> bool {
        match self {
            Self::SingleReal(_) => false,
            Self::SingleComplex(_) => true,
            Self::Numeric(value) => value.is_complex(),
        }
    }

    fn uses_double_precision(&self) -> bool {
        matches!(self, Self::Numeric(value) if !value.is_logical())
    }

    fn complex32_at(&self, offset: usize) -> Option<Complex32> {
        match self {
            Self::SingleReal(array) => array.as_slice().get(offset).copied().map(Complex32::from),
            Self::SingleComplex(array) => array.as_slice().get(offset).copied(),
            Self::Numeric(value) => value.complex_at(offset).map(|value| {
                Complex32::new(
                    single_from_double(value.real),
                    single_from_double(value.imaginary),
                )
            }),
        }
    }

    fn complex64_at(&self, offset: usize) -> Option<Complex64> {
        match self {
            Self::SingleReal(array) => array
                .as_slice()
                .get(offset)
                .map(|value| Complex64::new(f64::from(*value), 0.0)),
            Self::SingleComplex(array) => array
                .as_slice()
                .get(offset)
                .map(|value| Complex64::new(f64::from(value.re), f64::from(value.im))),
            Self::Numeric(value) => value.complex_at(offset),
        }
    }

    fn real32_at(&self, offset: usize) -> Option<f32> {
        let value = self.complex32_at(offset)?;
        (value.im == 0.0).then_some(value.re)
    }

    fn real64_at(&self, offset: usize) -> Option<f64> {
        let value = self.complex64_at(offset)?;
        (value.imaginary == 0.0).then_some(value.real)
    }
}

#[allow(clippy::cast_possible_truncation)]
fn single_from_double(value: f64) -> f32 {
    value as f32
}

enum IndexView<'a> {
    LogicalScalar(bool),
    RealScalar(f64),
    ComplexScalar,
    LogicalArray(&'a DenseArray<Logical>),
    RealArray(&'a DenseArray<f64>),
    ComplexArray(&'a DenseArray<ArrayComplex64>),
    SingleRealArray(&'a DenseArray<f32>),
    SingleComplexArray(&'a DenseArray<Complex32>),
    Char(&'a DenseArray<CharCodeUnit>),
    Integer(&'a IntegerArrayData),
}

impl<'a> IndexView<'a> {
    fn new(value: &'a Value) -> Option<Self> {
        match value {
            Value::Logical(value) => Some(Self::LogicalScalar(*value)),
            Value::Double(value) => Some(Self::RealScalar(*value)),
            Value::Complex(_) => Some(Self::ComplexScalar),
            Value::Array(ArrayData::Logical(array)) => Some(Self::LogicalArray(array)),
            Value::Array(ArrayData::F64(array)) => Some(Self::RealArray(array)),
            Value::Array(ArrayData::ComplexF64(array)) => Some(Self::ComplexArray(array)),
            Value::Array(ArrayData::F32(array)) => Some(Self::SingleRealArray(array)),
            Value::Array(ArrayData::ComplexF32(array)) => Some(Self::SingleComplexArray(array)),
            Value::Array(ArrayData::Char(array)) => Some(Self::Char(array)),
            Value::Array(ArrayData::Integer(integer)) => Some(Self::Integer(integer)),
            Value::String(_)
            | Value::Cell(_)
            | Value::Struct(_)
            | Value::Table(_)
            | Value::Sparse(_)
            | Value::Nothing
            | Value::Object(_)
            | Value::ObjectArray(_)
            | Value::Graphics(_)
            | Value::GraphicsArray(_)
            | Value::Function(_) => None,
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::LogicalScalar(_) | Self::RealScalar(_) | Self::ComplexScalar => {
                &SCALAR_DIMENSIONS
            }
            Self::LogicalArray(array) => array.shape().dimensions(),
            Self::RealArray(array) => array.shape().dimensions(),
            Self::ComplexArray(array) => array.shape().dimensions(),
            Self::SingleRealArray(array) => array.shape().dimensions(),
            Self::SingleComplexArray(array) => array.shape().dimensions(),
            Self::Char(array) => array.shape().dimensions(),
            Self::Integer(integer) => integer.shape().dimensions(),
        }
    }

    fn numel(&self) -> u64 {
        match self {
            Self::LogicalScalar(_) | Self::RealScalar(_) | Self::ComplexScalar => 1,
            Self::LogicalArray(array) => array.numel(),
            Self::RealArray(array) => array.numel(),
            Self::ComplexArray(array) => array.numel(),
            Self::SingleRealArray(array) => array.numel(),
            Self::SingleComplexArray(array) => array.numel(),
            Self::Char(array) => array.numel(),
            Self::Integer(integer) => integer.numel(),
        }
    }

    fn is_logical(&self) -> bool {
        matches!(self, Self::LogicalScalar(_) | Self::LogicalArray(_))
    }

    fn logical_at(&self, offset: usize) -> Option<bool> {
        match self {
            Self::LogicalScalar(value) if offset == 0 => Some(*value),
            Self::LogicalArray(array) => array.as_slice().get(offset).map(|value| value.get()),
            _ => None,
        }
    }
}

pub(crate) fn evaluate_condition(
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    match value {
        Value::Array(ArrayData::Char(array)) => {
            if array.numel() == 0 {
                return Ok(false);
            }
            for (index, value) in array.as_slice().iter().enumerate() {
                check_cancelled(cancellation, index)?;
                if value.get() == 0 {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        Value::Array(ArrayData::Integer(integer)) => {
            if integer.is_complex() {
                return Err(RuntimeErrorKind::InvalidCondition {
                    actual: value.kind(),
                });
            }
            if integer.numel() == 0 {
                return Ok(false);
            }
            for (index, element) in integer.elements().enumerate() {
                check_cancelled(cancellation, index)?;
                if element.real_component().is_zero() {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        _ => {}
    }
    match value
        .condition_with_interrupt(|| cancellation.is_cancelled())
        .map_err(|error| RuntimeErrorKind::InvalidCondition {
            actual: error.actual,
        })? {
        ConditionEvaluation::Complete(value) => Ok(value),
        ConditionEvaluation::Interrupted => Err(RuntimeErrorKind::Cancelled),
    }
}

#[derive(Clone, Copy)]
enum SwitchNumericComponent {
    Floating(f64),
    Integer(IntegerComponent),
}

#[derive(Clone, Copy)]
struct SwitchNumericScalar {
    real: SwitchNumericComponent,
    imaginary: SwitchNumericComponent,
}

pub(crate) fn switch_match(
    selector: &Value,
    case_value: &Value,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    validate_switch_selector(selector)?;

    if let Value::Cell(case_values) = case_value {
        for (index, case_value) in case_values.values().iter().enumerate() {
            check_cancelled(cancellation, index)?;
            if switch_match_single_case(selector, case_value, cancellation)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }

    switch_match_single_case(selector, case_value, cancellation)
}

fn switch_match_single_case(
    selector: &Value,
    case_value: &Value,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    match (selector, case_value) {
        (Value::Array(ArrayData::Char(selector)), Value::Array(ArrayData::Char(case_value))) => {
            exact_char_match(selector, case_value, cancellation)
        }
        (Value::String(selector), Value::String(case_value)) => {
            exact_string_match(selector, case_value, cancellation)
        }
        (Value::Array(ArrayData::Char(selector)), Value::String(case_value)) => {
            char_string_match(selector, case_value, cancellation)
        }
        (Value::String(selector), Value::Array(ArrayData::Char(case_value))) => {
            char_string_match(case_value, selector, cancellation)
        }
        (Value::String(_), other) => Err(invalid_switch_value("switch case value", other)),
        (_, Value::String(_)) => Err(invalid_switch_value("switch case value", case_value)),
        _ => match (
            switch_numeric_scalar(selector),
            switch_numeric_scalar(case_value),
        ) {
            (Some(selector), Some(case_value)) => Ok(switch_numeric_equal(selector, case_value)),
            (Some(_), None) if is_non_scalar_numeric(case_value) || is_char_value(case_value) => {
                Ok(false)
            }
            (None, Some(_)) if is_char_value(selector) => Ok(false),
            (None, None) if is_char_value(selector) && is_char_value(case_value) => {
                unreachable!("char pairs returned before numeric dispatch")
            }
            _ => Err(invalid_switch_value("switch case value", case_value)),
        },
    }
}

fn validate_switch_selector(selector: &Value) -> Result<(), RuntimeErrorKind> {
    match selector {
        Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Array(ArrayData::Char(_)) => Ok(()),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => Ok(()),
        Value::Array(ArrayData::ComplexF64(array)) if array.numel() == 1 => Ok(()),
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => Ok(()),
        Value::Array(ArrayData::Integer(array)) if array.numel() == 1 => Ok(()),
        Value::String(value) if value.numel() != 0 || value.dimensions() == [0, 0].as_slice() => {
            Ok(())
        }
        _ => Err(invalid_switch_value("switch selector", selector)),
    }
}

fn exact_char_match(
    selector: &DenseArray<CharCodeUnit>,
    case_value: &DenseArray<CharCodeUnit>,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    if selector.shape().dimensions() != case_value.shape().dimensions() {
        return Ok(false);
    }
    for (index, (selector, case_value)) in selector
        .as_slice()
        .iter()
        .zip(case_value.as_slice())
        .enumerate()
    {
        check_cancelled(cancellation, index)?;
        if selector != case_value {
            return Ok(false);
        }
    }
    Ok(true)
}

fn exact_string_match(
    selector: &StringValue,
    case_value: &StringValue,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    if selector.numel() == 0 || case_value.numel() == 0 {
        return Ok(false);
    }
    if selector.dimensions() != case_value.dimensions() {
        return Ok(false);
    }
    let length = usize::try_from(selector.numel()).map_err(|_| array_size_error())?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        let selector = selector
            .element(index)
            .expect("validated string selector element");
        let case_value = case_value
            .element(index)
            .expect("validated string case element");
        if selector.is_missing()
            || case_value.is_missing()
            || selector.code_units() != case_value.code_units()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn char_string_match(
    character: &DenseArray<CharCodeUnit>,
    string: &StringValue,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    let Some(string) = string.as_scalar() else {
        return if character.shape().dimensions() == [0, 0]
            || (character.shape().ndims() == 2 && character.shape().extent(0) == 1)
        {
            Ok(false)
        } else {
            Err(invalid_switch_kind(
                "switch character/string comparison",
                openmat_value::ValueKind::String,
            ))
        };
    };
    if string.is_missing() {
        return Ok(false);
    }
    let character_is_vector = character.shape().dimensions() == [0, 0]
        || (character.shape().ndims() == 2 && character.shape().extent(0) == 1);
    if !character_is_vector {
        return Err(invalid_switch_kind(
            "switch character/string comparison",
            openmat_value::ValueKind::Char,
        ));
    }
    if character.as_slice().len() != string.code_units().len() {
        return Ok(false);
    }
    for (index, (character, string)) in character
        .as_slice()
        .iter()
        .zip(string.code_units())
        .enumerate()
    {
        check_cancelled(cancellation, index)?;
        if character.get() != *string {
            return Ok(false);
        }
    }
    Ok(true)
}

fn switch_numeric_scalar(value: &Value) -> Option<SwitchNumericScalar> {
    let floating = |real, imaginary| SwitchNumericScalar {
        real: SwitchNumericComponent::Floating(real),
        imaginary: SwitchNumericComponent::Floating(imaginary),
    };
    let exact = |real, imaginary| SwitchNumericScalar {
        real: SwitchNumericComponent::Integer(real),
        imaginary: SwitchNumericComponent::Integer(imaginary),
    };
    match value {
        Value::Logical(value) => Some(exact(
            IntegerComponent::Unsigned(u128::from(*value)),
            IntegerComponent::Unsigned(0),
        )),
        Value::Double(value) => Some(floating(*value, 0.0)),
        Value::Complex(value) => Some(floating(value.real, value.imaginary)),
        Value::Array(ArrayData::F64(array)) if array.numel() == 1 => {
            Some(floating(array.as_slice()[0], 0.0))
        }
        Value::Array(ArrayData::ComplexF64(array)) if array.numel() == 1 => {
            Some(floating(array.as_slice()[0].re, array.as_slice()[0].im))
        }
        Value::Array(ArrayData::Logical(array)) if array.numel() == 1 => Some(exact(
            IntegerComponent::Unsigned(u128::from(array.as_slice()[0].get())),
            IntegerComponent::Unsigned(0),
        )),
        Value::Array(ArrayData::Integer(array)) if array.numel() == 1 => {
            array.element(0).map(integer_switch_scalar)
        }
        Value::Array(ArrayData::Char(array)) if array.numel() == 1 => Some(exact(
            IntegerComponent::Unsigned(u128::from(array.as_slice()[0].get())),
            IntegerComponent::Unsigned(0),
        )),
        _ => None,
    }
}

fn integer_switch_scalar(value: IntegerElementValue) -> SwitchNumericScalar {
    SwitchNumericScalar {
        real: SwitchNumericComponent::Integer(value.real_component()),
        imaginary: SwitchNumericComponent::Integer(
            value
                .imaginary_component()
                .unwrap_or(IntegerComponent::Signed(0)),
        ),
    }
}

fn switch_numeric_equal(lhs: SwitchNumericScalar, rhs: SwitchNumericScalar) -> bool {
    switch_component_equal(lhs.real, rhs.real)
        && switch_component_equal(lhs.imaginary, rhs.imaginary)
}

fn switch_component_equal(lhs: SwitchNumericComponent, rhs: SwitchNumericComponent) -> bool {
    match (lhs, rhs) {
        (SwitchNumericComponent::Floating(lhs), SwitchNumericComponent::Floating(rhs)) => {
            lhs.partial_cmp(&rhs).is_some_and(std::cmp::Ordering::is_eq)
        }
        (SwitchNumericComponent::Integer(lhs), SwitchNumericComponent::Integer(rhs)) => {
            integer_components_equal(lhs, rhs)
        }
        (SwitchNumericComponent::Floating(value), SwitchNumericComponent::Integer(integer))
        | (SwitchNumericComponent::Integer(integer), SwitchNumericComponent::Floating(value)) => {
            floating_integer_equal(value, integer)
        }
    }
}

fn integer_components_equal(lhs: IntegerComponent, rhs: IntegerComponent) -> bool {
    match (lhs, rhs) {
        (IntegerComponent::Signed(lhs), IntegerComponent::Signed(rhs)) => lhs == rhs,
        (IntegerComponent::Unsigned(lhs), IntegerComponent::Unsigned(rhs)) => lhs == rhs,
        (IntegerComponent::Signed(signed), IntegerComponent::Unsigned(unsigned))
        | (IntegerComponent::Unsigned(unsigned), IntegerComponent::Signed(signed)) => {
            signed >= 0 && u128::try_from(signed) == Ok(unsigned)
        }
    }
}

fn floating_integer_equal(value: f64, integer: IntegerComponent) -> bool {
    if !value.is_finite() || value.fract() != 0.0 {
        return false;
    }
    let exact = if value < 0.0 {
        if value < I64_MIN_AS_F64 {
            return false;
        }
        #[allow(clippy::cast_possible_truncation)]
        IntegerComponent::Signed(value as i128)
    } else {
        if value >= U64_EXCLUSIVE_LIMIT_AS_F64 {
            return false;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let exact = value as u128;
        IntegerComponent::Unsigned(exact)
    };
    integer_components_equal(exact, integer)
}

fn is_non_scalar_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(
            ArrayData::F64(_)
                | ArrayData::ComplexF64(_)
                | ArrayData::Logical(_)
                | ArrayData::Integer(_)
        )
    ) && value.numel() != Some(1)
}

fn is_char_value(value: &Value) -> bool {
    matches!(value, Value::Array(ArrayData::Char(_)))
}

fn invalid_switch_value(operation: &'static str, value: &Value) -> RuntimeErrorKind {
    invalid_switch_kind(operation, value.kind())
}

fn invalid_switch_kind(
    operation: &'static str,
    actual: openmat_value::ValueKind,
) -> RuntimeErrorKind {
    array_error(ArrayRuntimeError::InvalidOperand { operation, actual })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_matrix(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if rows.is_empty() || rows.iter().all(Vec::is_empty) {
        return real_array([0, 0], Vec::new());
    }
    if rows
        .iter()
        .flatten()
        .any(|value| matches!(value, Value::String(_)))
    {
        return build_string_matrix(rows, cancellation);
    }
    if rows
        .iter()
        .flatten()
        .any(|value| matches!(value, Value::Array(ArrayData::Char(_))))
    {
        return build_exact_matrix::<CharCodeUnit>(rows, cancellation);
    }
    if let Some(integer) = rows.iter().flatten().find_map(|value| match value {
        Value::Array(ArrayData::Integer(integer)) => Some(integer),
        _ => None,
    }) {
        return dispatch_integer_array!(integer, |array| {
            build_exact_matrix_for_array(rows, array, cancellation)
        });
    }
    if rows.iter().flatten().any(|value| {
        matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    }) {
        return build_single_matrix(rows, cancellation);
    }

    let mut row_heights = try_output_vec(rows.len(), cancellation)?;
    let mut row_widths = try_output_vec(rows.len(), cancellation)?;
    let mut complex = false;
    let mut logical = true;
    for row in rows {
        let mut height = None;
        let mut width = 0_u64;
        for value in row {
            let view = NumericView::new(value, "matrix concatenation")?;
            require_two_dimensions(&view, "matrix concatenation")?;
            let current_height = view.dimensions()[0];
            let current_width = view.dimensions()[1];
            if current_height != 0 || current_width != 0 {
                if let Some(expected) = height {
                    if current_height != expected {
                        return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                            axis: 0,
                            expected,
                            actual: current_height,
                        }));
                    }
                } else {
                    height = Some(current_height);
                }
            }
            width = width
                .checked_add(current_width)
                .ok_or_else(array_size_error)?;
            complex |= view.is_complex();
            logical &= view.is_logical();
        }
        row_heights.push(height.unwrap_or(0));
        row_widths.push(width);
    }

    let expected_width = row_heights
        .iter()
        .zip(&row_widths)
        .find_map(|(&height, &width)| (height != 0 || width != 0).then_some(width))
        .unwrap_or(0);
    for (&height, &actual) in row_heights.iter().zip(&row_widths) {
        if (height != 0 || actual != 0) && actual != expected_width {
            return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                axis: 1,
                expected: expected_width,
                actual,
            }));
        }
    }
    let total_height = row_heights.iter().try_fold(0_u64, |total, height| {
        total.checked_add(*height).ok_or_else(array_size_error)
    })?;
    let output_len = host_len(total_height, expected_width)?;

    if complex {
        let mut output = try_filled_output(output_len, ArrayComplex64::ZERO, cancellation)?;
        concatenate_into(
            rows,
            &row_heights,
            total_height,
            cancellation,
            |value, source, dst| {
                let element = source
                    .complex_at(value)
                    .expect("validated matrix element offset");
                output[dst] = element.into();
            },
        )?;
        complex_array([total_height, expected_width], output)
    } else if logical {
        let mut output = try_filled_output(output_len, Logical::FALSE, cancellation)?;
        concatenate_into(
            rows,
            &row_heights,
            total_height,
            cancellation,
            |value, source, dst| {
                output[dst] = Logical::from(
                    source
                        .logical_at(value)
                        .expect("validated logical matrix element offset"),
                );
            },
        )?;
        logical_array([total_height, expected_width], output)
    } else {
        let mut output = try_filled_output(output_len, 0.0, cancellation)?;
        concatenate_into(
            rows,
            &row_heights,
            total_height,
            cancellation,
            |value, source, dst| {
                output[dst] = source
                    .real_at(value)
                    .expect("non-complex matrix elements are real");
            },
        )?;
        real_array([total_height, expected_width], output)
    }
}

fn build_single_matrix(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let (row_heights, total_height, expected_width, complex) =
        single_matrix_layout(rows, cancellation)?;
    let output_len = host_len(total_height, expected_width)?;

    if complex {
        let mut output = try_filled_output(output_len, Complex32::ZERO, cancellation)?;
        concatenate_single_into(
            rows,
            &row_heights,
            total_height,
            cancellation,
            |value, source, dst| {
                output[dst] = source
                    .complex_at(value)
                    .expect("validated single matrix element offset");
            },
        )?;
        single_complex_array([total_height, expected_width], output)
    } else {
        let mut output = try_filled_output(output_len, 0.0, cancellation)?;
        concatenate_single_into(
            rows,
            &row_heights,
            total_height,
            cancellation,
            |value, source, dst| {
                output[dst] = source
                    .real_at(value)
                    .expect("non-complex single matrix elements are real");
            },
        )?;
        single_real_array([total_height, expected_width], output)
    }
}

fn single_matrix_layout(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Result<(Vec<u64>, u64, u64, bool), RuntimeErrorKind> {
    let mut row_heights = try_output_vec(rows.len(), cancellation)?;
    let mut row_widths = try_output_vec(rows.len(), cancellation)?;
    let mut complex = false;
    for row in rows {
        let mut height = None;
        let mut width = 0_u64;
        for value in row {
            let view = SingleMatrixView::new(value)?;
            if view.dimensions().len() != 2 {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "single matrix concatenation above two dimensions",
                    actual: value.kind(),
                }));
            }
            let current_height = view.dimensions()[0];
            let current_width = view.dimensions()[1];
            if current_height != 0 || current_width != 0 {
                if let Some(expected) = height {
                    if current_height != expected {
                        return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                            axis: 0,
                            expected,
                            actual: current_height,
                        }));
                    }
                } else {
                    height = Some(current_height);
                }
            }
            width = width
                .checked_add(current_width)
                .ok_or_else(array_size_error)?;
            complex |= view.is_complex();
        }
        row_heights.push(height.unwrap_or(0));
        row_widths.push(width);
    }

    let expected_width = row_heights
        .iter()
        .zip(&row_widths)
        .find_map(|(&height, &width)| (height != 0 || width != 0).then_some(width))
        .unwrap_or(0);
    for (&height, &actual) in row_heights.iter().zip(&row_widths) {
        if (height != 0 || actual != 0) && actual != expected_width {
            return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                axis: 1,
                expected: expected_width,
                actual,
            }));
        }
    }
    let total_height = row_heights.iter().try_fold(0_u64, |total, height| {
        total.checked_add(*height).ok_or_else(array_size_error)
    })?;
    Ok((row_heights, total_height, expected_width, complex))
}

fn concatenate_single_into(
    rows: &[Vec<Value>],
    row_heights: &[u64],
    output_rows: u64,
    cancellation: &CancellationToken,
    mut copy: impl FnMut(usize, &SingleMatrixView<'_>, usize),
) -> Result<(), RuntimeErrorKind> {
    let mut row_offset = 0_u64;
    let mut visited = 0_usize;
    for (row, &row_height) in rows.iter().zip(row_heights) {
        let mut column_offset = 0_u64;
        for value in row {
            let view = SingleMatrixView::new(value)?;
            let columns = view.dimensions()[1];
            for column in 0..columns {
                for source_row in 0..row_height {
                    check_cancelled(cancellation, visited)?;
                    let source = checked_usize(source_row + column * row_height)?;
                    let destination = checked_usize(
                        row_offset + source_row + (column_offset + column) * output_rows,
                    )?;
                    copy(source, &view, destination);
                    visited = visited.saturating_add(1);
                }
            }
            column_offset = column_offset
                .checked_add(columns)
                .ok_or_else(array_size_error)?;
        }
        row_offset = row_offset
            .checked_add(row_height)
            .ok_or_else(array_size_error)?;
    }
    Ok(())
}

fn build_exact_matrix_for_array<T: ArrayElement + Clone>(
    rows: &[Vec<Value>],
    _representative: &DenseArray<T>,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    build_exact_matrix::<T>(rows, cancellation)
}

fn build_exact_matrix<T: ArrayElement + Clone>(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let mut row_heights = try_output_vec(rows.len(), cancellation)?;
    let mut row_widths = try_output_vec(rows.len(), cancellation)?;
    for row in rows {
        let mut height = None;
        let mut width = 0_u64;
        for value in row {
            let Some(array) = value.as_array().and_then(ArrayData::as_typed::<T>) else {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "exact matrix concatenation",
                    actual: value.kind(),
                }));
            };
            if array.shape().ndims() != 2 {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "exact matrix concatenation above two dimensions",
                    actual: value.kind(),
                }));
            }
            let current_height = array.shape().extent(0);
            let current_width = array.shape().extent(1);
            if current_height != 0 || current_width != 0 {
                if let Some(expected) = height {
                    if current_height != expected {
                        return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                            axis: 0,
                            expected,
                            actual: current_height,
                        }));
                    }
                } else {
                    height = Some(current_height);
                }
            }
            width = width
                .checked_add(current_width)
                .ok_or_else(array_size_error)?;
        }
        row_heights.push(height.unwrap_or(0));
        row_widths.push(width);
    }

    let expected_width = row_heights
        .iter()
        .zip(&row_widths)
        .find_map(|(&height, &width)| (height != 0 || width != 0).then_some(width))
        .unwrap_or(0);
    for (&height, &actual) in row_heights.iter().zip(&row_widths) {
        if (height != 0 || actual != 0) && actual != expected_width {
            return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                axis: 1,
                expected: expected_width,
                actual,
            }));
        }
    }
    let total_height = row_heights.iter().try_fold(0_u64, |total, height| {
        total.checked_add(*height).ok_or_else(array_size_error)
    })?;
    let output_len = host_len(total_height, expected_width)?;
    let mut output = try_output_vec(output_len, cancellation)?;
    output.resize_with(output_len, || None);
    let mut row_offset = 0_u64;
    let mut visited = 0_usize;
    for (row, &row_height) in rows.iter().zip(&row_heights) {
        let mut column_offset = 0_u64;
        for value in row {
            let array = value
                .as_array()
                .and_then(ArrayData::as_typed::<T>)
                .expect("exact concatenation inputs were validated");
            let columns = array.shape().extent(1);
            for column in 0..columns {
                for source_row in 0..row_height {
                    check_cancelled(cancellation, visited)?;
                    let source = checked_usize(source_row + column * row_height)?;
                    let destination = checked_usize(
                        row_offset + source_row + (column_offset + column) * total_height,
                    )?;
                    output[destination] = Some(array.as_slice()[source].clone());
                    visited = visited.saturating_add(1);
                }
            }
            column_offset = column_offset
                .checked_add(columns)
                .ok_or_else(array_size_error)?;
        }
        row_offset = row_offset
            .checked_add(row_height)
            .ok_or_else(array_size_error)?;
    }
    let output = output
        .into_iter()
        .map(|value| value.expect("checked exact concatenation fills every output element"))
        .collect();
    let shape = Shape::new([total_height, expected_width]).map_err(|_| array_size_error())?;
    DenseArray::from_vec(shape, output)
        .map(ArrayData::from_typed)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

enum StringMatrixOperand<'a> {
    String(&'a StringValue),
    Char(&'a DenseArray<CharCodeUnit>),
}

impl<'a> StringMatrixOperand<'a> {
    fn new(value: &'a Value) -> Result<Self, RuntimeErrorKind> {
        match value {
            Value::String(value) => Ok(Self::String(value)),
            Value::Array(ArrayData::Char(array))
                if array.shape().dimensions() == [0, 0]
                    || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
            {
                Ok(Self::Char(array))
            }
            other => Err(array_error(ArrayRuntimeError::InvalidOperand {
                operation: "string matrix concatenation",
                actual: other.kind(),
            })),
        }
    }

    fn dimensions(&self) -> &[u64] {
        match self {
            Self::String(value) => value.dimensions(),
            Self::Char(_) => &SCALAR_DIMENSIONS,
        }
    }

    fn element(&self, offset: usize) -> Option<StringElement> {
        match self {
            Self::String(value) => value.element(offset).cloned(),
            Self::Char(array) if offset == 0 => Some(StringElement::from_code_units(
                array
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .collect::<Vec<_>>(),
            )),
            Self::Char(_) => None,
        }
    }
}

fn build_string_matrix(
    rows: &[Vec<Value>],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let mut row_heights = try_output_vec(rows.len(), cancellation)?;
    let mut row_widths = try_output_vec(rows.len(), cancellation)?;
    for row in rows {
        let mut height = None;
        let mut width = 0_u64;
        for value in row {
            let value = StringMatrixOperand::new(value)?;
            require_two_string_dimensions(&value)?;
            let current_height = value.dimensions()[0];
            let current_width = value.dimensions()[1];
            if current_height != 0 || current_width != 0 {
                if let Some(expected) = height {
                    if current_height != expected {
                        return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                            axis: 0,
                            expected,
                            actual: current_height,
                        }));
                    }
                } else {
                    height = Some(current_height);
                }
            }
            width = width
                .checked_add(current_width)
                .ok_or_else(array_size_error)?;
        }
        row_heights.push(height.unwrap_or(0));
        row_widths.push(width);
    }

    let expected_width = row_heights
        .iter()
        .zip(&row_widths)
        .find_map(|(&height, &width)| (height != 0 || width != 0).then_some(width))
        .unwrap_or(0);
    for (&height, &actual) in row_heights.iter().zip(&row_widths) {
        if (height != 0 || actual != 0) && actual != expected_width {
            return Err(array_error(ArrayRuntimeError::ConcatenationMismatch {
                axis: 1,
                expected: expected_width,
                actual,
            }));
        }
    }

    let total_height = row_heights.iter().try_fold(0_u64, |total, height| {
        total.checked_add(*height).ok_or_else(array_size_error)
    })?;
    let output_len = host_len(total_height, expected_width)?;
    let mut output = try_filled_output(output_len, StringElement::default(), cancellation)?;
    let mut row_offset = 0_u64;
    let mut visited = 0_usize;
    for (row, &row_height) in rows.iter().zip(&row_heights) {
        let mut column_offset = 0_u64;
        for value in row {
            let value = StringMatrixOperand::new(value)
                .expect("string concatenation inputs were validated");
            let columns = value.dimensions()[1];
            for column in 0..columns {
                for source_row in 0..row_height {
                    check_cancelled(cancellation, visited)?;
                    let source = source_row
                        .checked_add(
                            column
                                .checked_mul(row_height)
                                .ok_or_else(array_size_error)?,
                        )
                        .ok_or_else(array_size_error)?;
                    let destination = row_offset
                        .checked_add(source_row)
                        .and_then(|offset| {
                            column_offset
                                .checked_add(column)
                                .and_then(|column| column.checked_mul(total_height))
                                .and_then(|column| offset.checked_add(column))
                        })
                        .ok_or_else(array_size_error)?;
                    output[checked_usize(destination)?] = value
                        .element(checked_usize(source)?)
                        .expect("validated string matrix element offset");
                    visited = visited.saturating_add(1);
                }
            }
            column_offset = column_offset
                .checked_add(columns)
                .ok_or_else(array_size_error)?;
        }
        row_offset = row_offset
            .checked_add(row_height)
            .ok_or_else(array_size_error)?;
    }

    string_array([total_height, expected_width], output)
}

fn require_two_string_dimensions(value: &StringMatrixOperand<'_>) -> Result<(), RuntimeErrorKind> {
    if value.dimensions().len() == 2 {
        Ok(())
    } else {
        Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "string matrix concatenation",
            actual: openmat_value::ValueKind::String,
        }))
    }
}

fn concatenate_into(
    rows: &[Vec<Value>],
    row_heights: &[u64],
    output_rows: u64,
    cancellation: &CancellationToken,
    mut copy: impl FnMut(usize, &NumericView<'_>, usize),
) -> Result<(), RuntimeErrorKind> {
    let mut row_offset = 0_u64;
    let mut visited = 0_usize;
    for (row, &row_height) in rows.iter().zip(row_heights) {
        let mut column_offset = 0_u64;
        for value in row {
            let view = NumericView::new(value, "matrix concatenation")?;
            let columns = view.dimensions()[1];
            for column in 0..columns {
                for source_row in 0..row_height {
                    check_cancelled(cancellation, visited)?;
                    let source = checked_usize(source_row + column * row_height)?;
                    let destination = checked_usize(
                        row_offset + source_row + (column_offset + column) * output_rows,
                    )?;
                    copy(source, &view, destination);
                    visited = visited.saturating_add(1);
                }
            }
            column_offset = column_offset
                .checked_add(columns)
                .ok_or_else(array_size_error)?;
        }
        row_offset = row_offset
            .checked_add(row_height)
            .ok_or_else(array_size_error)?;
    }
    Ok(())
}

pub(crate) fn make_range(
    start: &Value,
    step: &Value,
    end: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let (Some(start), Some(step), Some(end)) = (
        start.as_real_number(),
        step.as_real_number(),
        end.as_real_number(),
    ) else {
        return Err(array_error(ArrayRuntimeError::InvalidRange {
            reason: RangeErrorKind::NonScalarReal,
        }));
    };
    if !start.is_finite() || !step.is_finite() || !end.is_finite() {
        return Err(array_error(ArrayRuntimeError::InvalidRange {
            reason: RangeErrorKind::NonFinite,
        }));
    }
    if step == 0.0 {
        return Err(array_error(ArrayRuntimeError::InvalidRange {
            reason: RangeErrorKind::ZeroStep,
        }));
    }
    if (step > 0.0 && start > end) || (step < 0.0 && start < end) {
        return real_array([1, 0], Vec::new());
    }

    // R2022b treats an endpoint within two scaled machine epsilons of the
    // nearest generated value as reachable.  It then constructs the first
    // ceil(count / 2) values from `start` and the remainder backwards from the
    // effective endpoint.  That symmetric construction is observable in the
    // exact bits of cases such as `0:0.1:0.3`; plain repeated addition or a
    // fused multiply-add does not match it.  This finite-f64 implementation
    // deliberately stops at the u64/host-allocation boundary below.
    let quotient = (end - start) / step;
    let nearest_intervals = quotient.round();
    let nearest_end = start + nearest_intervals * step;
    let tolerance = 2.0 * f64::EPSILON * start.abs().max(end.abs());
    let endpoint_is_reachable = (nearest_end - end).abs() <= tolerance;
    let intervals = if endpoint_is_reachable {
        nearest_intervals
    } else {
        quotient.floor()
    };
    if !intervals.is_finite() || !(0.0..U64_EXCLUSIVE_LIMIT_AS_F64).contains(&intervals) {
        return Err(array_error(ArrayRuntimeError::InvalidRange {
            reason: RangeErrorKind::TooLarge,
        }));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = intervals as u64 + 1;
    let capacity = checked_usize(count)?;
    let mut values = try_output_vec(capacity, cancellation)?;
    let effective_end = if endpoint_is_reachable {
        end
    } else {
        start + intervals * step
    };
    let start_generated = count.div_ceil(2);
    for index in 0..count {
        check_cancelled(cancellation, checked_usize(index)?)?;
        #[allow(clippy::cast_precision_loss)]
        let value = if index < start_generated {
            start + index as f64 * step
        } else {
            effective_end - (count - 1 - index) as f64 * step
        };
        values.push(value);
    }
    real_array([1, count], values)
}

pub(crate) fn transpose(
    value: &Value,
    conjugate: bool,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    match value {
        Value::Logical(_) | Value::Double(_) => Ok(value.clone()),
        Value::Complex(value) => Ok(Value::Complex(if conjugate {
            Complex64::new(value.real, -value.imaginary)
        } else {
            *value
        })),
        Value::Array(ArrayData::F32(array)) => transpose_array(array, cancellation, |value| *value)
            .and_then(|(shape, values)| {
                DenseArray::from_vec(shape, values)
                    .map(ArrayData::F32)
                    .map(Value::Array)
                    .map_err(|_| array_size_error())
            }),
        Value::Array(ArrayData::ComplexF32(array)) => {
            transpose_array(array, cancellation, |value| {
                if conjugate { value.conjugate() } else { *value }
            })
            .and_then(|(shape, values)| {
                DenseArray::from_vec(shape, values)
                    .map(ArrayData::ComplexF32)
                    .map(Value::Array)
                    .map_err(|_| array_size_error())
            })
        }
        Value::Array(ArrayData::F64(array)) => transpose_array(array, cancellation, |value| *value)
            .and_then(|array| real_array_data(array).map(Value::Array)),
        Value::Array(ArrayData::ComplexF64(array)) => {
            transpose_array(array, cancellation, |value| {
                if conjugate { value.conjugate() } else { *value }
            })
            .and_then(|array| complex_array_data(array).map(Value::Array))
        }
        Value::Array(ArrayData::Logical(array)) => {
            transpose_array(array, cancellation, |value| *value)
                .and_then(|array| logical_array_data(array).map(Value::Array))
        }
        Value::Array(ArrayData::Char(array)) => {
            transpose_array(array, cancellation, |value| *value).and_then(|(shape, values)| {
                DenseArray::from_vec(shape, values)
                    .map(ArrayData::Char)
                    .map(Value::Array)
                    .map_err(|_| array_size_error())
            })
        }
        Value::Array(ArrayData::Integer(integer)) => {
            transpose_integer(integer, conjugate, cancellation)
                .map(ArrayData::Integer)
                .map(Value::Array)
        }
        Value::Sparse(array) => array
            .try_transpose(conjugate, Some(cancellation.atomic_flag()))
            .map(Value::Sparse)
            .map_err(|error| sparse_runtime_error(&error)),
        other => Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "transpose",
            actual: other.kind(),
        })),
    }
}

fn transpose_integer(
    integer: &IntegerArrayData,
    conjugate: bool,
    cancellation: &CancellationToken,
) -> Result<IntegerArrayData, RuntimeErrorKind> {
    if conjugate && integer.is_complex() {
        return Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "complex integer conjugate transpose",
            actual: openmat_value::ValueKind::Integer,
        }));
    }
    macro_rules! real {
        ($array:expr, $variant:ident) => {{
            let (shape, values) = transpose_array($array, cancellation, |value| *value)?;
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::$variant)
                .map_err(|_| array_size_error())
        }};
    }
    macro_rules! complex_signed {
        ($array:expr, $variant:ident) => {{
            let (shape, values) = transpose_array($array, cancellation, |value| {
                ComplexInteger::new(
                    value.re(),
                    if conjugate {
                        value.im().saturating_neg()
                    } else {
                        value.im()
                    },
                )
            })?;
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::$variant)
                .map_err(|_| array_size_error())
        }};
    }
    macro_rules! complex_unsigned {
        ($array:expr, $variant:ident) => {{
            let (shape, values) = transpose_array($array, cancellation, |value| {
                ComplexInteger::new(value.re(), if conjugate { 0 } else { value.im() })
            })?;
            DenseArray::from_vec(shape, values)
                .map(IntegerArrayData::$variant)
                .map_err(|_| array_size_error())
        }};
    }

    match integer {
        IntegerArrayData::I8(array) => real!(array, I8),
        IntegerArrayData::ComplexI8(array) => complex_signed!(array, ComplexI8),
        IntegerArrayData::U8(array) => real!(array, U8),
        IntegerArrayData::ComplexU8(array) => complex_unsigned!(array, ComplexU8),
        IntegerArrayData::I16(array) => real!(array, I16),
        IntegerArrayData::ComplexI16(array) => complex_signed!(array, ComplexI16),
        IntegerArrayData::U16(array) => real!(array, U16),
        IntegerArrayData::ComplexU16(array) => complex_unsigned!(array, ComplexU16),
        IntegerArrayData::I32(array) => real!(array, I32),
        IntegerArrayData::ComplexI32(array) => complex_signed!(array, ComplexI32),
        IntegerArrayData::U32(array) => real!(array, U32),
        IntegerArrayData::ComplexU32(array) => complex_unsigned!(array, ComplexU32),
        IntegerArrayData::I64(array) => real!(array, I64),
        IntegerArrayData::ComplexI64(array) => complex_signed!(array, ComplexI64),
        IntegerArrayData::U64(array) => real!(array, U64),
        IntegerArrayData::ComplexU64(array) => complex_unsigned!(array, ComplexU64),
    }
}

fn transpose_array<T: Clone, U>(
    array: &DenseArray<T>,
    cancellation: &CancellationToken,
    mut transform: impl FnMut(&T) -> U,
) -> Result<(Shape, Vec<U>), RuntimeErrorKind> {
    if array.shape().ndims() != 2 {
        return Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "transpose of an N-D array",
            actual: openmat_value::ValueKind::Array,
        }));
    }
    let rows = array.shape().extent(0);
    let columns = array.shape().extent(1);
    let mut output = try_output_vec(array.as_slice().len(), cancellation)?;
    for output_column in 0..rows {
        for output_row in 0..columns {
            let visited = output.len();
            check_cancelled(cancellation, visited)?;
            let source = checked_usize(output_column + output_row * rows)?;
            output.push(transform(&array.as_slice()[source]));
        }
    }
    let shape = Shape::new([columns, rows]).map_err(|_| array_size_error())?;
    Ok((shape, output))
}

pub(crate) fn evaluate_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
    linalg_provider: &dyn LinalgProvider,
) -> Result<Value, RuntimeErrorKind> {
    if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
        && (matches!(lhs, Value::Graphics(_) | Value::GraphicsArray(_))
            || matches!(rhs, Value::Graphics(_) | Value::GraphicsArray(_)))
    {
        return graphics_equality(operator, lhs, rhs);
    }
    if operator == BinaryOperator::LeftDivide && !lhs.is_scalar() {
        return linalg_matrix_left_divide(
            linalg_provider,
            lhs,
            rhs,
            Some(cancellation.atomic_flag()),
        )
        .map_err(Into::into);
    }
    if operator == BinaryOperator::Divide && !rhs.is_scalar() {
        return linalg_matrix_right_divide(
            linalg_provider,
            lhs,
            rhs,
            Some(cancellation.atomic_flag()),
        )
        .map_err(Into::into);
    }
    if operator == BinaryOperator::Multiply
        && !lhs.is_scalar()
        && !rhs.is_scalar()
        && is_single_matrix(lhs)
        && is_single_matrix(rhs)
    {
        return linalg_matrix_multiply(linalg_provider, lhs, rhs).map_err(Into::into);
    }
    if operator == BinaryOperator::Power && !lhs.is_scalar() && rhs.is_scalar() {
        if matrix_power_exponent(rhs).is_some() {
            return matrix_integer_power(linalg_provider, lhs, rhs, cancellation);
        }
        return matrix_fractional_power(linalg_provider, lhs, rhs, cancellation);
    }
    if matches!(
        lhs,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) || matches!(
        rhs,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) {
        return single_binary(operator, lhs, rhs, cancellation);
    }
    if matches!(
        lhs.kind(),
        openmat_value::ValueKind::Char | openmat_value::ValueKind::Integer
    ) || matches!(
        rhs.kind(),
        openmat_value::ValueKind::Char | openmat_value::ValueKind::Integer
    ) {
        if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) {
            return exact_equality(operator, lhs, rhs, cancellation);
        }
        return Err(RuntimeErrorKind::UnsupportedArrayOperator { operator });
    }
    if lhs.is_scalar() && rhs.is_scalar() {
        return scalar_binary(operator, lhs, rhs);
    }
    if operator == BinaryOperator::Multiply && !lhs.is_scalar() && !rhs.is_scalar() {
        if is_double_matrix(lhs) && is_double_matrix(rhs) {
            return linalg_matrix_multiply(linalg_provider, lhs, rhs).map_err(Into::into);
        }
        return matrix_multiply(lhs, rhs, cancellation);
    }
    if matches!(operator, BinaryOperator::Power) {
        return Err(invalid_operands(operator, lhs, rhs));
    }
    elementwise_binary(operator, lhs, rhs, cancellation)
}

fn matrix_integer_power(
    provider: &dyn LinalgProvider,
    base: &Value,
    exponent: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    let (rows, columns) = matrix_power_dimensions(base)
        .ok_or_else(|| invalid_operands(BinaryOperator::Power, base, exponent))?;
    if rows != columns {
        return Err(invalid_operands(BinaryOperator::Power, base, exponent));
    }
    let (exponent, exponent_is_single) = matrix_power_exponent(exponent)
        .ok_or_else(|| invalid_operands(BinaryOperator::Power, base, exponent))?;
    let use_single = matches!(
        base,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) || exponent_is_single;
    let base = matrix_power_precision(base, use_single, cancellation)?;
    let identity = matrix_power_identity(rows, use_single, cancellation)?;
    if exponent == 0 {
        return Ok(identity);
    }

    let mut factor = if exponent.is_negative() {
        linalg_matrix_left_divide(provider, &base, &identity, Some(cancellation.atomic_flag()))
            .map_err(RuntimeErrorKind::from)?
    } else {
        base
    };
    let mut remaining = exponent.unsigned_abs();
    if remaining == 1 {
        return Ok(factor);
    }
    let mut result = identity;
    while remaining != 0 {
        check_cancelled(cancellation, 0)?;
        if remaining & 1 != 0 {
            result = linalg_matrix_multiply(provider, &result, &factor)
                .map_err(RuntimeErrorKind::from)?;
        }
        remaining >>= 1;
        if remaining != 0 {
            factor = linalg_matrix_multiply(provider, &factor, &factor)
                .map_err(RuntimeErrorKind::from)?;
        }
    }
    Ok(result)
}

fn matrix_fractional_power(
    provider: &dyn LinalgProvider,
    base: &Value,
    exponent: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    let (rows, columns) = matrix_power_dimensions(base)
        .ok_or_else(|| invalid_operands(BinaryOperator::Power, base, exponent))?;
    if rows != columns {
        return Err(invalid_operands(BinaryOperator::Power, base, exponent));
    }
    let exponent_value = exponent;
    let (matrix_exponent, exponent_is_single) = matrix_function_exponent(exponent_value)
        .ok_or_else(|| invalid_operands(BinaryOperator::Power, base, exponent))?;
    let use_single = matches!(
        base,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    ) || exponent_is_single;
    let base = matrix_power_precision(base, use_single, cancellation)?;
    if matrix_power_has_nonfinite(&base) {
        return matrix_power_nonfinite_result(&base, cancellation);
    }
    match base {
        Value::Array(ArrayData::F64(matrix)) => {
            let result = matrix_power_f64(
                provider,
                MatrixFunctionRequest::new(&matrix)
                    .with_cancellation_flag(cancellation.atomic_flag()),
                matrix_exponent,
            )
            .map_err(RuntimeLinalgError::from)?;
            matrix_function_value64(result, cancellation)
        }
        Value::Array(ArrayData::ComplexF64(matrix)) => {
            let result = matrix_power_complex64(
                provider,
                MatrixFunctionRequest::new(&matrix)
                    .with_cancellation_flag(cancellation.atomic_flag()),
                matrix_exponent,
            )
            .map_err(RuntimeLinalgError::from)?;
            matrix_function_value64(result, cancellation)
        }
        Value::Array(ArrayData::F32(matrix)) => {
            let exponent = Complex32::new(
                f64_to_f32(matrix_exponent.re),
                f64_to_f32(matrix_exponent.im),
            );
            let result = matrix_power_f32(
                provider,
                MatrixFunctionRequest::new(&matrix)
                    .with_cancellation_flag(cancellation.atomic_flag()),
                exponent,
            )
            .map_err(RuntimeLinalgError::from)?;
            matrix_function_value32(result, cancellation)
        }
        Value::Array(ArrayData::ComplexF32(matrix)) => {
            let exponent = Complex32::new(
                f64_to_f32(matrix_exponent.re),
                f64_to_f32(matrix_exponent.im),
            );
            let result = matrix_power_complex32(
                provider,
                MatrixFunctionRequest::new(&matrix)
                    .with_cancellation_flag(cancellation.atomic_flag()),
                exponent,
            )
            .map_err(RuntimeLinalgError::from)?;
            matrix_function_value32(result, cancellation)
        }
        _ => Err(invalid_operands(
            BinaryOperator::Power,
            &base,
            exponent_value,
        )),
    }
}

fn matrix_power_dimensions(value: &Value) -> Option<(u64, u64)> {
    let shape = match value {
        Value::Array(ArrayData::F64(array)) => array.shape(),
        Value::Array(ArrayData::ComplexF64(array)) => array.shape(),
        Value::Array(ArrayData::F32(array)) => array.shape(),
        Value::Array(ArrayData::ComplexF32(array)) => array.shape(),
        _ => return None,
    };
    (shape.ndims() == 2).then(|| (shape.extent(0), shape.extent(1)))
}

fn matrix_power_exponent(value: &Value) -> Option<(i64, bool)> {
    let (value, single) = if let Some(value) = value.as_real_single() {
        (f64::from(value), true)
    } else {
        (value.as_real_number()?, false)
    };
    if !value.is_finite()
        || value.fract() != 0.0
        || !(I64_MIN_AS_F64..-I64_MIN_AS_F64).contains(&value)
    {
        return None;
    }
    Some((f64_to_i64(value), single))
}

fn matrix_function_exponent(value: &Value) -> Option<(ArrayComplex64, bool)> {
    if let Some(value) = value.as_complex_single() {
        return Some((
            ArrayComplex64::new(f64::from(value.re), f64::from(value.im)),
            true,
        ));
    }
    value
        .as_complex_number()
        .map(|value| (ArrayComplex64::new(value.real, value.imaginary), false))
}

fn matrix_power_has_nonfinite(value: &Value) -> bool {
    match value {
        Value::Array(ArrayData::F64(array)) => {
            array.as_slice().iter().any(|value| !value.is_finite())
        }
        Value::Array(ArrayData::ComplexF64(array)) => array
            .as_slice()
            .iter()
            .any(|value| !value.re.is_finite() || !value.im.is_finite()),
        Value::Array(ArrayData::F32(array)) => {
            array.as_slice().iter().any(|value| !value.is_finite())
        }
        Value::Array(ArrayData::ComplexF32(array)) => array
            .as_slice()
            .iter()
            .any(|value| !value.re.is_finite() || !value.im.is_finite()),
        _ => false,
    }
}

fn matrix_power_nonfinite_result(
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    match value {
        Value::Array(ArrayData::F64(array)) => real_array_with_shape(
            array.shape().clone(),
            try_filled_output(array.as_slice().len(), f64::NAN, cancellation)?,
        ),
        Value::Array(ArrayData::ComplexF64(array)) => complex_array_with_shape(
            array.shape().clone(),
            try_filled_output(
                array.as_slice().len(),
                ArrayComplex64::new(f64::NAN, f64::NAN),
                cancellation,
            )?,
        ),
        Value::Array(ArrayData::F32(array)) => single_real_array_with_shape(
            array.shape().clone(),
            try_filled_output(array.as_slice().len(), f32::NAN, cancellation)?,
        ),
        Value::Array(ArrayData::ComplexF32(array)) => single_complex_array_with_shape(
            array.shape().clone(),
            try_filled_output(
                array.as_slice().len(),
                Complex32::new(f32::NAN, f32::NAN),
                cancellation,
            )?,
        ),
        _ => Err(invalid_operands(BinaryOperator::Power, value, value)),
    }
}

fn matrix_function_value64(
    array: DenseArray<ArrayComplex64>,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if array.as_slice().iter().all(|value| value.im == 0.0) {
        let mut values = try_output_vec(array.as_slice().len(), cancellation)?;
        for (index, value) in array.as_slice().iter().enumerate() {
            check_cancelled(cancellation, index)?;
            values.push(value.re);
        }
        real_array_with_shape(array.shape().clone(), values)
    } else {
        Ok(Value::Array(ArrayData::ComplexF64(array)))
    }
}

fn matrix_function_value32(
    array: DenseArray<Complex32>,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if array.as_slice().iter().all(|value| value.im == 0.0) {
        let mut values = try_output_vec(array.as_slice().len(), cancellation)?;
        for (index, value) in array.as_slice().iter().enumerate() {
            check_cancelled(cancellation, index)?;
            values.push(value.re);
        }
        single_real_array_with_shape(array.shape().clone(), values)
    } else {
        Ok(Value::Array(ArrayData::ComplexF32(array)))
    }
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_i64(value: f64) -> i64 {
    value as i64
}

fn matrix_power_precision(
    value: &Value,
    use_single: bool,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if !use_single
        || matches!(
            value,
            Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
        )
    {
        return Ok(value.clone());
    }
    match value {
        Value::Array(ArrayData::F64(array)) => {
            let mut values = try_output_vec(array.as_slice().len(), cancellation)?;
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled(cancellation, index)?;
                values.push(f64_to_f32(value));
            }
            single_real_array_with_shape(array.shape().clone(), values)
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let mut values = try_output_vec(array.as_slice().len(), cancellation)?;
            for (index, value) in array.as_slice().iter().copied().enumerate() {
                check_cancelled(cancellation, index)?;
                values.push(Complex32::new(f64_to_f32(value.re), f64_to_f32(value.im)));
            }
            single_complex_array_with_shape(array.shape().clone(), values)
        }
        _ => Err(invalid_operands(BinaryOperator::Power, value, value)),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn f64_to_f32(value: f64) -> f32 {
    value as f32
}

fn matrix_power_identity(
    order: u64,
    single: bool,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let length = host_len(order, order)?;
    if single {
        let mut values = try_filled_output(length, 0.0_f32, cancellation)?;
        for diagonal in 0..order {
            values[checked_usize(diagonal + diagonal * order)?] = 1.0;
        }
        single_real_array([order, order], values)
    } else {
        let mut values = try_filled_output(length, 0.0_f64, cancellation)?;
        for diagonal in 0..order {
            values[checked_usize(diagonal + diagonal * order)?] = 1.0;
        }
        real_array([order, order], values)
    }
}

fn is_double_matrix(value: &Value) -> bool {
    match value {
        Value::Array(ArrayData::F64(array)) => array.shape().ndims() == 2,
        Value::Array(ArrayData::ComplexF64(array)) => array.shape().ndims() == 2,
        _ => false,
    }
}

fn is_single_matrix(value: &Value) -> bool {
    matches!(value, Value::Array(array @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) if array.shape().ndims() == 2)
}

fn single_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if matches!(
        operator,
        BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Power
    ) || (operator == BinaryOperator::LeftDivide && (!lhs.is_scalar() || !rhs.is_scalar()))
    {
        return Err(RuntimeErrorKind::UnsupportedArrayOperator { operator });
    }
    let (Some(lhs_view), Some(rhs_view)) = (SingleBinaryView::new(lhs), SingleBinaryView::new(rhs))
    else {
        return Err(invalid_operands(operator, lhs, rhs));
    };
    let broadcast = BroadcastPlan::new(operator, lhs_view.dimensions(), rhs_view.dimensions())?;
    if matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::LessThan
            | BinaryOperator::LessThanOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterThanOrEqual
    ) {
        return single_comparison(
            operator,
            lhs,
            rhs,
            &lhs_view,
            &rhs_view,
            &broadcast,
            cancellation,
        );
    }
    single_arithmetic(operator, &lhs_view, &rhs_view, &broadcast, cancellation)
}

fn single_comparison(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    lhs_view: &SingleBinaryView<'_>,
    rhs_view: &SingleBinaryView<'_>,
    broadcast: &BroadcastPlan,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let ordered = matches!(
        operator,
        BinaryOperator::LessThan
            | BinaryOperator::LessThanOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterThanOrEqual
    );
    if ordered && (lhs_view.is_complex() || rhs_view.is_complex()) {
        return Err(invalid_operands(operator, lhs, rhs));
    }

    let length = checked_usize(broadcast.shape.numel())?;
    let use_double = lhs_view.uses_double_precision() || rhs_view.uses_double_precision();
    let mut output = try_output_vec(length, cancellation)?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        let (lhs_index, rhs_index) = broadcast.offsets(index)?;
        let result = if use_double {
            if ordered {
                compare_real(
                    operator,
                    lhs_view.real64_at(lhs_index).expect("real single operand"),
                    rhs_view.real64_at(rhs_index).expect("real single operand"),
                )
            } else {
                let equal = lhs_view.complex64_at(lhs_index) == rhs_view.complex64_at(rhs_index);
                if operator == BinaryOperator::Equal {
                    equal
                } else {
                    !equal
                }
            }
        } else if ordered {
            compare_real32(
                operator,
                lhs_view.real32_at(lhs_index).expect("real single operand"),
                rhs_view.real32_at(rhs_index).expect("real single operand"),
            )
        } else {
            let equal = lhs_view.complex32_at(lhs_index) == rhs_view.complex32_at(rhs_index);
            if operator == BinaryOperator::Equal {
                equal
            } else {
                !equal
            }
        };
        output.push(Logical::from(result));
    }
    if lhs_view.numel() == 1 && rhs_view.numel() == 1 {
        Ok(Value::Logical(
            output.first().is_some_and(|value| value.get()),
        ))
    } else {
        logical_array_with_shape(broadcast.shape.clone(), output)
    }
}

fn single_arithmetic(
    operator: BinaryOperator,
    lhs: &SingleBinaryView<'_>,
    rhs: &SingleBinaryView<'_>,
    broadcast: &BroadcastPlan,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let length = checked_usize(broadcast.shape.numel())?;
    let use_double = lhs.uses_double_precision() || rhs.uses_double_precision();
    let complex = lhs.is_complex() || rhs.is_complex();
    if operator == BinaryOperator::ElementPower {
        return single_element_power(
            lhs,
            rhs,
            broadcast,
            length,
            use_double,
            complex,
            cancellation,
        );
    }
    if complex {
        let mut output = try_output_vec(length, cancellation)?;
        for index in 0..length {
            check_cancelled(cancellation, index)?;
            let (lhs_index, rhs_index) = broadcast.offsets(index)?;
            let value = if use_double {
                let lhs = lhs.complex64_at(lhs_index).expect("single numeric operand");
                let rhs = rhs.complex64_at(rhs_index).expect("single numeric operand");
                if !complex64_is_finite(lhs) || !complex64_is_finite(rhs) {
                    return Err(RuntimeErrorKind::UnsupportedArrayOperator { operator });
                }
                complex32_from_complex64(arithmetic_single_complex64(operator, lhs, rhs))
            } else {
                let lhs = lhs.complex32_at(lhs_index).expect("single numeric operand");
                let rhs = rhs.complex32_at(rhs_index).expect("single numeric operand");
                if !complex32_is_finite(lhs) || !complex32_is_finite(rhs) {
                    return Err(RuntimeErrorKind::UnsupportedArrayOperator { operator });
                }
                arithmetic_complex32(operator, lhs, rhs)
            };
            output.push(value);
        }
        return single_complex_output(broadcast.shape.clone(), output, cancellation);
    }

    let mut output = try_output_vec(length, cancellation)?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        let (lhs_index, rhs_index) = broadcast.offsets(index)?;
        let value = if use_double {
            single_from_double(arithmetic_real64(
                operator,
                lhs.real64_at(lhs_index).expect("real single operand"),
                rhs.real64_at(rhs_index).expect("real single operand"),
            ))
        } else {
            arithmetic_real32(
                operator,
                lhs.real32_at(lhs_index).expect("real single operand"),
                rhs.real32_at(rhs_index).expect("real single operand"),
            )
        };
        output.push(value);
    }
    single_real_array_with_shape(broadcast.shape.clone(), output)
}

fn single_element_power(
    lhs: &SingleBinaryView<'_>,
    rhs: &SingleBinaryView<'_>,
    broadcast: &BroadcastPlan,
    length: usize,
    use_double: bool,
    complex: bool,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let mut output = try_output_vec(length, cancellation)?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        let (lhs_index, rhs_index) = broadcast.offsets(index)?;
        let value = if use_double {
            let lhs = lhs.complex64_at(lhs_index).expect("single numeric operand");
            let rhs = rhs.complex64_at(rhs_index).expect("single numeric operand");
            let result = if complex {
                complex_power64(lhs, rhs).ok_or(RuntimeErrorKind::UnsupportedArrayOperator {
                    operator: BinaryOperator::ElementPower,
                })?
            } else {
                real_power64(lhs.real, rhs.real)
            };
            complex32_from_complex64(result)
        } else {
            let lhs = lhs.complex32_at(lhs_index).expect("single numeric operand");
            let rhs = rhs.complex32_at(rhs_index).expect("single numeric operand");
            if complex {
                complex_power32(lhs, rhs).ok_or(RuntimeErrorKind::UnsupportedArrayOperator {
                    operator: BinaryOperator::ElementPower,
                })?
            } else {
                real_power32(lhs.re, rhs.re)
            }
        };
        output.push(value);
    }
    single_complex_output(broadcast.shape.clone(), output, cancellation)
}

fn single_complex_output(
    shape: Shape,
    values: Vec<Complex32>,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let mut complex = false;
    for (index, value) in values.iter().enumerate() {
        check_cancelled(cancellation, index)?;
        if value.im != 0.0 {
            complex = true;
            break;
        }
    }
    if complex {
        return single_complex_array_with_shape(shape, values);
    }
    let mut real = try_output_vec(values.len(), cancellation)?;
    for (index, value) in values.into_iter().enumerate() {
        check_cancelled(cancellation, index)?;
        real.push(value.re);
    }
    single_real_array_with_shape(shape, real)
}

fn exact_equality(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let (lhs_dimensions, rhs_dimensions) = match (lhs, rhs) {
        (Value::Array(ArrayData::Char(lhs)), Value::Array(ArrayData::Char(rhs))) => {
            (lhs.shape().dimensions(), rhs.shape().dimensions())
        }
        (Value::Array(ArrayData::Integer(lhs)), Value::Array(ArrayData::Integer(rhs)))
            if lhs.dtype() == rhs.dtype() =>
        {
            (lhs.shape().dimensions(), rhs.shape().dimensions())
        }
        _ => return Err(invalid_operands(operator, lhs, rhs)),
    };
    let broadcast = BroadcastPlan::new(operator, lhs_dimensions, rhs_dimensions)?;
    let length = checked_usize(broadcast.shape.numel())?;
    let mut values = try_output_vec(length, cancellation)?;
    for index in 0..length {
        check_cancelled(cancellation, index)?;
        let (lhs_index, rhs_index) = broadcast.offsets(index)?;
        let equal = match (lhs, rhs) {
            (Value::Array(ArrayData::Char(lhs)), Value::Array(ArrayData::Char(rhs))) => {
                lhs.as_slice()[lhs_index] == rhs.as_slice()[rhs_index]
            }
            (Value::Array(ArrayData::Integer(lhs)), Value::Array(ArrayData::Integer(rhs))) => {
                lhs.element(lhs_index) == rhs.element(rhs_index)
            }
            _ => unreachable!("exact equality operands were validated"),
        };
        values.push(Logical::from(if operator == BinaryOperator::Equal {
            equal
        } else {
            !equal
        }));
    }
    if lhs.is_scalar() && rhs.is_scalar() {
        return Ok(Value::Logical(
            values.first().is_some_and(|value| value.get()),
        ));
    }
    logical_array_with_shape(broadcast.shape, values)
}

fn scalar_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeErrorKind> {
    match operator {
        BinaryOperator::Equal => return Ok(Value::Logical(values_equal(lhs, rhs))),
        BinaryOperator::NotEqual => return Ok(Value::Logical(!values_equal(lhs, rhs))),
        BinaryOperator::LessThan
        | BinaryOperator::LessThanOrEqual
        | BinaryOperator::GreaterThan
        | BinaryOperator::GreaterThanOrEqual => {
            let (Some(lhs), Some(rhs)) = (lhs.as_real_number(), rhs.as_real_number()) else {
                return Err(invalid_operands(operator, lhs, rhs));
            };
            return Ok(Value::Logical(compare_real(operator, lhs, rhs)));
        }
        BinaryOperator::Power | BinaryOperator::ElementPower => {
            let (Some(lhs), Some(rhs)) = (lhs.as_real_number(), rhs.as_real_number()) else {
                return Err(invalid_operands(operator, lhs, rhs));
            };
            return Ok(Value::Double(lhs.powf(rhs)));
        }
        BinaryOperator::Add
        | BinaryOperator::Subtract
        | BinaryOperator::Multiply
        | BinaryOperator::ElementMultiply
        | BinaryOperator::Divide
        | BinaryOperator::LeftDivide
        | BinaryOperator::ElementDivide
        | BinaryOperator::ElementLeftDivide => {}
    }
    let (Some(lhs_value), Some(rhs_value)) = (lhs.as_complex_number(), rhs.as_complex_number())
    else {
        return Err(invalid_operands(operator, lhs, rhs));
    };
    let result = arithmetic_complex(operator, lhs_value, rhs_value);
    if lhs.is_complex_numeric() || rhs.is_complex_numeric() {
        Ok(Value::Complex(result))
    } else {
        Ok(Value::Double(result.real))
    }
}

fn elementwise_binary(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let lhs_view = NumericView::new(lhs, "element-wise binary operation")?;
    let rhs_view = NumericView::new(rhs, "element-wise binary operation")?;
    let broadcast = BroadcastPlan::new(operator, lhs_view.dimensions(), rhs_view.dimensions())?;
    let length = checked_usize(broadcast.shape.numel())?;
    let comparison = matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::LessThan
            | BinaryOperator::LessThanOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterThanOrEqual
    );
    if comparison {
        let mut output = try_output_vec(length, cancellation)?;
        for index in 0..length {
            check_cancelled(cancellation, index)?;
            let (lhs_index, rhs_index) = broadcast.offsets(index)?;
            let result = match operator {
                BinaryOperator::Equal | BinaryOperator::NotEqual => {
                    let equal = lhs_view.complex_at(lhs_index) == rhs_view.complex_at(rhs_index);
                    if operator == BinaryOperator::Equal {
                        equal
                    } else {
                        !equal
                    }
                }
                _ => {
                    let (Some(lhs), Some(rhs)) =
                        (lhs_view.real_at(lhs_index), rhs_view.real_at(rhs_index))
                    else {
                        return Err(invalid_operands(operator, lhs, rhs));
                    };
                    compare_real(operator, lhs, rhs)
                }
            };
            output.push(Logical::from(result));
        }
        return logical_array_with_shape(broadcast.shape, output);
    }

    let complex = lhs_view.is_complex() || rhs_view.is_complex();
    if complex && operator == BinaryOperator::ElementPower {
        return Err(invalid_operands(operator, lhs, rhs));
    }
    if complex {
        let mut output = try_output_vec(length, cancellation)?;
        for index in 0..length {
            check_cancelled(cancellation, index)?;
            let (lhs_index, rhs_index) = broadcast.offsets(index)?;
            let lhs = lhs_view
                .complex_at(lhs_index)
                .expect("validated numeric element");
            let rhs = rhs_view
                .complex_at(rhs_index)
                .expect("validated numeric element");
            output.push(ArrayComplex64::from(arithmetic_complex(operator, lhs, rhs)));
        }
        complex_array_with_shape(broadcast.shape, output)
    } else {
        let mut output = try_output_vec(length, cancellation)?;
        for index in 0..length {
            check_cancelled(cancellation, index)?;
            let (lhs_index, rhs_index) = broadcast.offsets(index)?;
            let lhs = lhs_view.real_at(lhs_index).expect("real numeric element");
            let rhs = rhs_view.real_at(rhs_index).expect("real numeric element");
            output.push(match operator {
                BinaryOperator::Add => lhs + rhs,
                BinaryOperator::Subtract => lhs - rhs,
                BinaryOperator::Multiply | BinaryOperator::ElementMultiply => lhs * rhs,
                BinaryOperator::Divide | BinaryOperator::ElementDivide => lhs / rhs,
                BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => rhs / lhs,
                BinaryOperator::ElementPower => lhs.powf(rhs),
                _ => unreachable!("comparison operators returned above"),
            });
        }
        real_array_with_shape(broadcast.shape, output)
    }
}

fn matrix_multiply(
    lhs: &Value,
    rhs: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let lhs = NumericView::new(lhs, "matrix multiplication")?;
    let rhs = NumericView::new(rhs, "matrix multiplication")?;
    require_two_dimensions(&lhs, "matrix multiplication")?;
    require_two_dimensions(&rhs, "matrix multiplication")?;
    let (rows, inner, rhs_inner, columns) = (
        lhs.dimensions()[0],
        lhs.dimensions()[1],
        rhs.dimensions()[0],
        rhs.dimensions()[1],
    );
    if inner != rhs_inner {
        return Err(array_error(ArrayRuntimeError::ShapeMismatch {
            operation: "matrix multiplication",
            lhs: lhs.dimensions().to_vec(),
            rhs: rhs.dimensions().to_vec(),
        }));
    }
    let output_len = host_len(rows, columns)?;
    let complex = lhs.is_complex() || rhs.is_complex();
    if complex {
        let mut output = try_filled_output(output_len, ArrayComplex64::ZERO, cancellation)?;
        let mut visited = 0_usize;
        for column in 0..columns {
            for row in 0..rows {
                let mut sum = Complex64::new(0.0, 0.0);
                for position in 0..inner {
                    check_cancelled(cancellation, visited)?;
                    let lhs_offset = checked_usize(row + position * rows)?;
                    let rhs_offset = checked_usize(position + column * rhs_inner)?;
                    sum = sum
                        + lhs.complex_at(lhs_offset).expect("matrix lhs offset")
                            * rhs.complex_at(rhs_offset).expect("matrix rhs offset");
                    visited = visited.saturating_add(1);
                }
                output[checked_usize(row + column * rows)?] = sum.into();
            }
        }
        complex_array([rows, columns], output)
    } else {
        let mut output = try_filled_output(output_len, 0.0, cancellation)?;
        let mut visited = 0_usize;
        for column in 0..columns {
            for row in 0..rows {
                let mut sum = 0.0_f64;
                for position in 0..inner {
                    check_cancelled(cancellation, visited)?;
                    let lhs_offset = checked_usize(row + position * rows)?;
                    let rhs_offset = checked_usize(position + column * rhs_inner)?;
                    sum = lhs
                        .real_at(lhs_offset)
                        .expect("real matrix lhs offset")
                        .mul_add(
                            rhs.real_at(rhs_offset).expect("real matrix rhs offset"),
                            sum,
                        );
                    visited = visited.saturating_add(1);
                }
                output[checked_usize(row + column * rows)?] = sum;
            }
        }
        real_array([rows, columns], output)
    }
}

pub(crate) fn index(
    target: &Value,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if let Value::Sparse(sparse) = target {
        return index_sparse(sparse, arguments, cancellation);
    }
    let view = ArrayView::new(target, "indexing")?;
    let resolved = resolve_index_selection(view.dimensions(), arguments, cancellation)?;
    gather_value(&view, &resolved, cancellation)
}

pub(crate) fn index_assign(
    target: &Value,
    arguments: &[IndexInput],
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if matches!(target, Value::Nothing) {
        return grow_undefined_numeric(arguments, value, cancellation);
    }
    let target_view = ArrayView::new(target, "indexed assignment")?;
    let resolved = resolve_index_selection(target_view.dimensions(), arguments, cancellation)?;
    match target {
        Value::String(target) => {
            return index_assign_string(target, &resolved, value, cancellation);
        }
        Value::Array(ArrayData::Char(target)) => {
            let Value::Array(ArrayData::Char(supplied)) = value else {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "char indexed assignment",
                    actual: value.kind(),
                }));
            };
            return assign_exact_array(target, supplied, &resolved, cancellation);
        }
        Value::Array(ArrayData::Integer(target)) => {
            let Value::Array(ArrayData::Integer(supplied)) = value else {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "integer indexed assignment",
                    actual: value.kind(),
                }));
            };
            return assign_integer_array(target, supplied, &resolved, cancellation)
                .map(ArrayData::Integer)
                .map(Value::Array);
        }
        Value::Array(target @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) => {
            let Value::Array(supplied @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) = value
            else {
                return Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "single indexed assignment",
                    actual: value.kind(),
                }));
            };
            return assign_single_array(target, supplied, &resolved, cancellation)
                .map(Value::Array);
        }
        _ => {}
    }
    let supplied = NumericView::new(value, "indexed assignment value")?;
    let selected = u64::try_from(resolved.offsets.len()).map_err(|_| array_size_error())?;
    if supplied.numel() != 1 && supplied.numel() != selected {
        return Err(array_error(ArrayRuntimeError::AssignmentSizeMismatch {
            selected,
            supplied: supplied.numel(),
        }));
    }
    if resolved.offsets.is_empty() {
        return Ok(target.clone());
    }

    let mut assigned = owned_assignment_array(target, supplied.is_complex(), cancellation)?;

    match &mut assigned {
        ArrayData::F64(array) => {
            let data = array.as_mut_slice();
            for (position, &offset) in resolved.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                let source = if supplied.numel() == 1 { 0 } else { position };
                data[offset] = supplied
                    .real_at(source)
                    .ok_or_else(|| invalid_operands(BinaryOperator::Add, target, value))?;
            }
        }
        ArrayData::ComplexF64(array) => {
            let data = array.as_mut_slice();
            for (position, &offset) in resolved.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                let source = if supplied.numel() == 1 { 0 } else { position };
                data[offset] = supplied
                    .complex_at(source)
                    .expect("validated numeric assignment value")
                    .into();
            }
        }
        ArrayData::Logical(array) => {
            let data = array.as_mut_slice();
            for (position, &offset) in resolved.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                let source = if supplied.numel() == 1 { 0 } else { position };
                let value = supplied
                    .complex_at(source)
                    .expect("validated numeric assignment value");
                data[offset] = Logical::from(value.real != 0.0 || value.imaginary != 0.0);
            }
        }
        ArrayData::F32(_)
        | ArrayData::ComplexF32(_)
        | ArrayData::Char(_)
        | ArrayData::Integer(_) => {
            unreachable!("exact indexed assignment returned before numeric dispatch")
        }
    }
    Ok(Value::Array(assigned))
}

fn owned_assignment_array(
    target: &Value,
    complex: bool,
    cancellation: &CancellationToken,
) -> Result<ArrayData, RuntimeErrorKind> {
    match owned_array(target)? {
        ArrayData::F64(real) if complex => {
            let values = promote_real_values(real.as_slice(), cancellation)?;
            Ok(ArrayData::ComplexF64(
                DenseArray::from_vec(real.shape().clone(), values)
                    .map_err(|_| array_size_error())?,
            ))
        }
        assigned => Ok(assigned),
    }
}

/// Updates one scalar numeric or logical element after completing every
/// fallible validation.
///
/// Returning `Ok(false)` leaves `target` untouched and asks the caller to use
/// the general transactional assignment path, including cases which promote a
/// real array to complex storage. This path lets a binding owner mutate unique
/// storage without VM-temporary clones forcing a complete copy; genuine
/// language aliases still detach inside `DenseArray::as_mut_slice`.
pub(crate) fn try_index_assign_scalar_in_place(
    target: &mut Value,
    index: &Value,
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<bool, RuntimeErrorKind> {
    if !is_direct_scalar_index(index) {
        return Ok(false);
    }
    match target {
        Value::Array(ArrayData::F64(array)) => {
            let supplied = checked_numeric_scalar(value)?;
            if supplied.is_complex() {
                return Ok(false);
            }
            let one_based = checked_scalar_index_value(index, 0, array.numel(), cancellation)?;
            *array
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") = supplied
                .real_at(0)
                .expect("validated real scalar assignment source");
            Ok(true)
        }
        Value::Array(ArrayData::ComplexF64(array)) => {
            let supplied = checked_numeric_scalar(value)?;
            let one_based = checked_scalar_index_value(index, 0, array.numel(), cancellation)?;
            let supplied = supplied
                .complex_at(0)
                .expect("validated numeric scalar assignment source");
            *array
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") =
                ArrayComplex64::new(supplied.real, supplied.imaginary);
            Ok(true)
        }
        Value::Array(ArrayData::Logical(array)) => {
            let supplied = checked_numeric_scalar(value)?;
            let one_based = checked_scalar_index_value(index, 0, array.numel(), cancellation)?;
            let supplied = supplied
                .complex_at(0)
                .expect("validated numeric scalar assignment source");
            *array
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") =
                Logical::from(supplied.real != 0.0 || supplied.imaginary != 0.0);
            Ok(true)
        }
        Value::Array(target @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) => {
            let Value::Array(supplied @ (ArrayData::F32(_) | ArrayData::ComplexF32(_))) = value
            else {
                return Ok(false);
            };
            if supplied.numel() != 1 {
                return Err(assignment_size_mismatch(supplied.numel()));
            }
            let one_based = checked_scalar_index_value(index, 0, target.numel(), cancellation)?;
            Ok(try_assign_single_scalar_in_place(
                target, supplied, one_based,
            ))
        }
        Value::Array(ArrayData::Integer(target)) => {
            let Value::Array(ArrayData::Integer(supplied)) = value else {
                return Ok(false);
            };
            if supplied.numel() != 1 {
                return Err(assignment_size_mismatch(supplied.numel()));
            }
            let one_based = checked_scalar_index_value(index, 0, target.numel(), cancellation)?;
            Ok(try_assign_integer_scalar_in_place(
                target, supplied, one_based,
            ))
        }
        Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Array(ArrayData::Char(_))
        | Value::String(_)
        | Value::Nothing
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Sparse(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => Ok(false),
    }
}

fn try_assign_single_scalar_in_place(
    target: &mut ArrayData,
    supplied: &ArrayData,
    one_based: u64,
) -> bool {
    match (target, supplied) {
        (ArrayData::F32(target), ArrayData::F32(supplied)) => {
            *target
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") = supplied.as_slice()[0];
            true
        }
        (ArrayData::ComplexF32(target), ArrayData::F32(supplied)) => {
            *target
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") =
                Complex32::from(supplied.as_slice()[0]);
            true
        }
        (ArrayData::ComplexF32(target), ArrayData::ComplexF32(supplied)) => {
            *target
                .get_mut_linear(one_based)
                .expect("validated scalar assignment index") = supplied.as_slice()[0];
            true
        }
        _ => false,
    }
}

fn index_sparse(
    target: &SparseArrayData,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let indexed = match arguments {
        [IndexInput::Value(index)] => {
            let index = checked_scalar_index_value(index, 0, target.numel(), cancellation)?;
            target.try_index_linear_one_based(index, Some(cancellation.atomic_flag()))
        }
        [IndexInput::Value(row), IndexInput::Value(column)] => {
            let row = checked_scalar_index_value(row, 0, target.shape().extent(0), cancellation)?;
            let column =
                checked_scalar_index_value(column, 1, target.shape().extent(1), cancellation)?;
            target.try_index_subscripts_one_based(row, column, Some(cancellation.atomic_flag()))
        }
        [] => {
            return Err(array_error(ArrayRuntimeError::InvalidIndex {
                argument: 0,
                reason: IndexErrorKind::NoSubscripts,
            }));
        }
        _ => {
            return Err(array_error(ArrayRuntimeError::InvalidOperand {
                operation: "non-scalar sparse indexing",
                actual: openmat_value::ValueKind::Sparse,
            }));
        }
    };
    indexed
        .map(Value::Sparse)
        .map_err(|error| sparse_runtime_error(&error))
}

fn sparse_runtime_error(error: &SparseError) -> RuntimeErrorKind {
    match error {
        SparseError::Cancelled => RuntimeErrorKind::Cancelled,
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
        _ => array_size_error(),
    }
}

fn checked_numeric_scalar(value: &Value) -> Result<NumericView<'_>, RuntimeErrorKind> {
    let supplied = NumericView::new(value, "indexed assignment value")?;
    if supplied.numel() != 1 {
        return Err(assignment_size_mismatch(supplied.numel()));
    }
    Ok(supplied)
}

fn assignment_size_mismatch(supplied: u64) -> RuntimeErrorKind {
    array_error(ArrayRuntimeError::AssignmentSizeMismatch {
        selected: 1,
        supplied,
    })
}

fn try_assign_integer_scalar_in_place(
    target: &mut IntegerArrayData,
    supplied: &IntegerArrayData,
    one_based: u64,
) -> bool {
    macro_rules! assign_classes {
        ($(($real:ident, $complex:ident, $component:ty)),+ $(,)?) => {
            match (target, supplied) {
                $((IntegerArrayData::$real(target), IntegerArrayData::$real(supplied)) => {
                    *target
                        .get_mut_linear(one_based)
                        .expect("validated scalar assignment index") = supplied.as_slice()[0];
                    true
                },
                (IntegerArrayData::$real(_), IntegerArrayData::$complex(_)) => false,
                (IntegerArrayData::$complex(target), IntegerArrayData::$real(supplied)) => {
                    *target
                        .get_mut_linear(one_based)
                        .expect("validated scalar assignment index") = ComplexInteger::new(
                            supplied.as_slice()[0],
                            <$component>::default(),
                        );
                    true
                },
                (IntegerArrayData::$complex(target), IntegerArrayData::$complex(supplied)) => {
                    *target
                        .get_mut_linear(one_based)
                        .expect("validated scalar assignment index") = supplied.as_slice()[0];
                    true
                },)+
                _ => false,
            }
        };
    }
    assign_classes!(
        (I8, ComplexI8, i8),
        (U8, ComplexU8, u8),
        (I16, ComplexI16, i16),
        (U16, ComplexU16, u16),
        (I32, ComplexI32, i32),
        (U32, ComplexU32, u32),
        (I64, ComplexI64, i64),
        (U64, ComplexU64, u64),
    )
}

pub(crate) fn try_index_real_scalar(
    target: &Value,
    index: &Value,
    cancellation: &CancellationToken,
) -> Result<Option<Value>, RuntimeErrorKind> {
    let Value::Array(ArrayData::F64(array)) = target else {
        return Ok(None);
    };
    if !is_direct_scalar_index(index) {
        return Ok(None);
    }
    let one_based = checked_scalar_index_value(index, 0, array.numel(), cancellation)?;
    Ok(Some(Value::Double(
        *array
            .get_linear(one_based)
            .expect("validated scalar indexing offset"),
    )))
}

fn is_direct_scalar_index(value: &Value) -> bool {
    IndexView::new(value).is_some_and(|view| view.numel() == 1 && !view.is_logical())
}

fn checked_scalar_index_value(
    value: &Value,
    argument: usize,
    extent: u64,
    cancellation: &CancellationToken,
) -> Result<u64, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    let view = IndexView::new(value).ok_or_else(|| {
        array_error(ArrayRuntimeError::InvalidIndex {
            argument,
            reason: IndexErrorKind::NonNumeric,
        })
    })?;
    if view.numel() != 1 || view.is_logical() {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument,
            reason: IndexErrorKind::NonInteger,
        }));
    }
    checked_index_at(&view, 0, argument, extent)
}

fn assign_single_array(
    target: &ArrayData,
    supplied: &ArrayData,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<ArrayData, RuntimeErrorKind> {
    assignment_length(supplied.numel(), selection.offsets.len())?;
    if selection.offsets.is_empty() {
        return Ok(target.clone());
    }

    match (target, supplied) {
        (ArrayData::F32(target), ArrayData::F32(supplied)) => {
            let mut assigned = target.clone();
            assign_single_real(assigned.as_mut_slice(), supplied, selection, cancellation)?;
            Ok(ArrayData::F32(assigned))
        }
        (ArrayData::F32(target), ArrayData::ComplexF32(supplied)) => {
            let values = promote_single_real_values(target.as_slice(), cancellation)?;
            let mut assigned = DenseArray::from_vec(target.shape().clone(), values)
                .map_err(|_| array_size_error())?;
            assign_single_complex(assigned.as_mut_slice(), supplied, selection, cancellation)?;
            Ok(ArrayData::ComplexF32(assigned))
        }
        (ArrayData::ComplexF32(target), ArrayData::F32(supplied)) => {
            let mut assigned = target.clone();
            let data = assigned.as_mut_slice();
            for (position, &offset) in selection.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                let source = if supplied.numel() == 1 { 0 } else { position };
                data[offset] = Complex32::from(supplied.as_slice()[source]);
            }
            Ok(ArrayData::ComplexF32(assigned))
        }
        (ArrayData::ComplexF32(target), ArrayData::ComplexF32(supplied)) => {
            let mut assigned = target.clone();
            assign_single_complex(assigned.as_mut_slice(), supplied, selection, cancellation)?;
            Ok(ArrayData::ComplexF32(assigned))
        }
        _ => unreachable!("single assignment operands validated by index_assign"),
    }
}

fn assign_single_real(
    target: &mut [f32],
    supplied: &DenseArray<f32>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<(), RuntimeErrorKind> {
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        target[offset] = supplied.as_slice()[source];
    }
    Ok(())
}

fn assign_single_complex(
    target: &mut [Complex32],
    supplied: &DenseArray<Complex32>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<(), RuntimeErrorKind> {
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        target[offset] = supplied.as_slice()[source];
    }
    Ok(())
}

fn assignment_length(supplied: u64, selected: usize) -> Result<(), RuntimeErrorKind> {
    let selected = u64::try_from(selected).map_err(|_| array_size_error())?;
    if supplied == 1 || supplied == selected {
        Ok(())
    } else {
        Err(array_error(ArrayRuntimeError::AssignmentSizeMismatch {
            selected,
            supplied,
        }))
    }
}

fn assign_exact_array<T: ArrayElement + Clone>(
    target: &DenseArray<T>,
    supplied: &DenseArray<T>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    assignment_length(supplied.numel(), selection.offsets.len())?;
    if selection.offsets.is_empty() {
        return Ok(Value::Array(ArrayData::from_typed(target.clone())));
    }
    let mut assigned = target.clone();
    let output = assigned.as_mut_slice();
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        output[offset] = supplied.as_slice()[source].clone();
    }
    Ok(Value::Array(ArrayData::from_typed(assigned)))
}

fn index_assign_string(
    target: &StringValue,
    selection: &ResolvedSelection,
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let Value::String(supplied) = value else {
        return Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "string indexed assignment",
            actual: value.kind(),
        }));
    };
    assignment_length(supplied.numel(), selection.offsets.len())?;
    if selection.offsets.is_empty() {
        return Ok(Value::String(target.clone()));
    }
    let mut assigned = match target {
        StringValue::Scalar(element) => StringArray::from_elements(
            Shape::new([1, 1]).map_err(|_| array_size_error())?,
            vec![element.clone()],
        )
        .map_err(|_| array_size_error())?,
        StringValue::Array(array) => array.clone(),
    };
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        assigned
            .replace_linear(
                u64::try_from(offset).map_err(|_| array_size_error())? + 1,
                supplied
                    .element(source)
                    .expect("validated string assignment source")
                    .clone(),
            )
            .map_err(|_| array_size_error())?;
    }
    Ok(Value::String(StringValue::Array(assigned)))
}

fn assign_integer_array(
    target: &IntegerArrayData,
    supplied: &IntegerArrayData,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<IntegerArrayData, RuntimeErrorKind> {
    assignment_length(supplied.numel(), selection.offsets.len())?;
    if selection.offsets.is_empty() {
        if target.class_name() == supplied.class_name() {
            return Ok(target.clone());
        }
        return Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "mixed integer-class indexed assignment",
            actual: openmat_value::ValueKind::Integer,
        }));
    }
    macro_rules! assign_classes {
        ($(($real:ident, $complex:ident, $component:ty)),+ $(,)?) => {
            match (target, supplied) {
                $((IntegerArrayData::$real(target), IntegerArrayData::$real(supplied)) => {
                    assign_integer_same(target, supplied, selection, cancellation)
                        .map(IntegerArrayData::$real)
                },
                (IntegerArrayData::$real(target), IntegerArrayData::$complex(supplied)) => {
                    assign_integer_promoted(target, supplied, selection, cancellation)
                        .map(IntegerArrayData::$complex)
                },
                (IntegerArrayData::$complex(target), IntegerArrayData::$real(supplied)) => {
                    assign_integer_real_into_complex(target, supplied, selection, cancellation)
                        .map(IntegerArrayData::$complex)
                },
                (IntegerArrayData::$complex(target), IntegerArrayData::$complex(supplied)) => {
                    assign_integer_same(target, supplied, selection, cancellation)
                        .map(IntegerArrayData::$complex)
                },)+
                _ => Err(array_error(ArrayRuntimeError::InvalidOperand {
                    operation: "mixed integer-class indexed assignment",
                    actual: openmat_value::ValueKind::Integer,
                })),
            }
        };
    }
    assign_classes!(
        (I8, ComplexI8, i8),
        (U8, ComplexU8, u8),
        (I16, ComplexI16, i16),
        (U16, ComplexU16, u16),
        (I32, ComplexI32, i32),
        (U32, ComplexU32, u32),
        (I64, ComplexI64, i64),
        (U64, ComplexU64, u64),
    )
}

fn assign_integer_same<T: IntegerElement + Copy>(
    target: &DenseArray<T>,
    supplied: &DenseArray<T>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<DenseArray<T>, RuntimeErrorKind> {
    assignment_length(supplied.numel(), selection.offsets.len())?;
    let mut assigned = target.clone();
    if selection.offsets.is_empty() {
        return Ok(assigned);
    }
    let output = assigned.as_mut_slice();
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        output[offset] = supplied.as_slice()[source];
    }
    Ok(assigned)
}

fn assign_integer_promoted<T: IntegerElement + Copy + Default>(
    target: &DenseArray<T>,
    supplied: &DenseArray<ComplexInteger<T>>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<DenseArray<ComplexInteger<T>>, RuntimeErrorKind>
where
    ComplexInteger<T>: IntegerElement,
{
    assignment_length(supplied.numel(), selection.offsets.len())?;
    let mut values = try_output_vec(target.as_slice().len(), cancellation)?;
    for (index, value) in target.as_slice().iter().copied().enumerate() {
        check_cancelled(cancellation, index)?;
        values.push(ComplexInteger::new(value, T::default()));
    }
    let mut assigned =
        DenseArray::from_vec(target.shape().clone(), values).map_err(|_| array_size_error())?;
    let output = assigned.as_mut_slice();
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        output[offset] = supplied.as_slice()[source];
    }
    Ok(assigned)
}

fn assign_integer_real_into_complex<T: IntegerElement + Copy + Default>(
    target: &DenseArray<ComplexInteger<T>>,
    supplied: &DenseArray<T>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<DenseArray<ComplexInteger<T>>, RuntimeErrorKind>
where
    ComplexInteger<T>: IntegerElement,
{
    assignment_length(supplied.numel(), selection.offsets.len())?;
    let mut assigned = target.clone();
    let output = assigned.as_mut_slice();
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        let source = if supplied.numel() == 1 { 0 } else { position };
        output[offset] = ComplexInteger::new(supplied.as_slice()[source], T::default());
    }
    Ok(assigned)
}

pub(crate) struct ResolvedSelection {
    pub(crate) offsets: Vec<usize>,
    pub(crate) shape: Shape,
}

struct DimensionSelection {
    indices: Vec<u64>,
    source_shape: Vec<u64>,
    colon: bool,
    logical: bool,
}

pub(crate) fn resolve_index_selection(
    dimensions: &[u64],
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<ResolvedSelection, RuntimeErrorKind> {
    let shape = Shape::new(dimensions.iter().copied()).map_err(|_| array_size_error())?;
    resolve_indices(&shape, arguments, cancellation)
}

fn resolve_indices(
    shape: &Shape,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<ResolvedSelection, RuntimeErrorKind> {
    if arguments.is_empty() {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NoSubscripts,
        }));
    }
    let extents = shape
        .effective_dimensions(arguments.len())
        .map_err(|_| array_size_error())?;
    let mut selections = try_output_vec(arguments.len(), cancellation)?;
    for (argument, (input, &extent)) in arguments.iter().zip(&extents).enumerate() {
        selections.push(resolve_dimension(input, extent, argument, cancellation)?);
    }

    let result_dimensions = if arguments.len() == 1 {
        linear_result_dimensions(shape, &selections[0])
    } else {
        let mut dimensions = try_output_vec(selections.len(), cancellation)?;
        for selection in &selections {
            dimensions
                .push(u64::try_from(selection.indices.len()).map_err(|_| array_size_error())?);
        }
        dimensions
    };
    let result_shape = Shape::new(result_dimensions).map_err(|_| array_size_error())?;
    let output_len = checked_usize(result_shape.numel())?;
    let mut offsets = try_output_vec(output_len, cancellation)?;
    if arguments.len() == 1 {
        for (position, &index) in selections[0].indices.iter().enumerate() {
            check_cancelled(cancellation, position)?;
            if index == 0 || index > shape.numel() {
                return Err(array_error(ArrayRuntimeError::IndexOutOfBounds {
                    argument: 0,
                    index,
                    extent: shape.numel(),
                }));
            }
            offsets.push(checked_usize(index - 1)?);
        }
    } else {
        for linear in 0..output_len {
            check_cancelled(cancellation, linear)?;
            let mut remaining = u64::try_from(linear).map_err(|_| array_size_error())?;
            let mut subscripts = try_reserved_vec(selections.len())?;
            for selection in &selections {
                let length =
                    u64::try_from(selection.indices.len()).map_err(|_| array_size_error())?;
                if length == 0 {
                    break;
                }
                let position = checked_usize(remaining % length)?;
                remaining /= length;
                subscripts.push(selection.indices[position]);
            }
            if subscripts.len() == selections.len() {
                let offset = shape
                    .offset_for_subscripts(&subscripts)
                    .map_err(|_| array_size_error())?;
                offsets.push(checked_usize(offset)?);
            }
        }
    }
    Ok(ResolvedSelection {
        offsets,
        shape: result_shape,
    })
}

pub(crate) fn scalar_linear_index(
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<u64, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    if arguments.is_empty() {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NoSubscripts,
        }));
    }
    if arguments.len() != 1 {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 1,
            reason: IndexErrorKind::NonNumeric,
        }));
    }
    let IndexInput::Value(value) = &arguments[0] else {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NonNumeric,
        }));
    };
    let view = IndexView::new(value).ok_or_else(|| {
        array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NonNumeric,
        })
    })?;
    if view.numel() != 1 || view.is_logical() {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NonInteger,
        }));
    }
    checked_index_at(&view, 0, 0, u64::MAX)
}

/// Largest positive numeric subscript, used when sizing an assignment target.
/// Logical selectors retain their separate indexing rules; an empty selection
/// must never grow a target. Values are validated without converting integers
/// through floating point.
pub(crate) fn numeric_assignment_extent(
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<Option<u64>, RuntimeErrorKind> {
    let Some(view) = IndexView::new(value) else {
        return Ok(None);
    };
    if view.is_logical() || view.numel() == 0 {
        return Ok(None);
    }
    let mut maximum = 0;
    let length =
        usize::try_from(view.numel()).map_err(|_| array_error(ArrayRuntimeError::SizeLimit))?;
    for offset in 0..length {
        check_cancelled(cancellation, offset)?;
        maximum = maximum.max(checked_index_at(&view, offset, 0, u64::MAX)?);
    }
    Ok(Some(maximum))
}

pub(crate) fn scalar_subscripts(
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<Vec<u64>, RuntimeErrorKind> {
    if arguments.is_empty() {
        return Err(array_error(ArrayRuntimeError::InvalidIndex {
            argument: 0,
            reason: IndexErrorKind::NoSubscripts,
        }));
    }
    let mut indices = try_output_vec(arguments.len(), cancellation)?;
    for (argument, input) in arguments.iter().enumerate() {
        check_cancelled(cancellation, argument)?;
        let IndexInput::Value(value) = input else {
            return Err(array_error(ArrayRuntimeError::InvalidIndex {
                argument,
                reason: IndexErrorKind::NonNumeric,
            }));
        };
        let view = IndexView::new(value).ok_or_else(|| {
            array_error(ArrayRuntimeError::InvalidIndex {
                argument,
                reason: IndexErrorKind::NonNumeric,
            })
        })?;
        if view.numel() != 1 || view.is_logical() {
            return Err(array_error(ArrayRuntimeError::InvalidIndex {
                argument,
                reason: IndexErrorKind::NonInteger,
            }));
        }
        indices.push(checked_index_at(&view, 0, argument, u64::MAX)?);
    }
    Ok(indices)
}

fn checked_index_at(
    view: &IndexView<'_>,
    offset: usize,
    argument: usize,
    extent: u64,
) -> Result<u64, RuntimeErrorKind> {
    let invalid = |reason| array_error(ArrayRuntimeError::InvalidIndex { argument, reason });
    let index = match view {
        IndexView::RealScalar(value) if offset == 0 => checked_float_index(*value, &invalid)?,
        IndexView::RealArray(array) => checked_float_index(array.as_slice()[offset], &invalid)?,
        IndexView::SingleRealArray(array) => {
            checked_float_index(f64::from(array.as_slice()[offset]), &invalid)?
        }
        IndexView::Char(array) => u64::from(array.as_slice()[offset].get()),
        IndexView::Integer(integer) if !integer.is_complex() => {
            match integer
                .element(offset)
                .expect("checked integer index offset")
                .real_component()
            {
                IntegerComponent::Signed(value) => {
                    if value <= 0 {
                        return Err(invalid(IndexErrorKind::NonPositive));
                    }
                    u64::try_from(value).map_err(|_| {
                        array_error(ArrayRuntimeError::IndexOutOfBounds {
                            argument,
                            index: u64::MAX,
                            extent,
                        })
                    })?
                }
                IntegerComponent::Unsigned(value) => {
                    if value == 0 {
                        return Err(invalid(IndexErrorKind::NonPositive));
                    }
                    u64::try_from(value).map_err(|_| {
                        array_error(ArrayRuntimeError::IndexOutOfBounds {
                            argument,
                            index: u64::MAX,
                            extent,
                        })
                    })?
                }
            }
        }
        IndexView::ComplexScalar
        | IndexView::ComplexArray(_)
        | IndexView::SingleComplexArray(_)
        | IndexView::Integer(_)
        | IndexView::LogicalScalar(_)
        | IndexView::LogicalArray(_) => return Err(invalid(IndexErrorKind::NonInteger)),
        IndexView::RealScalar(_) => unreachable!("scalar offset was checked"),
    };
    if index == 0 {
        return Err(invalid(IndexErrorKind::NonPositive));
    }
    if index > extent {
        return Err(array_error(ArrayRuntimeError::IndexOutOfBounds {
            argument,
            index,
            extent,
        }));
    }
    Ok(index)
}

fn checked_float_index(
    value: f64,
    invalid: &impl Fn(IndexErrorKind) -> RuntimeErrorKind,
) -> Result<u64, RuntimeErrorKind> {
    if !value.is_finite() {
        return Err(invalid(IndexErrorKind::NonFinite));
    }
    if value.fract() != 0.0 {
        return Err(invalid(IndexErrorKind::NonInteger));
    }
    if value <= 0.0 {
        return Err(invalid(IndexErrorKind::NonPositive));
    }
    if value >= U64_EXCLUSIVE_LIMIT_AS_F64 {
        return Ok(u64::MAX);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn grow_undefined_numeric(
    arguments: &[IndexInput],
    value: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let index = scalar_linear_index(arguments, cancellation)?;
    match value {
        Value::Array(ArrayData::Char(array)) => {
            return grow_undefined_exact(index, array, cancellation);
        }
        Value::Array(ArrayData::Integer(integer)) => {
            return dispatch_integer_array!(integer, |array| {
                grow_undefined_exact(index, array, cancellation)
            });
        }
        _ => {}
    }
    let supplied = NumericView::new(value, "indexed assignment value")?;
    if supplied.numel() != 1 {
        return Err(array_error(ArrayRuntimeError::AssignmentSizeMismatch {
            selected: 1,
            supplied: supplied.numel(),
        }));
    }
    let length = checked_usize(index)?;
    if supplied.is_logical() {
        let mut values = try_filled_output(length, Logical::FALSE, cancellation)?;
        let source = supplied.logical_at(0).expect("validated logical scalar");
        values[length - 1] = Logical::from(source);
        return logical_array([1, index], values);
    }
    if supplied.is_complex() {
        let mut values = try_filled_output(length, ArrayComplex64::ZERO, cancellation)?;
        values[length - 1] = supplied
            .complex_at(0)
            .expect("validated complex scalar")
            .into();
        return complex_array([1, index], values);
    }
    let mut values = try_filled_output(length, 0.0, cancellation)?;
    values[length - 1] = supplied.real_at(0).expect("validated real scalar");
    real_array([1, index], values)
}

fn grow_undefined_exact<T: ArrayElement + Clone + Default>(
    index: u64,
    supplied: &DenseArray<T>,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if supplied.numel() != 1 {
        return Err(array_error(ArrayRuntimeError::AssignmentSizeMismatch {
            selected: 1,
            supplied: supplied.numel(),
        }));
    }
    let length = checked_usize(index)?;
    let mut values = try_filled_output(length, T::default(), cancellation)?;
    values[length - 1] = supplied.as_slice()[0].clone();
    let shape = Shape::new([1, index]).map_err(|_| array_size_error())?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::from_typed)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn resolve_dimension(
    input: &IndexInput,
    extent: u64,
    argument: usize,
    cancellation: &CancellationToken,
) -> Result<DimensionSelection, RuntimeErrorKind> {
    match input {
        IndexInput::Colon => {
            let mut indices = try_output_vec(checked_usize(extent)?, cancellation)?;
            for index in 1..=extent {
                check_cancelled(cancellation, checked_usize(index - 1)?)?;
                indices.push(index);
            }
            Ok(DimensionSelection {
                indices,
                source_shape: vec![extent, 1],
                colon: true,
                logical: false,
            })
        }
        IndexInput::Value(value) => {
            let view = IndexView::new(value).ok_or_else(|| {
                array_error(ArrayRuntimeError::InvalidIndex {
                    argument,
                    reason: IndexErrorKind::NonNumeric,
                })
            })?;
            if view.is_logical() {
                if view.numel() != extent && view.numel() != 1 {
                    return Err(array_error(ArrayRuntimeError::InvalidIndex {
                        argument,
                        reason: IndexErrorKind::LogicalLength {
                            expected: extent,
                            actual: view.numel(),
                        },
                    }));
                }
                let mut indices = try_output_vec(checked_usize(view.numel())?, cancellation)?;
                for offset in 0..checked_usize(view.numel())? {
                    check_cancelled(cancellation, offset)?;
                    if view.logical_at(offset).expect("logical index element") {
                        indices.push(u64::try_from(offset).map_err(|_| array_size_error())? + 1);
                    }
                }
                return Ok(DimensionSelection {
                    indices,
                    source_shape: view.dimensions().to_vec(),
                    colon: false,
                    logical: true,
                });
            }

            let mut indices = try_output_vec(checked_usize(view.numel())?, cancellation)?;
            for offset in 0..checked_usize(view.numel())? {
                check_cancelled(cancellation, offset)?;
                indices.push(checked_index_at(&view, offset, argument, extent)?);
            }
            Ok(DimensionSelection {
                indices,
                source_shape: view.dimensions().to_vec(),
                colon: false,
                logical: false,
            })
        }
    }
}

fn linear_result_dimensions(shape: &Shape, selection: &DimensionSelection) -> Vec<u64> {
    let length = u64::try_from(selection.indices.len()).unwrap_or(u64::MAX);
    if selection.colon {
        return vec![length, 1];
    }
    let target_is_vector = shape.ndims() == 2 && (shape.extent(0) == 1 || shape.extent(1) == 1);
    let index_is_vector = selection.source_shape.len() == 2
        && (selection.source_shape[0] == 1 || selection.source_shape[1] == 1);
    if target_is_vector && index_is_vector {
        if shape.extent(0) == 1 {
            vec![1, length]
        } else {
            vec![length, 1]
        }
    } else if selection.logical {
        vec![length, 1]
    } else {
        selection.source_shape.clone()
    }
}

fn gather_value(
    view: &ArrayView<'_>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    match view {
        ArrayView::Numeric(view) => gather(view, selection, cancellation),
        ArrayView::SingleReal(value) => gather_exact_array(value, selection, cancellation),
        ArrayView::SingleComplex(value) => gather_exact_array(value, selection, cancellation),
        ArrayView::Char(array) => gather_exact_array(array, selection, cancellation),
        ArrayView::Integer(integer) => dispatch_integer_array!(integer, |array| {
            gather_exact_array(array, selection, cancellation)
        }),
        ArrayView::String(value) => gather_string(value, selection, cancellation),
    }
}

fn gather_exact_array<T: ArrayElement + Clone>(
    array: &DenseArray<T>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    let mut output = try_output_vec(selection.offsets.len(), cancellation)?;
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        output.push(array.as_slice()[offset].clone());
    }
    DenseArray::from_vec(selection.shape.clone(), output)
        .map(ArrayData::from_typed)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn gather_string(
    value: &StringValue,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if selection.offsets.len() == 1 {
        return Ok(Value::String(StringValue::scalar(
            value
                .element(selection.offsets[0])
                .expect("validated string selection offset")
                .clone(),
        )));
    }
    let mut output = try_output_vec(selection.offsets.len(), cancellation)?;
    for (position, &offset) in selection.offsets.iter().enumerate() {
        check_cancelled(cancellation, position)?;
        output.push(
            value
                .element(offset)
                .expect("validated string selection offset")
                .clone(),
        );
    }
    StringArray::from_elements(selection.shape.clone(), output)
        .map(StringValue::Array)
        .map(Value::String)
        .map_err(|_| array_size_error())
}

fn gather(
    view: &NumericView<'_>,
    selection: &ResolvedSelection,
    cancellation: &CancellationToken,
) -> Result<Value, RuntimeErrorKind> {
    if selection.offsets.len() == 1 {
        let offset = selection.offsets[0];
        return match view {
            NumericView::LogicalScalar(value) => Ok(Value::Logical(*value)),
            NumericView::RealScalar(value) => Ok(Value::Double(*value)),
            NumericView::ComplexScalar(value) => Ok(Value::Complex(*value)),
            NumericView::LogicalArray(array) => Ok(Value::Logical(array.as_slice()[offset].get())),
            NumericView::RealArray(array) => Ok(Value::Double(array.as_slice()[offset])),
            NumericView::ComplexArray(array) => Ok(Value::Complex(array.as_slice()[offset].into())),
        };
    }
    match view {
        NumericView::LogicalScalar(value) => logical_array_with_shape(
            selection.shape.clone(),
            try_filled_output(selection.offsets.len(), Logical::from(*value), cancellation)?,
        ),
        NumericView::RealScalar(value) => real_array_with_shape(
            selection.shape.clone(),
            try_filled_output(selection.offsets.len(), *value, cancellation)?,
        ),
        NumericView::ComplexScalar(value) => complex_array_with_shape(
            selection.shape.clone(),
            try_filled_output(selection.offsets.len(), (*value).into(), cancellation)?,
        ),
        NumericView::LogicalArray(array) => {
            let mut output = try_output_vec(selection.offsets.len(), cancellation)?;
            for (position, &offset) in selection.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                output.push(array.as_slice()[offset]);
            }
            logical_array_with_shape(selection.shape.clone(), output)
        }
        NumericView::RealArray(array) => {
            let mut output = try_output_vec(selection.offsets.len(), cancellation)?;
            for (position, &offset) in selection.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                output.push(array.as_slice()[offset]);
            }
            real_array_with_shape(selection.shape.clone(), output)
        }
        NumericView::ComplexArray(array) => {
            let mut output = try_output_vec(selection.offsets.len(), cancellation)?;
            for (position, &offset) in selection.offsets.iter().enumerate() {
                check_cancelled(cancellation, position)?;
                output.push(array.as_slice()[offset]);
            }
            complex_array_with_shape(selection.shape.clone(), output)
        }
    }
}

pub(crate) fn for_iteration(
    iterable: &Value,
    index: u64,
    cancellation: &CancellationToken,
) -> Result<Option<Value>, RuntimeErrorKind> {
    let view = ArrayView::new(iterable, "for iteration")?;
    require_two_or_more_array_dimensions(&view, "for iteration")?;
    let rows = view.dimensions()[0];
    let columns = view.dimensions()[1..]
        .iter()
        .try_fold(1_u64, |total, extent| {
            total.checked_mul(*extent).ok_or_else(array_size_error)
        })?;
    if index >= columns {
        return Ok(None);
    }
    let start = index.checked_mul(rows).ok_or_else(array_size_error)?;
    if rows == 1 {
        return gather_value(
            &view,
            &ResolvedSelection {
                offsets: vec![checked_usize(start)?],
                shape: Shape::new([1, 1]).map_err(|_| array_size_error())?,
            },
            cancellation,
        )
        .map(Some);
    }
    let mut offsets = try_output_vec(checked_usize(rows)?, cancellation)?;
    for row in 0..rows {
        check_cancelled(cancellation, checked_usize(row)?)?;
        offsets.push(checked_usize(start + row)?);
    }
    gather_value(
        &view,
        &ResolvedSelection {
            offsets,
            shape: Shape::new([rows, 1]).map_err(|_| array_size_error())?,
        },
        cancellation,
    )
    .map(Some)
}

fn owned_array(value: &Value) -> Result<ArrayData, RuntimeErrorKind> {
    match value {
        Value::Logical(value) => DenseArray::from_vec(
            Shape::new([1, 1]).map_err(|_| array_size_error())?,
            vec![Logical::from(*value)],
        )
        .map(ArrayData::Logical)
        .map_err(|_| array_size_error()),
        Value::Double(value) => DenseArray::from_vec(
            Shape::new([1, 1]).map_err(|_| array_size_error())?,
            vec![*value],
        )
        .map(ArrayData::F64)
        .map_err(|_| array_size_error()),
        Value::Complex(value) => DenseArray::from_vec(
            Shape::new([1, 1]).map_err(|_| array_size_error())?,
            vec![(*value).into()],
        )
        .map(ArrayData::ComplexF64)
        .map_err(|_| array_size_error()),
        Value::Array(array) => Ok(array.clone()),
        other => Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation: "indexed assignment",
            actual: other.kind(),
        })),
    }
}

fn require_two_dimensions(
    view: &NumericView<'_>,
    operation: &'static str,
) -> Result<(), RuntimeErrorKind> {
    if view.dimensions().len() == 2 {
        Ok(())
    } else {
        Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation,
            actual: openmat_value::ValueKind::Array,
        }))
    }
}

fn require_two_or_more_array_dimensions(
    view: &ArrayView<'_>,
    operation: &'static str,
) -> Result<(), RuntimeErrorKind> {
    if view.dimensions().len() >= 2 {
        Ok(())
    } else {
        Err(array_error(ArrayRuntimeError::InvalidOperand {
            operation,
            actual: openmat_value::ValueKind::Array,
        }))
    }
}

fn arithmetic_complex(operator: BinaryOperator, lhs: Complex64, rhs: Complex64) -> Complex64 {
    match operator {
        BinaryOperator::Add => lhs + rhs,
        BinaryOperator::Subtract => lhs - rhs,
        BinaryOperator::Multiply | BinaryOperator::ElementMultiply => lhs * rhs,
        BinaryOperator::Divide | BinaryOperator::ElementDivide => lhs / rhs,
        BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => rhs / lhs,
        _ => unreachable!("operator is not complex arithmetic"),
    }
}

fn arithmetic_complex32(operator: BinaryOperator, lhs: Complex32, rhs: Complex32) -> Complex32 {
    match operator {
        BinaryOperator::Add => Complex32::new(lhs.re + rhs.re, lhs.im + rhs.im),
        BinaryOperator::Subtract => Complex32::new(lhs.re - rhs.re, lhs.im - rhs.im),
        BinaryOperator::ElementMultiply => multiply_complex32(lhs, rhs),
        BinaryOperator::ElementDivide => divide_complex32(lhs, rhs),
        BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => {
            divide_complex32(rhs, lhs)
        }
        _ => unreachable!("operator is not single complex arithmetic"),
    }
}

fn arithmetic_single_complex64(
    operator: BinaryOperator,
    lhs: Complex64,
    rhs: Complex64,
) -> Complex64 {
    match operator {
        BinaryOperator::Add => lhs + rhs,
        BinaryOperator::Subtract => lhs - rhs,
        BinaryOperator::ElementMultiply => multiply_complex64(lhs, rhs),
        BinaryOperator::ElementDivide => divide_complex64(lhs, rhs),
        BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => {
            divide_complex64(rhs, lhs)
        }
        _ => unreachable!("operator is not mixed single complex arithmetic"),
    }
}

fn multiply_complex32(lhs: Complex32, rhs: Complex32) -> Complex32 {
    let products = [
        lhs.re * rhs.re,
        lhs.im * rhs.im,
        lhs.re * rhs.im,
        lhs.im * rhs.re,
    ];
    if products.iter().all(|value| value.is_finite()) {
        return Complex32::new(
            lhs.re.mul_add(rhs.re, -products[1]),
            lhs.re.mul_add(rhs.im, products[3]),
        );
    }
    let lhs_scale = lhs.re.abs().max(lhs.im.abs());
    let rhs_scale = rhs.re.abs().max(rhs.im.abs());
    let lhs_real = lhs.re / lhs_scale;
    let lhs_imaginary = lhs.im / lhs_scale;
    let rhs_real = rhs.re / rhs_scale;
    let rhs_imaginary = rhs.im / rhs_scale;
    Complex32::new(
        rescale_product32(
            lhs_real.mul_add(rhs_real, -(lhs_imaginary * rhs_imaginary)),
            lhs_scale,
            rhs_scale,
        ),
        rescale_product32(
            lhs_real.mul_add(rhs_imaginary, lhs_imaginary * rhs_real),
            lhs_scale,
            rhs_scale,
        ),
    )
}

fn multiply_complex64(lhs: Complex64, rhs: Complex64) -> Complex64 {
    let products = [
        lhs.real * rhs.real,
        lhs.imaginary * rhs.imaginary,
        lhs.real * rhs.imaginary,
        lhs.imaginary * rhs.real,
    ];
    if products.iter().all(|value| value.is_finite()) {
        return Complex64::new(
            lhs.real.mul_add(rhs.real, -products[1]),
            lhs.real.mul_add(rhs.imaginary, products[3]),
        );
    }
    let lhs_scale = lhs.real.abs().max(lhs.imaginary.abs());
    let rhs_scale = rhs.real.abs().max(rhs.imaginary.abs());
    let lhs_real = lhs.real / lhs_scale;
    let lhs_imaginary = lhs.imaginary / lhs_scale;
    let rhs_real = rhs.real / rhs_scale;
    let rhs_imaginary = rhs.imaginary / rhs_scale;
    Complex64::new(
        rescale_product64(
            lhs_real.mul_add(rhs_real, -(lhs_imaginary * rhs_imaginary)),
            lhs_scale,
            rhs_scale,
        ),
        rescale_product64(
            lhs_real.mul_add(rhs_imaginary, lhs_imaginary * rhs_real),
            lhs_scale,
            rhs_scale,
        ),
    )
}

fn divide_complex32(lhs: Complex32, rhs: Complex32) -> Complex32 {
    let denominator = rhs.re.mul_add(rhs.re, rhs.im * rhs.im);
    if denominator.is_finite() && denominator != 0.0 {
        return Complex32::new(
            lhs.re.mul_add(rhs.re, lhs.im * rhs.im) / denominator,
            lhs.im.mul_add(rhs.re, -(lhs.re * rhs.im)) / denominator,
        );
    }
    if rhs.re == 0.0 && rhs.im == 0.0 {
        return divide_by_complex_zero32(lhs, rhs.re);
    }
    let lhs_scale = lhs.re.abs().max(lhs.im.abs());
    let rhs_scale = rhs.re.abs().max(rhs.im.abs());
    let lhs_scale = if lhs_scale == 0.0 { 1.0 } else { lhs_scale };
    let lhs_real = lhs.re / lhs_scale;
    let lhs_imaginary = lhs.im / lhs_scale;
    let rhs_real = rhs.re / rhs_scale;
    let rhs_imaginary = rhs.im / rhs_scale;
    let denominator = rhs_real.mul_add(rhs_real, rhs_imaginary * rhs_imaginary);
    let scale = lhs_scale / rhs_scale;
    Complex32::new(
        rescale_quotient32(
            lhs_real.mul_add(rhs_real, lhs_imaginary * rhs_imaginary) / denominator,
            scale,
        ),
        rescale_quotient32(
            lhs_imaginary.mul_add(rhs_real, -(lhs_real * rhs_imaginary)) / denominator,
            scale,
        ),
    )
}

fn divide_complex64(lhs: Complex64, rhs: Complex64) -> Complex64 {
    let denominator = rhs.real.mul_add(rhs.real, rhs.imaginary * rhs.imaginary);
    if denominator.is_finite() && denominator != 0.0 {
        return Complex64::new(
            lhs.real.mul_add(rhs.real, lhs.imaginary * rhs.imaginary) / denominator,
            lhs.imaginary.mul_add(rhs.real, -(lhs.real * rhs.imaginary)) / denominator,
        );
    }
    if rhs.real == 0.0 && rhs.imaginary == 0.0 {
        return divide_by_complex_zero64(lhs, rhs.real);
    }
    let lhs_scale = lhs.real.abs().max(lhs.imaginary.abs());
    let rhs_scale = rhs.real.abs().max(rhs.imaginary.abs());
    let lhs_scale = if lhs_scale == 0.0 { 1.0 } else { lhs_scale };
    let lhs_real = lhs.real / lhs_scale;
    let lhs_imaginary = lhs.imaginary / lhs_scale;
    let rhs_real = rhs.real / rhs_scale;
    let rhs_imaginary = rhs.imaginary / rhs_scale;
    let denominator = rhs_real.mul_add(rhs_real, rhs_imaginary * rhs_imaginary);
    let scale = lhs_scale / rhs_scale;
    Complex64::new(
        rescale_quotient64(
            lhs_real.mul_add(rhs_real, lhs_imaginary * rhs_imaginary) / denominator,
            scale,
        ),
        rescale_quotient64(
            lhs_imaginary.mul_add(rhs_real, -(lhs_real * rhs_imaginary)) / denominator,
            scale,
        ),
    )
}

fn rescale_product32(component: f32, lhs_scale: f32, rhs_scale: f32) -> f32 {
    if component == 0.0 {
        component
    } else {
        (component * lhs_scale) * rhs_scale
    }
}

fn rescale_product64(component: f64, lhs_scale: f64, rhs_scale: f64) -> f64 {
    if component == 0.0 {
        component
    } else {
        (component * lhs_scale) * rhs_scale
    }
}

fn rescale_quotient32(component: f32, scale: f32) -> f32 {
    if component == 0.0 {
        component
    } else {
        component * scale
    }
}

fn rescale_quotient64(component: f64, scale: f64) -> f64 {
    if component == 0.0 {
        component
    } else {
        component * scale
    }
}

fn divide_by_complex_zero32(lhs: Complex32, divisor_real: f32) -> Complex32 {
    if lhs.re == 0.0 && lhs.im == 0.0 {
        return Complex32::new(f32::NAN, 0.0);
    }
    Complex32::new(
        infinite_complex_quotient_component32(lhs.re, divisor_real),
        infinite_complex_quotient_component32(lhs.im, divisor_real),
    )
}

fn divide_by_complex_zero64(lhs: Complex64, divisor_real: f64) -> Complex64 {
    if lhs.real == 0.0 && lhs.imaginary == 0.0 {
        return Complex64::new(f64::NAN, 0.0);
    }
    Complex64::new(
        infinite_complex_quotient_component64(lhs.real, divisor_real),
        infinite_complex_quotient_component64(lhs.imaginary, divisor_real),
    )
}

fn infinite_complex_quotient_component32(component: f32, divisor_real: f32) -> f32 {
    if component == 0.0 {
        0.0
    } else if component.is_sign_negative() == divisor_real.is_sign_negative() {
        f32::INFINITY
    } else {
        f32::NEG_INFINITY
    }
}

fn infinite_complex_quotient_component64(component: f64, divisor_real: f64) -> f64 {
    if component == 0.0 {
        0.0
    } else if component.is_sign_negative() == divisor_real.is_sign_negative() {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    }
}

fn arithmetic_real32(operator: BinaryOperator, lhs: f32, rhs: f32) -> f32 {
    match operator {
        BinaryOperator::Add => lhs + rhs,
        BinaryOperator::Subtract => lhs - rhs,
        BinaryOperator::ElementMultiply => lhs * rhs,
        BinaryOperator::ElementDivide => lhs / rhs,
        BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => rhs / lhs,
        _ => unreachable!("operator is not single real arithmetic"),
    }
}

fn arithmetic_real64(operator: BinaryOperator, lhs: f64, rhs: f64) -> f64 {
    match operator {
        BinaryOperator::Add => lhs + rhs,
        BinaryOperator::Subtract => lhs - rhs,
        BinaryOperator::ElementMultiply => lhs * rhs,
        BinaryOperator::ElementDivide => lhs / rhs,
        BinaryOperator::LeftDivide | BinaryOperator::ElementLeftDivide => rhs / lhs,
        _ => unreachable!("operator is not mixed single/double arithmetic"),
    }
}

fn real_power32(base: f32, exponent: f32) -> Complex32 {
    if base < 0.0 && exponent.is_finite() && exponent.fract() != 0.0 {
        let magnitude = (-base).powf(exponent);
        let (sine, cosine) = sin_cos_pi32(exponent);
        Complex32::new(magnitude * cosine, magnitude * sine)
    } else {
        Complex32::new(base.powf(exponent), 0.0)
    }
}

fn real_power64(base: f64, exponent: f64) -> Complex64 {
    if base < 0.0 && exponent.is_finite() && exponent.fract() != 0.0 {
        let magnitude = (-base).powf(exponent);
        let (sine, cosine) = sin_cos_pi64(exponent);
        Complex64::new(magnitude * cosine, magnitude * sine)
    } else {
        Complex64::new(base.powf(exponent), 0.0)
    }
}

fn complex_power32(base: Complex32, exponent: Complex32) -> Option<Complex32> {
    if !complex32_is_finite(base) || !complex32_is_finite(exponent) {
        return None;
    }
    if base.im == 0.0 && exponent.im == 0.0 {
        return Some(real_power32(base.re, exponent.re));
    }
    if exponent.im == 0.0 {
        match exponent.re.to_bits() {
            0x0000_0000 | 0x8000_0000 => return Some(Complex32::new(1.0, 0.0)),
            0x3f80_0000 => return Some(base),
            0xbf80_0000 => {
                return Some(divide_complex32(Complex32::new(1.0, 0.0), base));
            }
            _ => {}
        }
    }
    if base.re == 0.0 && base.im == 0.0 {
        return None;
    }
    let logarithm = complex_log_magnitude32(base);
    let argument = base.im.atan2(base.re);
    let magnitude = exponent
        .re
        .mul_add(logarithm, -(exponent.im * argument))
        .exp();
    let angle = exponent.im.mul_add(logarithm, exponent.re * argument);
    let (sine, cosine) = angle.sin_cos();
    Some(Complex32::new(magnitude * cosine, magnitude * sine))
}

fn complex_power64(base: Complex64, exponent: Complex64) -> Option<Complex64> {
    if !complex64_is_finite(base) || !complex64_is_finite(exponent) {
        return None;
    }
    if base.imaginary == 0.0 && exponent.imaginary == 0.0 {
        return Some(real_power64(base.real, exponent.real));
    }
    if exponent.imaginary == 0.0 {
        match exponent.real.to_bits() {
            0x0000_0000_0000_0000 | 0x8000_0000_0000_0000 => {
                return Some(Complex64::new(1.0, 0.0));
            }
            0x3ff0_0000_0000_0000 => return Some(base),
            0xbff0_0000_0000_0000 => {
                return Some(divide_complex64(Complex64::new(1.0, 0.0), base));
            }
            _ => {}
        }
    }
    if base.real == 0.0 && base.imaginary == 0.0 {
        return None;
    }
    let logarithm = complex_log_magnitude64(base);
    let argument = base.imaginary.atan2(base.real);
    let magnitude = exponent
        .real
        .mul_add(logarithm, -(exponent.imaginary * argument))
        .exp();
    let angle = exponent
        .imaginary
        .mul_add(logarithm, exponent.real * argument);
    let (sine, cosine) = angle.sin_cos();
    Some(Complex64::new(magnitude * cosine, magnitude * sine))
}

fn complex_log_magnitude32(value: Complex32) -> f32 {
    let scale = value.re.abs().max(value.im.abs());
    scale.ln() + (value.re / scale).hypot(value.im / scale).ln()
}

fn complex_log_magnitude64(value: Complex64) -> f64 {
    let scale = value.real.abs().max(value.imaginary.abs());
    scale.ln() + (value.real / scale).hypot(value.imaginary / scale).ln()
}

fn sin_cos_pi32(value: f32) -> (f32, f32) {
    let reduced = value.rem_euclid(2.0);
    match reduced.to_bits() {
        0x0000_0000 | 0x8000_0000 => (0.0, 1.0),
        0x3f00_0000 => (1.0, 0.0),
        0x3f80_0000 => (0.0, -1.0),
        0x3fc0_0000 => (-1.0, 0.0),
        _ => (std::f32::consts::PI * reduced).sin_cos(),
    }
}

fn sin_cos_pi64(value: f64) -> (f64, f64) {
    let reduced = value.rem_euclid(2.0);
    match reduced.to_bits() {
        0x0000_0000_0000_0000 | 0x8000_0000_0000_0000 => (0.0, 1.0),
        0x3fe0_0000_0000_0000 => (1.0, 0.0),
        0x3ff0_0000_0000_0000 => (0.0, -1.0),
        0x3ff8_0000_0000_0000 => (-1.0, 0.0),
        _ => (std::f64::consts::PI * reduced).sin_cos(),
    }
}

fn complex32_from_complex64(value: Complex64) -> Complex32 {
    Complex32::new(
        single_from_double(value.real),
        single_from_double(value.imaginary),
    )
}

fn complex32_is_finite(value: Complex32) -> bool {
    value.re.is_finite() && value.im.is_finite()
}

fn complex64_is_finite(value: Complex64) -> bool {
    value.real.is_finite() && value.imaginary.is_finite()
}

fn compare_real(operator: BinaryOperator, lhs: f64, rhs: f64) -> bool {
    match operator {
        BinaryOperator::LessThan => lhs < rhs,
        BinaryOperator::LessThanOrEqual => lhs <= rhs,
        BinaryOperator::GreaterThan => lhs > rhs,
        BinaryOperator::GreaterThanOrEqual => lhs >= rhs,
        _ => unreachable!("operator is not relational"),
    }
}

fn compare_real32(operator: BinaryOperator, lhs: f32, rhs: f32) -> bool {
    match operator {
        BinaryOperator::LessThan => lhs < rhs,
        BinaryOperator::LessThanOrEqual => lhs <= rhs,
        BinaryOperator::GreaterThan => lhs > rhs,
        BinaryOperator::GreaterThanOrEqual => lhs >= rhs,
        _ => unreachable!("operator is not single relational"),
    }
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    if let (Some(lhs), Some(rhs)) = (lhs.as_complex_number(), rhs.as_complex_number()) {
        lhs == rhs
    } else {
        lhs == rhs
    }
}

fn graphics_equality(
    operator: BinaryOperator,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeErrorKind> {
    let (left, left_shape) =
        graphics_handles(lhs).ok_or_else(|| invalid_operands(operator, lhs, rhs))?;
    let (right, right_shape) =
        graphics_handles(rhs).ok_or_else(|| invalid_operands(operator, lhs, rhs))?;
    let length = if left.len() == 1 {
        right.len()
    } else if right.len() == 1 || left_shape == right_shape {
        left.len()
    } else {
        return Err(invalid_operands(operator, lhs, rhs));
    };
    let invert = operator == BinaryOperator::NotEqual;
    let values = (0..length)
        .map(|index| {
            let equal = left[if left.len() == 1 { 0 } else { index }]
                == right[if right.len() == 1 { 0 } else { index }];
            Logical::from(equal != invert)
        })
        .collect::<Vec<_>>();
    if length == 1 {
        return Ok(Value::Logical(bool::from(values[0])));
    }
    let shape = if left.len() == 1 {
        right_shape
    } else {
        left_shape
    };
    logical_array_with_shape(shape, values)
}

fn graphics_handles(value: &Value) -> Option<(Vec<openmat_graphics_model::GraphicsHandle>, Shape)> {
    match value {
        Value::Graphics(handle) => Some((
            vec![*handle],
            Shape::new(SCALAR_DIMENSIONS).expect("scalar shape is valid"),
        )),
        Value::GraphicsArray(array) => Some((array.as_slice().to_vec(), array.shape().clone())),
        _ => None,
    }
}

fn invalid_operands(operator: BinaryOperator, lhs: &Value, rhs: &Value) -> RuntimeErrorKind {
    RuntimeErrorKind::InvalidOperands {
        operator,
        lhs: lhs.kind(),
        rhs: rhs.kind(),
    }
}

const fn operator_name(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "addition",
        BinaryOperator::Subtract => "subtraction",
        BinaryOperator::Multiply => "multiplication",
        BinaryOperator::ElementMultiply => "element-wise multiplication",
        BinaryOperator::Divide => "division",
        BinaryOperator::LeftDivide => "left division",
        BinaryOperator::ElementDivide => "element-wise division",
        BinaryOperator::ElementLeftDivide => "element-wise left division",
        BinaryOperator::Power => "power",
        BinaryOperator::ElementPower => "element-wise power",
        BinaryOperator::Equal => "equality comparison",
        BinaryOperator::NotEqual => "inequality comparison",
        BinaryOperator::LessThan => "less-than comparison",
        BinaryOperator::LessThanOrEqual => "less-than-or-equal comparison",
        BinaryOperator::GreaterThan => "greater-than comparison",
        BinaryOperator::GreaterThanOrEqual => "greater-than-or-equal comparison",
    }
}

fn check_cancelled(
    cancellation: &CancellationToken,
    progress: usize,
) -> Result<(), RuntimeErrorKind> {
    if progress.is_multiple_of(CANCELLATION_INTERVAL) && cancellation.is_cancelled() {
        Err(RuntimeErrorKind::Cancelled)
    } else {
        Ok(())
    }
}

fn try_reserved_vec<T>(capacity: usize) -> Result<Vec<T>, RuntimeErrorKind> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| array_size_error())?;
    Ok(values)
}

fn try_output_vec<T>(
    capacity: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<T>, RuntimeErrorKind> {
    check_cancelled(cancellation, 0)?;
    try_reserved_vec(capacity)
}

fn try_filled_output<T: Clone>(
    length: usize,
    value: T,
    cancellation: &CancellationToken,
) -> Result<Vec<T>, RuntimeErrorKind> {
    let mut output = try_output_vec(length, cancellation)?;
    for chunk_start in (0..length).step_by(CANCELLATION_INTERVAL) {
        check_cancelled(cancellation, chunk_start)?;
        let chunk_end = chunk_start
            .saturating_add(CANCELLATION_INTERVAL)
            .min(length);
        for _ in chunk_start..chunk_end {
            output.push(value.clone());
        }
    }
    Ok(output)
}

fn promote_real_values(
    values: &[f64],
    cancellation: &CancellationToken,
) -> Result<Vec<ArrayComplex64>, RuntimeErrorKind> {
    let mut promoted = try_output_vec(values.len(), cancellation)?;
    for (index, value) in values.iter().enumerate() {
        check_cancelled(cancellation, index)?;
        promoted.push(ArrayComplex64::new(*value, 0.0));
    }
    Ok(promoted)
}

fn promote_single_real_values(
    values: &[f32],
    cancellation: &CancellationToken,
) -> Result<Vec<Complex32>, RuntimeErrorKind> {
    let mut promoted = try_output_vec(values.len(), cancellation)?;
    for (index, value) in values.iter().enumerate() {
        check_cancelled(cancellation, index)?;
        promoted.push(Complex32::from(*value));
    }
    Ok(promoted)
}

fn checked_usize(value: u64) -> Result<usize, RuntimeErrorKind> {
    usize::try_from(value).map_err(|_| array_size_error())
}

fn host_len(rows: u64, columns: u64) -> Result<usize, RuntimeErrorKind> {
    rows.checked_mul(columns)
        .ok_or_else(array_size_error)
        .and_then(checked_usize)
}

fn array_size_error() -> RuntimeErrorKind {
    array_error(ArrayRuntimeError::SizeLimit)
}

fn real_array(dimensions: [u64; 2], values: Vec<f64>) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    real_array_with_shape(shape, values)
}

fn single_real_array(dimensions: [u64; 2], values: Vec<f32>) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    single_real_array_with_shape(shape, values)
}

fn single_real_array_with_shape(shape: Shape, values: Vec<f32>) -> Result<Value, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F32)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn single_complex_array(
    dimensions: [u64; 2],
    values: Vec<Complex32>,
) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    single_complex_array_with_shape(shape, values)
}

fn single_complex_array_with_shape(
    shape: Shape,
    values: Vec<Complex32>,
) -> Result<Value, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::ComplexF32)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn complex_array(
    dimensions: [u64; 2],
    values: Vec<ArrayComplex64>,
) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    complex_array_with_shape(shape, values)
}

fn logical_array(dimensions: [u64; 2], values: Vec<Logical>) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    logical_array_with_shape(shape, values)
}

fn string_array(
    dimensions: [u64; 2],
    values: Vec<StringElement>,
) -> Result<Value, RuntimeErrorKind> {
    let shape = Shape::new(dimensions).map_err(|_| array_size_error())?;
    StringArray::from_elements(shape, values)
        .map(Value::from)
        .map_err(|_| array_size_error())
}

fn real_array_with_shape(shape: Shape, values: Vec<f64>) -> Result<Value, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn complex_array_with_shape(
    shape: Shape,
    values: Vec<ArrayComplex64>,
) -> Result<Value, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::ComplexF64)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn logical_array_with_shape(shape: Shape, values: Vec<Logical>) -> Result<Value, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map(Value::Array)
        .map_err(|_| array_size_error())
}

fn real_array_data((shape, values): (Shape, Vec<f64>)) -> Result<ArrayData, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map_err(|_| array_size_error())
}

fn complex_array_data(
    (shape, values): (Shape, Vec<ArrayComplex64>),
) -> Result<ArrayData, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::ComplexF64)
        .map_err(|_| array_size_error())
}

fn logical_array_data(
    (shape, values): (Shape, Vec<Logical>),
) -> Result<ArrayData, RuntimeErrorKind> {
    DenseArray::from_vec(shape, values)
        .map(ArrayData::Logical)
        .map_err(|_| array_size_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_graphics_model::{GraphicsClass, GraphicsHandle};
    use openmat_value::{GraphicsHandleArray, ObjectHandle};

    #[test]
    fn broadcast_plan_maps_nd_column_major_offsets() {
        let plan = BroadcastPlan::new(BinaryOperator::Add, &[2, 1, 3], &[1, 4]).unwrap();
        assert_eq!(plan.shape.dimensions(), &[2, 4, 3]);
        let offsets = (0..24)
            .map(|offset| plan.offsets(offset).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            offsets,
            vec![
                (0, 0),
                (1, 0),
                (0, 1),
                (1, 1),
                (0, 2),
                (1, 2),
                (0, 3),
                (1, 3),
                (2, 0),
                (3, 0),
                (2, 1),
                (3, 1),
                (2, 2),
                (3, 2),
                (2, 3),
                (3, 3),
                (4, 0),
                (5, 0),
                (4, 1),
                (5, 1),
                (4, 2),
                (5, 2),
                (4, 3),
                (5, 3),
            ]
        );
    }

    #[test]
    fn numeric_and_exact_operators_apply_r2022b_implicit_expansion() {
        let provider = openmat_linalg::ReferenceProvider;
        let cancellation = CancellationToken::new();
        let column = real_array([2, 1], vec![1.0, 2.0]).unwrap();
        let row = real_array([1, 3], vec![10.0, 20.0, 30.0]).unwrap();

        let sum =
            evaluate_binary(BinaryOperator::Add, &column, &row, &cancellation, &provider).unwrap();
        let Value::Array(ArrayData::F64(sum)) = sum else {
            panic!("expanded addition must return a double array");
        };
        assert_eq!(sum.shape().dimensions(), &[2, 3]);
        assert_eq!(sum.as_slice(), &[11.0, 12.0, 21.0, 22.0, 31.0, 32.0]);

        let less = evaluate_binary(
            BinaryOperator::LessThan,
            &column,
            &row,
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::Logical(less)) = less else {
            panic!("expanded comparison must return a logical array");
        };
        assert_eq!(less.shape().dimensions(), &[2, 3]);
        assert!(less.as_slice().iter().all(|value| value.get()));

        let characters = |dimensions, values: &[u8]| {
            Value::Array(ArrayData::Char(
                DenseArray::from_vec(
                    Shape::new(dimensions).unwrap(),
                    values
                        .iter()
                        .copied()
                        .map(u16::from)
                        .map(CharCodeUnit::new)
                        .collect(),
                )
                .unwrap(),
            ))
        };
        let char_equal = evaluate_binary(
            BinaryOperator::Equal,
            &characters([1, 2], b"ab"),
            &characters([2, 1], b"ab"),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::Logical(char_equal)) = char_equal else {
            panic!("expanded char equality must return a logical array");
        };
        assert_eq!(char_equal.shape().dimensions(), &[2, 2]);
        assert_eq!(
            char_equal.as_slice(),
            &[Logical::TRUE, Logical::FALSE, Logical::FALSE, Logical::TRUE,]
        );
    }

    #[test]
    fn integer_matrix_power_preserves_precision_and_uses_provider_solve_for_negative_exponents() {
        let provider = openmat_linalg::ReferenceProvider;
        let cancellation = CancellationToken::new();
        let matrix = real_array([2, 2], vec![1.0, 3.0, 2.0, 4.0]).unwrap();

        let squared = evaluate_binary(
            BinaryOperator::Power,
            &matrix,
            &Value::Double(2.0),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(squared)) = squared else {
            panic!("double integer matrix power must return a double matrix");
        };
        assert_eq!(squared.shape().dimensions(), &[2, 2]);
        assert_eq!(squared.as_slice(), &[7.0, 15.0, 10.0, 22.0]);

        let inverse = evaluate_binary(
            BinaryOperator::Power,
            &matrix,
            &Value::Double(-1.0),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(inverse)) = inverse else {
            panic!("negative matrix power must preserve double storage");
        };
        for (actual, expected) in inverse.as_slice().iter().zip([-2.0, 1.5, 1.0, -0.5]) {
            assert!((actual - expected).abs() < 1.0e-12);
        }

        let single_exponent = single_real_array([1, 1], vec![2.0]).unwrap();
        let single = evaluate_binary(
            BinaryOperator::Power,
            &matrix,
            &single_exponent,
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F32(single)) = single else {
            panic!("a single exponent must select MATLAB single matrix-power precision");
        };
        assert_eq!(single.as_slice(), &[7.0, 15.0, 10.0, 22.0]);

        let empty = real_array([0, 0], Vec::new()).unwrap();
        let empty_power = evaluate_binary(
            BinaryOperator::Power,
            &empty,
            &Value::Double(0.0),
            &cancellation,
            &provider,
        )
        .unwrap();
        assert_eq!(empty_power.dimensions(), Some([0, 0].as_slice()));

        let triangular = real_array([2, 2], vec![4.0, 0.0, 1.0, 9.0]).unwrap();
        let half = evaluate_binary(
            BinaryOperator::Power,
            &triangular,
            &Value::Double(0.5),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(half)) = half else {
            panic!("positive-spectrum fractional power must remain real double");
        };
        for (actual, expected) in half.as_slice().iter().zip([2.0, 0.0, 0.2, 3.0]) {
            assert!((actual - expected).abs() < 1.0e-10);
        }

        let negative = real_array([2, 2], vec![-1.0, 0.0, 0.0, 4.0]).unwrap();
        let principal = evaluate_binary(
            BinaryOperator::Power,
            &negative,
            &Value::Double(0.5),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::ComplexF64(principal)) = principal else {
            panic!("negative spectrum must select the complex principal branch");
        };
        assert_eq!(principal.as_slice()[0], ArrayComplex64::new(0.0, 1.0));
        assert_eq!(principal.as_slice()[3], ArrayComplex64::new(2.0, 0.0));

        let single_half = single_real_array([1, 1], vec![0.5]).unwrap();
        let single_result = evaluate_binary(
            BinaryOperator::Power,
            &triangular,
            &single_half,
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F32(single_result)) = single_result else {
            panic!("a single non-integer exponent must select single matrix power");
        };
        assert!((single_result.as_slice()[2] - 0.2).abs() < 5.0e-5);
    }

    #[test]
    fn implicit_expansion_preserves_empty_dimensions_and_reports_real_mismatch() {
        let provider = openmat_linalg::ReferenceProvider;
        let cancellation = CancellationToken::new();
        let empty_column = real_array([0, 1], Vec::new()).unwrap();
        let row = real_array([1, 3], vec![1.0, 2.0, 3.0]).unwrap();
        let expanded = evaluate_binary(
            BinaryOperator::Add,
            &empty_column,
            &row,
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(expanded)) = expanded else {
            panic!("empty expansion must return a double array");
        };
        assert_eq!(expanded.shape().dimensions(), &[0, 3]);

        let incompatible = real_array([2, 2], vec![1.0; 4]).unwrap();
        let error = evaluate_binary(
            BinaryOperator::Add,
            &incompatible,
            &row,
            &cancellation,
            &provider,
        )
        .unwrap_err();
        assert_eq!(
            error,
            array_error(ArrayRuntimeError::ShapeMismatch {
                operation: "addition",
                lhs: vec![2, 2],
                rhs: vec![1, 3],
            })
        );
    }

    #[test]
    fn owned_scalar_assignment_detaches_every_supported_numeric_storage_class() {
        let cancellation = CancellationToken::new();
        let index = Value::Double(2.0);

        let mut complex = complex_array(
            [1, 2],
            vec![ArrayComplex64::new(1.0, 2.0), ArrayComplex64::new(3.0, 4.0)],
        )
        .unwrap();
        let complex_alias = complex.clone();
        assert!(complex.shares_storage_with(&complex_alias));
        assert_eq!(
            try_index_assign_scalar_in_place(
                &mut complex,
                &index,
                &Value::Complex(Complex64::new(9.0, -2.0)),
                &cancellation,
            ),
            Ok(true)
        );
        assert!(!complex.shares_storage_with(&complex_alias));
        assert_eq!(
            complex,
            complex_array(
                [1, 2],
                vec![
                    ArrayComplex64::new(1.0, 2.0),
                    ArrayComplex64::new(9.0, -2.0)
                ],
            )
            .unwrap()
        );

        let mut logical = logical_array([1, 2], vec![Logical::TRUE, Logical::TRUE]).unwrap();
        let logical_alias = logical.clone();
        assert_eq!(
            try_index_assign_scalar_in_place(
                &mut logical,
                &index,
                &Value::Double(0.0),
                &cancellation,
            ),
            Ok(true)
        );
        assert!(!logical.shares_storage_with(&logical_alias));
        assert_eq!(
            logical,
            logical_array([1, 2], vec![Logical::TRUE, Logical::FALSE]).unwrap()
        );

        let mut single = single_real_array([1, 2], vec![1.0, 2.0]).unwrap();
        let single_alias = single.clone();
        let supplied_single = single_real_array([1, 1], vec![9.0]).unwrap();
        assert_eq!(
            try_index_assign_scalar_in_place(&mut single, &index, &supplied_single, &cancellation,),
            Ok(true)
        );
        assert!(!single.shares_storage_with(&single_alias));
        assert_eq!(single, single_real_array([1, 2], vec![1.0, 9.0]).unwrap());

        let shape = Shape::new([1, 2]).unwrap();
        let mut integer = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
            DenseArray::from_vec(shape, vec![1, 2]).unwrap(),
        )));
        let integer_alias = integer.clone();
        let supplied_integer = Value::Array(ArrayData::Integer(IntegerArrayData::I8(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![9]).unwrap(),
        )));
        assert_eq!(
            try_index_assign_scalar_in_place(
                &mut integer,
                &index,
                &supplied_integer,
                &cancellation,
            ),
            Ok(true)
        );
        assert!(!integer.shares_storage_with(&integer_alias));
        assert_eq!(
            integer,
            Value::Array(ArrayData::Integer(IntegerArrayData::I8(
                DenseArray::from_vec(Shape::new([1, 2]).unwrap(), vec![1, 9]).unwrap(),
            )))
        );
    }

    #[test]
    fn owned_scalar_assignment_validates_before_mutation_and_defers_promotion() {
        let cancellation = CancellationToken::new();
        let original = real_array([1, 2], vec![1.0, 2.0]).unwrap();
        let mut target = original.clone();
        assert!(
            try_index_assign_scalar_in_place(
                &mut target,
                &Value::Double(1.0),
                &Value::Complex(Complex64::new(3.0, 4.0)),
                &cancellation,
            )
            .is_ok_and(|updated| !updated)
        );
        assert_eq!(target, original);
        assert!(
            try_index_assign_scalar_in_place(
                &mut target,
                &Value::Double(0.0),
                &Value::Double(9.0),
                &cancellation,
            )
            .is_err()
        );
        assert_eq!(target, original);
    }

    #[test]
    fn long_running_array_instructions_observe_cancellation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            make_range(
                &Value::Double(1.0),
                &Value::Double(1.0),
                &Value::Double(3.0),
                &cancellation,
            ),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn complex_promotion_observes_cancellation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            promote_real_values(&[1.0, 2.0], &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn condition_array_traversal_observes_cancellation() {
        let value = real_array([1, 2_049], vec![1.0; 2_049]).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert_eq!(
            evaluate_condition(&value, &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn string_concatenation_observes_cancellation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert_eq!(
            build_matrix(&[vec![Value::from("alpha")]], &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn single_concatenation_observes_cancellation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let value = single_real_array([1, 1], vec![1.0]).unwrap();

        assert_eq!(
            build_matrix(&[vec![value]], &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn single_element_arithmetic_observes_cancellation() {
        let provider = openmat_linalg::ReferenceProvider;
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let lhs = single_real_array([1, 2_049], vec![1.0; 2_049]).unwrap();
        let rhs = single_real_array([1, 1], vec![2.0]).unwrap();

        assert_eq!(
            evaluate_binary(BinaryOperator::Add, &lhs, &rhs, &cancellation, &provider),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn range_capacity_overflow_is_a_structured_size_limit() {
        let error = make_range(
            &Value::Double(0.0),
            &Value::Double(1.0),
            // This count exceeds Vec<f64>'s architectural capacity, so
            // try_reserve_exact rejects it without asking the allocator.
            &Value::Double(9_223_372_036_854_775_808.0),
            &CancellationToken::new(),
        )
        .expect_err("an unrepresentable Vec capacity must fail");
        let RuntimeErrorKind::InvalidExecutionState {
            array: Some(detail),
            ..
        } = error
        else {
            panic!("expected structured array size error");
        };
        assert_eq!(detail.as_ref(), &ArrayRuntimeError::SizeLimit);
    }

    #[test]
    fn switch_match_rejects_unresolved_object_values_without_panicking() {
        assert_eq!(
            switch_match(
                &Value::Object(ObjectHandle::new(7)),
                &Value::Object(ObjectHandle::new(7)),
                &CancellationToken::new(),
            ),
            Err(array_error(ArrayRuntimeError::InvalidOperand {
                operation: "switch selector",
                actual: openmat_value::ValueKind::Object,
            }))
        );
    }

    #[test]
    fn switch_match_array_scans_observe_cancellation() {
        let shape = Shape::new([1, 2_049]).unwrap();
        let values = vec![CharCodeUnit::new(u16::from(b'a')); 2_049];
        let selector = Value::Array(ArrayData::Char(
            DenseArray::from_vec(shape.clone(), values.clone()).unwrap(),
        ));
        let case_value = Value::Array(ArrayData::Char(
            DenseArray::from_vec(shape, values).unwrap(),
        ));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert_eq!(
            switch_match(&selector, &case_value, &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
    }

    #[test]
    fn graphics_equality_preserves_identity_and_handle_column_shape() {
        let first = GraphicsHandle::new(1, 1, GraphicsClass::LineSeries).unwrap();
        let second = GraphicsHandle::new(2, 1, GraphicsClass::LineSeries).unwrap();
        let column = Value::GraphicsArray(
            GraphicsHandleArray::column(GraphicsClass::LineSeries, vec![first, second]).unwrap(),
        );
        let provider = openmat_linalg::ReferenceProvider;
        let cancellation = CancellationToken::new();

        assert_eq!(
            evaluate_binary(
                BinaryOperator::Equal,
                &Value::Graphics(first),
                &Value::Graphics(first),
                &cancellation,
                &provider,
            ),
            Ok(Value::Logical(true))
        );
        let equal = evaluate_binary(
            BinaryOperator::Equal,
            &column,
            &Value::Graphics(first),
            &cancellation,
            &provider,
        )
        .unwrap();
        let Value::Array(ArrayData::Logical(equal)) = equal else {
            panic!("graphics column equality must return a logical column")
        };
        assert_eq!(equal.shape().dimensions(), &[2, 1]);
        assert_eq!(
            equal.as_slice(),
            &[Logical::from(true), Logical::from(false)]
        );
    }

    #[test]
    fn sparse_scalar_index_and_transposes_remain_sparse() {
        let sparse = SparseArrayData::try_from_complex_f64_coo(
            2,
            3,
            vec![
                openmat_value::CooEntry::new(0, 1, ArrayComplex64::new(1.0, 2.0)),
                openmat_value::CooEntry::new(1, 0, ArrayComplex64::new(3.0, -4.0)),
            ],
            2,
            None,
        )
        .unwrap();
        let value = Value::Sparse(sparse);
        let cancellation = CancellationToken::new();

        let indexed = index(
            &value,
            &[
                IndexInput::Value(Value::Double(1.0)),
                IndexInput::Value(Value::Double(2.0)),
            ],
            &cancellation,
        )
        .unwrap();
        let Value::Sparse(indexed) = indexed else {
            panic!("sparse scalar indexing must return sparse storage")
        };
        assert_eq!(indexed.shape().dimensions(), &[1, 1]);
        assert!(indexed.is_complex());

        let plain = transpose(&value, false, &cancellation).unwrap();
        let conjugate = transpose(&value, true, &cancellation).unwrap();
        let Value::Sparse(SparseArrayData::ComplexF64(plain)) = plain else {
            panic!("plain sparse transpose")
        };
        let Value::Sparse(SparseArrayData::ComplexF64(conjugate)) = conjugate else {
            panic!("conjugate sparse transpose")
        };
        assert_eq!(plain.shape().dimensions(), &[3, 2]);
        assert_eq!(
            plain.get_zero_based(1, 0),
            Some(&ArrayComplex64::new(1.0, 2.0))
        );
        assert_eq!(
            conjugate.get_zero_based(1, 0),
            Some(&ArrayComplex64::new(1.0, -2.0))
        );
    }

    #[test]
    fn sparse_runtime_work_observes_preexisting_cancellation() {
        let sparse = SparseArrayData::try_speye(4, 4, None).unwrap();
        let value = Value::Sparse(sparse);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            transpose(&value, false, &cancellation),
            Err(RuntimeErrorKind::Cancelled)
        );
        assert_eq!(
            index(
                &value,
                &[IndexInput::Value(Value::Double(1.0))],
                &cancellation,
            ),
            Err(RuntimeErrorKind::Cancelled)
        );
    }
}
