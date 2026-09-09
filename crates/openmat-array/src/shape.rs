use crate::ArrayError;

/// A canonical MATLAB-compatible dense-array shape.
///
/// Shapes always retain at least two dimensions. Trailing singleton dimensions
/// beyond the second dimension are removed, while internal singleton dimensions
/// and zero dimensions are preserved. Strides and element counts use checked
/// `u64` arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shape {
    dimensions: Vec<u64>,
    strides: Vec<u64>,
    numel: u64,
}

impl Shape {
    /// Constructs and canonicalizes a shape.
    ///
    /// An empty dimension list becomes `1x1`; a single dimension `n` becomes
    /// `nx1`. Trailing singleton dimensions above rank two are discarded.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::SizeOverflow`] if a column-major stride or the
    /// element count cannot be represented by `u64`.
    pub fn new(dimensions: impl IntoIterator<Item = u64>) -> Result<Self, ArrayError> {
        let mut dimensions: Vec<u64> = dimensions.into_iter().collect();
        match dimensions.len() {
            0 => dimensions.extend([1, 1]),
            1 => dimensions.push(1),
            _ => {}
        }
        while dimensions.len() > 2 && dimensions.last() == Some(&1) {
            dimensions.pop();
        }

        let mut strides = Vec::with_capacity(dimensions.len());
        let mut cumulative = 1_u64;
        for (dimension, &extent) in dimensions.iter().enumerate() {
            strides.push(cumulative);
            cumulative = cumulative
                .checked_mul(extent)
                .ok_or(ArrayError::SizeOverflow {
                    dimension,
                    partial: cumulative,
                    extent,
                })?;
        }

        Ok(Self {
            dimensions,
            strides,
            numel: cumulative,
        })
    }

    /// Returns the canonical dimension list.
    #[must_use]
    pub fn dimensions(&self) -> &[u64] {
        &self.dimensions
    }

    /// Returns the canonical rank, which is always at least two.
    #[must_use]
    pub fn ndims(&self) -> usize {
        self.dimensions.len()
    }

    /// Returns the total number of elements.
    #[must_use]
    pub const fn numel(&self) -> u64 {
        self.numel
    }

    /// Returns whether at least one dimension is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.numel == 0
    }

    /// Returns the extent at a zero-based internal dimension.
    ///
    /// Dimensions beyond the canonical rank are trailing singletons.
    #[must_use]
    pub fn extent(&self, dimension: usize) -> u64 {
        self.dimensions.get(dimension).copied().unwrap_or(1)
    }

    /// Returns the checked column-major stride for a stored dimension.
    ///
    /// Dimensions beyond the canonical rank have a conceptual stride of
    /// `numel`; because their only valid index is one, that stride never enters
    /// a valid offset calculation.
    #[must_use]
    pub fn stride(&self, dimension: usize) -> u64 {
        self.strides.get(dimension).copied().unwrap_or(self.numel)
    }

    /// Returns dimensions as observed by an indexing expression with
    /// `subscript_count` subscripts.
    ///
    /// One subscript addresses the linearized array. If fewer subscripts than
    /// stored dimensions are supplied, the final supplied dimension collapses
    /// all remaining dimensions. Additional subscripts address trailing
    /// singleton dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::NoSubscripts`] when `subscript_count` is zero, or
    /// [`ArrayError::SizeOverflow`] if a collapsed extent overflows.
    pub fn effective_dimensions(&self, subscript_count: usize) -> Result<Vec<u64>, ArrayError> {
        if subscript_count == 0 {
            return Err(ArrayError::NoSubscripts);
        }
        if subscript_count == 1 {
            return Ok(vec![self.numel]);
        }

        let mut effective = Vec::with_capacity(subscript_count);
        if subscript_count < self.ndims() {
            effective.extend_from_slice(&self.dimensions[..subscript_count - 1]);
            let mut collapsed = 1_u64;
            for (dimension, &extent) in self.dimensions[subscript_count - 1..].iter().enumerate() {
                collapsed = collapsed
                    .checked_mul(extent)
                    .ok_or(ArrayError::SizeOverflow {
                        dimension: dimension + subscript_count - 1,
                        partial: collapsed,
                        extent,
                    })?;
            }
            effective.push(collapsed);
        } else {
            effective.extend_from_slice(&self.dimensions);
            effective.resize(subscript_count, 1);
        }
        Ok(effective)
    }

    /// Converts a one-based linear index to a zero-based internal offset.
    ///
    /// # Errors
    ///
    /// Returns [`ArrayError::LinearIndexOutOfBounds`] for zero, an index beyond
    /// `numel`, or any index into an empty array.
    pub fn linear_offset(&self, one_based_index: u64) -> Result<u64, ArrayError> {
        if one_based_index == 0 || one_based_index > self.numel {
            return Err(ArrayError::LinearIndexOutOfBounds {
                index: one_based_index,
                numel: self.numel,
            });
        }
        Ok(one_based_index - 1)
    }

    /// Converts one-based language subscripts to a zero-based column-major
    /// offset.
    ///
    /// The number of supplied subscripts follows the collapsing and trailing
    /// singleton rules described by [`Self::effective_dimensions`].
    ///
    /// # Errors
    ///
    /// Returns an indexing error for missing, zero, or out-of-range subscripts,
    /// and [`ArrayError::OffsetOverflow`] if checked offset arithmetic fails.
    pub fn offset_for_subscripts(&self, subscripts: &[u64]) -> Result<u64, ArrayError> {
        if subscripts.len() == 1 {
            return self.linear_offset(subscripts[0]);
        }
        let dimensions = self.effective_dimensions(subscripts.len())?;
        let mut stride = 1_u64;
        let mut offset = 0_u64;
        for (dimension, (&index, &extent)) in subscripts.iter().zip(dimensions.iter()).enumerate() {
            if index == 0 || index > extent {
                return Err(ArrayError::IndexOutOfBounds {
                    dimension,
                    index,
                    extent,
                });
            }
            let contribution = (index - 1)
                .checked_mul(stride)
                .ok_or(ArrayError::OffsetOverflow)?;
            offset = offset
                .checked_add(contribution)
                .ok_or(ArrayError::OffsetOverflow)?;
            stride = stride
                .checked_mul(extent)
                .ok_or(ArrayError::OffsetOverflow)?;
        }
        Ok(offset)
    }

    /// Converts one-based language subscripts to a one-based linear index.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::offset_for_subscripts`].
    pub fn linear_index_for_subscripts(&self, subscripts: &[u64]) -> Result<u64, ArrayError> {
        self.offset_for_subscripts(subscripts)?
            .checked_add(1)
            .ok_or(ArrayError::OffsetOverflow)
    }

    /// Converts a one-based linear index to one-based subscripts.
    ///
    /// `subscript_count` controls collapse or trailing singleton expansion in
    /// the same way as [`Self::effective_dimensions`].
    ///
    /// # Errors
    ///
    /// Returns an indexing error if the linear index is invalid or no
    /// subscripts were requested.
    pub fn subscripts_for_linear_index(
        &self,
        one_based_index: u64,
        subscript_count: usize,
    ) -> Result<Vec<u64>, ArrayError> {
        let mut offset = self.linear_offset(one_based_index)?;
        let dimensions = self.effective_dimensions(subscript_count)?;
        let mut subscripts = Vec::with_capacity(dimensions.len());
        for extent in dimensions {
            debug_assert!(extent > 0, "valid linear indices imply nonempty extents");
            subscripts.push((offset % extent) + 1);
            offset /= extent;
        }
        Ok(subscripts)
    }
}
