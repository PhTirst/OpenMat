use crate::range::{COLS, ROWS, Rect};
use crate::{Error, Result};
use calamine::{Data, Reader, open_workbook_auto};
use oex::Logical;
use rust_xlsxwriter::Workbook;
use std::path::Path;

const MAX_CELLS: usize = 10_000_000;

pub enum Sheet {
    Index(usize),
    Name(String),
}
impl Sheet {
    pub fn select<'a>(&self, names: &'a [String]) -> Result<&'a str> {
        match self {
            Self::Index(i) => names.get(*i),
            Self::Name(name) => names.iter().find(|s| *s == name),
        }
        .map(String::as_str)
        .ok_or_else(|| Error::new("Sheet", "worksheet does not exist"))
    }
}

fn check(cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(Error::new("Cancelled", "Excel operation cancelled"))
    } else {
        Ok(())
    }
}
fn io(error: impl std::fmt::Display) -> Error {
    Error::new("IO", error.to_string())
}
fn write_error(error: impl std::fmt::Display) -> Error {
    Error::new("Write", error.to_string())
}

pub fn sheet_names(path: &Path) -> Result<Vec<String>> {
    let workbook = open_workbook_auto(path).map_err(io)?;
    Ok(workbook.sheet_names().to_vec())
}

pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub origin: (u32, u32),
    pub values: Vec<f64>,
    pub kinds: Vec<u8>,
}

fn reserve<T>(n: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| Error::new("Allocation", "cannot allocate spreadsheet output"))?;
    Ok(v)
}

pub fn read(
    path: &Path,
    selector: &Sheet,
    requested: Option<Rect>,
    with_kinds: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Matrix> {
    check(&cancelled)?;
    let mut book = open_workbook_auto(path).map_err(io)?;
    let name = selector.select(&book.sheet_names())?.to_owned();
    check(&cancelled)?;
    let range = book.worksheet_range(&name).map_err(io)?;
    check(&cancelled)?;
    let used = range
        .start()
        .zip(range.end())
        .map(|(start, end)| Rect { start, end });
    let Some(rect) = requested.or(used) else {
        return Ok(Matrix {
            rows: 0,
            cols: 0,
            origin: (0, 0),
            values: vec![],
            kinds: vec![],
        });
    };
    let (rows, cols) = rect.shape();
    let count = rows
        .checked_mul(cols)
        .filter(|&n| n <= MAX_CELLS)
        .ok_or_else(|| {
            Error::new(
                "Size",
                "a read may return at most 10,000,000 cells; select a smaller range",
            )
        })?;
    let mut values = reserve(count)?;
    let mut kinds = reserve(if with_kinds { count } else { 0 })?;
    // OEX arrays are column-major; Calamine positions are absolute (row, col).
    for col in rect.start.1..=rect.end.1 {
        for row in rect.start.0..=rect.end.0 {
            if values.len() % 4096 == 0 {
                check(&cancelled)?;
            }
            let (value, kind) = match range.get_value((row, col)) {
                None | Some(Data::Empty) => (f64::NAN, 0),
                Some(Data::Int(n)) => (*n as f64, 1),
                Some(Data::Float(n)) => (*n, 1),
                Some(Data::Bool(b)) => (f64::from(*b), 2),
                Some(Data::String(_)) => (f64::NAN, 3),
                Some(Data::DateTime(_) | Data::DateTimeIso(_) | Data::DurationIso(_)) => {
                    (f64::NAN, 4)
                }
                Some(Data::Error(_)) => (f64::NAN, 5),
            };
            values.push(value);
            if with_kinds {
                kinds.push(kind);
            }
        }
    }
    check(&cancelled)?;
    Ok(Matrix {
        rows,
        cols,
        origin: rect.start,
        values,
        kinds,
    })
}

pub struct WriteOptions<'a> {
    pub sheet: &'a str,
    pub start: (u32, u32),
    pub overwrite: bool,
}

pub enum CellValue {
    Number(f64),
    Bool(bool),
}
pub trait ExcelNumber: Copy {
    fn cell(self) -> Result<CellValue>;
}
macro_rules! numbers {
    ($($ty:ty),*) => {$(
        impl ExcelNumber for $ty {
            fn cell(self) -> Result<CellValue> { Ok(CellValue::Number(f64::from(self))) }
        }
    )*};
}
numbers!(f64, f32, i8, u8, i16, u16, i32, u32);
impl ExcelNumber for i64 {
    fn cell(self) -> Result<CellValue> {
        if self.unsigned_abs() > (1_u64 << 53) {
            return Err(Error::new(
                "Precision",
                "int64 values outside +/-2^53 are not supported as Excel numbers",
            ));
        }
        Ok(CellValue::Number(self as f64))
    }
}
impl ExcelNumber for u64 {
    fn cell(self) -> Result<CellValue> {
        if self > (1_u64 << 53) {
            return Err(Error::new(
                "Precision",
                "uint64 values above 2^53 are not supported as Excel numbers",
            ));
        }
        Ok(CellValue::Number(self as f64))
    }
}
impl ExcelNumber for Logical {
    fn cell(self) -> Result<CellValue> {
        Ok(CellValue::Bool(self.get()))
    }
}

pub fn write<T: ExcelNumber>(
    path: &Path,
    dimensions: &[u64],
    values: &[T],
    options: &WriteOptions<'_>,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    check(&cancelled)?;
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        return Err(Error::new("Format", "writing supports .xlsx files only"));
    }
    let rows = dimensions.first().copied().unwrap_or(0);
    let cols = dimensions.get(1).copied().unwrap_or(1);
    if dimensions.iter().skip(2).any(|&n| n != 1) || rows == 0 || cols == 0 {
        return Err(Error::new(
            "Size",
            "write expects a nonempty two-dimensional matrix",
        ));
    }
    if rows > u64::from(ROWS.saturating_sub(options.start.0))
        || cols > u64::from(COLS.saturating_sub(options.start.1))
        || rows.checked_mul(cols) != Some(values.len() as u64)
        || values.len() > MAX_CELLS
    {
        return Err(Error::new(
            "Size",
            "matrix exceeds the worksheet bounds or the 10,000,000-cell limit",
        ));
    }
    if !options.overwrite && path.try_exists().map_err(io)? {
        return Err(Error::new(
            "Exists",
            "destination exists; pass overwrite=true to replace the entire workbook",
        ));
    }
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(options.sheet).map_err(write_error)?;
    for (i, &value) in values.iter().enumerate() {
        if i % 4096 == 0 {
            check(&cancelled)?;
        }
        let row = options.start.0 + (i as u64 % rows) as u32;
        let col = (options.start.1 + (i as u64 / rows) as u32) as u16;
        match value.cell()? {
            CellValue::Bool(b) => {
                worksheet.write_boolean(row, col, b).map_err(write_error)?;
            }
            CellValue::Number(n) if n.is_nan() => {} // Missing data becomes an empty cell.
            CellValue::Number(n) if n.is_infinite() => {
                return Err(Error::new(
                    "NonFinite",
                    "Excel cannot store infinity; use NaN for blank cells",
                ));
            }
            CellValue::Number(n) => {
                worksheet.write_number(row, col, n).map_err(write_error)?;
            }
        }
    }
    check(&cancelled)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Stage in the destination directory: failed writes leave the original intact.
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    workbook
        .save_to_writer(temporary.as_file_mut())
        .map_err(write_error)?;
    temporary.as_file().sync_all().map_err(io)?;
    check(&cancelled)?;
    if options.overwrite {
        temporary.persist(path).map_err(io)?;
    } else {
        temporary.persist_noclobber(path).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new("Exists", "destination already exists")
            } else {
                io(e)
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
