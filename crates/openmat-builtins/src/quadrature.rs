//! Adaptive numerical quadrature shared by callback-driven integration built-ins.

use std::{cmp::Ordering, error::Error, fmt};

use num_complex::Complex64;
use openmat_array::{ArrayData, Complex64 as ArrayComplex64, DenseArray, Shape};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult, OutputEvent,
};
use openmat_value::{Complex64 as ValueComplex64, Value};

use crate::expect_max_outputs;

const KRONROD_ABSCISSAE: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_5,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_48,
    0.0,
];
const KRONROD_WEIGHTS: [f64; 8] = [
    0.022_935_322_010_529_224,
    0.063_092_092_629_978_56,
    0.104_790_010_322_250_19,
    0.140_653_259_715_525_92,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_42,
    0.204_432_940_075_298_89,
    0.209_482_141_084_727_82,
];
const GAUSS_WEIGHTS: [f64; 4] = [
    0.129_484_966_168_869_7,
    0.279_705_391_489_276_64,
    0.381_830_050_505_118_9,
    0.417_959_183_673_469_4,
];
const RULE_EVALUATIONS: usize = 15;
const QUADGK_DOUBLE_ABSOLUTE_TOLERANCE: f64 = 1.0e-10;
const QUADGK_DOUBLE_RELATIVE_TOLERANCE: f64 = 1.0e-6;
const QUADGK_MAXIMUM_INTERVALS: usize = 650;

/// Controls one finite-interval adaptive Gauss–Kronrod integration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveQuadratureOptions {
    /// Absolute error target for the complete integral.
    pub absolute_tolerance: f64,
    /// Relative error target, scaled by the current integral magnitude.
    pub relative_tolerance: f64,
    /// Maximum number of active subintervals.
    pub maximum_intervals: usize,
}

impl Default for AdaptiveQuadratureOptions {
    fn default() -> Self {
        Self {
            absolute_tolerance: 1.0e-10,
            relative_tolerance: 1.0e-8,
            maximum_intervals: 10_000,
        }
    }
}

/// A converged adaptive quadrature result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveQuadratureResult {
    /// Best integral estimate. Real integrands have a zero imaginary component.
    pub value: Complex64,
    /// Sum of the accepted local absolute-error estimates.
    pub absolute_error: f64,
    /// Number of integrand sample points requested from the evaluator.
    pub evaluations: usize,
    /// Number of accepted subintervals in the final partition.
    pub intervals: usize,
}

/// Failure reported by the reusable adaptive quadrature core.
#[derive(Debug)]
pub enum AdaptiveQuadratureError<E> {
    /// An endpoint was NaN or infinite.
    InvalidBounds,
    /// A tolerance was negative, non-finite, or both tolerances were zero.
    InvalidTolerance,
    /// At least one active interval must be allowed.
    InvalidIntervalLimit,
    /// A real waypoint was NaN or infinite.
    NonFiniteWaypoint {
        /// Zero-based index in the supplied waypoint slice.
        index: usize,
    },
    /// The initial waypoint partition exceeded host capacity.
    WaypointCapacity,
    /// The supplied evaluator failed.
    Evaluation(E),
    /// The evaluator returned the wrong number of values for a sample batch.
    ValueCount {
        /// Required number of values.
        expected: usize,
        /// Actual number of values.
        actual: usize,
    },
    /// The evaluator returned NaN or infinity.
    NonFiniteValue {
        /// Sample point producing the invalid value.
        point: f64,
    },
    /// The error target was not reached before exhausting the interval budget.
    IntervalLimit {
        /// Configured active-interval limit.
        maximum: usize,
        /// Best result available when the limit was reached.
        estimate: AdaptiveQuadratureResult,
    },
    /// Floating-point resolution prevented further subdivision.
    Stagnated {
        /// Left endpoint of the indivisible interval.
        start: f64,
        /// Right endpoint of the indivisible interval.
        end: f64,
        /// Best result available at stagnation.
        estimate: AdaptiveQuadratureResult,
    },
}

impl<E: fmt::Display> fmt::Display for AdaptiveQuadratureError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => formatter.write_str("quadrature bounds must be finite"),
            Self::InvalidTolerance => formatter.write_str(
                "quadrature tolerances must be finite and nonnegative, with at least one positive",
            ),
            Self::InvalidIntervalLimit => {
                formatter.write_str("quadrature must allow at least one active interval")
            }
            Self::NonFiniteWaypoint { index } => {
                write!(formatter, "quadrature waypoint {index} must be finite")
            }
            Self::WaypointCapacity => {
                formatter.write_str("quadrature waypoints exceed host capacity")
            }
            Self::Evaluation(error) => write!(formatter, "integrand evaluation failed: {error}"),
            Self::ValueCount { expected, actual } => write!(
                formatter,
                "integrand returned {actual} values for {expected} sample points"
            ),
            Self::NonFiniteValue { point } => {
                write!(
                    formatter,
                    "integrand returned a non-finite value at x = {point}"
                )
            }
            Self::IntervalLimit { maximum, .. } => write!(
                formatter,
                "quadrature did not converge within {maximum} active intervals"
            ),
            Self::Stagnated { start, end, .. } => write!(
                formatter,
                "quadrature cannot subdivide the interval [{start}, {end}] further"
            ),
        }
    }
}

impl<E: Error + 'static> Error for AdaptiveQuadratureError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Evaluation(error) => Some(error),
            Self::InvalidBounds
            | Self::InvalidTolerance
            | Self::InvalidIntervalLimit
            | Self::NonFiniteWaypoint { .. }
            | Self::WaypointCapacity
            | Self::ValueCount { .. }
            | Self::NonFiniteValue { .. }
            | Self::IntervalLimit { .. }
            | Self::Stagnated { .. } => None,
        }
    }
}

#[derive(Debug)]
struct IntervalEstimate {
    start: f64,
    end: f64,
    value: Complex64,
    error: f64,
    sequence: u64,
}

impl PartialEq for IntervalEstimate {
    fn eq(&self, other: &Self) -> bool {
        self.sequence == other.sequence
    }
}

impl Eq for IntervalEstimate {}

impl PartialOrd for IntervalEstimate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IntervalEstimate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.error
            .total_cmp(&other.error)
            .then_with(|| self.sequence.cmp(&other.sequence))
    }
}

/// Integrates a scalar real- or complex-valued function over one finite interval.
///
/// The evaluator receives all 15 points for one embedded Gauss 7 / Kronrod 15 rule in a
/// single batch and must return one value per point in the same order. The interval with the
/// largest estimated error is split first. Successful termination requires
/// `error <= max(absolute_tolerance, relative_tolerance * abs(value))`.
///
/// # Errors
///
/// Returns a structured error for invalid options, evaluator failures, non-finite values,
/// malformed evaluator output, exhausted interval capacity, or floating-point stagnation.
pub fn adaptive_gauss_kronrod<E, F>(
    start: f64,
    end: f64,
    options: AdaptiveQuadratureOptions,
    evaluate: F,
) -> Result<AdaptiveQuadratureResult, AdaptiveQuadratureError<E>>
where
    F: FnMut(&[f64]) -> Result<Vec<Complex64>, E>,
{
    adaptive_gauss_kronrod_with_waypoints(start, end, &[], options, evaluate)
}

/// Integrates over a finite real interval using waypoints as the initial adaptive partition.
///
/// Finite waypoints strictly inside the unoriented interval are sorted, deduplicated, and then
/// traversed from `start` to `end`. Waypoints at or outside the bounds are ignored. Every initial
/// segment participates in the same global error estimate and maximum-error priority queue.
///
/// # Errors
///
/// Returns the same failures as [`adaptive_gauss_kronrod`], plus a structured error for a
/// non-finite waypoint or an initial partition that exceeds host capacity.
pub fn adaptive_gauss_kronrod_with_waypoints<E, F>(
    start: f64,
    end: f64,
    waypoints: &[f64],
    options: AdaptiveQuadratureOptions,
    mut evaluate: F,
) -> Result<AdaptiveQuadratureResult, AdaptiveQuadratureError<E>>
where
    F: FnMut(&[f64]) -> Result<Vec<Complex64>, E>,
{
    validate_options(start, end, options)?;
    let breakpoints = normalized_breakpoints(start, end, waypoints)?;
    if same_coordinate(start, end) {
        return Ok(AdaptiveQuadratureResult {
            value: Complex64::new(0.0, 0.0),
            absolute_error: 0.0,
            evaluations: 0,
            intervals: 1,
        });
    }

    let mut sequence = 0_u64;
    let initial_intervals = breakpoints.len() - 1;
    let mut value = Complex64::new(0.0, 0.0);
    let mut error = 0.0;
    let mut evaluations = 0_usize;
    let mut intervals = std::collections::BinaryHeap::new();
    intervals
        .try_reserve(initial_intervals)
        .map_err(|_| AdaptiveQuadratureError::WaypointCapacity)?;
    for segment in breakpoints.windows(2) {
        let estimate = evaluate_interval(segment[0], segment[1], sequence, &mut evaluate)?;
        sequence = sequence.wrapping_add(1);
        value += estimate.value;
        error += estimate.error;
        evaluations = evaluations.saturating_add(RULE_EVALUATIONS);
        intervals.push(estimate);
    }

    while error > tolerance(options, value) {
        if intervals.len() >= options.maximum_intervals {
            return Err(AdaptiveQuadratureError::IntervalLimit {
                maximum: options.maximum_intervals,
                estimate: result(value, error, evaluations, intervals.len()),
            });
        }
        let Some(worst) = intervals.pop() else {
            return Err(AdaptiveQuadratureError::InvalidIntervalLimit);
        };
        let midpoint = 0.5 * worst.start + 0.5 * worst.end;
        if same_coordinate(midpoint, worst.start) || same_coordinate(midpoint, worst.end) {
            let stagnated_start = worst.start;
            let stagnated_end = worst.end;
            intervals.push(worst);
            return Err(AdaptiveQuadratureError::Stagnated {
                start: stagnated_start,
                end: stagnated_end,
                estimate: result(value, error, evaluations, intervals.len()),
            });
        }

        let left = evaluate_interval(worst.start, midpoint, sequence, &mut evaluate)?;
        sequence = sequence.wrapping_add(1);
        let right = evaluate_interval(midpoint, worst.end, sequence, &mut evaluate)?;
        sequence = sequence.wrapping_add(1);
        evaluations = evaluations.saturating_add(2 * RULE_EVALUATIONS);
        value += left.value + right.value - worst.value;
        error = (error + left.error + right.error - worst.error).max(0.0);
        intervals.push(left);
        intervals.push(right);
    }

    Ok(result(value, error, evaluations, intervals.len()))
}

fn normalized_breakpoints<E>(
    start: f64,
    end: f64,
    waypoints: &[f64],
) -> Result<Vec<f64>, AdaptiveQuadratureError<E>> {
    let capacity = waypoints
        .len()
        .checked_add(2)
        .ok_or(AdaptiveQuadratureError::WaypointCapacity)?;
    let mut breakpoints = Vec::new();
    breakpoints
        .try_reserve_exact(capacity)
        .map_err(|_| AdaptiveQuadratureError::WaypointCapacity)?;
    let lower = start.min(end);
    let upper = start.max(end);
    for (index, waypoint) in waypoints.iter().copied().enumerate() {
        if !waypoint.is_finite() {
            return Err(AdaptiveQuadratureError::NonFiniteWaypoint { index });
        }
        if waypoint > lower && waypoint < upper {
            breakpoints.push(waypoint);
        }
    }
    breakpoints.sort_by(f64::total_cmp);
    breakpoints.dedup_by(|left, right| same_coordinate(*left, *right));
    if start.total_cmp(&end) == Ordering::Greater {
        breakpoints.reverse();
    }
    breakpoints.insert(0, start);
    breakpoints.push(end);
    Ok(breakpoints)
}

/// Integrates an `OpenMat` callable through [`BuiltinContext`].
///
/// Each callback receives a `1 x 15` double batch and must return a real or complex floating
/// array with 15 elements. Callback exceptions and cancellation retain their existing
/// [`BuiltinError`] metadata.
///
/// # Errors
///
/// Returns the original callback error or an `OpenMat:Quadrature:*` domain/capacity failure.
pub fn adaptive_gauss_kronrod_callable(
    context: &mut BuiltinContext<'_>,
    callable: &Value,
    start: f64,
    end: f64,
    options: AdaptiveQuadratureOptions,
) -> Result<AdaptiveQuadratureResult, BuiltinError> {
    adaptive_gauss_kronrod(start, end, options, |points| {
        evaluate_callable(context, callable, points)
    })
    .map_err(quadrature_builtin_error)
}

pub(super) fn quadgk_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    if arguments.len() < 3 || !(arguments.len() - 3).is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `quadgk` expects a function, two bounds, and complete name/value pairs but received {} inputs",
                arguments.len()
            ),
        ));
    }
    expect_max_outputs("quadgk", context, 2)?;
    context.check_cancelled()?;
    if !matches!(arguments[0], Value::Function(_)) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!(
                "input 1 to `quadgk` must be a function handle, found {}",
                arguments[0].kind()
            ),
        ));
    }

    let start = quadgk_bound(2, &arguments[1])?;
    let end = quadgk_bound(3, &arguments[2])?;
    let parsed = quadgk_options(&arguments[3..])?;
    let Some(domain) = QuadgkDomain::from_path(start, end, &parsed.waypoints)? else {
        let mut outputs = vec![Value::Double(f64::NAN)];
        if context.requested_outputs() > 1 {
            outputs.push(Value::Double(f64::NAN));
        }
        return Ok(outputs);
    };
    let (integration_start, integration_end) = domain.integration_bounds();
    let transformed_waypoints = domain.transform_waypoints(&parsed.waypoints)?;
    let mut complex_result = domain.is_contour();
    let integration = adaptive_gauss_kronrod_with_waypoints(
        integration_start,
        integration_end,
        &transformed_waypoints,
        parsed.quadrature,
        |points| {
            let values = evaluate_quadgk_domain(context, &arguments[0], &domain, points)?;
            complex_result |= values.complex;
            Ok::<_, BuiltinError>(values.values)
        },
    );
    let integrated = match integration {
        Ok(integrated) => integrated,
        Err(AdaptiveQuadratureError::IntervalLimit { maximum, estimate }) => {
            quadgk_warning(
                context,
                "MATLAB:quadgk:MaxIntervalCountReached",
                format!(
                    "`quadgk` reached MaxIntervalCount={maximum}; returning the best estimate with approximate absolute error {}",
                    estimate.absolute_error
                ),
            )?;
            estimate
        }
        Err(AdaptiveQuadratureError::Stagnated {
            start,
            end,
            estimate,
        }) => {
            let start = domain.original_point(start);
            let end = domain.original_point(end);
            quadgk_warning(
                context,
                "MATLAB:quadgk:MinStepSize",
                format!(
                    "`quadgk` reached floating-point spacing on [{}, {}]; returning the best estimate with approximate absolute error {}",
                    format_quadgk_point(start),
                    format_quadgk_point(end),
                    estimate.absolute_error
                ),
            )?;
            estimate
        }
        Err(AdaptiveQuadratureError::NonFiniteValue { point }) => {
            return Err(quadgk_non_finite_value_error(domain.original_point(point)));
        }
        Err(error) => return Err(quadrature_builtin_error(error)),
    };

    let integral = if complex_result {
        Value::Complex(ValueComplex64::new(
            integrated.value.re,
            integrated.value.im,
        ))
    } else {
        Value::Double(integrated.value.re)
    };
    let mut outputs = vec![integral];
    if context.requested_outputs() > 1 {
        outputs.push(Value::Double(integrated.absolute_error));
    }
    Ok(outputs)
}

fn quadgk_bound(position: usize, value: &Value) -> Result<Complex64, BuiltinError> {
    double_complex_scalar(value).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `quadgk` must be a double scalar"),
        )
    })
}

#[derive(Debug, PartialEq)]
enum QuadgkDomain {
    Finite {
        start: f64,
        end: f64,
    },
    RightInfinite {
        bound: f64,
        direction: f64,
    },
    LeftInfinite {
        bound: f64,
        direction: f64,
    },
    DoublyInfinite {
        direction: f64,
    },
    Contour {
        vertices: Vec<Complex64>,
        end_parameter: f64,
    },
}

impl QuadgkDomain {
    fn from_path(
        start: Complex64,
        end: Complex64,
        waypoints: &[Complex64],
    ) -> Result<Option<Self>, BuiltinError> {
        let contour = has_imaginary(start)
            || has_imaginary(end)
            || waypoints.iter().copied().any(has_imaginary);
        if !contour {
            if start.re.is_nan() || end.re.is_nan() {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "inputs 2 and 3 to `quadgk` must not be NaN",
                )
                .with_identifier("OpenMat:quadgk:NonFiniteBound"));
            }
            return Ok(Self::from_bounds(start.re, end.re));
        }
        if !complex_is_finite(start)
            || !complex_is_finite(end)
            || waypoints
                .iter()
                .copied()
                .any(|point| !complex_is_finite(point))
        {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "complex `quadgk` contours must contain only finite vertices",
            )
            .with_identifier("MATLAB:quadgk:nonFiniteContourError"));
        }

        let capacity = waypoints.len().checked_add(2).ok_or_else(|| {
            quadgk_capacity_error("quadrature contour vertices exceed host capacity")
        })?;
        let mut vertices = Vec::new();
        vertices.try_reserve_exact(capacity).map_err(|_| {
            quadgk_capacity_error("quadrature contour vertices exceed host capacity")
        })?;
        vertices.push(start);
        for vertex in waypoints.iter().copied().chain(std::iter::once(end)) {
            if vertices
                .last()
                .is_none_or(|previous| !same_complex_coordinate(*previous, vertex))
            {
                vertices.push(vertex);
            }
        }
        if vertices.len() == 1 {
            return Ok(Some(Self::Finite {
                start: 0.0,
                end: 0.0,
            }));
        }
        let segment_count = u32::try_from(vertices.len() - 1)
            .map_err(|_| quadgk_capacity_error("quadrature contour has too many line segments"))?;
        Ok(Some(Self::Contour {
            vertices,
            end_parameter: f64::from(segment_count),
        }))
    }

    fn from_bounds(start: f64, end: f64) -> Option<Self> {
        match (start.is_infinite(), end.is_infinite()) {
            (false, false) => Some(Self::Finite { start, end }),
            (false, true) if end.is_sign_positive() => Some(Self::RightInfinite {
                bound: start,
                direction: 1.0,
            }),
            (true, false) if start.is_sign_positive() => Some(Self::RightInfinite {
                bound: end,
                direction: -1.0,
            }),
            (true, false) => Some(Self::LeftInfinite {
                bound: end,
                direction: 1.0,
            }),
            (false, true) => Some(Self::LeftInfinite {
                bound: start,
                direction: -1.0,
            }),
            (true, true) if start.is_sign_negative() != end.is_sign_negative() => {
                Some(Self::DoublyInfinite {
                    direction: if start.is_sign_negative() { 1.0 } else { -1.0 },
                })
            }
            (true, true) => None,
        }
    }

    fn integration_bounds(&self) -> (f64, f64) {
        match self {
            Self::Finite { start, end } => (*start, *end),
            Self::RightInfinite { .. } | Self::LeftInfinite { .. } => (0.0, 1.0),
            Self::DoublyInfinite { .. } => (-1.0, 1.0),
            Self::Contour { end_parameter, .. } => (0.0, *end_parameter),
        }
    }

    fn transform_waypoints(&self, waypoints: &[Complex64]) -> Result<Vec<f64>, BuiltinError> {
        let mut transformed = Vec::new();
        transformed
            .try_reserve_exact(waypoints.len())
            .map_err(|_| quadgk_capacity_error("quadrature waypoints exceed host capacity"))?;
        match self {
            Self::Contour { vertices, .. } => {
                for index in 1..vertices.len() - 1 {
                    let index = u32::try_from(index).map_err(|_| {
                        quadgk_capacity_error("quadrature contour has too many line segments")
                    })?;
                    transformed.push(f64::from(index));
                }
            }
            _ => transformed.extend(
                waypoints
                    .iter()
                    .filter_map(|waypoint| self.transform_waypoint(waypoint.re)),
            ),
        }
        Ok(transformed)
    }

    fn transform_waypoint(&self, waypoint: f64) -> Option<f64> {
        match self {
            Self::Finite { .. } => Some(waypoint),
            Self::RightInfinite { bound, .. } if waypoint > *bound => {
                Some(semi_infinite_parameter((waypoint - *bound).sqrt()))
            }
            Self::LeftInfinite { bound, .. } if waypoint < *bound => {
                Some(semi_infinite_parameter((*bound - waypoint).sqrt()))
            }
            Self::DoublyInfinite { .. } => Some(doubly_infinite_parameter(waypoint)),
            Self::RightInfinite { .. } | Self::LeftInfinite { .. } | Self::Contour { .. } => None,
        }
    }

    fn transformed_sample(&self, parameter: f64) -> (Complex64, Complex64) {
        match self {
            Self::Finite { .. } => (Complex64::new(parameter, 0.0), Complex64::new(1.0, 0.0)),
            Self::RightInfinite { bound, direction } => {
                // x = bound + (t / (1 - t))^2 maps 0 <= t < 1 onto [bound, +Inf).
                let denominator = 1.0 - parameter;
                let radius = parameter / denominator;
                (
                    Complex64::new(radius.mul_add(radius, *bound), 0.0),
                    Complex64::new(*direction * 2.0 * parameter / denominator.powi(3), 0.0),
                )
            }
            Self::LeftInfinite { bound, direction } => {
                // x = bound - (t / (1 - t))^2 maps 0 <= t < 1 onto (-Inf, bound].
                let denominator = 1.0 - parameter;
                let radius = parameter / denominator;
                (
                    Complex64::new((-radius).mul_add(radius, *bound), 0.0),
                    Complex64::new(*direction * 2.0 * parameter / denominator.powi(3), 0.0),
                )
            }
            Self::DoublyInfinite { direction } => {
                // x = t / (1 - t^2) maps -1 < t < 1 onto the complete real line.
                let square = parameter * parameter;
                let denominator = 1.0 - square;
                (
                    Complex64::new(parameter / denominator, 0.0),
                    Complex64::new(
                        *direction * (1.0 + square) / (denominator * denominator),
                        0.0,
                    ),
                )
            }
            Self::Contour {
                vertices,
                end_parameter,
            } => contour_sample(vertices, *end_parameter, parameter),
        }
    }

    fn original_point(&self, parameter: f64) -> Complex64 {
        self.transformed_sample(parameter).0
    }

    fn is_contour(&self) -> bool {
        matches!(self, Self::Contour { .. })
    }
}

fn quadgk_capacity_error(message: &'static str) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Other, message)
        .with_identifier("OpenMat:Quadrature:Capacity")
}

fn complex_is_finite(point: Complex64) -> bool {
    point.re.is_finite() && point.im.is_finite()
}

#[allow(clippy::float_cmp)]
fn has_imaginary(point: Complex64) -> bool {
    point.im != 0.0
}

#[allow(clippy::float_cmp)]
fn same_complex_coordinate(left: Complex64, right: Complex64) -> bool {
    left.re == right.re && left.im == right.im
}

fn format_quadgk_point(point: Complex64) -> String {
    if has_imaginary(point) {
        point.to_string()
    } else {
        point.re.to_string()
    }
}

fn quadgk_non_finite_value_error(point: Complex64) -> BuiltinError {
    let coordinate = if has_imaginary(point) { "z" } else { "x" };
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!(
            "quadrature callback returned a non-finite value at {coordinate} = {}",
            format_quadgk_point(point)
        ),
    )
    .with_identifier("OpenMat:Quadrature:NonFiniteValue")
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn contour_sample(
    vertices: &[Complex64],
    end_parameter: f64,
    parameter: f64,
) -> (Complex64, Complex64) {
    let last_segment = vertices.len() - 2;
    let (segment, local_parameter) = if parameter <= 0.0 {
        (0, 0.0)
    } else if parameter >= end_parameter {
        (last_segment, 1.0)
    } else {
        let segment_start = parameter.floor();
        (segment_start as usize, parameter - segment_start)
    };
    let delta = vertices[segment + 1] - vertices[segment];
    (vertices[segment] + local_parameter * delta, delta)
}

fn semi_infinite_parameter(radius: f64) -> f64 {
    if radius <= 1.0 {
        radius / (1.0 + radius)
    } else {
        1.0 - 1.0 / (1.0 + radius)
    }
}

fn doubly_infinite_parameter(point: f64) -> f64 {
    if point == 0.0 {
        return point;
    }
    let magnitude = point.abs();
    let parameter = if magnitude <= 1.0 {
        2.0 * magnitude / (1.0 + (2.0 * magnitude).hypot(1.0))
    } else {
        let reciprocal = magnitude.recip();
        2.0 / (reciprocal + reciprocal.hypot(2.0))
    };
    parameter.copysign(point)
}

#[derive(Debug, PartialEq)]
struct QuadgkOptions {
    quadrature: AdaptiveQuadratureOptions,
    waypoints: Vec<Complex64>,
}

fn quadgk_options(arguments: &[Value]) -> Result<QuadgkOptions, BuiltinError> {
    let mut options = QuadgkOptions {
        quadrature: AdaptiveQuadratureOptions {
            absolute_tolerance: QUADGK_DOUBLE_ABSOLUTE_TOLERANCE,
            relative_tolerance: QUADGK_DOUBLE_RELATIVE_TOLERANCE,
            maximum_intervals: QUADGK_MAXIMUM_INTERVALS,
        },
        waypoints: Vec::new(),
    };
    for (pair_index, pair) in arguments.chunks_exact(2).enumerate() {
        let name_position = 4 + 2 * pair_index;
        let value_position = name_position + 1;
        let name = scalar_text(name_position, &pair[0])?.to_ascii_lowercase();
        match name.as_str() {
            "abstol" => {
                options.quadrature.absolute_tolerance = quadgk_tolerance(value_position, &pair[1])?;
            }
            "reltol" => {
                options.quadrature.relative_tolerance = quadgk_tolerance(value_position, &pair[1])?;
            }
            "maxintervalcount" => {
                options.quadrature.maximum_intervals =
                    quadgk_interval_limit(value_position, &pair[1])?;
            }
            "waypoints" => {
                options.waypoints = quadgk_waypoints(value_position, &pair[1])?;
            }
            _ => {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    format!(
                        "`quadgk` currently supports only the AbsTol, RelTol, MaxIntervalCount, and Waypoints name/value options, not `{}`",
                        scalar_text(name_position, &pair[0])?
                    ),
                )
                .with_identifier("OpenMat:quadgk:UnsupportedOption"));
            }
        }
    }
    if options.quadrature.absolute_tolerance == 0.0 && options.quadrature.relative_tolerance == 0.0
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`quadgk` AbsTol and RelTol cannot both be zero",
        )
        .with_identifier("OpenMat:quadgk:InvalidTolerance"));
    }
    if options.quadrature.relative_tolerance > 0.0
        && options.quadrature.relative_tolerance < 100.0 * f64::EPSILON
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`quadgk` RelTol must be zero or at least 100*eps for double integration",
        )
        .with_identifier("OpenMat:quadgk:InvalidTolerance"));
    }
    Ok(options)
}

fn quadgk_waypoints(position: usize, value: &Value) -> Result<Vec<Complex64>, BuiltinError> {
    let not_vector = || {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `quadgk` must be a double waypoint vector"),
        )
        .with_identifier("MATLAB:quadgk:WaypointsNotVector")
    };
    let waypoints = match value {
        Value::Double(value) => vec![Complex64::new(*value, 0.0)],
        Value::Complex(value) => vec![Complex64::new(value.real, value.imaginary)],
        Value::Array(ArrayData::F64(values)) if waypoint_vector_shape(values.shape()) => values
            .as_slice()
            .iter()
            .map(|value| Complex64::new(*value, 0.0))
            .collect(),
        Value::Array(ArrayData::ComplexF64(values)) if waypoint_vector_shape(values.shape()) => {
            values
                .as_slice()
                .iter()
                .map(|value| Complex64::new(value.re, value.im))
                .collect()
        }
        Value::Array(ArrayData::F64(_) | ArrayData::ComplexF64(_)) => {
            return Err(not_vector());
        }
        _ => return Err(not_vector()),
    };
    if waypoints
        .iter()
        .copied()
        .any(|waypoint| !complex_is_finite(waypoint))
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `quadgk` contains a non-finite waypoint"),
        )
        .with_identifier("MATLAB:quadgk:WaypointsNotFinite"));
    }
    Ok(waypoints)
}

fn waypoint_vector_shape(shape: &Shape) -> bool {
    shape.dimensions() == [0, 0]
        || (shape.ndims() == 2 && (shape.extent(0) == 1 || shape.extent(1) == 1))
}

fn quadgk_tolerance(position: usize, value: &Value) -> Result<f64, BuiltinError> {
    let tolerance = double_real_scalar(value).ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `quadgk` must be a real double tolerance scalar"),
        )
    })?;
    if tolerance.is_finite() && tolerance >= 0.0 {
        Ok(tolerance)
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `quadgk` must be a finite nonnegative tolerance"),
        )
        .with_identifier("OpenMat:quadgk:InvalidTolerance"))
    }
}

fn quadgk_interval_limit(position: usize, value: &Value) -> Result<usize, BuiltinError> {
    let invalid = || {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `quadgk` must be a positive integer MaxIntervalCount"),
        )
        .with_identifier("MATLAB:quadgk:invalidMaxIntervalCount")
    };
    let limit = double_real_scalar(value).ok_or_else(invalid)?;
    if !limit.is_finite() || limit < 1.0 || limit.fract() != 0.0 {
        return Err(invalid());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let integer = limit as u128;
    usize::try_from(integer).map_err(|_| invalid())
}

fn quadgk_warning(
    context: &mut BuiltinContext<'_>,
    identifier: &str,
    message: String,
) -> Result<(), BuiltinError> {
    let event = context
        .warning_enabled(identifier)?
        .then(|| OutputEvent::CommandText(format!("Warning: {message}\n")));
    context.set_last_warning(message, identifier)?;
    if let Some(event) = event {
        context.emit(event)?;
    }
    Ok(())
}

#[allow(clippy::float_cmp)]
fn double_real_scalar(value: &Value) -> Option<f64> {
    match value {
        Value::Double(value) => Some(*value),
        Value::Complex(value) if value.imaginary == 0.0 => Some(value.real),
        Value::Array(ArrayData::F64(values)) if values.numel() == 1 => Some(values.as_slice()[0]),
        Value::Array(ArrayData::ComplexF64(values))
            if values.numel() == 1 && values.as_slice()[0].im == 0.0 =>
        {
            Some(values.as_slice()[0].re)
        }
        _ => None,
    }
}

fn double_complex_scalar(value: &Value) -> Option<Complex64> {
    match value {
        Value::Double(value) => Some(Complex64::new(*value, 0.0)),
        Value::Complex(value) => Some(Complex64::new(value.real, value.imaginary)),
        Value::Array(ArrayData::F64(values)) if values.numel() == 1 => {
            Some(Complex64::new(values.as_slice()[0], 0.0))
        }
        Value::Array(ArrayData::ComplexF64(values)) if values.numel() == 1 => {
            let value = values.as_slice()[0];
            Some(Complex64::new(value.re, value.im))
        }
        _ => None,
    }
}

fn scalar_text(position: usize, value: &Value) -> Result<String, BuiltinError> {
    let code_units = match value {
        Value::String(string) => string
            .as_scalar()
            .filter(|element| !element.is_missing())
            .map(|element| element.code_units().to_vec()),
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            Some(array.as_slice().iter().map(|value| value.get()).collect())
        }
        _ => None,
    }
    .ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Type,
            format!("input {position} to `quadgk` must be a nonmissing string scalar or char row"),
        )
    })?;
    String::from_utf16(&code_units).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `quadgk` contains invalid UTF-16 text"),
        )
    })
}

fn validate_options<E>(
    start: f64,
    end: f64,
    options: AdaptiveQuadratureOptions,
) -> Result<(), AdaptiveQuadratureError<E>> {
    if !start.is_finite() || !end.is_finite() {
        return Err(AdaptiveQuadratureError::InvalidBounds);
    }
    if !options.absolute_tolerance.is_finite()
        || !options.relative_tolerance.is_finite()
        || options.absolute_tolerance < 0.0
        || options.relative_tolerance < 0.0
        || (options.absolute_tolerance == 0.0 && options.relative_tolerance == 0.0)
    {
        return Err(AdaptiveQuadratureError::InvalidTolerance);
    }
    if options.maximum_intervals == 0 {
        return Err(AdaptiveQuadratureError::InvalidIntervalLimit);
    }
    Ok(())
}

fn tolerance(options: AdaptiveQuadratureOptions, value: Complex64) -> f64 {
    options
        .absolute_tolerance
        .max(options.relative_tolerance * value.norm())
}

#[allow(clippy::float_cmp)]
fn same_coordinate(left: f64, right: f64) -> bool {
    // Exact equality is intentional: this detects zero-width intervals and subdivision
    // stagnation at the floating-point representation boundary.
    left == right
}

fn result(
    value: Complex64,
    absolute_error: f64,
    evaluations: usize,
    intervals: usize,
) -> AdaptiveQuadratureResult {
    AdaptiveQuadratureResult {
        value,
        absolute_error,
        evaluations,
        intervals,
    }
}

fn evaluate_interval<E, F>(
    start: f64,
    end: f64,
    sequence: u64,
    evaluate: &mut F,
) -> Result<IntervalEstimate, AdaptiveQuadratureError<E>>
where
    F: FnMut(&[f64]) -> Result<Vec<Complex64>, E>,
{
    let center = 0.5 * start + 0.5 * end;
    let half_length = 0.5 * end - 0.5 * start;
    let mut points = [0.0; RULE_EVALUATIONS];
    points[0] = center;
    for (index, abscissa) in KRONROD_ABSCISSAE[..7].iter().enumerate() {
        let offset = half_length * abscissa;
        points[1 + 2 * index] = center - offset;
        points[2 + 2 * index] = center + offset;
    }
    let values = evaluate(&points).map_err(AdaptiveQuadratureError::Evaluation)?;
    if values.len() != RULE_EVALUATIONS {
        return Err(AdaptiveQuadratureError::ValueCount {
            expected: RULE_EVALUATIONS,
            actual: values.len(),
        });
    }
    if let Some((index, _)) = values
        .iter()
        .enumerate()
        .find(|(_, value)| !value.re.is_finite() || !value.im.is_finite())
    {
        return Err(AdaptiveQuadratureError::NonFiniteValue {
            point: points[index],
        });
    }

    let center_value = values[0];
    let mut kronrod = KRONROD_WEIGHTS[7] * center_value;
    let mut gauss = GAUSS_WEIGHTS[3] * center_value;
    let mut absolute = KRONROD_WEIGHTS[7] * center_value.norm();
    for index in 0..7 {
        let left = values[1 + 2 * index];
        let right = values[2 + 2 * index];
        let pair = left + right;
        kronrod += KRONROD_WEIGHTS[index] * pair;
        absolute += KRONROD_WEIGHTS[index] * (left.norm() + right.norm());
        if index % 2 == 1 {
            gauss += GAUSS_WEIGHTS[index / 2] * pair;
        }
    }

    let reference_mean = 0.5 * kronrod;
    let mut ascendant = KRONROD_WEIGHTS[7] * (center_value - reference_mean).norm();
    for index in 0..7 {
        let left = values[1 + 2 * index];
        let right = values[2 + 2 * index];
        ascendant += KRONROD_WEIGHTS[index]
            * ((left - reference_mean).norm() + (right - reference_mean).norm());
    }

    let absolute_half_length = half_length.abs();
    let value = half_length * kronrod;
    let absolute = absolute_half_length * absolute;
    let ascendant = absolute_half_length * ascendant;
    let mut error = (half_length * (kronrod - gauss)).norm();
    if ascendant > 0.0 && error > 0.0 {
        error = ascendant * (200.0 * error / ascendant).powf(1.5).min(1.0);
    }
    if absolute > f64::MIN_POSITIVE / (50.0 * f64::EPSILON) {
        error = error.max(50.0 * f64::EPSILON * absolute);
    }

    Ok(IntervalEstimate {
        start,
        end,
        value,
        error,
        sequence,
    })
}

fn evaluate_callable(
    context: &mut BuiltinContext<'_>,
    callable: &Value,
    points: &[f64],
) -> Result<Vec<Complex64>, BuiltinError> {
    let value = invoke_callable(context, callable, points)?;
    floating_values(value)
}

fn invoke_callable(
    context: &mut BuiltinContext<'_>,
    callable: &Value,
    points: &[f64],
) -> Result<Value, BuiltinError> {
    let sample_count = u64::try_from(points.len()).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("quadrature sample count exceeds array capacity: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let shape = Shape::new([1, sample_count]).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("failed to allocate quadrature sample shape: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let samples = DenseArray::from_vec(shape, points.to_vec()).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("failed to allocate quadrature samples: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let mut returned =
        context.invoke_owned(callable, vec![Value::Array(ArrayData::F64(samples))], 1)?;
    returned.pop().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "quadrature callback returned no value",
        )
        .with_identifier("OpenMat:Quadrature:MissingValue")
    })
}

fn invoke_complex_callable(
    context: &mut BuiltinContext<'_>,
    callable: &Value,
    points: &[Complex64],
) -> Result<Value, BuiltinError> {
    let sample_count = u64::try_from(points.len()).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("quadrature sample count exceeds array capacity: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let shape = Shape::new([1, sample_count]).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("failed to allocate quadrature sample shape: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(points.len())
        .map_err(|_| quadgk_capacity_error("quadrature complex samples exceed host capacity"))?;
    values.extend(
        points
            .iter()
            .map(|point| ArrayComplex64::new(point.re, point.im)),
    );
    let samples = DenseArray::from_vec(shape, values).map_err(|error| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!("failed to allocate quadrature samples: {error}"),
        )
        .with_identifier("OpenMat:Quadrature:Capacity")
    })?;
    let mut returned = context.invoke_owned(
        callable,
        vec![Value::Array(ArrayData::ComplexF64(samples))],
        1,
    )?;
    returned.pop().ok_or_else(|| {
        BuiltinError::new(
            BuiltinErrorCategory::Other,
            "quadrature callback returned no value",
        )
        .with_identifier("OpenMat:Quadrature:MissingValue")
    })
}

fn evaluate_quadgk_domain(
    context: &mut BuiltinContext<'_>,
    callable: &Value,
    domain: &QuadgkDomain,
    parameters: &[f64],
) -> Result<DoubleFloatingValues, BuiltinError> {
    if matches!(domain, QuadgkDomain::Finite { .. }) {
        return double_floating_values(invoke_callable(context, callable, parameters)?);
    }

    let mut values = if domain.is_contour() {
        let mut points = Vec::new();
        points.try_reserve_exact(parameters.len()).map_err(|_| {
            quadgk_capacity_error("quadrature transformed samples exceed host capacity")
        })?;
        points.extend(
            parameters
                .iter()
                .map(|parameter| domain.original_point(*parameter)),
        );
        double_floating_values(invoke_complex_callable(context, callable, &points)?)?
    } else {
        let mut points = Vec::new();
        points.try_reserve_exact(parameters.len()).map_err(|_| {
            quadgk_capacity_error("quadrature transformed samples exceed host capacity")
        })?;
        points.extend(
            parameters
                .iter()
                .map(|parameter| domain.original_point(*parameter).re),
        );
        double_floating_values(invoke_callable(context, callable, &points)?)?
    };
    for (value, parameter) in values.values.iter_mut().zip(parameters) {
        let (_, jacobian) = domain.transformed_sample(*parameter);
        *value *= jacobian;
    }
    values.complex |= domain.is_contour();
    Ok(values)
}

struct DoubleFloatingValues {
    values: Vec<Complex64>,
    complex: bool,
}

fn double_floating_values(value: Value) -> Result<DoubleFloatingValues, BuiltinError> {
    let (values, complex) = match value {
        Value::Double(value) => (vec![Complex64::new(value, 0.0)], false),
        Value::Complex(value) => (vec![Complex64::new(value.real, value.imaginary)], true),
        Value::Array(ArrayData::F64(values)) => (
            values
                .as_slice()
                .iter()
                .map(|value| Complex64::new(*value, 0.0))
                .collect(),
            false,
        ),
        Value::Array(ArrayData::ComplexF64(values)) => (
            values
                .as_slice()
                .iter()
                .map(|value| Complex64::new(value.re, value.im))
                .collect(),
            true,
        ),
        other => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                format!(
                    "`quadgk` callback must return a double array in this implementation, received `{}`",
                    other.kind()
                ),
            )
            .with_identifier("OpenMat:quadgk:UnsupportedIntegrandClass"));
        }
    };
    Ok(DoubleFloatingValues { values, complex })
}

fn floating_values(value: Value) -> Result<Vec<Complex64>, BuiltinError> {
    let values = match value {
        Value::Double(value) => vec![Complex64::new(value, 0.0)],
        Value::Complex(value) => vec![Complex64::new(value.real, value.imaginary)],
        Value::Array(ArrayData::F64(values)) => values
            .as_slice()
            .iter()
            .map(|value| Complex64::new(*value, 0.0))
            .collect(),
        Value::Array(ArrayData::ComplexF64(values)) => values
            .as_slice()
            .iter()
            .map(|value| Complex64::new(value.re, value.im))
            .collect(),
        Value::Array(ArrayData::F32(values)) => values
            .as_slice()
            .iter()
            .map(|value| Complex64::new(f64::from(*value), 0.0))
            .collect(),
        Value::Array(ArrayData::ComplexF32(values)) => values
            .as_slice()
            .iter()
            .map(|value| Complex64::new(f64::from(value.re), f64::from(value.im)))
            .collect(),
        other => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                format!(
                    "quadrature callback must return a real or complex floating array, received `{}`",
                    other.kind()
                ),
            )
            .with_identifier("OpenMat:Quadrature:InvalidValue"));
        }
    };
    Ok(values)
}

fn quadrature_builtin_error(error: AdaptiveQuadratureError<BuiltinError>) -> BuiltinError {
    match error {
        AdaptiveQuadratureError::Evaluation(error) => error,
        AdaptiveQuadratureError::InvalidBounds => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "quadrature bounds must be finite",
        )
        .with_identifier("OpenMat:Quadrature:InvalidBounds"),
        AdaptiveQuadratureError::InvalidTolerance => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "quadrature tolerances must be finite and nonnegative, with at least one positive",
        )
        .with_identifier("OpenMat:Quadrature:InvalidTolerance"),
        AdaptiveQuadratureError::InvalidIntervalLimit => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "quadrature must allow at least one active interval",
        )
        .with_identifier("OpenMat:Quadrature:InvalidIntervalLimit"),
        AdaptiveQuadratureError::NonFiniteWaypoint { index } => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("quadrature waypoint {index} must be finite"),
        )
        .with_identifier("OpenMat:Quadrature:NonFiniteWaypoint"),
        AdaptiveQuadratureError::WaypointCapacity => BuiltinError::new(
            BuiltinErrorCategory::Other,
            "quadrature waypoints exceed host capacity",
        )
        .with_identifier("OpenMat:Quadrature:Capacity"),
        AdaptiveQuadratureError::ValueCount { expected, actual } => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("quadrature callback returned {actual} values for {expected} sample points"),
        )
        .with_identifier("OpenMat:Quadrature:ValueCount"),
        AdaptiveQuadratureError::NonFiniteValue { point } => BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("quadrature callback returned a non-finite value at x = {point}"),
        )
        .with_identifier("OpenMat:Quadrature:NonFiniteValue"),
        AdaptiveQuadratureError::IntervalLimit { maximum, estimate } => BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!(
                "quadrature did not reach its error target within {maximum} intervals; estimated absolute error is {}",
                estimate.absolute_error
            ),
        )
        .with_identifier("OpenMat:Quadrature:IntervalLimit"),
        AdaptiveQuadratureError::Stagnated {
            start,
            end,
            estimate,
        } => BuiltinError::new(
            BuiltinErrorCategory::Other,
            format!(
                "quadrature cannot subdivide [{start}, {end}] further; estimated absolute error is {}",
                estimate.absolute_error
            ),
        )
        .with_identifier("OpenMat:Quadrature:Stagnated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_values(points: &[f64], function: impl Fn(f64) -> f64) -> Vec<Complex64> {
        points
            .iter()
            .map(|point| Complex64::new(function(*point), 0.0))
            .collect()
    }

    #[test]
    fn smooth_real_and_complex_integrals_converge_with_embedded_error_estimates() {
        let real = adaptive_gauss_kronrod(
            0.0,
            std::f64::consts::PI,
            AdaptiveQuadratureOptions::default(),
            |points| Ok::<_, ()>(real_values(points, f64::sin)),
        )
        .unwrap();
        assert!((real.value.re - 2.0).abs() <= 1.0e-12);
        assert!(real.value.im.abs() <= f64::EPSILON);
        assert!(real.absolute_error <= 1.0e-10);

        let complex = adaptive_gauss_kronrod(
            0.0,
            std::f64::consts::PI,
            AdaptiveQuadratureOptions::default(),
            |points| {
                Ok::<_, ()>(
                    points
                        .iter()
                        .map(|point| Complex64::new(point.cos(), point.sin()))
                        .collect(),
                )
            },
        )
        .unwrap();
        assert!(complex.value.re.abs() <= 1.0e-12);
        assert!((complex.value.im - 2.0).abs() <= 1.0e-12);
    }

    #[test]
    fn localized_nonsmooth_integrand_drives_adaptive_subdivision() {
        let split = 0.123_456_789;
        let integrated = adaptive_gauss_kronrod(
            -1.0,
            1.0,
            AdaptiveQuadratureOptions {
                absolute_tolerance: 1.0e-12,
                relative_tolerance: 1.0e-12,
                maximum_intervals: 1_000,
            },
            |points| Ok::<_, ()>(real_values(points, |point| (point - split).abs())),
        )
        .unwrap();
        let expected = 1.0 + split * split;
        assert!((integrated.value.re - expected).abs() <= 5.0e-13);
        assert!(integrated.intervals > 1);
        assert_eq!(
            integrated.evaluations,
            RULE_EVALUATIONS * (2 * integrated.intervals - 1)
        );
    }

    #[test]
    fn waypoints_seed_one_global_partition_and_resolve_a_known_corner() {
        let split = 0.123_456_789;
        let options = AdaptiveQuadratureOptions {
            absolute_tolerance: 1.0e-12,
            relative_tolerance: 0.0,
            maximum_intervals: 2,
        };
        let without_waypoint = adaptive_gauss_kronrod(0.0, 1.0, options, |points| {
            Ok::<_, ()>(real_values(points, |point| (point - split).abs()))
        });
        assert!(matches!(
            without_waypoint,
            Err(AdaptiveQuadratureError::IntervalLimit { maximum: 2, .. })
        ));

        let with_waypoint = adaptive_gauss_kronrod_with_waypoints(
            0.0,
            1.0,
            &[2.0, 0.75, split, split, -1.0],
            options,
            |points| Ok::<_, ()>(real_values(points, |point| (point - split).abs())),
        )
        .unwrap();
        let expected = 0.5 * split * split + 0.5 * (1.0 - split).powi(2);
        assert!((with_waypoint.value.re - expected).abs() <= 1.0e-13);
        assert_eq!(with_waypoint.intervals, 3);
        assert_eq!(with_waypoint.evaluations, 3 * RULE_EVALUATIONS);

        let reverse =
            adaptive_gauss_kronrod_with_waypoints(1.0, 0.0, &[0.75, split], options, |points| {
                Ok::<_, ()>(real_values(points, |point| (point - split).abs()))
            })
            .unwrap();
        assert!((reverse.value.re + expected).abs() <= 1.0e-13);
        assert_eq!(reverse.intervals, 3);

        let invalid =
            adaptive_gauss_kronrod_with_waypoints(0.0, 1.0, &[f64::NAN], options, |points| {
                Ok::<_, ()>(real_values(points, f64::sin))
            })
            .unwrap_err();
        assert!(matches!(
            invalid,
            AdaptiveQuadratureError::NonFiniteWaypoint { index: 0 }
        ));
    }

    #[test]
    fn reversed_and_empty_intervals_preserve_orientation_without_sampling_empty_ranges() {
        let forward =
            adaptive_gauss_kronrod(0.0, 2.0, AdaptiveQuadratureOptions::default(), |points| {
                Ok::<_, ()>(real_values(points, |point| point * point))
            })
            .unwrap();
        let reverse =
            adaptive_gauss_kronrod(2.0, 0.0, AdaptiveQuadratureOptions::default(), |points| {
                Ok::<_, ()>(real_values(points, |point| point * point))
            })
            .unwrap();
        assert!((forward.value + reverse.value).norm() <= 1.0e-14);

        let empty = adaptive_gauss_kronrod(
            4.0,
            4.0,
            AdaptiveQuadratureOptions::default(),
            |_points| -> Result<Vec<Complex64>, ()> { panic!("empty interval must not evaluate") },
        )
        .unwrap();
        assert_eq!(empty.value, Complex64::new(0.0, 0.0));
        assert_eq!(empty.evaluations, 0);
    }

    #[test]
    fn malformed_nonfinite_and_exhausted_evaluations_are_structured() {
        let wrong_count =
            adaptive_gauss_kronrod(0.0, 1.0, AdaptiveQuadratureOptions::default(), |_points| {
                Ok::<_, ()>(Vec::new())
            })
            .unwrap_err();
        assert!(matches!(
            wrong_count,
            AdaptiveQuadratureError::ValueCount {
                expected: RULE_EVALUATIONS,
                actual: 0
            }
        ));

        let non_finite =
            adaptive_gauss_kronrod(0.0, 1.0, AdaptiveQuadratureOptions::default(), |points| {
                Ok::<_, ()>(real_values(points, |_| f64::NAN))
            })
            .unwrap_err();
        assert!(matches!(
            non_finite,
            AdaptiveQuadratureError::NonFiniteValue { .. }
        ));

        let limited = adaptive_gauss_kronrod(
            -1.0,
            1.0,
            AdaptiveQuadratureOptions {
                absolute_tolerance: 1.0e-30,
                relative_tolerance: 0.0,
                maximum_intervals: 1,
            },
            |points| Ok::<_, ()>(real_values(points, f64::abs)),
        )
        .unwrap_err();
        assert!(matches!(
            limited,
            AdaptiveQuadratureError::IntervalLimit { maximum: 1, .. }
        ));
    }

    #[test]
    fn quadgk_options_use_r2022b_double_defaults_and_validate_overrides() {
        let defaults = quadgk_options(&[]).unwrap();
        assert_eq!(
            defaults,
            QuadgkOptions {
                quadrature: AdaptiveQuadratureOptions {
                    absolute_tolerance: 1.0e-10,
                    relative_tolerance: 1.0e-6,
                    maximum_intervals: 650,
                },
                waypoints: Vec::new(),
            }
        );

        let overridden = quadgk_options(&[
            Value::from("RELTOL"),
            Value::Double(0.0),
            Value::from("AbsTol"),
            Value::Double(1.0e-12),
            Value::from("MaxIntervalCount"),
            Value::Double(42.0),
        ])
        .unwrap();
        assert!(overridden.quadrature.relative_tolerance.abs() <= f64::EPSILON);
        assert!((overridden.quadrature.absolute_tolerance - 1.0e-12).abs() <= f64::EPSILON);
        assert_eq!(overridden.quadrature.maximum_intervals, 42);

        let unsupported = quadgk_options(&[Value::from("Trace"), Value::Double(1.0)]).unwrap_err();
        assert_eq!(unsupported.category, BuiltinErrorCategory::Domain);
        assert_eq!(
            unsupported.identifier.as_deref(),
            Some("OpenMat:quadgk:UnsupportedOption")
        );

        for invalid in [
            vec![Value::from("AbsTol"), Value::Double(-1.0)],
            vec![
                Value::from("AbsTol"),
                Value::Double(0.0),
                Value::from("RelTol"),
                Value::Double(0.0),
            ],
            vec![Value::from("RelTol"), Value::Double(f64::EPSILON)],
        ] {
            let error = quadgk_options(&invalid).unwrap_err();
            assert_eq!(
                error.identifier.as_deref(),
                Some("OpenMat:quadgk:InvalidTolerance")
            );
        }

        for invalid in [-1.0, 0.0, 1.5, f64::INFINITY] {
            let error = quadgk_options(&[Value::from("MaxIntervalCount"), Value::Double(invalid)])
                .unwrap_err();
            assert_eq!(
                error.identifier.as_deref(),
                Some("MATLAB:quadgk:invalidMaxIntervalCount")
            );
        }
    }

    #[test]
    fn quadgk_waypoint_options_accept_vectors_and_reject_invalid_shapes_or_values() {
        let row = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([1, 3]).unwrap(), vec![0.75, 0.25, 0.25]).unwrap(),
        ));
        let parsed = quadgk_options(&[Value::from("Waypoints"), row]).unwrap();
        assert_eq!(
            parsed.waypoints,
            [
                Complex64::new(0.75, 0.0),
                Complex64::new(0.25, 0.0),
                Complex64::new(0.25, 0.0),
            ]
        );

        let matrix = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([2, 2]).unwrap(), vec![0.1, 0.2, 0.3, 0.4]).unwrap(),
        ));
        let not_vector = quadgk_options(&[Value::from("Waypoints"), matrix]).unwrap_err();
        assert_eq!(
            not_vector.identifier.as_deref(),
            Some("MATLAB:quadgk:WaypointsNotVector")
        );

        let non_finite =
            quadgk_options(&[Value::from("Waypoints"), Value::Double(f64::INFINITY)]).unwrap_err();
        assert_eq!(
            non_finite.identifier.as_deref(),
            Some("MATLAB:quadgk:WaypointsNotFinite")
        );

        let complex = quadgk_options(&[
            Value::from("Waypoints"),
            Value::Complex(ValueComplex64::new(0.5, 1.0)),
        ])
        .unwrap();
        assert_eq!(complex.waypoints, [Complex64::new(0.5, 1.0)]);
    }

    #[test]
    fn infinite_domains_transform_waypoints_and_preserve_direction() {
        let domains = [
            (
                QuadgkDomain::from_bounds(2.0, f64::INFINITY).unwrap(),
                11.0,
                1.0,
            ),
            (
                QuadgkDomain::from_bounds(f64::INFINITY, 2.0).unwrap(),
                11.0,
                -1.0,
            ),
            (
                QuadgkDomain::from_bounds(f64::NEG_INFINITY, -2.0).unwrap(),
                -11.0,
                1.0,
            ),
            (
                QuadgkDomain::from_bounds(-2.0, f64::NEG_INFINITY).unwrap(),
                -11.0,
                -1.0,
            ),
            (
                QuadgkDomain::from_bounds(f64::NEG_INFINITY, f64::INFINITY).unwrap(),
                3.0,
                1.0,
            ),
            (
                QuadgkDomain::from_bounds(f64::INFINITY, f64::NEG_INFINITY).unwrap(),
                3.0,
                -1.0,
            ),
        ];
        for (domain, waypoint, direction) in domains {
            let transformed = domain.transform_waypoint(waypoint).unwrap();
            let (round_trip, jacobian) = domain.transformed_sample(transformed);
            assert!((round_trip.re - waypoint).abs() <= 32.0 * f64::EPSILON * waypoint.abs());
            assert!(round_trip.im.abs() <= f64::EPSILON);
            assert!((jacobian.re.signum() - direction).abs() <= f64::EPSILON);
            assert!(jacobian.im.abs() <= f64::EPSILON);
            let (start, end) = domain.integration_bounds();
            assert!(transformed > start && transformed < end);
        }

        let right = QuadgkDomain::from_bounds(2.0, f64::INFINITY).unwrap();
        assert_eq!(right.transform_waypoint(2.0), None);
        assert_eq!(right.transform_waypoint(-1.0), None);
        let left = QuadgkDomain::from_bounds(f64::NEG_INFINITY, -2.0).unwrap();
        assert_eq!(left.transform_waypoint(-2.0), None);
        assert_eq!(left.transform_waypoint(1.0), None);
        assert_eq!(
            QuadgkDomain::from_bounds(f64::INFINITY, f64::INFINITY),
            None
        );
        assert_eq!(
            QuadgkDomain::from_bounds(f64::NEG_INFINITY, f64::NEG_INFINITY),
            None
        );
    }

    #[test]
    fn complex_contours_preserve_order_remove_adjacent_duplicates_and_require_finite_vertices() {
        let waypoints = [
            Complex64::new(0.0, 1.0),
            Complex64::new(0.0, 1.0),
            Complex64::new(-1.0, 0.0),
            Complex64::new(0.0, -1.0),
        ];
        let domain = QuadgkDomain::from_path(
            Complex64::new(1.0, 0.0),
            Complex64::new(1.0, 0.0),
            &waypoints,
        )
        .unwrap()
        .unwrap();
        assert!(domain.is_contour());
        assert_eq!(domain.integration_bounds(), (0.0, 4.0));
        assert_eq!(
            domain.transform_waypoints(&waypoints).unwrap(),
            [1.0, 2.0, 3.0]
        );
        let (point, jacobian) = domain.transformed_sample(0.5);
        assert!((point - Complex64::new(0.5, 0.5)).norm() <= f64::EPSILON);
        assert!((jacobian - Complex64::new(-1.0, 1.0)).norm() <= f64::EPSILON);

        let empty =
            QuadgkDomain::from_path(Complex64::new(1.0, 1.0), Complex64::new(1.0, 1.0), &[])
                .unwrap()
                .unwrap();
        assert!(!empty.is_contour());
        assert_eq!(empty.integration_bounds(), (0.0, 0.0));

        let error = QuadgkDomain::from_path(
            Complex64::new(0.0, 0.0),
            Complex64::new(f64::INFINITY, 0.0),
            &[Complex64::new(0.0, 1.0)],
        )
        .unwrap_err();
        assert_eq!(
            error.identifier.as_deref(),
            Some("MATLAB:quadgk:nonFiniteContourError")
        );
    }
}
