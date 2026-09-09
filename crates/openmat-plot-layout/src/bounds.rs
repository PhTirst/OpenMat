use crate::LayoutError;

/// A continuous, one-dimensional numerical view owned by the layout caller.
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

/// Layout input independent from graphics-model and protocol DTOs.
#[derive(Clone, Copy, Debug)]
pub struct SeriesLayoutInput<'a> {
    /// `None` means MATLAB-style implicit x coordinates `1..=numel(y)`.
    pub x: Option<NumericView<'a>>,
    pub y: NumericView<'a>,
}

impl SeriesLayoutInput<'_> {
    /// Validates the independent contiguous x/y views.
    ///
    /// # Errors
    /// Returns [`LayoutError::LengthMismatch`] when explicit x and y lengths differ.
    pub fn validate(self) -> Result<(), LayoutError> {
        if let Some(x) = self.x
            && x.len() != self.y.len()
        {
            return Err(LayoutError::LengthMismatch {
                x: x.len(),
                y: self.y.len(),
            });
        }
        Ok(())
    }

    /// Computes x/y finite bounds and non-finite counts.
    ///
    /// # Errors
    /// Returns [`LayoutError`] for mismatched lengths or an implicit-index overflow.
    pub fn bounds(self) -> Result<(BoundsSummary, BoundsSummary), LayoutError> {
        self.validate()?;
        let y_bounds = BoundsSummary::from_view(self.y);
        let x_bounds = if let Some(x) = self.x {
            BoundsSummary::from_view(x)
        } else {
            BoundsSummary::from_implicit_indices(self.y.len())?
        };
        Ok((x_bounds, y_bounds))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundsSummary {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub finite_count: usize,
    pub nan_count: usize,
    pub positive_infinity_count: usize,
    pub negative_infinity_count: usize,
}

impl BoundsSummary {
    #[must_use]
    pub fn from_view(view: NumericView<'_>) -> Self {
        let mut bounds = Self::default();
        for index in 0..view.len() {
            if let Some(value) = view.get(index) {
                bounds.push(value);
            }
        }
        bounds
    }

    /// Constructs the bounds of MATLAB-style implicit x coordinates `1..=length`.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when `length` cannot be represented exactly.
    pub fn from_implicit_indices(length: usize) -> Result<Self, LayoutError> {
        if length == 0 {
            return Ok(Self::default());
        }
        let maximum = u32::try_from(length)
            .map_err(|_| LayoutError::ArithmeticOverflow("implicit x coordinate"))?
            .into();
        Ok(Self {
            minimum: Some(1.0),
            maximum: Some(maximum),
            finite_count: length,
            ..Self::default()
        })
    }

    pub fn push(&mut self, value: f64) {
        if value.is_nan() {
            self.nan_count = self.nan_count.saturating_add(1);
        } else if value == f64::INFINITY {
            self.positive_infinity_count = self.positive_infinity_count.saturating_add(1);
        } else if value == f64::NEG_INFINITY {
            self.negative_infinity_count = self.negative_infinity_count.saturating_add(1);
        } else {
            let value = canonical_zero(value);
            self.minimum = Some(self.minimum.map_or(value, |current| current.min(value)));
            self.maximum = Some(self.maximum.map_or(value, |current| current.max(value)));
            self.finite_count = self.finite_count.saturating_add(1);
        }
    }

    /// Merges another finite/non-finite summary without losing count overflow.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when any observation count exceeds [`usize`].
    pub fn checked_merge(&mut self, other: Self) -> Result<(), LayoutError> {
        self.minimum = match (self.minimum, other.minimum) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        };
        self.maximum = match (self.maximum, other.maximum) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        self.finite_count = checked_count(self.finite_count, other.finite_count)?;
        self.nan_count = checked_count(self.nan_count, other.nan_count)?;
        self.positive_infinity_count =
            checked_count(self.positive_infinity_count, other.positive_infinity_count)?;
        self.negative_infinity_count =
            checked_count(self.negative_infinity_count, other.negative_infinity_count)?;
        Ok(())
    }
}

fn checked_count(left: usize, right: usize) -> Result<usize, LayoutError> {
    left.checked_add(right)
        .ok_or(LayoutError::ArithmeticOverflow("bounds observation count"))
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_ignore_non_finite_values_but_report_them() {
        let summary = BoundsSummary::from_view(NumericView::F64(&[
            -0.0,
            3.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ]));
        assert_eq!(summary.minimum, Some(0.0));
        assert_eq!(summary.maximum, Some(3.0));
        assert_eq!(summary.finite_count, 2);
        assert_eq!(summary.nan_count, 1);
        assert_eq!(summary.positive_infinity_count, 1);
        assert_eq!(summary.negative_infinity_count, 1);
        assert!(!summary.minimum.unwrap().is_sign_negative());
    }

    #[test]
    fn independent_series_input_supports_implicit_x_and_f32() {
        let input = SeriesLayoutInput {
            x: None,
            y: NumericView::F32(&[4.0, 5.0, 6.0]),
        };
        let (x, y) = input.bounds().unwrap();
        assert_eq!((x.minimum, x.maximum), (Some(1.0), Some(3.0)));
        assert_eq!((y.minimum, y.maximum), (Some(4.0), Some(6.0)));
    }
}
