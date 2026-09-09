use crate::{BoundsSummary, LayoutError, SeriesLayoutInput};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    lower: f64,
    upper: f64,
}

impl Limits {
    /// Creates finite, strictly increasing axis limits while preserving signed zero.
    ///
    /// # Errors
    /// Returns [`LayoutError::InvalidLimits`] for non-finite or unordered limits.
    pub fn new(lower: f64, upper: f64) -> Result<Self, LayoutError> {
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return Err(LayoutError::InvalidLimits);
        }
        Ok(Self { lower, upper })
    }

    #[must_use]
    pub const fn lower(self) -> f64 {
        self.lower
    }

    #[must_use]
    pub const fn upper(self) -> f64 {
        self.upper
    }

    #[must_use]
    pub fn span(self) -> f64 {
        self.upper - self.lower
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticLimits(Limits);

impl SemanticLimits {
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self(limits)
    }

    #[must_use]
    pub const fn get(self) -> Limits {
        self.0
    }
}

/// A renderer-only navigation domain. It is never an authoritative MATLAB limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalViewLimits(Limits);

impl LocalViewLimits {
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self(limits)
    }

    #[must_use]
    pub const fn get(self) -> Limits {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitMode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticAxisRequest {
    pub mode: LimitMode,
    pub manual: Option<Limits>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoLimitOptions {
    pub constant_relative_half_span: f64,
    pub constant_minimum_half_span: f64,
    pub empty_default: Limits,
}

impl Default for AutoLimitOptions {
    fn default() -> Self {
        Self {
            constant_relative_half_span: 0.1,
            constant_minimum_half_span: 1.0,
            empty_default: Limits {
                lower: 0.0,
                upper: 1.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoLimitReason {
    Manual,
    EmptyDefault,
    ConstantExpanded,
    FiniteData,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSemanticAxis {
    pub limits: SemanticLimits,
    pub mode: LimitMode,
    pub reason: AutoLimitReason,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticAxesRequest {
    pub x: SemanticAxisRequest,
    pub y: SemanticAxisRequest,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedSemanticAxes {
    pub x: ResolvedSemanticAxis,
    pub y: ResolvedSemanticAxis,
}

/// Aggregates all series and resolves authoritative semantic X/Y limits.
///
/// This is the pure-Rust entry point intended for the Kernel graphics session.
///
/// # Errors
/// Returns [`LayoutError`] for invalid views, count overflow, or invalid limit requests.
pub fn resolve_semantic_axes(
    series: &[SeriesLayoutInput<'_>],
    request: SemanticAxesRequest,
    options: AutoLimitOptions,
) -> Result<ResolvedSemanticAxes, LayoutError> {
    let mut x_bounds = BoundsSummary::default();
    let mut y_bounds = BoundsSummary::default();
    for &input in series {
        let (input_x, input_y) = input.bounds()?;
        x_bounds.checked_merge(input_x)?;
        y_bounds.checked_merge(input_y)?;
    }
    Ok(ResolvedSemanticAxes {
        x: resolve_semantic_axis(request.x, x_bounds, options)?,
        y: resolve_semantic_axis(request.y, y_bounds, options)?,
    })
}

/// Resolves the authoritative semantic limits stored by the Kernel graphics session.
///
/// # Errors
/// Returns [`LayoutError`] for an invalid mode/options combination or unrepresentable span.
#[allow(clippy::float_cmp)]
pub fn resolve_semantic_axis(
    request: SemanticAxisRequest,
    bounds: BoundsSummary,
    options: AutoLimitOptions,
) -> Result<ResolvedSemanticAxis, LayoutError> {
    validate_options(options)?;
    if request.mode == LimitMode::Manual {
        let limits = request.manual.ok_or(LayoutError::InvalidLimits)?;
        return Ok(ResolvedSemanticAxis {
            limits: SemanticLimits::new(limits),
            mode: LimitMode::Manual,
            reason: AutoLimitReason::Manual,
        });
    }
    if request.manual.is_some() {
        return Err(LayoutError::InvalidLimits);
    }
    let (Some(minimum), Some(maximum)) = (bounds.minimum, bounds.maximum) else {
        return Ok(ResolvedSemanticAxis {
            limits: SemanticLimits::new(options.empty_default),
            mode: LimitMode::Auto,
            reason: AutoLimitReason::EmptyDefault,
        });
    };
    let (limits, reason) = if minimum == maximum {
        (
            expand_constant(minimum, options)?,
            AutoLimitReason::ConstantExpanded,
        )
    } else {
        (data_limits(minimum, maximum)?, AutoLimitReason::FiniteData)
    };
    Ok(ResolvedSemanticAxis {
        limits: SemanticLimits::new(limits),
        mode: LimitMode::Auto,
        reason,
    })
}

fn validate_options(options: AutoLimitOptions) -> Result<(), LayoutError> {
    if !options.constant_relative_half_span.is_finite()
        || options.constant_relative_half_span < 0.0
        || !options.constant_minimum_half_span.is_finite()
        || options.constant_minimum_half_span <= 0.0
    {
        return Err(LayoutError::NonFiniteInput("constant auto-limit expansion"));
    }
    Ok(())
}

#[allow(clippy::float_cmp)]
fn expand_constant(value: f64, options: AutoLimitOptions) -> Result<Limits, LayoutError> {
    let magnitude = value.abs();
    let mut delta =
        (magnitude * options.constant_relative_half_span).max(options.constant_minimum_half_span);
    if !delta.is_finite() {
        delta = magnitude / 2.0;
    }
    let mut lower = value - delta;
    let mut upper = value + delta;
    if !lower.is_finite() {
        lower = -f64::MAX;
    }
    if !upper.is_finite() {
        upper = f64::MAX;
    }
    if lower == value {
        lower = value.next_down();
    }
    if upper == value {
        upper = value.next_up();
    }
    if lower == 0.0 && value > 0.0 {
        lower = -0.0;
    }
    Limits::new(lower, upper)
}

fn data_limits(minimum: f64, maximum: f64) -> Result<Limits, LayoutError> {
    let lower = if minimum == 0.0 { -0.0 } else { minimum };
    Limits::new(lower, maximum)
}

pub(crate) fn nice_step(raw_step: f64) -> Option<f64> {
    if !raw_step.is_finite() || raw_step <= 0.0 {
        return None;
    }
    let exponent = raw_step.log10().floor();
    let power = 10.0_f64.powf(exponent);
    if !power.is_finite() || power == 0.0 {
        return Some(raw_step);
    }
    let fraction = raw_step / power;
    let nice_fraction = if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 2.5 {
        2.5
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice_fraction * power;
    step.is_finite().then_some(step)
}

pub(crate) fn nice_step_r2022b(raw_step: f64) -> Option<f64> {
    if !raw_step.is_finite() || raw_step <= 0.0 {
        return None;
    }
    let exponent = raw_step.log10().floor();
    let power = 10.0_f64.powf(exponent);
    if !power.is_finite() || power == 0.0 {
        return Some(raw_step);
    }
    let fraction = raw_step / power;
    let nice_fraction = if fraction <= 1.25 {
        1.0
    } else if fraction <= 2.5 {
        2.0
    } else if fraction <= 7.5 {
        5.0
    } else {
        10.0
    };
    let step = nice_fraction * power;
    step.is_finite().then_some(step)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn bounds(minimum: f64, maximum: f64) -> BoundsSummary {
        BoundsSummary {
            minimum: Some(minimum),
            maximum: Some(maximum),
            finite_count: 2,
            ..BoundsSummary::default()
        }
    }

    #[test]
    fn empty_constant_and_signed_zero_have_explicit_results() {
        let request = SemanticAxisRequest {
            mode: LimitMode::Auto,
            manual: None,
        };
        let empty = resolve_semantic_axis(
            request,
            BoundsSummary::default(),
            AutoLimitOptions::default(),
        )
        .unwrap();
        assert_eq!(empty.reason, AutoLimitReason::EmptyDefault);
        assert_eq!(
            (empty.limits.get().lower(), empty.limits.get().upper()),
            (0.0, 1.0)
        );

        let constant =
            resolve_semantic_axis(request, bounds(-0.0, 0.0), AutoLimitOptions::default()).unwrap();
        assert_eq!(constant.reason, AutoLimitReason::ConstantExpanded);
        assert_eq!(
            (constant.limits.get().lower(), constant.limits.get().upper()),
            (-1.0, 1.0)
        );
    }

    #[test]
    fn large_offset_small_span_remains_visible() {
        let resolved = resolve_semantic_axis(
            SemanticAxisRequest {
                mode: LimitMode::Auto,
                manual: None,
            },
            bounds(1.0e12, 1.0e12 + 3.0),
            AutoLimitOptions::default(),
        )
        .unwrap();
        assert!(resolved.limits.get().upper() - resolved.limits.get().lower() >= 3.0);
        assert!(resolved.limits.get().lower() <= 1.0e12);
        assert!(resolved.limits.get().upper() >= 1.0e12 + 3.0);
    }

    #[test]
    fn matches_stable_r2022b_auto_limit_observations() {
        let request = SemanticAxisRequest {
            mode: LimitMode::Auto,
            manual: None,
        };
        let options = AutoLimitOptions::default();
        let cases = [
            (bounds(1.0, 3.0), (1.0, 3.0)),
            (bounds(5.0, 5.0), (4.0, 6.0)),
            (bounds(-3.0, -1.0), (-3.0, -1.0)),
            (bounds(-10.0, 10.0), (-10.0, 10.0)),
            (bounds(0.0, 0.0), (-1.0, 1.0)),
        ];
        for (input, expected) in cases {
            let resolved = resolve_semantic_axis(request, input, options).unwrap();
            assert_eq!(
                (resolved.limits.get().lower(), resolved.limits.get().upper()),
                expected
            );
        }

        let single_one = resolve_semantic_axis(request, bounds(1.0, 1.0), options).unwrap();
        assert_eq!(
            single_one.limits.get().lower().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(single_one.limits.get().upper(), 2.0);

        let zero_to_two = resolve_semantic_axis(request, bounds(0.0, 2.0), options).unwrap();
        assert_eq!(
            zero_to_two.limits.get().lower().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(zero_to_two.limits.get().upper(), 2.0);
    }

    #[test]
    fn extreme_finite_range_does_not_overflow() {
        let resolved = resolve_semantic_axis(
            SemanticAxisRequest {
                mode: LimitMode::Auto,
                manual: None,
            },
            bounds(-f64::MAX, f64::MAX),
            AutoLimitOptions::default(),
        )
        .unwrap();
        assert_eq!(resolved.limits.get().lower(), -f64::MAX);
        assert_eq!(resolved.limits.get().upper(), f64::MAX);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn complete_series_api_matches_r2022b_observations() {
        use crate::NumericView;

        let auto = SemanticAxisRequest {
            mode: LimitMode::Auto,
            manual: None,
        };
        let request = SemanticAxesRequest { x: auto, y: auto };
        let options = AutoLimitOptions::default();
        let cases: Vec<(SeriesLayoutInput<'_>, [u64; 4])> = vec![
            (
                SeriesLayoutInput {
                    x: Some(NumericView::F64(&[1.0, 2.0, 3.0])),
                    y: NumericView::F64(&[5.0, 5.0, 5.0]),
                },
                [
                    1.0_f64.to_bits(),
                    3.0_f64.to_bits(),
                    4.0_f64.to_bits(),
                    6.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: None,
                    y: NumericView::F64(&[1.0]),
                },
                [
                    (-0.0_f64).to_bits(),
                    2.0_f64.to_bits(),
                    (-0.0_f64).to_bits(),
                    2.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: Some(NumericView::F64(&[1.0e12, 1.0e12 + 1.0, 1.0e12 + 2.0])),
                    y: NumericView::F64(&[0.0, 1.0, 2.0]),
                },
                [
                    1.0e12_f64.to_bits(),
                    (1.0e12_f64 + 2.0).to_bits(),
                    (-0.0_f64).to_bits(),
                    2.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: Some(NumericView::F64(&[1.0, 2.0, 3.0])),
                    y: NumericView::F64(&[f64::NAN, 2.0, f64::INFINITY]),
                },
                [
                    1.0_f64.to_bits(),
                    3.0_f64.to_bits(),
                    1.0_f64.to_bits(),
                    3.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: None,
                    y: NumericView::F64(&[]),
                },
                [
                    0.0_f64.to_bits(),
                    1.0_f64.to_bits(),
                    0.0_f64.to_bits(),
                    1.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: Some(NumericView::F64(&[-3.0, -2.0, -1.0])),
                    y: NumericView::F64(&[-10.0, 0.0, 10.0]),
                },
                [
                    (-3.0_f64).to_bits(),
                    (-1.0_f64).to_bits(),
                    (-10.0_f64).to_bits(),
                    10.0_f64.to_bits(),
                ],
            ),
            (
                SeriesLayoutInput {
                    x: None,
                    y: NumericView::F64(&[0.0, 0.0, 0.0]),
                },
                [
                    1.0_f64.to_bits(),
                    3.0_f64.to_bits(),
                    (-1.0_f64).to_bits(),
                    1.0_f64.to_bits(),
                ],
            ),
        ];
        for (input, expected) in cases {
            let resolved = resolve_semantic_axes(&[input], request, options).unwrap();
            let actual = [
                resolved.x.limits.get().lower().to_bits(),
                resolved.x.limits.get().upper().to_bits(),
                resolved.y.limits.get().lower().to_bits(),
                resolved.y.limits.get().upper().to_bits(),
            ];
            assert_eq!(actual, expected);
        }
    }
}
