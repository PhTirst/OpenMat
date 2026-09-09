//! Independent Excel OEX plugin. See the README for the m-language contract.
mod range;
mod workbook;

use oex::{Call, Char16, DataType, Logical, ValueRef};
use std::fmt;
use std::path::Path;
use workbook::{Sheet, WriteOptions};

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
struct Error {
    code: &'static str,
    message: String,
}
impl Error {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<Error> for oex::Error {
    fn from(error: Error) -> Self {
        if error.code == "Cancelled" {
            return Self::with_kind(oex::ErrorKind::Cancelled, error.message);
        }
        Self::new(format!("OpenMat:Excel:{}", error.code), error.message)
    }
}

fn text(value: ValueRef<'_, '_>, label: &str) -> oex::Result<String> {
    if !value.is_dense::<Char16>()? {
        return Err(Error::new(
            "Argument",
            format!("{label} must be a character vector (use single quotes)"),
        )
        .into());
    }
    let array = value.array::<Char16>()?;
    if array.dimensions().iter().filter(|&&n| n > 1).count() > 1 {
        return Err(Error::new("Argument", format!("{label} must be a character vector")).into());
    }
    let units: Vec<_> = array.as_slice().iter().map(|c| c.0).collect();
    let text = String::from_utf16(&units)
        .map_err(|_| Error::new("Argument", format!("{label} contains invalid UTF-16")))?;
    if text.contains('\0') {
        return Err(Error::new("Argument", format!("{label} contains a NUL character")).into());
    }
    Ok(text)
}

fn sheet(call: &Call<'_>, slot: u32) -> oex::Result<Sheet> {
    if call.input_count() <= slot {
        return Ok(Sheet::Index(0));
    }
    let value = call.input(slot)?;
    if value.is_dense::<Char16>()? {
        let name = text(value, "sheet")?;
        return Ok(if name.is_empty() {
            Sheet::Index(0)
        } else {
            Sheet::Name(name)
        });
    }
    let n = value.as_f64()?;
    if !n.is_finite() || n < 1.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
        return Err(Error::new("Sheet", "sheet index must be a positive one-based integer").into());
    }
    Ok(Sheet::Index(n as usize - 1))
}

fn read(call: &mut Call<'_>) -> oex::Result<()> {
    let path = text(call.input(0)?, "filename")?;
    let sheet = sheet(call, 1)?;
    let rect = if call.input_count() > 2 {
        let a1 = text(call.input(2)?, "range")?;
        if a1.is_empty() {
            None
        } else {
            Some(range::Rect::parse(&a1)?)
        }
    } else {
        None
    };
    let cancellation = call.cancellation();
    let data = workbook::read(
        Path::new(&path),
        &sheet,
        rect,
        call.output_count() > 1,
        || cancellation.is_requested(),
    )?;
    let dims = [data.rows as u64, data.cols as u64];
    call.set_output(0, call.array(&dims, &data.values)?)?;
    if call.output_count() > 1 {
        call.set_output(1, call.array(&dims, &data.kinds)?)?;
    }
    if call.output_count() > 2 {
        call.set_output(
            2,
            call.array(
                &[1, 2],
                &[data.origin.0 as f64 + 1.0, data.origin.1 as f64 + 1.0],
            )?,
        )?;
    }
    Ok(())
}

fn write(call: &mut Call<'_>) -> oex::Result<()> {
    let path = text(call.input(0)?, "filename")?;
    let value = call.input(1)?;
    let name = if call.input_count() > 2 {
        text(call.input(2)?, "sheet name")?
    } else {
        "Sheet1".into()
    };
    let start = if call.input_count() > 3 {
        range::cell(&text(call.input(3)?, "start cell")?)?
    } else {
        (0, 0)
    };
    let overwrite = if call.input_count() > 4 {
        let v = call.input(4)?;
        if v.is_scalar::<Logical>()? {
            v.array::<Logical>()?.as_slice()[0].get()
        } else {
            let n = v.as_f64()?;
            if n != 0.0 && n != 1.0 {
                return Err(
                    Error::new("Argument", "overwrite must be a logical scalar or 0/1").into(),
                );
            }
            n == 1.0
        }
    } else {
        false
    };
    let options = WriteOptions {
        sheet: &name,
        start,
        overwrite,
    };
    let cancellation = call.cancellation();
    macro_rules! numeric {
        ($ty:ty) => {{
            let array = value.array::<$ty>()?;
            workbook::write(
                Path::new(&path),
                array.dimensions(),
                array.as_slice(),
                &options,
                || cancellation.is_requested(),
            )?;
        }};
    }
    match value.data_type()? {
        DataType::F64 => numeric!(f64),
        DataType::F32 => numeric!(f32),
        DataType::I8 => numeric!(i8),
        DataType::U8 => numeric!(u8),
        DataType::I16 => numeric!(i16),
        DataType::U16 => numeric!(u16),
        DataType::I32 => numeric!(i32),
        DataType::U32 => numeric!(u32),
        DataType::I64 => numeric!(i64),
        DataType::U64 => numeric!(u64),
        DataType::Logical => numeric!(Logical),
        _ => {
            return Err(Error::new(
                "Argument",
                "data must be a real dense numeric or logical matrix",
            )
            .into());
        }
    }
    Ok(())
}

fn sheet_count(call: &mut Call<'_>) -> oex::Result<()> {
    call.check_cancelled()?;
    let path = text(call.input(0)?, "filename")?;
    let names = workbook::sheet_names(Path::new(&path))?;
    call.set_output(0, call.scalar(names.len() as f64)?)
}

fn sheet_name(call: &mut Call<'_>) -> oex::Result<()> {
    call.check_cancelled()?;
    let path = text(call.input(0)?, "filename")?;
    let names = workbook::sheet_names(Path::new(&path))?;
    let selector = sheet(call, 1)?;
    let name = selector.select(&names)?;
    let units: Vec<_> = name.encode_utf16().map(Char16).collect();
    call.set_output(0, call.array(&[1, units.len() as u64], &units)?)
}

oex::export_plugin! {
    name: "OpenMat Excel", version: "0.1.0",
    functions: [
        oex::function!("excel_read", 1..=3, 1..=3, read),
        oex::function!("excel_write", 2..=5, 0..=0, write),
        oex::function!("excel_sheet_count", 1..=1, 1..=1, sheet_count),
        oex::function!("excel_sheet_name", 2..=2, 1..=1, sheet_name),
    ],
    classes: [],
}
