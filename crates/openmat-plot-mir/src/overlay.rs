use std::collections::HashSet;

use crate::{CssPoint, CssPx, CssRect, CssSize, MirError, Rgba};

/// Exact MATLAB character data represented as UTF-16 code units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Utf16Text {
    pub code_units: Vec<u16>,
    /// Optional best-effort text for accessibility systems that require Unicode scalars.
    pub lossy_accessibility_text: Option<String>,
}

impl Utf16Text {
    #[must_use]
    pub fn from_code_units(code_units: Vec<u16>) -> Self {
        let lossy_accessibility_text = Some(String::from_utf16_lossy(&code_units));
        Self {
            code_units,
            lossy_accessibility_text,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisDimension {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssLineSegment {
    pub start: CssPoint,
    pub end: CssPoint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayAxisLine {
    pub segment: CssLineSegment,
    pub color: Rgba,
    pub width_css_px: CssPx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalAlignment {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalAlignment {
    Top,
    Middle,
    Baseline,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayTextRole {
    TickLabel,
    Title,
    XLabel,
    YLabel,
    ZLabel,
    LegendLabel,
    Annotation,
}

/// Backend-neutral MATLAB text interpreter intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextInterpreter {
    /// MATLAB TeX subset.
    #[default]
    Tex,
    /// LaTeX math.
    Latex,
    /// Literal text.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontWeight(u16);

impl FontWeight {
    pub const NORMAL: Self = Self(400);
    pub const BOLD: Self = Self(700);

    /// Creates a CSS-compatible font weight.
    ///
    /// # Errors
    /// Returns [`MirError::InvalidFontWeight`] unless `value` is in `1..=1000`.
    pub fn new(value: u16) -> Result<Self, MirError> {
        if (1..=1000).contains(&value) {
            Ok(Self(value))
        } else {
            Err(MirError::InvalidFontWeight)
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverlayFontStyle {
    Normal,
    Italic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayFont {
    pub family_utf16: Vec<u16>,
    pub size_css_px: CssPx,
    pub weight: FontWeight,
    pub style: OverlayFontStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayText {
    pub key: String,
    pub role: OverlayTextRole,
    pub text: Utf16Text,
    pub anchor: CssPoint,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub rotation_radians: f32,
    pub color: Rgba,
    pub font: OverlayFont,
    pub interpreter: TextInterpreter,
    /// Bounds reported by the authoritative layout pass in CSS pixels.
    pub measured_size: CssSize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayTick {
    pub key: String,
    pub axis: AxisDimension,
    pub value: f64,
    pub mark: CssLineSegment,
    pub label_key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    pub key: String,
    pub label: Utf16Text,
    pub swatch: CssLineSegment,
    pub color: Rgba,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendPlan {
    pub key: String,
    pub bounds: CssRect,
    pub entries: Vec<LegendEntry>,
    pub background: Rgba,
    pub border: Rgba,
    pub font: OverlayFont,
    pub interpreter: TextInterpreter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityNode {
    pub key: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayPlan {
    pub viewport_css: CssRect,
    pub axis_lines: Vec<OverlayAxisLine>,
    pub ticks: Vec<OverlayTick>,
    pub text: Vec<OverlayText>,
    pub legend: Option<LegendPlan>,
    pub accessibility: Vec<AccessibilityNode>,
}

impl OverlayPlan {
    /// Validates finite overlay geometry and stable text/tick key relationships.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid coordinates, duplicate keys, or missing tick labels.
    pub fn validate(&self) -> Result<(), MirError> {
        self.viewport_css.validate()?;
        for line in &self.axis_lines {
            line.segment.start.validate()?;
            line.segment.end.validate()?;
            line.color.validate()?;
        }
        let mut text_keys = HashSet::with_capacity(self.text.len());
        for text in &self.text {
            if !text_keys.insert(text.key.as_str()) {
                return Err(MirError::InvalidCommandStream("duplicate overlay text key"));
            }
            text.anchor.validate()?;
            text.color.validate()?;
            if !text.rotation_radians.is_finite() {
                return Err(MirError::NonFinite("overlay text rotation"));
            }
        }
        let mut tick_keys = HashSet::with_capacity(self.ticks.len());
        for tick in &self.ticks {
            if !tick_keys.insert(tick.key.as_str()) {
                return Err(MirError::InvalidCommandStream("duplicate overlay tick key"));
            }
            if !tick.value.is_finite() {
                return Err(MirError::NonFinite("overlay tick value"));
            }
            tick.mark.start.validate()?;
            tick.mark.end.validate()?;
            if !text_keys.contains(tick.label_key.as_str()) {
                return Err(MirError::InvalidCommandStream(
                    "overlay tick label key does not resolve",
                ));
            }
        }
        if let Some(legend) = &self.legend {
            legend.bounds.validate()?;
            legend.background.validate()?;
            legend.border.validate()?;
            for entry in &legend.entries {
                entry.swatch.start.validate()?;
                entry.swatch.end.validate()?;
                entry.color.validate()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_text_preserves_unpaired_surrogates() {
        let text = Utf16Text::from_code_units(vec![0x0061, 0xD800, 0x0062]);
        assert_eq!(text.code_units, vec![0x0061, 0xD800, 0x0062]);
        assert_eq!(text.lossy_accessibility_text.as_deref(), Some("a�b"));
    }

    #[test]
    fn font_weight_has_a_strong_bounded_representation() {
        assert_eq!(FontWeight::new(1).unwrap().get(), 1);
        assert_eq!(FontWeight::new(1000).unwrap().get(), 1000);
        assert_eq!(FontWeight::new(0), Err(MirError::InvalidFontWeight));
        assert_eq!(FontWeight::new(1001), Err(MirError::InvalidFontWeight));
    }

    #[test]
    fn overlay_validation_rejects_missing_tick_label_and_non_finite_rotation() {
        let tick = OverlayTick {
            key: "x-0".to_owned(),
            axis: AxisDimension::X,
            value: 0.0,
            mark: CssLineSegment {
                start: CssPoint::default(),
                end: CssPoint::default(),
            },
            label_key: "missing".to_owned(),
        };
        let mut overlay = OverlayPlan {
            ticks: vec![tick],
            ..OverlayPlan::default()
        };
        assert!(matches!(
            overlay.validate(),
            Err(MirError::InvalidCommandStream(_))
        ));
        overlay.ticks.clear();
        overlay.text.push(OverlayText {
            key: "title".to_owned(),
            role: OverlayTextRole::Title,
            text: Utf16Text::default(),
            anchor: CssPoint::default(),
            horizontal_alignment: HorizontalAlignment::Center,
            vertical_alignment: VerticalAlignment::Top,
            rotation_radians: f32::NAN,
            color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            font: OverlayFont {
                family_utf16: Vec::new(),
                size_css_px: CssPx::new(12.0).unwrap(),
                weight: FontWeight::NORMAL,
                style: OverlayFontStyle::Normal,
            },
            interpreter: TextInterpreter::Tex,
            measured_size: CssSize::default(),
        });
        assert_eq!(
            overlay.validate(),
            Err(MirError::NonFinite("overlay text rotation"))
        );
    }
}
