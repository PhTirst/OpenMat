use crate::ArrayError;

/// A one-based, inclusive integer range used for language-level indexing.
///
/// The descriptor does not allocate an index vector. Direction mismatches (for
/// example `5:1:1`) describe an empty selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexRange {
    start: u64,
    step: i64,
    stop: u64,
}

impl IndexRange {
    /// Creates an inclusive index range.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::ZeroRangeStep`] when `step` is zero.
    pub const fn new(start: u64, increment: i64, stop: u64) -> Result<Self, ArrayError> {
        if increment == 0 {
            return Err(ArrayError::ZeroRangeStep);
        }
        Ok(Self {
            start,
            step: increment,
            stop,
        })
    }

    /// Returns the first value in the descriptor.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Returns the signed increment.
    #[must_use]
    pub const fn step(self) -> i64 {
        self.step
    }

    /// Returns the inclusive stop value.
    #[must_use]
    pub const fn stop(self) -> u64 {
        self.stop
    }

    /// Returns the number of produced indices without materializing them.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::RangeLengthOverflow`] if the length cannot be
    /// represented by `u64`.
    pub fn len(self) -> Result<u64, ArrayError> {
        let distance = if self.step > 0 {
            if self.start > self.stop {
                return Ok(0);
            }
            self.stop - self.start
        } else {
            if self.start < self.stop {
                return Ok(0);
            }
            self.start - self.stop
        };
        distance
            .checked_div(self.step.unsigned_abs())
            .and_then(|steps| steps.checked_add(1))
            .ok_or(ArrayError::RangeLengthOverflow)
    }

    /// Returns whether the range produces no indices.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::RangeLengthOverflow`] if its length overflows.
    pub fn is_empty(self) -> Result<bool, ArrayError> {
        self.len().map(|length| length == 0)
    }
}

/// A low-level descriptor for one dimension of an indexing expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexSelection {
    /// Every index in the dimension (`:`).
    Colon,
    /// One one-based subscript.
    Scalar(u64),
    /// An inclusive, signed-step range.
    Range(IndexRange),
}

impl IndexSelection {
    /// Resolves this descriptor against a dimension extent without allocating.
    ///
    /// Empty ranges are valid even if their descriptor endpoints lie outside
    /// the dimension, because they produce no actual indices.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::IndexOutOfBounds`] if any produced index is zero or
    /// exceeds `extent`.
    pub fn resolve(self, extent: u64) -> Result<ResolvedIndex, ArrayError> {
        match self {
            Self::Colon => Ok(ResolvedIndex {
                first: 1,
                step: 1,
                len: extent,
            }),
            Self::Scalar(index) => {
                validate_endpoint(index, extent)?;
                Ok(ResolvedIndex {
                    first: index,
                    step: 1,
                    len: 1,
                })
            }
            Self::Range(range) => {
                let len = range.len()?;
                if len == 0 {
                    return Ok(ResolvedIndex {
                        first: range.start,
                        step: range.step,
                        len,
                    });
                }
                let last = range_value(range.start, range.step, len - 1)
                    .ok_or(ArrayError::OffsetOverflow)?;
                validate_endpoint(range.start, extent)?;
                validate_endpoint(last, extent)?;
                Ok(ResolvedIndex {
                    first: range.start,
                    step: range.step,
                    len,
                })
            }
        }
    }
}

fn validate_endpoint(index: u64, extent: u64) -> Result<(), ArrayError> {
    if index == 0 || index > extent {
        return Err(ArrayError::IndexOutOfBounds {
            dimension: 0,
            index,
            extent,
        });
    }
    Ok(())
}

fn range_value(first: u64, step: i64, position: u64) -> Option<u64> {
    let value = i128::from(first) + i128::from(step) * i128::from(position);
    u64::try_from(value).ok()
}

/// An allocation-free index sequence after validation against an extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedIndex {
    first: u64,
    step: i64,
    len: u64,
}

impl ResolvedIndex {
    /// Returns the number of selected indices.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.len
    }

    /// Returns whether no indices are selected.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Returns the index at a zero-based position in the sequence.
    #[must_use]
    pub fn get(self, position: u64) -> Option<u64> {
        if position >= self.len {
            return None;
        }
        range_value(self.first, self.step, position)
    }

    /// Iterates the selected one-based indices.
    #[must_use]
    pub const fn iter(self) -> ResolvedIndices {
        ResolvedIndices {
            resolved: self,
            position: 0,
        }
    }
}

/// Iterator over a [`ResolvedIndex`].
#[derive(Clone, Debug)]
pub struct ResolvedIndices {
    resolved: ResolvedIndex,
    position: u64,
}

impl Iterator for ResolvedIndices {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.resolved.get(self.position)?;
        self.position += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.resolved.len - self.position;
        match usize::try_from(remaining) {
            Ok(exact) => (exact, Some(exact)),
            Err(_) => (usize::MAX, None),
        }
    }
}

impl std::iter::FusedIterator for ResolvedIndices {}
