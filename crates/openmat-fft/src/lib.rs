#![doc = "Provider-neutral fast Fourier transforms for `OpenMat`."]
#![forbid(unsafe_code)]

use std::{
    error::Error,
    fmt,
    sync::{Mutex, atomic::AtomicBool, atomic::Ordering},
};

use rustfft::{FftPlanner, num_complex::Complex};

/// One binary64 complex value crossing the replaceable FFT provider boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FftComplex64 {
    /// Real component.
    pub real: f64,
    /// Imaginary component.
    pub imaginary: f64,
}

impl FftComplex64 {
    /// Creates one provider-neutral complex value.
    #[must_use]
    pub const fn new(real: f64, imaginary: f64) -> Self {
        Self { real, imaginary }
    }
}

/// One binary32 complex value crossing the replaceable FFT provider boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FftComplex32 {
    /// Real component.
    pub real: f32,
    /// Imaginary component.
    pub imaginary: f32,
}

impl FftComplex32 {
    /// Creates one provider-neutral complex value.
    #[must_use]
    pub const fn new(real: f32, imaginary: f32) -> Self {
        Self { real, imaginary }
    }
}

/// Direction of an unnormalized provider transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FftDirection {
    /// Negative-sign forward transform.
    Forward,
    /// Positive-sign inverse transform, without the MATLAB `1/N` scale.
    Inverse,
}

/// Stable provider failure classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FftErrorKind {
    /// The transform length is zero or does not divide the batch buffer.
    InvalidShape,
    /// Cooperative cancellation was requested.
    Cancelled,
    /// The selected implementation failed without exposing backend details.
    ProviderFailure,
}

/// Provider-independent FFT failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FftError {
    /// Stable failure class.
    pub kind: FftErrorKind,
}

impl fmt::Display for FftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            FftErrorKind::InvalidShape => "FFT batch shape is invalid",
            FftErrorKind::Cancelled => "FFT execution was cancelled",
            FftErrorKind::ProviderFailure => "FFT provider execution failed",
        })
    }
}

impl Error for FftError {}

/// Replaceable backend contract for contiguous batches of equal-length transforms.
///
/// Each consecutive `length` values form one independent transform. Inverse
/// transforms are deliberately unnormalized so MATLAB-visible scaling remains
/// owned by the built-in layer.
pub trait FftProvider: Send + Sync {
    /// Transforms a binary64 complex batch in place.
    ///
    /// # Errors
    ///
    /// Returns a stable shape, cancellation, or provider failure.
    fn transform_f64(
        &self,
        values: &mut [FftComplex64],
        length: usize,
        direction: FftDirection,
        cancellation: &AtomicBool,
    ) -> Result<(), FftError>;

    /// Transforms a binary32 complex batch in place.
    ///
    /// # Errors
    ///
    /// Returns a stable shape, cancellation, or provider failure.
    fn transform_f32(
        &self,
        values: &mut [FftComplex32],
        length: usize,
        direction: FftDirection,
        cancellation: &AtomicBool,
    ) -> Result<(), FftError>;
}

/// Default memory-safe CPU backend using cached `rustfft` plans.
pub struct RustFftProvider {
    planner_f64: Mutex<FftPlanner<f64>>,
    planner_f32: Mutex<FftPlanner<f32>>,
}

impl Default for RustFftProvider {
    fn default() -> Self {
        Self {
            planner_f64: Mutex::new(FftPlanner::new()),
            planner_f32: Mutex::new(FftPlanner::new()),
        }
    }
}

impl FftProvider for RustFftProvider {
    fn transform_f64(
        &self,
        values: &mut [FftComplex64],
        length: usize,
        direction: FftDirection,
        cancellation: &AtomicBool,
    ) -> Result<(), FftError> {
        validate_batch(values.len(), length, cancellation)?;
        let plan = {
            let mut planner = self.planner_f64.lock().map_err(|_| provider_failure())?;
            match direction {
                FftDirection::Forward => planner.plan_fft_forward(length),
                FftDirection::Inverse => planner.plan_fft_inverse(length),
            }
        };
        let mut backend = values
            .iter()
            .map(|value| Complex::new(value.real, value.imaginary))
            .collect::<Vec<_>>();
        for transform in backend.chunks_mut(length) {
            if cancellation.load(Ordering::Relaxed) {
                return Err(FftError {
                    kind: FftErrorKind::Cancelled,
                });
            }
            plan.process(transform);
        }
        for (output, value) in values.iter_mut().zip(backend) {
            *output = FftComplex64::new(value.re, value.im);
        }
        Ok(())
    }

    fn transform_f32(
        &self,
        values: &mut [FftComplex32],
        length: usize,
        direction: FftDirection,
        cancellation: &AtomicBool,
    ) -> Result<(), FftError> {
        validate_batch(values.len(), length, cancellation)?;
        let plan = {
            let mut planner = self.planner_f32.lock().map_err(|_| provider_failure())?;
            match direction {
                FftDirection::Forward => planner.plan_fft_forward(length),
                FftDirection::Inverse => planner.plan_fft_inverse(length),
            }
        };
        let mut backend = values
            .iter()
            .map(|value| Complex::new(value.real, value.imaginary))
            .collect::<Vec<_>>();
        for transform in backend.chunks_mut(length) {
            if cancellation.load(Ordering::Relaxed) {
                return Err(FftError {
                    kind: FftErrorKind::Cancelled,
                });
            }
            plan.process(transform);
        }
        for (output, value) in values.iter_mut().zip(backend) {
            *output = FftComplex32::new(value.re, value.im);
        }
        Ok(())
    }
}

fn validate_batch(
    value_count: usize,
    length: usize,
    cancellation: &AtomicBool,
) -> Result<(), FftError> {
    if cancellation.load(Ordering::Relaxed) {
        return Err(FftError {
            kind: FftErrorKind::Cancelled,
        });
    }
    if length == 0 || !value_count.is_multiple_of(length) {
        return Err(FftError {
            kind: FftErrorKind::InvalidShape,
        });
    }
    Ok(())
}

const fn provider_failure() -> FftError {
    FftError {
        kind: FftErrorKind::ProviderFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_transforms_batches_and_leaves_inverse_unnormalized() {
        let provider = RustFftProvider::default();
        let cancellation = AtomicBool::new(false);
        let mut values = [
            FftComplex64::new(1.0, 0.0),
            FftComplex64::new(2.0, 0.0),
            FftComplex64::new(3.0, 0.0),
            FftComplex64::new(4.0, 0.0),
            FftComplex64::new(1.0, 0.0),
            FftComplex64::new(1.0, 0.0),
            FftComplex64::new(1.0, 0.0),
            FftComplex64::new(1.0, 0.0),
        ];
        provider
            .transform_f64(&mut values, 4, FftDirection::Forward, &cancellation)
            .unwrap();
        assert_eq!(values[0], FftComplex64::new(10.0, 0.0));
        assert_eq!(values[1], FftComplex64::new(-2.0, 2.0));
        assert_eq!(values[4], FftComplex64::new(4.0, 0.0));
        provider
            .transform_f64(&mut values, 4, FftDirection::Inverse, &cancellation)
            .unwrap();
        assert_eq!(values[0], FftComplex64::new(4.0, 0.0));
        assert_eq!(values[3], FftComplex64::new(16.0, 0.0));
    }

    #[test]
    fn provider_validates_shapes_and_cancellation_without_backend_types() {
        let provider = RustFftProvider::default();
        let mut values = [FftComplex32::default(); 3];
        let active = AtomicBool::new(false);
        assert_eq!(
            provider.transform_f32(&mut values, 2, FftDirection::Forward, &active),
            Err(FftError {
                kind: FftErrorKind::InvalidShape,
            })
        );
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            provider.transform_f32(&mut values, 3, FftDirection::Forward, &cancelled),
            Err(FftError {
                kind: FftErrorKind::Cancelled,
            })
        );
    }
}
