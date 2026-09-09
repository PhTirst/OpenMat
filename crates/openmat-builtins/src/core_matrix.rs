use openmat_array::{
    ArrayData, Complex32 as ArrayComplex32, Complex64 as ArrayComplex64, ComplexInteger,
    DenseArray, IntegerArrayData, IntegerComponent, IntegerElement, Logical, Shape,
};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::Value;

use crate::{
    array_error, exact_real_integer_scalar, expect_argument_count_range, expect_max_outputs,
    type_error,
};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;

trait MatrixArithmetic: Copy + Default {
    fn multiply(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
}

macro_rules! impl_real_matrix_arithmetic {
    ($($type:ty),+ $(,)?) => {
        $(
            impl MatrixArithmetic for $type {
                fn multiply(self, right: Self) -> Self {
                    self * right
                }

                fn subtract(self, right: Self) -> Self {
                    self - right
                }
            }
        )+
    };
}

impl_real_matrix_arithmetic!(f32, f64);

macro_rules! impl_integer_matrix_arithmetic {
    ($($type:ty),+ $(,)?) => {
        $(
            impl MatrixArithmetic for $type {
                fn multiply(self, right: Self) -> Self {
                    self.saturating_mul(right)
                }

                fn subtract(self, right: Self) -> Self {
                    self.saturating_sub(right)
                }
            }
        )+
    };
}

impl_integer_matrix_arithmetic!(i8, u8, i16, u16, i32, u32, i64, u64);

impl MatrixArithmetic for ArrayComplex64 {
    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }
}

impl MatrixArithmetic for ArrayComplex32 {
    fn multiply(self, right: Self) -> Self {
        Self::new(
            self.re * right.re - self.im * right.im,
            self.re * right.im + self.im * right.re,
        )
    }

    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }
}

#[derive(Clone, Copy)]
enum MeshgridVector<'a> {
    F32(&'a [f32]),
    F64(&'a [f64]),
}

impl MeshgridVector<'_> {
    const fn len(self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }
}

pub(super) fn meshgrid_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("meshgrid", arguments, 1, 3)?;
    expect_max_outputs("meshgrid", context, 3)?;
    context.check_cancelled()?;
    let requested_outputs = context.requested_outputs().max(1);
    if arguments.len() == 2 && requested_outputs == 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`meshgrid` requires either one or three inputs when three outputs are requested",
        ));
    }
    let x = meshgrid_vector(1, &arguments[0])?;
    let y = if arguments.len() == 1 {
        x
    } else {
        meshgrid_vector(2, &arguments[1])?
    };
    let is_three_dimensional = arguments.len() == 3 || requested_outputs == 3;
    let z = if is_three_dimensional {
        Some(if arguments.len() == 3 {
            meshgrid_vector(3, &arguments[2])?
        } else {
            x
        })
    } else {
        None
    };
    let rows = u64::try_from(y.len()).map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    let columns = u64::try_from(x.len()).map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    let pages = z.map_or(1, MeshgridVector::len);
    let pages_u64 = u64::try_from(pages).map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    let shape = if is_three_dimensional {
        Shape::new([rows, columns, pages_u64])
    } else {
        Shape::new([rows, columns])
    }
    .map_err(|error| array_error(&error))?;
    let mut outputs = Vec::with_capacity(requested_outputs);
    outputs.push(meshgrid_x(x, y.len(), pages, shape.clone(), context)?);
    if requested_outputs >= 2 {
        outputs.push(meshgrid_y(y, x.len(), pages, shape.clone(), context)?);
    }
    if requested_outputs == 3 {
        outputs.push(meshgrid_z(
            z.expect("three outputs always select a Z vector"),
            x.len(),
            y.len(),
            shape,
            context,
        )?);
    }
    context.check_cancelled()?;
    Ok(outputs)
}

fn meshgrid_vector(position: usize, value: &Value) -> Result<MeshgridVector<'_>, BuiltinError> {
    match value {
        Value::Double(value) => Ok(MeshgridVector::F64(std::slice::from_ref(value))),
        Value::Array(ArrayData::F64(array)) if array.shape().ndims() == 2 => {
            let dimensions = array.shape().dimensions();
            if array.is_empty() || dimensions[0] == 1 || dimensions[1] == 1 {
                Ok(MeshgridVector::F64(array.as_slice()))
            } else {
                Err(meshgrid_vector_error(position))
            }
        }
        Value::Array(ArrayData::F32(array)) if array.shape().ndims() == 2 => {
            let dimensions = array.shape().dimensions();
            if array.is_empty() || dimensions[0] == 1 || dimensions[1] == 1 {
                Ok(MeshgridVector::F32(array.as_slice()))
            } else {
                Err(meshgrid_vector_error(position))
            }
        }
        _ => Err(type_error(
            "meshgrid",
            position,
            "real double or single vector",
            value,
        )),
    }
}

fn meshgrid_vector_error(position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `meshgrid` must be a vector"),
    )
}

fn meshgrid_x(
    x: MeshgridVector<'_>,
    rows: usize,
    pages: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match x {
        MeshgridVector::F32(values) => meshgrid_x_values(values, rows, pages, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F32)
            .map(Value::Array),
        MeshgridVector::F64(values) => meshgrid_x_values(values, rows, pages, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F64)
            .map(Value::Array),
    }
}

fn meshgrid_y(
    y: MeshgridVector<'_>,
    columns: usize,
    pages: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match y {
        MeshgridVector::F32(values) => meshgrid_y_values(values, columns, pages, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F32)
            .map(Value::Array),
        MeshgridVector::F64(values) => meshgrid_y_values(values, columns, pages, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F64)
            .map(Value::Array),
    }
}

fn meshgrid_z(
    z: MeshgridVector<'_>,
    columns: usize,
    rows: usize,
    shape: Shape,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError> {
    match z {
        MeshgridVector::F32(values) => meshgrid_z_values(values, columns, rows, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F32)
            .map(Value::Array),
        MeshgridVector::F64(values) => meshgrid_z_values(values, columns, rows, context)
            .and_then(|values| DenseArray::from_vec(shape, values).map_err(|e| array_error(&e)))
            .map(ArrayData::F64)
            .map(Value::Array),
    }
}

fn meshgrid_x_values<T: Copy>(
    x: &[T],
    rows: usize,
    pages: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let capacity = rows
        .checked_mul(x.len())
        .and_then(|plane| plane.checked_mul(pages))
        .ok_or_else(|| host_length_error("meshgrid", u64::MAX))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    for page in 0..pages {
        for (column, value) in x.iter().copied().enumerate() {
            check_cancelled_at(context, page.saturating_mul(x.len()) + column)?;
            values.extend(std::iter::repeat_n(value, rows));
        }
    }
    Ok(values)
}

fn meshgrid_y_values<T: Copy>(
    y: &[T],
    columns: usize,
    pages: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let capacity = columns
        .checked_mul(y.len())
        .and_then(|plane| plane.checked_mul(pages))
        .ok_or_else(|| host_length_error("meshgrid", u64::MAX))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    for page in 0..pages {
        for column in 0..columns {
            check_cancelled_at(context, page.saturating_mul(columns) + column)?;
            values.extend_from_slice(y);
        }
    }
    Ok(values)
}

fn meshgrid_z_values<T: Copy>(
    z: &[T],
    columns: usize,
    rows: usize,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let plane = columns
        .checked_mul(rows)
        .ok_or_else(|| host_length_error("meshgrid", u64::MAX))?;
    let capacity = plane
        .checked_mul(z.len())
        .ok_or_else(|| host_length_error("meshgrid", u64::MAX))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| host_length_error("meshgrid", u64::MAX))?;
    for (page, value) in z.iter().copied().enumerate() {
        check_cancelled_at(context, page)?;
        values.extend(std::iter::repeat_n(value, plane));
    }
    Ok(values)
}

pub(super) fn diag_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("diag", arguments, 1, 2)?;
    expect_max_outputs("diag", context, 1)?;
    context.check_cancelled()?;
    let offset = arguments
        .get(1)
        .map_or(Ok(0), |value| diagonal_offset("diag", 2, value))?;
    let output = match &arguments[0] {
        Value::Logical(value) => {
            let input = scalar_dense(Logical::from(*value))?;
            Value::Array(ArrayData::Logical(diagonal_dense(
                "diag", &input, offset, context,
            )?))
        }
        Value::Double(value) => {
            let input = scalar_dense(*value)?;
            Value::Array(ArrayData::F64(diagonal_dense(
                "diag", &input, offset, context,
            )?))
        }
        Value::Complex(value) => {
            let input = scalar_dense(ArrayComplex64::new(value.real, value.imaginary))?;
            let output = diagonal_dense("diag", &input, offset, context)?;
            complex_f64_output(output)?
        }
        Value::Array(ArrayData::F32(array)) => Value::Array(ArrayData::F32(diagonal_dense(
            "diag", array, offset, context,
        )?)),
        Value::Array(ArrayData::ComplexF32(array)) => {
            let output = diagonal_dense("diag", array, offset, context)?;
            complex_f32_output(output)?
        }
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::Logical(
            diagonal_dense("diag", array, offset, context)?,
        )),
        Value::Array(ArrayData::F64(array)) => Value::Array(ArrayData::F64(diagonal_dense(
            "diag", array, offset, context,
        )?)),
        Value::Array(ArrayData::ComplexF64(array)) => {
            let output = diagonal_dense("diag", array, offset, context)?;
            complex_f64_output(output)?
        }
        Value::Array(ArrayData::Char(array)) => Value::Array(ArrayData::Char(diagonal_dense(
            "diag", array, offset, context,
        )?)),
        Value::Array(ArrayData::Integer(integer)) => Value::Array(ArrayData::Integer(
            diagonal_integer(integer, offset, context)?,
        )),
        value => {
            return Err(type_error(
                "diag",
                1,
                "numeric, logical, char, or integer vector or matrix",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn diagonal_dense<T: Clone + Default>(
    name: &str,
    input: &DenseArray<T>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let dimensions = input.shape().dimensions();
    if dimensions.len() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a vector or two-dimensional matrix"),
        ));
    }
    let rows = dimensions[0];
    let columns = dimensions[1];
    if input.numel() != 0 && (rows == 1 || columns == 1) {
        diagonal_matrix_from_vector(name, input, offset, context)
    } else {
        diagonal_vector_from_matrix(name, input, offset, context)
    }
}

fn diagonal_matrix_from_vector<T: Clone + Default>(
    name: &str,
    input: &DenseArray<T>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let length = input.numel();
    let distance = u64::try_from(offset.unsigned_abs()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("the diagonal offset to `{name}` does not fit the runtime shape model"),
        )
    })?;
    let order = length.checked_add(distance).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("the requested `{name}` output order overflowed"),
        )
    })?;
    let shape = Shape::new([order, order]).map_err(|error| array_error(&error))?;
    let output_length = checked_host_length(name, shape.numel())?;
    let mut values = filled_values(name, output_length, T::default(), context)?;
    let (start_row, start_column) = if offset < 0 {
        (distance, 0)
    } else {
        (0, distance)
    };
    for (index, value) in input.as_slice().iter().enumerate() {
        check_cancelled_at(context, index)?;
        let index = u64::try_from(index).map_err(|_| host_length_error(name, length))?;
        let row = start_row
            .checked_add(index)
            .ok_or_else(|| offset_error(name))?;
        let column = start_column
            .checked_add(index)
            .ok_or_else(|| offset_error(name))?;
        let output = column
            .checked_mul(order)
            .and_then(|value| value.checked_add(row))
            .ok_or_else(|| offset_error(name))?;
        let output = usize::try_from(output).map_err(|_| host_length_error(name, shape.numel()))?;
        *values.get_mut(output).ok_or_else(|| offset_error(name))? = value.clone();
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn diagonal_vector_from_matrix<T: Clone + Default>(
    name: &str,
    input: &DenseArray<T>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    let rows = input.shape().extent(0);
    let columns = input.shape().extent(1);
    let distance = u64::try_from(offset.unsigned_abs()).unwrap_or(u64::MAX);
    let (start_row, start_column, length) = if offset < 0 {
        (distance, 0, rows.saturating_sub(distance).min(columns))
    } else {
        (0, distance, rows.min(columns.saturating_sub(distance)))
    };
    let output_shape = if input.shape().dimensions() == [0, 0] {
        Shape::new([0, 0])
    } else {
        Shape::new([length, 1])
    }
    .map_err(|error| array_error(&error))?;
    let output_length = checked_host_length(name, length)?;
    let mut values = reserved_values(name, output_length)?;
    for index in 0..length {
        let host_index = usize::try_from(index).map_err(|_| host_length_error(name, length))?;
        check_cancelled_at(context, host_index)?;
        let row = start_row
            .checked_add(index)
            .ok_or_else(|| offset_error(name))?;
        let column = start_column
            .checked_add(index)
            .ok_or_else(|| offset_error(name))?;
        let input_offset = column
            .checked_mul(rows)
            .and_then(|value| value.checked_add(row))
            .ok_or_else(|| offset_error(name))?;
        let input_offset =
            usize::try_from(input_offset).map_err(|_| host_length_error(name, input.numel()))?;
        values.push(
            input
                .as_slice()
                .get(input_offset)
                .ok_or_else(|| offset_error(name))?
                .clone(),
        );
    }
    DenseArray::from_vec(output_shape, values).map_err(|error| array_error(&error))
}

fn diagonal_integer(
    input: &IntegerArrayData,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! real_variant {
        ($array:expr) => {
            diagonal_dense("diag", $array, offset, context).map(IntegerArrayData::from_typed)
        };
    }
    macro_rules! complex_variant {
        ($array:expr) => {
            diagonal_complex_integer($array, offset, context)
        };
    }
    match input {
        IntegerArrayData::I8(array) => real_variant!(array),
        IntegerArrayData::ComplexI8(array) => complex_variant!(array),
        IntegerArrayData::U8(array) => real_variant!(array),
        IntegerArrayData::ComplexU8(array) => complex_variant!(array),
        IntegerArrayData::I16(array) => real_variant!(array),
        IntegerArrayData::ComplexI16(array) => complex_variant!(array),
        IntegerArrayData::U16(array) => real_variant!(array),
        IntegerArrayData::ComplexU16(array) => complex_variant!(array),
        IntegerArrayData::I32(array) => real_variant!(array),
        IntegerArrayData::ComplexI32(array) => complex_variant!(array),
        IntegerArrayData::U32(array) => real_variant!(array),
        IntegerArrayData::ComplexU32(array) => complex_variant!(array),
        IntegerArrayData::I64(array) => real_variant!(array),
        IntegerArrayData::ComplexI64(array) => complex_variant!(array),
        IntegerArrayData::U64(array) => real_variant!(array),
        IntegerArrayData::ComplexU64(array) => complex_variant!(array),
    }
}

fn diagonal_complex_integer<T>(
    input: &DenseArray<ComplexInteger<T>>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError>
where
    T: IntegerElement + Copy + Default + PartialEq,
    ComplexInteger<T>: IntegerElement,
{
    let output = diagonal_dense("diag", input, offset, context)?;
    if output
        .as_slice()
        .iter()
        .all(|value| value.im() == T::default())
    {
        let shape = output.shape().clone();
        let values = output.as_slice().iter().map(|value| value.re()).collect();
        DenseArray::from_vec(shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    } else {
        Ok(IntegerArrayData::from_typed(output))
    }
}

#[derive(Clone, Copy)]
enum Triangle {
    Upper,
    Lower,
}

impl Triangle {
    const fn name(self) -> &'static str {
        match self {
            Self::Upper => "triu",
            Self::Lower => "tril",
        }
    }

    const fn keeps(self, row: u64, column: u64, offset: i128) -> bool {
        let diagonal = column as i128 - row as i128;
        match self {
            Self::Upper => diagonal >= offset,
            Self::Lower => diagonal <= offset,
        }
    }
}

pub(super) fn triu_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    triangular_builtin(Triangle::Upper, arguments, context)
}

pub(super) fn tril_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    triangular_builtin(Triangle::Lower, arguments, context)
}

fn triangular_builtin(
    triangle: Triangle,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    let name = triangle.name();
    expect_argument_count_range(name, arguments, 1, 2)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let offset = arguments
        .get(1)
        .map_or(Ok(0), |value| diagonal_offset(name, 2, value))?;
    let output = match &arguments[0] {
        Value::Logical(value) => Value::Logical(triangle.keeps(0, 0, offset) && *value),
        Value::Double(value) => Value::Double(if triangle.keeps(0, 0, offset) {
            *value
        } else {
            0.0
        }),
        Value::Complex(_) => {
            if triangle.keeps(0, 0, offset) {
                arguments[0].clone()
            } else {
                Value::Double(0.0)
            }
        }
        Value::Array(ArrayData::F32(array)) => Value::Array(ArrayData::F32(triangular_dense(
            name, triangle, array, offset, context,
        )?)),
        Value::Array(ArrayData::ComplexF32(array)) => {
            let output = triangular_dense(name, triangle, array, offset, context)?;
            complex_f32_output(output)?
        }
        Value::Array(ArrayData::Logical(array)) => Value::Array(ArrayData::Logical(
            triangular_dense(name, triangle, array, offset, context)?,
        )),
        Value::Array(ArrayData::F64(array)) => Value::Array(ArrayData::F64(triangular_dense(
            name, triangle, array, offset, context,
        )?)),
        Value::Array(ArrayData::ComplexF64(array)) => {
            let output = triangular_dense(name, triangle, array, offset, context)?;
            complex_f64_output(output)?
        }
        Value::Array(ArrayData::Char(array)) => Value::Array(ArrayData::Char(triangular_dense(
            name, triangle, array, offset, context,
        )?)),
        Value::Array(ArrayData::Integer(integer)) => Value::Array(ArrayData::Integer(
            triangular_integer(name, triangle, integer, offset, context)?,
        )),
        value => {
            return Err(type_error(
                name,
                1,
                "numeric, logical, char, or integer matrix",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn triangular_dense<T: Clone + Default>(
    name: &str,
    triangle: Triangle,
    input: &DenseArray<T>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    if input.shape().ndims() != 2 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a two-dimensional matrix"),
        ));
    }
    let rows = input.shape().extent(0);
    let mut values = clone_values(name, input.as_slice(), context)?;
    for (index, value) in values.iter_mut().enumerate() {
        check_cancelled_at(context, index)?;
        let index = u64::try_from(index).map_err(|_| host_length_error(name, input.numel()))?;
        let row = if rows == 0 { 0 } else { index % rows };
        let column = if rows == 0 { 0 } else { index / rows };
        if !triangle.keeps(row, column, offset) {
            *value = T::default();
        }
    }
    DenseArray::from_vec(input.shape().clone(), values).map_err(|error| array_error(&error))
}

fn triangular_integer(
    name: &str,
    triangle: Triangle,
    input: &IntegerArrayData,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! real_variant {
        ($array:expr) => {
            triangular_dense(name, triangle, $array, offset, context)
                .map(IntegerArrayData::from_typed)
        };
    }
    macro_rules! complex_variant {
        ($array:expr) => {
            triangular_complex_integer(name, triangle, $array, offset, context)
        };
    }
    match input {
        IntegerArrayData::I8(array) => real_variant!(array),
        IntegerArrayData::ComplexI8(array) => complex_variant!(array),
        IntegerArrayData::U8(array) => real_variant!(array),
        IntegerArrayData::ComplexU8(array) => complex_variant!(array),
        IntegerArrayData::I16(array) => real_variant!(array),
        IntegerArrayData::ComplexI16(array) => complex_variant!(array),
        IntegerArrayData::U16(array) => real_variant!(array),
        IntegerArrayData::ComplexU16(array) => complex_variant!(array),
        IntegerArrayData::I32(array) => real_variant!(array),
        IntegerArrayData::ComplexI32(array) => complex_variant!(array),
        IntegerArrayData::U32(array) => real_variant!(array),
        IntegerArrayData::ComplexU32(array) => complex_variant!(array),
        IntegerArrayData::I64(array) => real_variant!(array),
        IntegerArrayData::ComplexI64(array) => complex_variant!(array),
        IntegerArrayData::U64(array) => real_variant!(array),
        IntegerArrayData::ComplexU64(array) => complex_variant!(array),
    }
}

fn triangular_complex_integer<T>(
    name: &str,
    triangle: Triangle,
    input: &DenseArray<ComplexInteger<T>>,
    offset: i128,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError>
where
    T: IntegerElement + Copy + Default + PartialEq,
    ComplexInteger<T>: IntegerElement,
{
    let output = triangular_dense(name, triangle, input, offset, context)?;
    if output
        .as_slice()
        .iter()
        .all(|value| value.im() == T::default())
    {
        let shape = output.shape().clone();
        let values = output.as_slice().iter().map(|value| value.re()).collect();
        DenseArray::from_vec(shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))
    } else {
        Ok(IntegerArrayData::from_typed(output))
    }
}

pub(super) fn trace_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    crate::expect_argument_count("trace", arguments, 1)?;
    expect_max_outputs("trace", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::Array(ArrayData::F32(input)) => {
            let value = trace_dense("trace", input, context, |left, right| left + right)?;
            let output = scalar_dense(value)?;
            Value::Array(ArrayData::F32(output))
        }
        Value::Array(ArrayData::ComplexF32(input)) => {
            let value = trace_dense("trace", input, context, |left, right| {
                ArrayComplex32::new(left.re + right.re, left.im + right.im)
            })?;
            complex_f32_output(scalar_dense(value)?)?
        }
        value => {
            let input = complex_f64_matrix("trace", 1, value, context)?;
            let result = trace_dense("trace", &input, context, |left, right| {
                ArrayComplex64::new(left.re + right.re, left.im + right.im)
            })?;
            if result.im == 0.0 {
                Value::Double(result.re)
            } else {
                Value::Complex(openmat_value::Complex64::new(result.re, result.im))
            }
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn trace_dense<T, F>(
    name: &str,
    input: &DenseArray<T>,
    context: &BuiltinContext<'_>,
    add: F,
) -> Result<T, BuiltinError>
where
    T: Copy + Default,
    F: Fn(T, T) -> T,
{
    ensure_square_matrix(name, input.shape())?;
    let order = checked_host_length(name, input.shape().extent(0))?;
    let mut result = T::default();
    for diagonal in 0..order {
        check_cancelled_at(context, diagonal)?;
        result = add(result, input.as_slice()[diagonal * order + diagonal]);
    }
    Ok(result)
}

pub(super) fn kron_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    crate::expect_argument_count("kron", arguments, 2)?;
    expect_max_outputs("kron", context, 1)?;
    context.check_cancelled()?;
    let output = match (&arguments[0], &arguments[1]) {
        (Value::Array(ArrayData::Integer(left)), Value::Array(ArrayData::Integer(right))) => {
            Value::Array(ArrayData::Integer(kron_integer(left, right, context)?))
        }
        (Value::Array(ArrayData::Integer(_)), _) | (_, Value::Array(ArrayData::Integer(_))) => {
            return Err(mixed_integer_error("kron"));
        }
        (left, right) if is_single(left) || is_single(right) => {
            let left = complex_f32_matrix("kron", 1, left, context)?;
            let right = complex_f32_matrix("kron", 2, right, context)?;
            complex_f32_output(kron_dense("kron", &left, &right, context)?)?
        }
        (left, right) => {
            let left = complex_f64_matrix("kron", 1, left, context)?;
            let right = complex_f64_matrix("kron", 2, right, context)?;
            complex_f64_output(kron_dense("kron", &left, &right, context)?)?
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn kron_dense<T: MatrixArithmetic>(
    name: &str,
    left: &DenseArray<T>,
    right: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    ensure_matrix(name, left.shape())?;
    ensure_matrix(name, right.shape())?;
    let left_rows = checked_host_length(name, left.shape().extent(0))?;
    let left_columns = checked_host_length(name, left.shape().extent(1))?;
    let right_rows = checked_host_length(name, right.shape().extent(0))?;
    let right_columns = checked_host_length(name, right.shape().extent(1))?;
    let rows = left
        .shape()
        .extent(0)
        .checked_mul(right.shape().extent(0))
        .ok_or_else(|| host_length_error(name, u64::MAX))?;
    let columns = left
        .shape()
        .extent(1)
        .checked_mul(right.shape().extent(1))
        .ok_or_else(|| host_length_error(name, u64::MAX))?;
    let shape = Shape::new([rows, columns]).map_err(|error| array_error(&error))?;
    let mut values = reserved_values(name, checked_host_length(name, shape.numel())?)?;
    for left_column in 0..left_columns {
        for right_column in 0..right_columns {
            context.check_cancelled()?;
            for left_row in 0..left_rows {
                let scale = left.as_slice()[left_column * left_rows + left_row];
                for right_row in 0..right_rows {
                    values.push(
                        scale.multiply(right.as_slice()[right_column * right_rows + right_row]),
                    );
                }
            }
        }
    }
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn kron_integer(
    left: &IntegerArrayData,
    right: &IntegerArrayData,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! same_variant {
        ($left:expr, $right:expr) => {
            kron_dense("kron", $left, $right, context).map(IntegerArrayData::from_typed)
        };
    }
    match (left, right) {
        (IntegerArrayData::I8(left), IntegerArrayData::I8(right)) => same_variant!(left, right),
        (IntegerArrayData::U8(left), IntegerArrayData::U8(right)) => same_variant!(left, right),
        (IntegerArrayData::I16(left), IntegerArrayData::I16(right)) => same_variant!(left, right),
        (IntegerArrayData::U16(left), IntegerArrayData::U16(right)) => same_variant!(left, right),
        (IntegerArrayData::I32(left), IntegerArrayData::I32(right)) => same_variant!(left, right),
        (IntegerArrayData::U32(left), IntegerArrayData::U32(right)) => same_variant!(left, right),
        (IntegerArrayData::I64(left), IntegerArrayData::I64(right)) => same_variant!(left, right),
        (IntegerArrayData::U64(left), IntegerArrayData::U64(right)) => same_variant!(left, right),
        _ => Err(mixed_integer_error("kron")),
    }
}

pub(super) fn cross_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("cross", arguments, 2, 3)?;
    expect_max_outputs("cross", context, 1)?;
    context.check_cancelled()?;
    let requested_dimension = arguments
        .get(2)
        .map(|value| vector_dimension("cross", 3, value))
        .transpose()?;
    let output = match (&arguments[0], &arguments[1]) {
        (Value::Array(ArrayData::Integer(left)), Value::Array(ArrayData::Integer(right))) => {
            Value::Array(ArrayData::Integer(cross_integer(
                left,
                right,
                requested_dimension,
                context,
            )?))
        }
        (Value::Array(ArrayData::Integer(_)), _) | (_, Value::Array(ArrayData::Integer(_))) => {
            return Err(mixed_integer_error("cross"));
        }
        (left, right) if is_single(left) || is_single(right) => {
            let left = complex_f32_matrix("cross", 1, left, context)?;
            let right = complex_f32_matrix("cross", 2, right, context)?;
            complex_f32_output(cross_dense(
                "cross",
                &left,
                &right,
                requested_dimension,
                context,
            )?)?
        }
        (left, right) => {
            let left = complex_f64_matrix("cross", 1, left, context)?;
            let right = complex_f64_matrix("cross", 2, right, context)?;
            complex_f64_output(cross_dense(
                "cross",
                &left,
                &right,
                requested_dimension,
                context,
            )?)?
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn cross_dense<T: MatrixArithmetic>(
    name: &str,
    left: &DenseArray<T>,
    right: &DenseArray<T>,
    requested_dimension: Option<usize>,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<T>, BuiltinError> {
    if left.shape() != right.shape() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("inputs to `{name}` must have the same size"),
        ));
    }
    let dimension = requested_dimension
        .or_else(|| {
            left.shape()
                .dimensions()
                .iter()
                .position(|extent| *extent == 3)
        })
        .ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("inputs to `{name}` must have a dimension of length 3"),
            )
        })?;
    if left.shape().extent(dimension) != 3 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("the selected dimension of `{name}` must have length 3"),
        ));
    }
    let stride = left.shape().dimensions()[..dimension]
        .iter()
        .try_fold(1_usize, |value, extent| {
            value.checked_mul(usize::try_from(*extent).ok()?)
        })
        .ok_or_else(|| host_length_error(name, left.numel()))?;
    let block = stride
        .checked_mul(3)
        .ok_or_else(|| host_length_error(name, left.numel()))?;
    let length = checked_host_length(name, left.numel())?;
    let outer = if block == 0 { 0 } else { length / block };
    let mut values = filled_values(
        name,
        checked_host_length(name, left.numel())?,
        T::default(),
        context,
    )?;
    for outer_index in 0..outer {
        context.check_cancelled()?;
        for inner in 0..stride {
            let first = outer_index * block + inner;
            let second = first + stride;
            let third = second + stride;
            values[first] = left.as_slice()[second]
                .multiply(right.as_slice()[third])
                .subtract(left.as_slice()[third].multiply(right.as_slice()[second]));
            values[second] = left.as_slice()[third]
                .multiply(right.as_slice()[first])
                .subtract(left.as_slice()[first].multiply(right.as_slice()[third]));
            values[third] = left.as_slice()[first]
                .multiply(right.as_slice()[second])
                .subtract(left.as_slice()[second].multiply(right.as_slice()[first]));
        }
    }
    DenseArray::from_vec(left.shape().clone(), values).map_err(|error| array_error(&error))
}

fn cross_integer(
    left: &IntegerArrayData,
    right: &IntegerArrayData,
    dimension: Option<usize>,
    context: &BuiltinContext<'_>,
) -> Result<IntegerArrayData, BuiltinError> {
    macro_rules! same_variant {
        ($left:expr, $right:expr) => {
            cross_dense("cross", $left, $right, dimension, context)
                .map(IntegerArrayData::from_typed)
        };
    }
    match (left, right) {
        (IntegerArrayData::I8(left), IntegerArrayData::I8(right)) => same_variant!(left, right),
        (IntegerArrayData::U8(left), IntegerArrayData::U8(right)) => same_variant!(left, right),
        (IntegerArrayData::I16(left), IntegerArrayData::I16(right)) => same_variant!(left, right),
        (IntegerArrayData::U16(left), IntegerArrayData::U16(right)) => same_variant!(left, right),
        (IntegerArrayData::I32(left), IntegerArrayData::I32(right)) => same_variant!(left, right),
        (IntegerArrayData::U32(left), IntegerArrayData::U32(right)) => same_variant!(left, right),
        (IntegerArrayData::I64(left), IntegerArrayData::I64(right)) => same_variant!(left, right),
        (IntegerArrayData::U64(left), IntegerArrayData::U64(right)) => same_variant!(left, right),
        _ => Err(mixed_integer_error("cross")),
    }
}

fn complex_f64_matrix(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<ArrayComplex64>, BuiltinError> {
    let (shape, values) = match value {
        Value::Logical(value) => (
            Shape::new([1, 1]).map_err(|error| array_error(&error))?,
            vec![ArrayComplex64::new(f64::from(u8::from(*value)), 0.0)],
        ),
        Value::Double(value) => (
            Shape::new([1, 1]).map_err(|error| array_error(&error))?,
            vec![ArrayComplex64::new(*value, 0.0)],
        ),
        Value::Complex(value) => (
            Shape::new([1, 1]).map_err(|error| array_error(&error))?,
            vec![ArrayComplex64::new(value.real, value.imaginary)],
        ),
        Value::Array(ArrayData::F32(input)) => (
            input.shape().clone(),
            map_values(name, input.as_slice(), context, |value| {
                ArrayComplex64::new(f64::from(*value), 0.0)
            })?,
        ),
        Value::Array(ArrayData::ComplexF32(input)) => (
            input.shape().clone(),
            map_values(name, input.as_slice(), context, |value| {
                ArrayComplex64::new(f64::from(value.re), f64::from(value.im))
            })?,
        ),
        Value::Array(ArrayData::F64(input)) => (
            input.shape().clone(),
            map_values(name, input.as_slice(), context, |value| {
                ArrayComplex64::new(*value, 0.0)
            })?,
        ),
        Value::Array(ArrayData::ComplexF64(input)) => (
            input.shape().clone(),
            clone_values(name, input.as_slice(), context)?,
        ),
        Value::Array(ArrayData::Logical(input)) => (
            input.shape().clone(),
            map_values(name, input.as_slice(), context, |value| {
                ArrayComplex64::new(f64::from(u8::from(value.get())), 0.0)
            })?,
        ),
        Value::Array(ArrayData::Char(input)) => (
            input.shape().clone(),
            map_values(name, input.as_slice(), context, |value| {
                ArrayComplex64::new(f64::from(value.get()), 0.0)
            })?,
        ),
        Value::Array(ArrayData::Integer(input)) => {
            let mut values = reserved_values(name, checked_host_length(name, input.numel())?)?;
            for (index, value) in input.elements().enumerate() {
                check_cancelled_at(context, index)?;
                values.push(ArrayComplex64::new(
                    integer_component_f64(value.real_component()),
                    value
                        .imaginary_component()
                        .map_or(0.0, integer_component_f64),
                ));
            }
            (input.shape().clone(), values)
        }
        _ => {
            return Err(type_error(
                name,
                position,
                "numeric, logical, or char array",
                value,
            ));
        }
    };
    DenseArray::from_vec(shape, values).map_err(|error| array_error(&error))
}

fn complex_f32_matrix(
    name: &str,
    position: usize,
    value: &Value,
    context: &BuiltinContext<'_>,
) -> Result<DenseArray<ArrayComplex32>, BuiltinError> {
    let double = complex_f64_matrix(name, position, value, context)?;
    #[allow(clippy::cast_possible_truncation)]
    let values = map_values(name, double.as_slice(), context, |value| {
        ArrayComplex32::new(value.re as f32, value.im as f32)
    })?;
    DenseArray::from_vec(double.shape().clone(), values).map_err(|error| array_error(&error))
}

fn map_values<T, R, F>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
    map: F,
) -> Result<Vec<R>, BuiltinError>
where
    F: Fn(&T) -> R,
{
    let mut output = reserved_values(name, input.len())?;
    for (index, value) in input.iter().enumerate() {
        check_cancelled_at(context, index)?;
        output.push(map(value));
    }
    Ok(output)
}

fn integer_component_f64(value: IntegerComponent) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    match value {
        IntegerComponent::Signed(value) => value as f64,
        IntegerComponent::Unsigned(value) => value as f64,
    }
}

fn is_single(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(ArrayData::F32(_) | ArrayData::ComplexF32(_))
    )
}

fn ensure_matrix(name: &str, shape: &Shape) -> Result<(), BuiltinError> {
    if shape.ndims() == 2 {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires two-dimensional matrix inputs"),
        ))
    }
}

fn ensure_square_matrix(name: &str, shape: &Shape) -> Result<(), BuiltinError> {
    ensure_matrix(name, shape)?;
    if shape.extent(0) == shape.extent(1) {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` requires a square matrix"),
        ))
    }
}

fn vector_dimension(name: &str, position: usize, value: &Value) -> Result<usize, BuiltinError> {
    let one_based = if let Some(dimension) = exact_real_integer_scalar(value) {
        match dimension {
            IntegerComponent::Signed(value) => usize::try_from(value).ok(),
            IntegerComponent::Unsigned(value) => usize::try_from(value).ok(),
        }
    } else {
        let numeric = value
            .as_real_number()
            .or_else(|| value.as_real_single().map(f64::from));
        numeric.and_then(|value| {
            if value.is_finite() && value.fract() == 0.0 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(value as usize)
            } else {
                None
            }
        })
    }
    .filter(|value| *value > 0)
    .ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a positive integer scalar"),
        )
    })?;
    Ok(one_based - 1)
}

fn mixed_integer_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!("`{name}` does not combine integer arrays with a different numeric class"),
    )
}

fn complex_f64_output(input: DenseArray<ArrayComplex64>) -> Result<Value, BuiltinError> {
    if input.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = input.shape().clone();
        let values = input.as_slice().iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        Ok(Value::Array(ArrayData::ComplexF64(input)))
    }
}

fn complex_f32_output(input: DenseArray<ArrayComplex32>) -> Result<Value, BuiltinError> {
    if input.as_slice().iter().all(|value| value.im == 0.0) {
        let shape = input.shape().clone();
        let values = input.as_slice().iter().map(|value| value.re).collect();
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F32)
            .map(Value::Array)
            .map_err(|error| array_error(&error))
    } else {
        Ok(Value::Array(ArrayData::ComplexF32(input)))
    }
}

fn diagonal_offset(name: &str, position: usize, value: &Value) -> Result<i128, BuiltinError> {
    if let Some(value) = exact_real_integer_scalar(value) {
        return Ok(match value {
            IntegerComponent::Signed(value) => value,
            IntegerComponent::Unsigned(value) => i128::try_from(value).unwrap_or(i128::MAX),
        });
    }
    let value = value
        .as_real_number()
        .or_else(|| value.as_real_single().map(f64::from))
        .ok_or_else(|| type_error(name, position, "real integer scalar offset", value))?;
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a finite real integer scalar"),
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i128)
}

fn scalar_dense<T>(value: T) -> Result<DenseArray<T>, BuiltinError> {
    DenseArray::from_vec(
        Shape::new([1, 1]).map_err(|error| array_error(&error))?,
        vec![value],
    )
    .map_err(|error| array_error(&error))
}

fn clone_values<T: Clone>(
    name: &str,
    input: &[T],
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_values(name, input.len())?;
    for chunk in input.chunks(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()?;
        output.extend_from_slice(chunk);
    }
    Ok(output)
}

fn filled_values<T: Clone>(
    name: &str,
    length: usize,
    value: T,
    context: &BuiltinContext<'_>,
) -> Result<Vec<T>, BuiltinError> {
    let mut output = reserved_values(name, length)?;
    while output.len() < length {
        context.check_cancelled()?;
        let target = output
            .len()
            .saturating_add(CANCELLATION_CHECK_INTERVAL)
            .min(length);
        output.resize(target, value.clone());
    }
    Ok(output)
}

fn reserved_values<T>(name: &str, length: usize) -> Result<Vec<T>, BuiltinError> {
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} elements"),
        )
    })?;
    Ok(output)
}

fn checked_host_length(name: &str, length: u64) -> Result<usize, BuiltinError> {
    usize::try_from(length).map_err(|_| host_length_error(name, length))
}

fn check_cancelled_at(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn host_length_error(name: &str, length: u64) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` array length {length} does not fit this host"),
    )
}

fn offset_error(name: &str) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` computed an invalid checked column-major offset"),
    )
}
