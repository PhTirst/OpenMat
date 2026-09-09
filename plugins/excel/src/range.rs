use crate::{Error, Result};

pub const ROWS: u32 = 1_048_576;
pub const COLS: u32 = 16_384;

/// Zero-based, inclusive spreadsheet coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub start: (u32, u32),
    pub end: (u32, u32),
}

impl Rect {
    pub fn parse(text: &str) -> Result<Self> {
        let mut parts = text.split(':');
        let start = cell(parts.next().unwrap_or_default())?;
        let end = parts.next().map(cell).transpose()?.unwrap_or(start);
        if parts.next().is_some() || start.0 > end.0 || start.1 > end.1 {
            return Err(Error::new(
                "Range",
                "expected an ordered A1 range, e.g. B2:D8",
            ));
        }
        Ok(Self { start, end })
    }

    pub fn shape(self) -> (usize, usize) {
        (
            (self.end.0 - self.start.0 + 1) as usize,
            (self.end.1 - self.start.1 + 1) as usize,
        )
    }
}

pub fn cell(text: &str) -> Result<(u32, u32)> {
    let invalid = || Error::new("Range", "expected a cell between A1 and XFD1048576");
    let mut chars = text.trim().bytes().peekable();
    if chars.peek() == Some(&b'$') {
        chars.next();
    }
    let mut col = 0_u32;
    while let Some(c) = chars.next_if(u8::is_ascii_alphabetic) {
        col = col
            .checked_mul(26)
            .and_then(|v| v.checked_add(u32::from(c.to_ascii_uppercase() - b'A' + 1)))
            .filter(|&v| v <= COLS)
            .ok_or_else(invalid)?;
    }
    if chars.peek() == Some(&b'$') {
        chars.next();
    }
    let mut row = 0_u32;
    for c in chars {
        if !c.is_ascii_digit() {
            return Err(invalid());
        }
        row = row
            .checked_mul(10)
            .and_then(|v| v.checked_add(u32::from(c - b'0')))
            .filter(|&v| v <= ROWS)
            .ok_or_else(invalid)?;
    }
    if row == 0 || col == 0 {
        return Err(invalid());
    }
    Ok((row - 1, col - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_coordinates_and_bounds() {
        assert_eq!(cell(" $xfd$1048576 ").unwrap(), (ROWS - 1, COLS - 1));
        assert_eq!(Rect::parse("B3:AA5").unwrap().shape(), (3, 26));
        assert_eq!(Rect::parse("C4").unwrap().shape(), (1, 1));
        for bad in [
            "",
            "A0",
            "1",
            "A",
            "XFE1",
            "A1048577",
            "A-1",
            "A1B",
            "é1",
            "A99999999999999",
            "Sheet1!A1",
            "A1:B2:C3",
            "B2:A1",
        ] {
            assert!(Rect::parse(bad).is_err(), "accepted {bad}");
        }
    }
}
