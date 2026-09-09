use openmat_array::Complex64;

pub(crate) trait ReferenceScalar:
    Copy + Default + Send + Sync + std::fmt::Debug + 'static
{
    fn zero() -> Self;
    fn add(self, right: Self) -> Self;
    fn subtract(self, right: Self) -> Self;
    fn multiply(self, right: Self) -> Self;
    fn divide(self, right: Self) -> Self;
    fn conjugate(self) -> Self;
    fn norm_squared(self) -> f64;
    fn real(self) -> f64;
    fn imaginary(self) -> f64;
    fn from_real(value: f64) -> Self;

    fn is_zero(self) -> bool {
        self.norm_squared() == 0.0
    }
}

impl ReferenceScalar for f64 {
    fn zero() -> Self {
        0.0
    }
    fn add(self, right: Self) -> Self {
        self + right
    }
    fn subtract(self, right: Self) -> Self {
        self - right
    }
    fn multiply(self, right: Self) -> Self {
        self * right
    }
    fn divide(self, right: Self) -> Self {
        self / right
    }
    fn conjugate(self) -> Self {
        self
    }
    fn norm_squared(self) -> f64 {
        self * self
    }
    fn real(self) -> f64 {
        self
    }
    fn imaginary(self) -> f64 {
        0.0
    }
    fn from_real(value: f64) -> Self {
        value
    }
}

impl ReferenceScalar for Complex64 {
    fn zero() -> Self {
        Self::ZERO
    }
    fn add(self, right: Self) -> Self {
        self + right
    }
    fn subtract(self, right: Self) -> Self {
        Self::new(self.re - right.re, self.im - right.im)
    }
    fn multiply(self, right: Self) -> Self {
        self * right
    }
    fn divide(self, right: Self) -> Self {
        if right.re.abs() >= right.im.abs() {
            let ratio = right.im / right.re;
            let denominator = right.re + right.im * ratio;
            Self::new(
                (self.re + self.im * ratio) / denominator,
                (self.im - self.re * ratio) / denominator,
            )
        } else {
            let ratio = right.re / right.im;
            let denominator = right.re * ratio + right.im;
            Self::new(
                (self.re * ratio + self.im) / denominator,
                (self.im * ratio - self.re) / denominator,
            )
        }
    }
    fn conjugate(self) -> Self {
        self.conjugate()
    }
    fn norm_squared(self) -> f64 {
        self.re.mul_add(self.re, self.im * self.im)
    }
    fn real(self) -> f64 {
        self.re
    }
    fn imaginary(self) -> f64 {
        self.im
    }
    fn from_real(value: f64) -> Self {
        Self::new(value, 0.0)
    }
}
