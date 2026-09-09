use crate::GeometryError;

#[derive(Clone, Copy, Debug)]
pub enum NumericView<'a> {
    F32(&'a [f32]),
    F64(&'a [f64]),
}

impl NumericView<'_> {
    #[must_use]
    pub const fn len(self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<f64> {
        match self {
            Self::F32(values) => values.get(index).map(|&value| f64::from(value)),
            Self::F64(values) => values.get(index).copied(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SeriesGeometryInput<'a> {
    /// `None` means MATLAB-style implicit x coordinates `1..=numel(y)`.
    pub x: Option<NumericView<'a>>,
    pub y: NumericView<'a>,
}

impl SeriesGeometryInput<'_> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.y.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.y.is_empty()
    }

    /// Validates the independent contiguous x/y views.
    ///
    /// # Errors
    /// Returns [`GeometryError::LengthMismatch`] when explicit x and y lengths differ.
    pub fn validate(self) -> Result<(), GeometryError> {
        if let Some(x) = self.x
            && x.len() != self.y.len()
        {
            return Err(GeometryError::LengthMismatch {
                x: x.len(),
                y: self.y.len(),
            });
        }
        Ok(())
    }

    pub(crate) fn point(self, index: usize) -> Result<(f64, f64), GeometryError> {
        let y = self
            .y
            .get(index)
            .ok_or(GeometryError::ArithmeticOverflow("series index"))?;
        let x = if let Some(x) = self.x {
            x.get(index)
                .ok_or(GeometryError::ArithmeticOverflow("series index"))?
        } else {
            let one_based = index
                .checked_add(1)
                .ok_or(GeometryError::ArithmeticOverflow("implicit x coordinate"))?;
            let x: f64 = u32::try_from(one_based)
                .map_err(|_| GeometryError::ArithmeticOverflow("implicit x coordinate"))?
                .into();
            x
        };
        Ok((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_input_supports_f32_f64_and_implicit_x() {
        let input = SeriesGeometryInput {
            x: None,
            y: NumericView::F32(&[2.0, 3.0]),
        };
        assert_eq!(input.point(0).unwrap(), (1.0, 2.0));
        assert_eq!(input.point(1).unwrap(), (2.0, 3.0));

        let mismatch = SeriesGeometryInput {
            x: Some(NumericView::F64(&[1.0])),
            y: NumericView::F64(&[1.0, 2.0]),
        };
        assert_eq!(
            mismatch.validate(),
            Err(GeometryError::LengthMismatch { x: 1, y: 2 })
        );
    }
}
