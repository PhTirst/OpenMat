use std::fmt::Write as _;

use openmat_array::ArrayData;
use openmat_value::{
    CellArray, SparseArrayData, SparseScalarValue, StringElement, TableArray, Value,
};

use crate::{Interpreter, NumericFormat};

const DISPLAY_SUMMARY_MAX_ELEMENTS: usize = 16;
const DISPLAY_MATRIX_MAX_ELEMENTS: usize = 4_096;
const DISPLAY_MAX_TEXT_BYTES: usize = 4_096;

/// Formats one language value using the command-window display contract.
#[must_use]
pub fn format_display_value_with(
    value: &Value,
    interpreter: &Interpreter,
    numeric_format: NumericFormat,
) -> String {
    match value {
        Value::Nothing => "[]".to_owned(),
        Value::Logical(value) => format_display_cell(if *value { "1" } else { "0" }),
        Value::Double(value) => format_display_cell(&format_numeric_real(*value, numeric_format)),
        Value::Complex(value) => format_display_cell(&format_numeric_complex(
            value.real,
            value.imaginary,
            numeric_format,
        )),
        Value::String(value) => format_string(value),
        Value::Array(array) => format_array(array, numeric_format),
        Value::Sparse(array) => format_sparse_array(array, numeric_format),
        Value::Cell(array) => format_cell(array, interpreter, numeric_format),
        Value::Struct(array) => format!(
            "struct [{}]",
            array
                .shape()
                .dimensions()
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("x")
        ),
        Value::Table(array) => format_table(array, interpreter, numeric_format),
        Value::Object(handle) => format!(
            "{} object({})",
            interpreter.value_class_name(value),
            handle.identifier()
        ),
        Value::ObjectArray(array) => format!(
            "{} object [{}]",
            interpreter.value_class_name(value),
            array
                .shape()
                .dimensions()
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("x")
        ),
        Value::Graphics(handle) => format!("{} handle", handle.class().class_name()),
        Value::GraphicsArray(array) => format!(
            "{} handle [{}]",
            array.class().class_name(),
            array
                .shape()
                .dimensions()
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("x")
        ),
        Value::Function(handle) => format!("{handle:?}"),
    }
}

fn format_sparse_array(array: &SparseArrayData, numeric_format: NumericFormat) -> String {
    let mut output = format!(
        "{} sparse [{}] with {} stored entries",
        array.class_name(),
        array
            .shape()
            .dimensions()
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("x"),
        array.nnz()
    );
    for position in 0..array.nnz().min(DISPLAY_SUMMARY_MAX_ELEMENTS) {
        let Some(entry) = array.stored_entry(position) else {
            break;
        };
        let value = match entry.value {
            SparseScalarValue::Logical(value) => u8::from(value).to_string(),
            SparseScalarValue::F64(value) => format_numeric_real(value, numeric_format),
            SparseScalarValue::ComplexF64(value) => {
                format_numeric_complex(value.re, value.im, numeric_format)
            }
        };
        let _ = write!(
            output,
            "\n  ({},{})  {value}",
            entry.row + 1,
            entry.column + 1
        );
    }
    if array.nnz() > DISPLAY_SUMMARY_MAX_ELEMENTS {
        output.push_str("\n  ...");
    }
    output
}

fn format_table(
    table: &TableArray,
    interpreter: &Interpreter,
    numeric_format: NumericFormat,
) -> String {
    let rows = usize::try_from(table.row_count()).unwrap_or(usize::MAX);
    let maximum_rows = if table.variable_count() == 0 {
        0
    } else {
        (DISPLAY_MATRIX_MAX_ELEMENTS / table.variable_count().max(1)).min(64)
    };
    let shown_rows = rows.min(maximum_rows);
    let has_row_names = table.row_names().is_some();
    let mut headings = Vec::with_capacity(table.variable_count() + usize::from(has_row_names));
    if has_row_names {
        headings.push(String::from("Row"));
    }
    headings.extend(
        table
            .variable_names()
            .iter()
            .map(|name| name.as_str().to_owned()),
    );
    if headings.is_empty() {
        return format!("{}x0 table", table.row_count());
    }

    let mut cells = Vec::with_capacity(shown_rows);
    for row in 0..shown_rows {
        let mut values = Vec::with_capacity(headings.len());
        if let Some(row_names) = table.row_names() {
            values.push(
                row_names
                    .get(row)
                    .map_or_else(String::new, |name| name.as_str().to_owned()),
            );
        }
        for variable in 0..table.variable_count() {
            values.push(table.variable(variable).map_or_else(
                || String::from("<invalid>"),
                |value| format_table_variable_row(value, row, rows, interpreter, numeric_format),
            ));
        }
        cells.push(values);
    }

    let mut widths = headings
        .iter()
        .map(|heading| heading.chars().count().max(3))
        .collect::<Vec<_>>();
    for row in &cells {
        for (column, value) in row.iter().enumerate() {
            widths[column] = widths[column].max(value.chars().count()).min(48);
        }
    }
    let mut output = String::new();
    write_table_display_row(&mut output, &headings, &widths);
    output.push('\n');
    let separators = widths
        .iter()
        .map(|width| "_".repeat(*width))
        .collect::<Vec<_>>();
    write_table_display_row(&mut output, &separators, &widths);
    for row in &cells {
        output.push('\n');
        write_table_display_row(&mut output, row, &widths);
        if output.len() >= DISPLAY_MAX_TEXT_BYTES {
            return bounded_text(&output, DISPLAY_MAX_TEXT_BYTES);
        }
    }
    if shown_rows < rows {
        write!(&mut output, "\n… {} more rows", rows - shown_rows).ok();
    }
    bounded_text(&output, DISPLAY_MAX_TEXT_BYTES)
}

fn format_cell(
    cell: &CellArray,
    interpreter: &Interpreter,
    numeric_format: NumericFormat,
) -> String {
    if cell.numel() == 0 {
        return "{}".to_owned();
    }
    if cell.shape().ndims() == 2
        && cell.numel() <= u64::try_from(DISPLAY_MATRIX_MAX_ELEMENTS).unwrap_or(u64::MAX)
    {
        let values = cell
            .values()
            .iter()
            .map(|value| {
                bounded_text(
                    &format!(
                        "{{{}}}",
                        format_table_nested_value(value, interpreter, numeric_format)
                            .replace('\n', " ")
                    ),
                    48,
                )
            })
            .collect::<Vec<_>>();
        if let Some(matrix) = format_matrix_values(cell.shape(), &values) {
            return matrix;
        }
    }
    format!(
        "cell [{}]",
        cell.shape()
            .dimensions()
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("x")
    )
}

fn write_table_display_row(output: &mut String, values: &[String], widths: &[usize]) {
    for (column, (value, width)) in values.iter().zip(widths).enumerate() {
        if column != 0 {
            output.push_str("    ");
        }
        let value = bounded_text(value, *width);
        write!(output, "{value:<width$}").ok();
    }
}

fn format_table_variable_row(
    value: &Value,
    row: usize,
    rows: usize,
    interpreter: &Interpreter,
    numeric_format: NumericFormat,
) -> String {
    fn row_values<T>(
        values: &[T],
        row: usize,
        rows: usize,
        format: impl Fn(&T) -> String,
    ) -> String {
        if rows == 0 || row >= rows {
            return String::from("<invalid>");
        }
        let values = (0..values.len() / rows)
            .filter_map(|column| values.get(column * rows + row))
            .map(format)
            .collect::<Vec<_>>();
        match values.as_slice() {
            [value] => value.clone(),
            _ => format!("[{}]", values.join(" ")),
        }
    }

    let result = match value {
        Value::Logical(value) if row == 0 => value.to_string(),
        Value::Double(value) if row == 0 => format_table_numeric_real(*value, numeric_format),
        Value::Complex(value) if row == 0 => {
            format_table_numeric_complex(value.real, value.imaginary, numeric_format)
        }
        Value::Array(ArrayData::F32(array)) => row_values(array.as_slice(), row, rows, |value| {
            format_numeric_real_f32(*value, numeric_format)
        }),
        Value::Array(ArrayData::ComplexF32(array)) => {
            row_values(array.as_slice(), row, rows, |value| {
                format_numeric_complex_f32(value.re, value.im, numeric_format)
            })
        }
        Value::Array(ArrayData::F64(array)) => row_values(array.as_slice(), row, rows, |value| {
            format_table_numeric_real(*value, numeric_format)
        }),
        Value::Array(ArrayData::ComplexF64(array)) => {
            row_values(array.as_slice(), row, rows, |value| {
                format_table_numeric_complex(value.re, value.im, numeric_format)
            })
        }
        Value::Array(ArrayData::Logical(array)) => {
            row_values(array.as_slice(), row, rows, |value| value.get().to_string())
        }
        Value::Array(ArrayData::Integer(array)) => {
            let values = (0..usize::try_from(array.numel()).unwrap_or(0))
                .filter_map(|offset| array.element(offset))
                .collect::<Vec<_>>();
            row_values(&values, row, rows, |value| format_integer_element(*value))
        }
        Value::Array(ArrayData::Char(array)) => {
            let units = (0..array.as_slice().len() / rows.max(1))
                .filter_map(|column| array.as_slice().get(column * rows.max(1) + row))
                .map(|unit| unit.get())
                .collect::<Vec<_>>();
            format!("'{}'", String::from_utf16_lossy(&units))
        }
        Value::String(strings) => {
            let values = (0..usize::try_from(strings.numel()).unwrap_or(0))
                .filter_map(|offset| strings.element(offset))
                .collect::<Vec<_>>();
            row_values(&values, row, rows, |value| {
                if value.is_missing() {
                    String::from("<missing>")
                } else {
                    format!("\"{}\"", value.to_utf8_lossy())
                }
            })
        }
        Value::Cell(cell) => row_values(cell.values(), row, rows, |value| {
            format!(
                "{{{}}}",
                format_table_nested_value(value, interpreter, numeric_format)
            )
        }),
        other => format!(
            "{} [{}]",
            interpreter.value_class_name(other),
            other
                .dimensions()
                .unwrap_or(&[])
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("x")
        ),
    };
    bounded_text(&result.replace('\n', " "), 48)
}

fn format_table_numeric_real(value: f64, format: NumericFormat) -> String {
    if format == NumericFormat::Short && value.is_finite() {
        format_general(value, 5)
    } else {
        format_numeric_real(value, format)
    }
}

fn format_table_numeric_complex(real: f64, imaginary: f64, format: NumericFormat) -> String {
    if format != NumericFormat::Short || !real.is_finite() || !imaginary.is_finite() {
        return format_numeric_complex(real, imaginary, format);
    }
    let sign = if imaginary.is_sign_negative() {
        " - "
    } else {
        " + "
    };
    format!(
        "{}{sign}{}i",
        format_general(real, 5),
        format_general(imaginary.abs(), 5)
    )
}

fn format_table_nested_value(
    value: &Value,
    interpreter: &Interpreter,
    numeric_format: NumericFormat,
) -> String {
    match value {
        Value::Array(ArrayData::Char(array)) if array.numel() == 0 => format!(
            "{}x{} char",
            array.shape().extent(0),
            array.shape().extent(1)
        ),
        Value::Array(ArrayData::Char(array)) => format_char_matrix(array).map_or_else(
            || {
                format!(
                    "char [{}]",
                    array
                        .shape()
                        .dimensions()
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join("x")
                )
            },
            |text| format!("'{text}'"),
        ),
        Value::Table(table) => format!("{}x{} table", table.row_count(), table.variable_count()),
        _ => format_display_value_with(value, interpreter, numeric_format),
    }
}

fn format_array(array: &ArrayData, numeric_format: NumericFormat) -> String {
    if array.numel() == 0 {
        return "[]".to_owned();
    }
    if array.shape().ndims() == 2
        && array.numel() <= u64::try_from(DISPLAY_MATRIX_MAX_ELEMENTS).unwrap_or(u64::MAX)
    {
        if let ArrayData::Char(array) = array {
            if let Some(matrix) = format_char_matrix(array) {
                return matrix;
            }
        } else {
            let values = match array {
                ArrayData::F32(array) => array
                    .as_slice()
                    .iter()
                    .map(|value| format_numeric_real_f32(*value, numeric_format))
                    .collect::<Vec<_>>(),
                ArrayData::ComplexF32(array) => array
                    .as_slice()
                    .iter()
                    .map(|value| format_numeric_complex_f32(value.re, value.im, numeric_format))
                    .collect::<Vec<_>>(),

                ArrayData::F64(array) => array
                    .as_slice()
                    .iter()
                    .map(|value| format_numeric_real(*value, numeric_format))
                    .collect::<Vec<_>>(),
                ArrayData::ComplexF64(array) => array
                    .as_slice()
                    .iter()
                    .map(|value| format_numeric_complex(value.re, value.im, numeric_format))
                    .collect::<Vec<_>>(),
                ArrayData::Logical(array) => array
                    .as_slice()
                    .iter()
                    .map(|value| if value.get() { "1" } else { "0" }.to_owned())
                    .collect::<Vec<_>>(),
                ArrayData::Integer(integer) => {
                    integer.elements().map(format_integer_element).collect()
                }
                ArrayData::Char(_) => unreachable!("character array handled above"),
            };
            if let Some(matrix) = format_matrix_values(array.shape(), &values) {
                return matrix;
            }
        }
    }
    format_array_summary(array, numeric_format)
}

fn format_array_summary(array: &ArrayData, numeric_format: NumericFormat) -> String {
    let dimensions = array
        .shape()
        .dimensions()
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join("x");
    let mut values = match array {
        ArrayData::F32(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| format_numeric_real_f32(*value, numeric_format))
            .collect::<Vec<_>>(),
        ArrayData::ComplexF32(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| format_numeric_complex_f32(value.re, value.im, numeric_format))
            .collect::<Vec<_>>(),

        ArrayData::F64(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| format_numeric_real(*value, numeric_format))
            .collect::<Vec<_>>(),
        ArrayData::ComplexF64(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| format_numeric_complex(value.re, value.im, numeric_format))
            .collect::<Vec<_>>(),
        ArrayData::Logical(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| value.get().to_string())
            .collect::<Vec<_>>(),
        ArrayData::Char(array) => array
            .as_slice()
            .iter()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(|value| format!("0x{:04X}", value.get()))
            .collect::<Vec<_>>(),
        ArrayData::Integer(integer) => integer
            .elements()
            .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
            .map(format_integer_element)
            .collect::<Vec<_>>(),
    };
    if array.numel() > u64::try_from(DISPLAY_SUMMARY_MAX_ELEMENTS).unwrap_or(u64::MAX) {
        values.push("…".to_owned());
    }
    format!(
        "{} [{}] [{}]",
        array.class_name(),
        dimensions,
        values.join(", ")
    )
}

fn format_matrix_values(shape: &openmat_array::Shape, values: &[String]) -> Option<String> {
    let rows = usize::try_from(shape.extent(0)).unwrap_or(usize::MAX);
    let columns = usize::try_from(shape.extent(1)).unwrap_or(usize::MAX);
    if rows.checked_mul(columns) != Some(values.len()) {
        return None;
    }
    let minimum_width = if values.iter().all(|value| {
        let digits = value.strip_prefix('-').unwrap_or(value);
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        6
    } else {
        10
    };
    let mut column_widths = vec![minimum_width; columns];
    for (column, width) in column_widths.iter_mut().enumerate() {
        for row in 0..rows {
            let offset = column.checked_mul(rows)?.checked_add(row)?;
            let value = values.get(offset)?;
            *width = (*width).max(value.chars().count().checked_add(1)?);
        }
    }
    let row_bytes = column_widths
        .iter()
        .try_fold(0_usize, |total, width| total.checked_add(*width))?;
    let text_bytes = row_bytes
        .checked_mul(rows)?
        .checked_add(rows.saturating_sub(1))?;
    if text_bytes > DISPLAY_MAX_TEXT_BYTES {
        return None;
    }

    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut line = String::with_capacity(row_bytes);
        for column in 0..columns {
            let offset = column.checked_mul(rows)?.checked_add(row)?;
            let value = values.get(offset)?;
            let width = *column_widths.get(column)?;
            write!(&mut line, "{value:>width$}").ok()?;
        }
        lines.push(line);
    }
    Some(lines.join("\n"))
}

fn format_char_matrix(
    array: &openmat_array::DenseArray<openmat_array::CharCodeUnit>,
) -> Option<String> {
    let rows = usize::try_from(array.shape().extent(0)).unwrap_or(usize::MAX);
    let columns = usize::try_from(array.shape().extent(1)).unwrap_or(usize::MAX);
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut units = Vec::with_capacity(columns);
        for column in 0..columns {
            let offset = column
                .checked_mul(rows)
                .and_then(|offset| offset.checked_add(row))?;
            let value = array.as_slice().get(offset)?;
            units.push(value.get());
        }
        lines.push(String::from_utf16_lossy(&units));
    }
    let matrix = lines.join("\n");
    (matrix.len() <= DISPLAY_MAX_TEXT_BYTES).then_some(matrix)
}

fn format_display_cell(value: &str) -> String {
    format!("{value:>6}")
}

fn format_numeric_real(value: f64, format: NumericFormat) -> String {
    if !value.is_finite() {
        return format_real(value);
    }
    match format {
        NumericFormat::Short => format_short_real(value),
        NumericFormat::Long => format!("{value:.15}"),
        NumericFormat::ShortE => format!("{value:.4e}"),
        NumericFormat::LongE => format!("{value:.15e}"),
        NumericFormat::ShortG => format_general(value, 5),
        NumericFormat::LongG => format_general(value, 15),
        NumericFormat::ShortEng => format_engineering(value, 4),
        NumericFormat::LongEng => format_engineering(value, 15),
        NumericFormat::Bank => format!("{value:.2}"),
        NumericFormat::Hex => format!("{:016x}", value.to_bits()),
        NumericFormat::Rational => format_rational(value),
    }
}

fn format_numeric_complex(real: f64, imaginary: f64, format: NumericFormat) -> String {
    let sign = if imaginary.is_sign_negative() {
        " - "
    } else {
        " + "
    };
    format!(
        "{}{sign}{}i",
        format_numeric_real(real, format),
        format_numeric_real(imaginary.abs(), format)
    )
}

fn format_numeric_real_f32(value: f32, format: NumericFormat) -> String {
    if !value.is_finite() {
        return format_real_f32(value);
    }
    match format {
        NumericFormat::Short => format_short_real_f32(value),
        NumericFormat::Long => format!("{value:.7}"),
        NumericFormat::ShortE => format!("{value:.4e}"),
        NumericFormat::LongE => format!("{value:.7e}"),
        NumericFormat::ShortG => format_general(f64::from(value), 5),
        NumericFormat::LongG => format_general(f64::from(value), 7),
        NumericFormat::ShortEng => format_engineering(f64::from(value), 4),
        NumericFormat::LongEng => format_engineering(f64::from(value), 7),
        NumericFormat::Bank => format!("{value:.2}"),
        NumericFormat::Hex => format!("{:08x}", value.to_bits()),
        NumericFormat::Rational => format_rational(f64::from(value)),
    }
}

fn format_numeric_complex_f32(real: f32, imaginary: f32, format: NumericFormat) -> String {
    let sign = if imaginary.is_sign_negative() {
        " - "
    } else {
        " + "
    };
    format!(
        "{}{sign}{}i",
        format_numeric_real_f32(real, format),
        format_numeric_real_f32(imaginary.abs(), format)
    )
}

#[allow(clippy::cast_possible_truncation)]
fn format_general(value: f64, significant_digits: usize) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let exponent = value.abs().log10().floor() as i32;
    if exponent < -4 || exponent >= i32::try_from(significant_digits).unwrap_or(i32::MAX) {
        let fraction_digits = significant_digits.saturating_sub(1);
        return trim_scientific(format!("{value:.fraction_digits$e}"));
    }
    let decimals = i32::try_from(significant_digits)
        .unwrap_or(i32::MAX)
        .saturating_sub(exponent)
        .saturating_sub(1);
    trim_decimal(format!(
        "{value:.decimals$}",
        decimals = usize::try_from(decimals).unwrap_or(0)
    ))
}

#[allow(clippy::cast_possible_truncation)]
fn format_engineering(value: f64, fraction_digits: usize) -> String {
    if value == 0.0 {
        return format!("{value:.fraction_digits$}e+000");
    }
    let exponent = (value.abs().log10().floor() as i32).div_euclid(3) * 3;
    let scaled = value / 10_f64.powi(exponent);
    format!("{scaled:.fraction_digits$}e{exponent:+04}")
}

fn trim_decimal(mut value: String) -> String {
    if value.contains('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
    }
    value
}

fn trim_scientific(value: String) -> String {
    let Some((mantissa, exponent)) = value.split_once('e') else {
        return value;
    };
    format!("{}e{exponent}", trim_decimal(mantissa.to_owned()))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn format_rational(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let negative = value.is_sign_negative();
    let target = value.abs();
    let (mut previous_numerator, mut numerator) = (0_i64, 1_i64);
    let (mut previous_denominator, mut denominator) = (1_i64, 0_i64);
    let mut remainder = target;
    for _ in 0..32 {
        let whole = remainder.floor();
        if whole > i64::MAX as f64 {
            return format_real(value);
        }
        let whole = whole as i64;
        let Some(next_numerator) = whole
            .checked_mul(numerator)
            .and_then(|term| term.checked_add(previous_numerator))
        else {
            break;
        };
        let Some(next_denominator) = whole
            .checked_mul(denominator)
            .and_then(|term| term.checked_add(previous_denominator))
        else {
            break;
        };
        if next_denominator > 1_000_000 {
            break;
        }
        previous_numerator = numerator;
        numerator = next_numerator;
        previous_denominator = denominator;
        denominator = next_denominator;
        if ((numerator as f64 / denominator as f64) - target).abs() <= 1.0e-12 {
            break;
        }
        let fractional = remainder - whole as f64;
        if fractional == 0.0 {
            break;
        }
        remainder = fractional.recip();
    }
    if negative {
        numerator = -numerator;
    }
    if denominator == 1 {
        numerator.to_string()
    } else {
        format!("{numerator}/{denominator}")
    }
}

fn format_short_real(value: f64) -> String {
    if value.is_nan() || value.is_infinite() {
        return format_real(value);
    }
    if value == 0.0 || (value.fract() == 0.0 && value.abs() < 1.0e10) {
        return format!("{value:.0}");
    }
    let magnitude = value.abs();
    if (1.0e-4..1.0e5).contains(&magnitude) {
        format!("{value:.4}")
    } else {
        format!("{value:.4e}")
    }
}

fn format_short_real_f32(value: f32) -> String {
    if value.is_nan() || value.is_infinite() {
        return format_real_f32(value);
    }
    if value == 0.0 || (value.fract() == 0.0 && value.abs() < 1.0e10_f32) {
        return format!("{value:.0}");
    }
    let magnitude = value.abs();
    if (1.0e-4_f32..1.0e5_f32).contains(&magnitude) {
        format!("{value:.4}")
    } else {
        format!("{value:.4e}")
    }
}

fn format_string(value: &openmat_value::StringValue) -> String {
    if let Some(value) = value.as_scalar() {
        return format_string_element(value);
    }
    let dimensions = value
        .dimensions()
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join("x");
    let mut values = (0..value.numel())
        .take(DISPLAY_SUMMARY_MAX_ELEMENTS)
        .filter_map(|offset| usize::try_from(offset).ok())
        .filter_map(|offset| value.element(offset))
        .map(format_string_element)
        .collect::<Vec<_>>();
    if value.numel() > u64::try_from(DISPLAY_SUMMARY_MAX_ELEMENTS).unwrap_or(u64::MAX) {
        values.push("…".to_owned());
    }
    bounded_text(
        &format!("string [{dimensions}] [{}]", values.join(", ")),
        DISPLAY_MAX_TEXT_BYTES,
    )
}

fn format_string_element(value: &StringElement) -> String {
    if value.is_missing() {
        return "<missing>".to_owned();
    }
    bounded_text(&value.to_utf8_lossy(), DISPLAY_MAX_TEXT_BYTES)
}

fn format_integer_element(value: openmat_array::IntegerElementValue) -> String {
    let complex = value.is_complex();
    let decimal = value.canonical_decimal();
    let (real, imaginary) = decimal.into_components();
    if !complex {
        return real;
    }
    if let Some(magnitude) = imaginary.strip_prefix('-') {
        format!("{real} - {magnitude}i")
    } else {
        format!("{real} + {imaginary}i")
    }
}

fn format_real(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value == f64::INFINITY {
        "Inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        value.to_string()
    }
}

fn format_real_f32(value: f32) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value == f32::INFINITY {
        "Inf".to_owned()
    } else if value == f32::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        value.to_string()
    }
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut bounded = value[..end].to_owned();
    bounded.push('…');
    bounded
}
