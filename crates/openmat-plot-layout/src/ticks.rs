use crate::limits::{nice_step, nice_step_r2022b};
use crate::{AxisDirection, AxisScale, LayoutError, Limits, LocalViewLimits, SemanticLimits};

const MAX_SAFE_INDEX: f64 = 9_007_199_254_740_991.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TickDomain {
    Semantic(SemanticLimits),
    /// Semantic domain with non-linear mapping or reversed presentation.
    SemanticScaled {
        limits: SemanticLimits,
        scale: AxisScale,
        direction: AxisDirection,
    },
    /// Renderer-local navigation is deliberately outside the MATLAB compatibility surface.
    RendererLocal(LocalViewLimits),
}

impl TickDomain {
    #[must_use]
    pub const fn limits(self) -> Limits {
        match self {
            Self::Semantic(limits) | Self::SemanticScaled { limits, .. } => limits.get(),
            Self::RendererLocal(limits) => limits.get(),
        }
    }

    #[must_use]
    pub const fn scale(self) -> AxisScale {
        match self {
            Self::Semantic(_) | Self::RendererLocal(_) => AxisScale::Linear,
            Self::SemanticScaled { scale, .. } => scale,
        }
    }

    /// Maps one visible tick to an axes-local coordinate.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when the value is outside a logarithmic domain
    /// or cannot be represented as a normalized coordinate.
    pub fn normalize(self, value: f64) -> Result<f32, LayoutError> {
        let limits = self.limits();
        let scale = self.scale();
        let map = |value: f64| match scale {
            AxisScale::Linear if value.is_finite() => Ok(value),
            AxisScale::Log10 if value.is_finite() && value > 0.0 => Ok(value.log10()),
            _ => Err(LayoutError::NonFiniteInput("tick coordinate")),
        };
        let lower = map(limits.lower())?;
        let upper = map(limits.upper())?;
        let normalized = (map(value)? - lower) / (upper - lower);
        let reversed = matches!(
            self,
            Self::SemanticScaled {
                direction: AxisDirection::Reverse,
                ..
            }
        );
        let normalized = if reversed {
            1.0 - normalized
        } else {
            normalized
        };
        if !normalized.is_finite() || !(0.0..=1.0).contains(&normalized) {
            return Err(LayoutError::ArithmeticOverflow("tick axes coordinate"));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(normalized as f32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label_code_units: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TickSet {
    pub step: f64,
    pub ticks: Vec<Tick>,
}

/// Selects and formats deterministic linear ticks for an explicit semantic or local domain.
///
/// # Errors
/// Returns [`LayoutError`] for an invalid target or unrepresentable tick range/count.
pub fn linear_ticks(domain: TickDomain, target_count: u32) -> Result<TickSet, LayoutError> {
    linear_ticks_with_step(domain, target_count, nice_step, false)
}

/// Generates base-10 major ticks for a logarithmic semantic domain.
///
/// # Errors
/// Returns [`LayoutError`] for a non-logarithmic/invalid domain, invalid target,
/// or an unrepresentable exponent range.
pub fn log_ticks(domain: TickDomain, target_count: u32) -> Result<TickSet, LayoutError> {
    if target_count < 2 || domain.scale() != AxisScale::Log10 {
        return Err(LayoutError::InvalidTickTarget);
    }
    let limits = domain.limits();
    if limits.lower() <= 0.0 {
        return Err(LayoutError::NonFiniteInput("logarithmic tick domain"));
    }
    let first = limits.lower().log10().ceil();
    let last = limits.upper().log10().floor();
    if !first.is_finite() || !last.is_finite() {
        return Err(LayoutError::ArithmeticOverflow("logarithmic tick range"));
    }
    let count = (last - first).max(0.0) + 1.0;
    let stride = (count / f64::from(target_count)).ceil().max(1.0);
    let mut ticks = Vec::new();
    let mut exponent = first;
    while exponent <= last {
        let value = 10.0_f64.powf(exponent);
        if value.is_finite() {
            ticks.push(Tick {
                value,
                label_code_units: format!("10^{{{exponent:.0}}}").encode_utf16().collect(),
            });
        }
        exponent += stride;
    }
    Ok(TickSet {
        step: stride,
        ticks,
    })
}

/// Selects MATLAB R2022b-style linear ticks using the observed 1/2/5 decade
/// sequence. This is kept separate from the general 1/2/2.5/5 layout policy.
///
/// # Errors
/// Returns [`LayoutError`] for an invalid target or unrepresentable tick range/count.
pub fn linear_ticks_r2022b(domain: TickDomain, target_count: u32) -> Result<TickSet, LayoutError> {
    linear_ticks_with_step(domain, target_count, nice_step_r2022b, true)
}

fn linear_ticks_with_step(
    domain: TickDomain,
    target_count: u32,
    resolve_step: fn(f64) -> Option<f64>,
    add_from_first: bool,
) -> Result<TickSet, LayoutError> {
    if target_count < 2 {
        return Err(LayoutError::InvalidTickTarget);
    }
    let limits = domain.limits();
    let raw_step = limits.span() / f64::from(target_count - 1);
    let step = resolve_step(raw_step).ok_or(LayoutError::ArithmeticOverflow("tick step"))?;
    let first_index = (limits.lower() / step).ceil();
    let last_index = (limits.upper() / step).floor();
    if !first_index.is_finite()
        || !last_index.is_finite()
        || first_index.abs() > MAX_SAFE_INDEX
        || last_index.abs() > MAX_SAFE_INDEX
    {
        return Err(LayoutError::ArithmeticOverflow("tick range"));
    }
    let approximate_count = (last_index - first_index).max(0.0) + 1.0;
    if approximate_count > 100_000.0 {
        return Err(LayoutError::ArithmeticOverflow("tick count"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let capacity = approximate_count as usize;
    let mut ticks = Vec::with_capacity(capacity);
    let mut index = first_index;
    let first_value = first_index * step;
    let mut offset = 0.0;
    while index <= last_index {
        let mut value = if add_from_first {
            first_value + offset * step
        } else {
            index * step
        };
        if value == 0.0 {
            value = 0.0;
        }
        if value.is_finite() && value >= limits.lower() && value <= limits.upper() {
            let label = format_tick(value, step);
            ticks.push(Tick {
                value,
                label_code_units: label.encode_utf16().collect(),
            });
        }
        index += 1.0;
        offset += 1.0;
    }
    Ok(TickSet { step, ticks })
}

/// Builds visible ticks from exact language-level values. Non-finite and
/// offscreen values retain their property position but produce no overlay
/// tick. Manual labels cycle when shorter and are truncated when longer.
///
/// # Errors
/// Returns [`LayoutError::InvalidTickSequence`] for NaN, descending, or
/// duplicate values.
pub fn explicit_ticks(
    domain: TickDomain,
    values: &[f64],
    manual_labels: Option<&[Vec<u16>]>,
) -> Result<TickSet, LayoutError> {
    if values.iter().any(|value| value.is_nan()) || values.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(LayoutError::InvalidTickSequence);
    }
    let limits = domain.limits();
    let step = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>()
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|difference| difference.is_finite() && *difference > 0.0)
        .fold(f64::INFINITY, f64::min);
    let step = if step.is_finite() {
        step
    } else {
        limits.span()
    };
    let ticks = values
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, value)| {
            value.is_finite() && *value >= limits.lower() && *value <= limits.upper()
        })
        .map(|(index, value)| {
            let label_code_units = match manual_labels {
                Some([]) => Vec::new(),
                Some(labels) => labels[index % labels.len()].clone(),
                None if domain.scale() == AxisScale::Log10 => {
                    format!("10^{{{:.0}}}", value.log10().round())
                        .encode_utf16()
                        .collect()
                }
                None => format_tick(value, step).encode_utf16().collect(),
            };
            Tick {
                value,
                label_code_units,
            }
        })
        .collect();
    Ok(TickSet { step, ticks })
}

fn format_tick(value: f64, step: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    if !(1.0e-4..1.0e7).contains(&magnitude) {
        let precision = scientific_precision(magnitude, step);
        return normalize_exponent(format!("{value:.precision$e}"));
    }
    let precision = decimal_precision(step).min(15);
    let mut label = format!("{value:.precision$}");
    if label.contains('.') {
        while label.ends_with('0') {
            label.pop();
        }
        if label.ends_with('.') {
            label.pop();
        }
    }
    if label == "-0" { "0".to_owned() } else { label }
}

fn decimal_precision(step: f64) -> usize {
    if step >= 1.0 {
        0
    } else {
        let mut multiplier = 1.0;
        for precision in 0..=15 {
            let scaled = step * multiplier;
            if (scaled - scaled.round()).abs() <= f64::EPSILON * scaled.abs().max(1.0) {
                return precision;
            }
            multiplier *= 10.0;
        }
        15
    }
}

fn scientific_precision(magnitude: f64, step: f64) -> usize {
    let mut ratio = magnitude / step.abs();
    let mut precision = 0;
    while ratio > 1.0 && precision < 15 {
        ratio /= 10.0;
        precision += 1;
    }
    precision
}

fn normalize_exponent(mut label: String) -> String {
    if let Some(position) = label.find('e') {
        let exponent = label.split_off(position + 1);
        label.pop();
        let parsed = exponent.parse::<i32>().unwrap_or(0);
        format!("{label}e{parsed}")
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_deterministic_ordered_and_canonicalize_zero() {
        let limits = SemanticLimits::new(Limits::new(-1.0, 1.0).unwrap());
        let first = linear_ticks(TickDomain::Semantic(limits), 5).unwrap();
        let second = linear_ticks(TickDomain::Semantic(limits), 5).unwrap();
        assert_eq!(first, second);
        assert!(
            first
                .ticks
                .windows(2)
                .all(|pair| pair[0].value < pair[1].value)
        );
        let zero = first.ticks.iter().find(|tick| tick.value == 0.0).unwrap();
        assert_eq!(String::from_utf16(&zero.label_code_units).unwrap(), "0");
        assert!(!zero.value.is_sign_negative());
    }

    #[test]
    fn local_navigation_is_an_explicit_tick_domain() {
        let local = LocalViewLimits::new(Limits::new(10.0, 20.0).unwrap());
        let ticks = linear_ticks(TickDomain::RendererLocal(local), 6).unwrap();
        assert!(
            ticks
                .ticks
                .iter()
                .all(|tick| (10.0..=20.0).contains(&tick.value))
        );
    }

    #[test]
    fn logarithmic_ticks_use_decades_and_respect_reverse_direction() {
        let domain = TickDomain::SemanticScaled {
            limits: SemanticLimits::new(Limits::new(1.0, 1_000.0).unwrap()),
            scale: AxisScale::Log10,
            direction: AxisDirection::Reverse,
        };
        let ticks = log_ticks(domain, 4).unwrap();
        assert_eq!(
            ticks
                .ticks
                .iter()
                .map(|tick| tick.value)
                .collect::<Vec<_>>(),
            [1.0, 10.0, 100.0, 1_000.0]
        );
        assert!((domain.normalize(1.0).unwrap() - 1.0).abs() < f32::EPSILON);
        assert!((domain.normalize(10.0).unwrap() - 2.0_f32 / 3.0).abs() < f32::EPSILON);
        assert!(domain.normalize(1_000.0).unwrap().abs() < f32::EPSILON);
    }

    #[test]
    fn r2022b_ticks_use_the_observed_one_two_five_decade_sequence() {
        let limits = SemanticLimits::new(
            Limits::new(-0.999_999_251_732_804, 0.999_999_114_134_739_9).unwrap(),
        );
        let tall = linear_ticks_r2022b(TickDomain::Semantic(limits), 9).unwrap();
        assert_eq!(
            tall.ticks.iter().map(|tick| tick.value).collect::<Vec<_>>(),
            vec![
                -0.8,
                -0.600_000_000_000_000_1,
                -0.4,
                -0.199_999_999_999_999_96,
                0.0,
                0.199_999_999_999_999_96,
                0.400_000_000_000_000_13,
                0.600_000_000_000_000_1,
                0.8,
            ]
        );
        let short = linear_ticks_r2022b(TickDomain::Semantic(limits), 8).unwrap();
        assert_eq!(
            short
                .ticks
                .iter()
                .map(|tick| tick.value)
                .collect::<Vec<_>>(),
            vec![-0.5, 0.0, 0.5]
        );
    }

    #[test]
    fn large_offset_small_step_labels_remain_distinct() {
        let limits = SemanticLimits::new(Limits::new(1.0e12, 1.0e12 + 2.0).unwrap());
        let ticks = linear_ticks(TickDomain::Semantic(limits), 5).unwrap();
        let labels: Vec<String> = ticks
            .ticks
            .iter()
            .map(|tick| String::from_utf16(&tick.label_code_units).unwrap())
            .collect();
        assert!(labels.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn explicit_ticks_filter_non_visible_values_and_cycle_labels_by_source_position() {
        let domain = TickDomain::Semantic(SemanticLimits::new(Limits::new(0.0, 10.0).unwrap()));
        let labels = vec![
            "A".encode_utf16().collect::<Vec<_>>(),
            "B".encode_utf16().collect::<Vec<_>>(),
        ];
        let ticks = explicit_ticks(
            domain,
            &[f64::NEG_INFINITY, 0.0, 5.0, 11.0, f64::INFINITY],
            Some(&labels),
        )
        .unwrap();
        assert_eq!(
            ticks
                .ticks
                .iter()
                .map(|tick| (tick.value, String::from_utf16_lossy(&tick.label_code_units)))
                .collect::<Vec<_>>(),
            vec![(0.0, "B".to_owned()), (5.0, "A".to_owned())]
        );
        assert_eq!(
            explicit_ticks(domain, &[0.0, 0.0], None),
            Err(LayoutError::InvalidTickSequence)
        );
    }
}
