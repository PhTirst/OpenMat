use crate::MirError;

/// A finite point in the axes-local unit cube used by Plot MIR 3D primitives.
///
/// High-magnitude source values are normalized in f64 before they enter this
/// representation. The renderer therefore receives compact coordinates while
/// the language-facing layer remains responsible for exact axis limits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxesPoint3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AxesPoint3D {
    /// Creates a finite axes-local 3D point.
    ///
    /// # Errors
    /// Returns [`MirError`] when any coordinate is non-finite.
    pub fn new(x: f32, y: f32, z: f32) -> Result<Self, MirError> {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(Self { x, y, z })
        } else {
            Err(MirError::NonFinite("axes-local 3D point"))
        }
    }

    /// Validates a point assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] when any coordinate is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.x, self.y, self.z).map(|_| ())
    }
}

/// A finite direction in axes-local coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxesVector3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AxesVector3D {
    /// Creates a finite axes-local vector.
    ///
    /// # Errors
    /// Returns [`MirError`] when any component is non-finite.
    pub fn new(x: f32, y: f32, z: f32) -> Result<Self, MirError> {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(Self { x, y, z })
        } else {
            Err(MirError::NonFinite("axes-local 3D vector"))
        }
    }

    /// Validates a vector assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] when any component is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.x, self.y, self.z).map(|_| ())
    }
}

/// A column-major transform from axes-local coordinates to WebGPU clip space.
///
/// WebGPU uses X/Y in `-1..=1` and Z in `0..=1` after perspective division.
/// The geometry crate constructs matrices with that depth convention; MIR only
/// validates and transports the backend-neutral matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewProjection3D {
    columns: [[f32; 4]; 4],
}

impl ViewProjection3D {
    /// Creates a finite column-major view-projection matrix.
    ///
    /// # Errors
    /// Returns [`MirError`] when any matrix element is non-finite.
    pub fn new(columns: [[f32; 4]; 4]) -> Result<Self, MirError> {
        if columns.iter().flatten().all(|value| value.is_finite()) {
            Ok(Self { columns })
        } else {
            Err(MirError::NonFinite("3D view-projection matrix"))
        }
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self {
            columns: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    #[must_use]
    pub const fn columns(self) -> [[f32; 4]; 4] {
        self.columns
    }

    /// Validates a matrix assembled through deserialization or a public copy.
    ///
    /// # Errors
    /// Returns [`MirError`] when any matrix element is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.columns).map(|_| ())
    }
}

impl Default for ViewProjection3D {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_dimensional_coordinates_and_matrix_reject_nonfinite_values() {
        assert!(AxesPoint3D::new(0.0, 0.5, 1.0).is_ok());
        assert_eq!(
            AxesPoint3D::new(f32::NAN, 0.0, 0.0),
            Err(MirError::NonFinite("axes-local 3D point"))
        );
        let mut columns = ViewProjection3D::identity().columns();
        columns[3][2] = f32::INFINITY;
        assert_eq!(
            ViewProjection3D::new(columns),
            Err(MirError::NonFinite("3D view-projection matrix"))
        );
    }
}
