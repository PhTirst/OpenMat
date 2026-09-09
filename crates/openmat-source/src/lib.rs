#![doc = "Source identifiers, checked byte ranges, and compiler diagnostics."]

use core::fmt;

/// Stable identity for a source buffer within a compilation session.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u32);

impl SourceId {
    /// Creates an identifier from its session-local numeric representation.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the session-local numeric representation.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// A failure to represent or construct a source byte range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeError {
    /// The end offset precedes the start offset.
    Reversed { start: u32, end: u32 },
    /// A host-sized offset cannot be represented by the 32-bit source model.
    OffsetOverflow { offset: usize },
    /// Adding an offset would exceed the 32-bit source model.
    AdditionOverflow,
}

impl fmt::Display for RangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reversed { start, end } => {
                write!(formatter, "range end {end} precedes start {start}")
            }
            Self::OffsetOverflow { offset } => {
                write!(formatter, "source offset {offset} exceeds u32::MAX")
            }
            Self::AdditionOverflow => formatter.write_str("source range addition overflowed"),
        }
    }
}

impl std::error::Error for RangeError {}

/// A half-open UTF-8 byte range (`start..end`) in a source buffer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextRange {
    start: u32,
    end: u32,
}

impl TextRange {
    /// Constructs a checked range.
    ///
    /// # Errors
    ///
    /// Returns [`RangeError::Reversed`] when `end < start`.
    pub const fn new(start: u32, end: u32) -> Result<Self, RangeError> {
        if end < start {
            Err(RangeError::Reversed { start, end })
        } else {
            Ok(Self { start, end })
        }
    }

    /// Constructs a checked range from host-sized byte offsets.
    ///
    /// # Errors
    ///
    /// Returns an overflow error when either offset exceeds `u32::MAX`, or a
    /// reversed-range error when `end < start`.
    pub fn from_usize(start: usize, end: usize) -> Result<Self, RangeError> {
        let start =
            u32::try_from(start).map_err(|_| RangeError::OffsetOverflow { offset: start })?;
        let end = u32::try_from(end).map_err(|_| RangeError::OffsetOverflow { offset: end })?;
        Self::new(start, end)
    }

    /// Creates an empty range at `offset`.
    #[must_use]
    pub const fn empty(offset: u32) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Returns the start byte offset.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the exclusive end byte offset.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Returns the range length in bytes.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Returns whether this range contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns whether the byte offset lies within this half-open range.
    #[must_use]
    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns the smallest range covering both inputs.
    #[must_use]
    pub const fn cover(self, other: Self) -> Self {
        Self {
            start: if self.start < other.start {
                self.start
            } else {
                other.start
            },
            end: if self.end > other.end {
                self.end
            } else {
                other.end
            },
        }
    }

    /// Shifts both bounds by `offset` while checking overflow.
    ///
    /// # Errors
    ///
    /// Returns [`RangeError::AdditionOverflow`] if either bound exceeds
    /// `u32::MAX`.
    pub const fn checked_add(self, offset: u32) -> Result<Self, RangeError> {
        let Some(start) = self.start.checked_add(offset) else {
            return Err(RangeError::AdditionOverflow);
        };
        let Some(end) = self.end.checked_add(offset) else {
            return Err(RangeError::AdditionOverflow);
        };
        Ok(Self { start, end })
    }
}

/// Diagnostic severity independent of any particular frontend phase.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// A compiler diagnostic tied to an exact source and byte range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub source_id: SourceId,
    pub range: TextRange,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic without a machine-readable code.
    pub fn new(
        source_id: SourceId,
        range: TextRange,
        severity: Severity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source_id,
            range,
            severity,
            code: None,
            message: message.into(),
        }
    }

    /// Adds a stable `OpenMat` diagnostic code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Convenience constructor for an error diagnostic.
    pub fn error(source_id: SourceId, range: TextRange, message: impl Into<String>) -> Self {
        Self::new(source_id, range, Severity::Error, message)
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, RangeError, Severity, SourceId, TextRange};

    #[test]
    fn accepts_empty_and_maximal_ranges() {
        let empty = TextRange::new(0, 0).expect("empty ranges are valid");
        let maximal = TextRange::new(0, u32::MAX).expect("u32 boundary is valid");

        assert!(empty.is_empty());
        assert_eq!(maximal.len(), u32::MAX);
        assert!(maximal.contains(u32::MAX - 1));
        assert!(!maximal.contains(u32::MAX));
    }

    #[test]
    fn rejects_reversed_ranges() {
        assert_eq!(
            TextRange::new(9, 3),
            Err(RangeError::Reversed { start: 9, end: 3 })
        );
    }

    #[test]
    fn detects_host_offset_and_addition_overflow() {
        if usize::BITS > 32 {
            let too_large = (u32::MAX as usize) + 1;
            assert_eq!(
                TextRange::from_usize(0, too_large),
                Err(RangeError::OffsetOverflow { offset: too_large })
            );
        }

        let tail = TextRange::new(u32::MAX - 1, u32::MAX).expect("valid tail range");
        assert_eq!(tail.checked_add(1), Err(RangeError::AdditionOverflow));
    }

    #[test]
    fn diagnostics_retain_source_range_and_code() {
        let source_id = SourceId::new(7);
        let range = TextRange::new(2, 5).expect("valid range");
        let diagnostic = Diagnostic::new(source_id, range, Severity::Warning, "ambiguous syntax")
            .with_code("OM1001");

        assert_eq!(diagnostic.source_id, source_id);
        assert_eq!(diagnostic.range, range);
        assert_eq!(diagnostic.code.as_deref(), Some("OM1001"));
    }
}
