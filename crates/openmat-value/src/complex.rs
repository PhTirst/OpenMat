use std::ops::{Add, Div, Mul, Sub};

/// A scalar double-precision complex number.
///
/// This scalar-optimized representation deliberately remains separate from
/// [`openmat_array::Complex64`], which is the element type stored in dense
/// arrays. Both nevertheless use the same interleaved C field layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex64 {
    /// Real component.
    pub real: f64,
    /// Imaginary component.
    pub imaginary: f64,
}

impl Complex64 {
    /// Creates a complex scalar.
    #[must_use]
    pub const fn new(real: f64, imaginary: f64) -> Self {
        Self { real, imaginary }
    }

    /// Returns the magnitude of the scalar.
    #[must_use]
    pub fn abs(self) -> f64 {
        self.real.hypot(self.imaginary)
    }

    /// Returns whether both components are zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.real == 0.0 && self.imaginary == 0.0
    }
}

impl From<f64> for Complex64 {
    fn from(value: f64) -> Self {
        Self::new(value, 0.0)
    }
}

impl From<openmat_array::Complex64> for Complex64 {
    fn from(value: openmat_array::Complex64) -> Self {
        Self::new(value.re, value.im)
    }
}

impl From<Complex64> for openmat_array::Complex64 {
    fn from(value: Complex64) -> Self {
        Self::new(value.real, value.imaginary)
    }
}

impl Add for Complex64 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.real + rhs.real, self.imaginary + rhs.imaginary)
    }
}

impl Sub for Complex64 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.real - rhs.real, self.imaginary - rhs.imaginary)
    }
}

impl Mul for Complex64 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self::new(
            self.real
                .mul_add(rhs.real, -(self.imaginary * rhs.imaginary)),
            self.real.mul_add(rhs.imaginary, self.imaginary * rhs.real),
        )
    }
}

impl Div for Complex64 {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        let denominator = rhs.real.mul_add(rhs.real, rhs.imaginary * rhs.imaginary);
        Self::new(
            self.real.mul_add(rhs.real, self.imaginary * rhs.imaginary) / denominator,
            self.imaginary
                .mul_add(rhs.real, -(self.real * rhs.imaginary))
                / denominator,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::Complex64;

    #[test]
    fn scalar_layout_matches_oex_complex_f64() {
        assert_eq!(size_of::<Complex64>(), 16);
        assert_eq!(align_of::<Complex64>(), 8);
    }
}
