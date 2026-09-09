use std::collections::{HashMap, HashSet};

use openmat_plot_mir::{
    CssLineSegment, CssPoint, CssPx, CssRect, CssSize, FontWeight, HorizontalAlignment,
    LegendEntry, LegendPlan, OverlayFont, OverlayFontStyle, OverlayPlan, OverlayText,
    OverlayTextRole, Rgba, TextInterpreter, Utf16Text, VerticalAlignment,
};

use crate::LayoutError;

#[derive(Clone, Debug, PartialEq)]
pub struct FontKey {
    pub family_utf16: Vec<u16>,
    pub size_css_px: CssPx,
    pub weight: FontWeight,
    pub style: OverlayFontStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextMeasureRequest {
    pub key: String,
    pub code_units: Vec<u16>,
    pub font: FontKey,
    pub font_revision: u64,
    pub interpreter: TextInterpreter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextMetrics {
    pub width_css_px: CssPx,
    pub height_css_px: CssPx,
    pub ascent_css_px: CssPx,
    pub descent_css_px: CssPx,
    pub advance_css_px: CssPx,
}

impl TextMetrics {
    /// Creates measured CSS-pixel font metrics.
    ///
    /// # Errors
    /// Returns [`LayoutError`] for negative, non-finite, or inconsistent metrics.
    pub fn new(
        width: f32,
        height: f32,
        ascent: f32,
        descent: f32,
        advance: f32,
    ) -> Result<Self, LayoutError> {
        if ascent + descent > height + f32::EPSILON {
            return Err(LayoutError::NonFiniteInput("font vertical metrics"));
        }
        Ok(Self {
            width_css_px: css_px(width)?,
            height_css_px: css_px(height)?,
            ascent_css_px: css_px(ascent)?,
            descent_css_px: css_px(descent)?,
            advance_css_px: css_px(advance)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextMeasurement {
    pub key: String,
    pub metrics: TextMetrics,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextIntent {
    pub key: String,
    pub role: OverlayTextRole,
    pub text: Utf16Text,
    pub anchor: CssPoint,
    pub horizontal_alignment: HorizontalAlignment,
    pub vertical_alignment: VerticalAlignment,
    pub rotation_radians: f32,
    pub color: Rgba,
    pub font: FontKey,
    pub interpreter: TextInterpreter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegendPlacement {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
    SouthOutside,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntryIntent {
    pub key: String,
    pub label: Utf16Text,
    pub color: Rgba,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendIntent {
    pub key: String,
    pub placement: LegendPlacement,
    pub entries: Vec<LegendEntryIntent>,
    pub background: Rgba,
    pub border: Rgba,
    pub font: FontKey,
    pub interpreter: TextInterpreter,
    pub columns: usize,
}

/// First-pass output. `font_cache_revision` invalidates host-side font caches.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstLayoutPass {
    pub font_cache_revision: u64,
    pub requests: Vec<TextMeasureRequest>,
    base_overlay: OverlayPlan,
    intents: Vec<TextIntent>,
    legend_placement: Option<LegendPlacement>,
    legend_columns: Option<usize>,
    position_axes_text: bool,
}

impl FirstLayoutPass {
    #[must_use]
    pub fn new(viewport: CssRect, font_cache_revision: u64, intents: Vec<TextIntent>) -> Self {
        Self::from_overlay(
            OverlayPlan {
                viewport_css: viewport,
                ..OverlayPlan::default()
            },
            font_cache_revision,
            intents,
        )
    }

    #[must_use]
    pub fn from_overlay(
        base_overlay: OverlayPlan,
        font_cache_revision: u64,
        intents: Vec<TextIntent>,
    ) -> Self {
        let requests = intents
            .iter()
            .map(|intent| TextMeasureRequest {
                key: intent.key.clone(),
                code_units: intent.text.code_units.clone(),
                font: intent.font.clone(),
                font_revision: font_cache_revision,
                interpreter: intent.interpreter,
            })
            .collect();
        Self {
            font_cache_revision,
            requests,
            base_overlay,
            intents,
            legend_placement: None,
            legend_columns: None,
            position_axes_text: true,
        }
    }

    /// Keeps caller-provided title/axis-label anchors. This is used by projected
    /// 3D layouts, whose anchors already depend on the active camera.
    #[must_use]
    pub fn preserve_axes_text_anchors(mut self) -> Self {
        self.position_axes_text = false;
        self
    }

    /// Adds a measured, viewport-relative legend to this layout pass.
    #[must_use]
    pub fn with_legend(mut self, legend: LegendIntent) -> Self {
        let viewport = self.base_overlay.viewport_css;
        let placeholder = CssPoint {
            x: viewport.right(),
            y: viewport.origin.y,
        };
        for entry in &legend.entries {
            let intent = TextIntent {
                key: entry.key.clone(),
                role: OverlayTextRole::LegendLabel,
                text: entry.label.clone(),
                anchor: placeholder,
                horizontal_alignment: HorizontalAlignment::Start,
                vertical_alignment: VerticalAlignment::Middle,
                rotation_radians: 0.0,
                color: legend.border,
                font: legend.font.clone(),
                interpreter: legend.interpreter,
            };
            self.requests.push(TextMeasureRequest {
                key: intent.key.clone(),
                code_units: intent.text.code_units.clone(),
                font: intent.font.clone(),
                font_revision: self.font_cache_revision,
                interpreter: intent.interpreter,
            });
            self.intents.push(intent);
        }
        self.base_overlay.legend = Some(LegendPlan {
            key: legend.key,
            bounds: CssRect::default(),
            entries: legend
                .entries
                .into_iter()
                .map(|entry| LegendEntry {
                    key: entry.key,
                    label: entry.label,
                    swatch: CssLineSegment {
                        start: placeholder,
                        end: placeholder,
                    },
                    color: entry.color,
                })
                .collect(),
            background: legend.background,
            border: legend.border,
            font: OverlayFont {
                family_utf16: legend.font.family_utf16,
                size_css_px: legend.font.size_css_px,
                weight: legend.font.weight,
                style: legend.font.style,
            },
            interpreter: legend.interpreter,
        });
        self.legend_placement = Some(legend.placement);
        self.legend_columns = Some(legend.columns.max(1));
        self
    }

    /// Returns the positioned non-text overlay primitives produced by the
    /// first pass. Browser adapters use this together with [`Self::intents`]
    /// while host font measurements are pending.
    #[must_use]
    pub const fn base_overlay(&self) -> &OverlayPlan {
        &self.base_overlay
    }

    /// Returns the positioned text intents whose metrics must be supplied to
    /// [`Self::complete`].
    #[must_use]
    pub fn intents(&self) -> &[TextIntent] {
        &self.intents
    }

    /// Applies an exact set of host measurements to produce the second-pass overlay plan.
    ///
    /// # Errors
    /// Returns [`LayoutError`] for missing, duplicate, or unexpected measurement keys.
    pub fn complete(
        mut self,
        measurements: Vec<TextMeasurement>,
    ) -> Result<OverlayPlan, LayoutError> {
        let expected: HashSet<&str> = self
            .requests
            .iter()
            .map(|request| request.key.as_str())
            .collect();
        let mut by_key = HashMap::with_capacity(measurements.len());
        for measurement in measurements {
            if !expected.contains(measurement.key.as_str()) {
                return Err(LayoutError::UnexpectedTextMeasurement(measurement.key));
            }
            let key = measurement.key.clone();
            if by_key.insert(key.clone(), measurement.metrics).is_some() {
                return Err(LayoutError::DuplicateTextMeasurement(key));
            }
        }
        if self.position_axes_text {
            position_axes_text(&mut self.intents, &self.base_overlay, &by_key)?;
        }
        position_legend(
            &mut self.intents,
            &mut self.base_overlay,
            self.legend_placement,
            self.legend_columns,
            &by_key,
        )?;
        let mut text = Vec::with_capacity(self.intents.len());
        for intent in self.intents {
            let metrics = by_key
                .remove(&intent.key)
                .ok_or_else(|| LayoutError::MissingTextMeasurement(intent.key.clone()))?;
            text.push(OverlayText {
                key: intent.key,
                role: intent.role,
                text: intent.text,
                anchor: intent.anchor,
                horizontal_alignment: intent.horizontal_alignment,
                vertical_alignment: intent.vertical_alignment,
                rotation_radians: intent.rotation_radians,
                color: intent.color,
                font: OverlayFont {
                    family_utf16: intent.font.family_utf16,
                    size_css_px: intent.font.size_css_px,
                    weight: intent.font.weight,
                    style: intent.font.style,
                },
                interpreter: intent.interpreter,
                measured_size: CssSize {
                    width: metrics.width_css_px,
                    height: metrics.height_css_px,
                },
            });
        }
        let mut overlay = self.base_overlay;
        overlay.text = text;
        overlay
            .validate()
            .map_err(|error| LayoutError::InvalidOverlay(error.to_string()))?;
        Ok(overlay)
    }
}

fn position_axes_text(
    intents: &mut [TextIntent],
    overlay: &OverlayPlan,
    metrics: &HashMap<String, TextMetrics>,
) -> Result<(), LayoutError> {
    let viewport = overlay.viewport_css;
    let x_tick_height = overlay
        .ticks
        .iter()
        .filter(|tick| tick.axis == openmat_plot_mir::AxisDimension::X)
        .filter_map(|tick| metrics.get(&tick.label_key))
        .map(|value| value.height_css_px.get())
        .fold(0.0_f32, f32::max);
    let y_tick_width = overlay
        .ticks
        .iter()
        .filter(|tick| tick.axis == openmat_plot_mir::AxisDimension::Y)
        .filter_map(|tick| metrics.get(&tick.label_key))
        .map(|value| value.width_css_px.get())
        .fold(0.0_f32, f32::max);
    let polar_north_tick_y = intents
        .iter()
        .filter(|intent| {
            intent.key.starts_with("theta-tick-") && !intent.text.code_units.is_empty()
        })
        .map(|intent| intent.anchor.y)
        .min_by(f32::total_cmp);
    for intent in intents {
        match intent.role {
            OverlayTextRole::Title => {
                intent.anchor.y = viewport.origin.y - 8.0;
                if let Some(north_tick_y) = polar_north_tick_y {
                    let title_height = metrics
                        .get(&intent.key)
                        .map_or(intent.font.size_css_px.get(), |value| {
                            value.height_css_px.get()
                        });
                    intent.anchor.y = intent.anchor.y.min(north_tick_y - title_height - 8.0);
                }
                intent.vertical_alignment = VerticalAlignment::Bottom;
            }
            OverlayTextRole::XLabel => {
                intent.anchor.x = viewport.origin.x + viewport.size.width.get() / 2.0;
                intent.anchor.y = viewport.bottom() + x_tick_height + 16.0;
                intent.horizontal_alignment = HorizontalAlignment::Center;
                intent.vertical_alignment = VerticalAlignment::Top;
            }
            OverlayTextRole::YLabel => {
                intent.anchor.x = viewport.origin.x - y_tick_width - 18.0;
                intent.anchor.y = viewport.origin.y + viewport.size.height.get() / 2.0;
                intent.horizontal_alignment = HorizontalAlignment::Center;
                intent.vertical_alignment = VerticalAlignment::Middle;
            }
            OverlayTextRole::ZLabel => {
                intent.anchor.x = viewport.right() + 18.0;
                intent.anchor.y = viewport.origin.y + viewport.size.height.get() / 2.0;
                intent.horizontal_alignment = HorizontalAlignment::Center;
                intent.vertical_alignment = VerticalAlignment::Middle;
            }
            OverlayTextRole::TickLabel
            | OverlayTextRole::LegendLabel
            | OverlayTextRole::Annotation => {}
        }
        intent
            .anchor
            .validate()
            .map_err(|_| LayoutError::ArithmeticOverflow("positioned axes text"))?;
    }
    Ok(())
}

fn position_legend(
    intents: &mut [TextIntent],
    overlay: &mut OverlayPlan,
    placement: Option<LegendPlacement>,
    columns: Option<usize>,
    metrics: &HashMap<String, TextMetrics>,
) -> Result<(), LayoutError> {
    let viewport = overlay.viewport_css;
    let Some(legend) = overlay.legend.as_mut() else {
        return Ok(());
    };
    let placement = placement.unwrap_or(LegendPlacement::NorthEast);
    let padding = 8.0_f32;
    let swatch_width = 24.0_f32;
    let swatch_gap = 8.0_f32;
    let row_gap = 5.0_f32;
    let column_gap = 16.0_f32;
    let column_count = columns.unwrap_or(1).max(1).min(legend.entries.len().max(1));
    let row_count = legend.entries.len().div_ceil(column_count);
    let mut column_widths = vec![0.0_f32; column_count];
    for (index, entry) in legend.entries.iter().enumerate() {
        if let Some(value) = metrics.get(&entry.key) {
            column_widths[index % column_count] =
                column_widths[index % column_count].max(value.width_css_px.get());
        }
    }
    let row_height = legend
        .entries
        .iter()
        .filter_map(|entry| metrics.get(&entry.key))
        .map(|value| value.height_css_px.get())
        .fold(legend.font.size_css_px.get(), f32::max);
    #[allow(clippy::cast_precision_loss)]
    let row_count_f32 = row_count as f32;
    #[allow(clippy::cast_precision_loss)]
    let column_count_f32 = column_count as f32;
    let natural_width = padding * 2.0
        + column_widths
            .iter()
            .map(|width| swatch_width + swatch_gap + width)
            .sum::<f32>()
        + (column_count_f32 - 1.0).max(0.0) * column_gap;
    let natural_height =
        padding * 2.0 + row_count_f32 * row_height + (row_count_f32 - 1.0).max(0.0) * row_gap;
    let inset = 10.0_f32
        .min(viewport.size.width.get() / 4.0)
        .min(viewport.size.height.get() / 4.0);
    let width = natural_width.min((viewport.size.width.get() - inset * 2.0).max(1.0));
    let height = natural_height.min((viewport.size.height.get() - inset * 2.0).max(1.0));
    let left = viewport.origin.x + inset;
    let right = viewport.right() - inset - width;
    let top = viewport.origin.y + inset;
    let bottom = viewport.bottom() - inset - height;
    let center_x = viewport.origin.x + (viewport.size.width.get() - width) / 2.0;
    let center_y = viewport.origin.y + (viewport.size.height.get() - height) / 2.0;
    let (x, y) = match placement {
        LegendPlacement::North => (center_x, top),
        LegendPlacement::South => (center_x, bottom),
        LegendPlacement::East => (right, center_y),
        LegendPlacement::West => (left, center_y),
        LegendPlacement::NorthEast => (right, top),
        LegendPlacement::NorthWest => (left, top),
        LegendPlacement::SouthEast => (right, bottom),
        LegendPlacement::SouthWest => (left, bottom),
        LegendPlacement::SouthOutside => (center_x, viewport.bottom() + inset),
    };
    legend.bounds = CssRect::new(x, y, width, height)
        .map_err(|_| LayoutError::ArithmeticOverflow("legend bounds"))?;
    let mut column_offsets = Vec::with_capacity(column_count);
    let mut offset = padding;
    for width in &column_widths {
        column_offsets.push(offset);
        offset += swatch_width + swatch_gap + width + column_gap;
    }
    for (index, entry) in legend.entries.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let row = (index / column_count) as f32;
        let column = index % column_count;
        let column_x = x + column_offsets[column];
        let row_y = y + padding + row_height / 2.0 + row * (row_height + row_gap);
        entry.swatch = CssLineSegment {
            start: CssPoint::new(column_x, row_y)
                .map_err(|_| LayoutError::ArithmeticOverflow("legend swatch"))?,
            end: CssPoint::new(column_x + swatch_width, row_y)
                .map_err(|_| LayoutError::ArithmeticOverflow("legend swatch"))?,
        };
        if let Some(intent) = intents.iter_mut().find(|intent| intent.key == entry.key) {
            intent.anchor = CssPoint::new(column_x + swatch_width + swatch_gap, row_y)
                .map_err(|_| LayoutError::ArithmeticOverflow("legend label"))?;
        }
    }
    Ok(())
}

fn css_px(value: f32) -> Result<CssPx, LayoutError> {
    CssPx::new(value).map_err(|_| LayoutError::NonFiniteInput("font metric"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn intent() -> TextIntent {
        TextIntent {
            key: "title".to_owned(),
            role: OverlayTextRole::Title,
            text: Utf16Text::from_code_units(vec![0xD800]),
            anchor: CssPoint::default(),
            horizontal_alignment: HorizontalAlignment::Center,
            vertical_alignment: VerticalAlignment::Top,
            rotation_radians: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            font: FontKey {
                family_utf16: "sans-serif".encode_utf16().collect(),
                size_css_px: CssPx::new(12.0).unwrap(),
                weight: FontWeight::BOLD,
                style: OverlayFontStyle::Normal,
            },
            interpreter: TextInterpreter::Tex,
        }
    }

    #[test]
    fn two_pass_measurement_preserves_utf16_truth_and_revision() {
        let pass = FirstLayoutPass::new(
            CssRect::new(0.0, 0.0, 100.0, 80.0).unwrap(),
            42,
            vec![intent()],
        );
        assert_eq!(pass.font_cache_revision, 42);
        assert_eq!(pass.requests[0].code_units, vec![0xD800]);
        let overlay = pass
            .complete(vec![TextMeasurement {
                key: "title".to_owned(),
                metrics: TextMetrics::new(10.0, 12.0, 8.0, 3.0, 10.0).unwrap(),
            }])
            .unwrap();
        assert_eq!(overlay.text[0].text.code_units, vec![0xD800]);
        assert_eq!(overlay.text[0].font.weight, FontWeight::BOLD);
        assert_eq!(overlay.text[0].measured_size.width.get(), 10.0);
    }

    #[test]
    fn second_pass_rejects_missing_or_duplicate_measurements() {
        let viewport = CssRect::new(0.0, 0.0, 100.0, 80.0).unwrap();
        let missing = FirstLayoutPass::new(viewport, 0, vec![intent()]).complete(Vec::new());
        assert_eq!(
            missing,
            Err(LayoutError::MissingTextMeasurement("title".to_owned()))
        );

        let metrics = TextMetrics::new(10.0, 12.0, 8.0, 3.0, 10.0).unwrap();
        let duplicate = FirstLayoutPass::new(viewport, 0, vec![intent()]).complete(vec![
            TextMeasurement {
                key: "title".to_owned(),
                metrics: metrics.clone(),
            },
            TextMeasurement {
                key: "title".to_owned(),
                metrics,
            },
        ]);
        assert_eq!(
            duplicate,
            Err(LayoutError::DuplicateTextMeasurement("title".to_owned()))
        );
    }

    #[test]
    fn measured_legend_stays_in_viewport_and_preserves_unicode_labels() {
        let viewport = CssRect::new(10.0, 20.0, 240.0, 120.0).unwrap();
        let pass = FirstLayoutPass::new(viewport, 7, Vec::new()).with_legend(LegendIntent {
            key: "legend-main".to_owned(),
            placement: LegendPlacement::SouthEast,
            entries: vec![
                LegendEntryIntent {
                    key: "legend-label-0".to_owned(),
                    label: Utf16Text::from_code_units("温度 🌡️".encode_utf16().collect()),
                    color: Rgba::new(0.0, 0.45, 0.74, 1.0).unwrap(),
                },
                LegendEntryIntent {
                    key: "legend-label-1".to_owned(),
                    label: Utf16Text::from_code_units("pressure".encode_utf16().collect()),
                    color: Rgba::new(0.85, 0.32, 0.1, 1.0).unwrap(),
                },
            ],
            background: Rgba::new(1.0, 1.0, 1.0, 0.9).unwrap(),
            border: Rgba::new(0.2, 0.2, 0.2, 1.0).unwrap(),
            font: FontKey {
                family_utf16: "Arial".encode_utf16().collect(),
                size_css_px: CssPx::new(11.0).unwrap(),
                weight: FontWeight::NORMAL,
                style: OverlayFontStyle::Normal,
            },
            interpreter: TextInterpreter::Tex,
            columns: 2,
        });
        assert_eq!(pass.requests.len(), 2);
        assert!(
            pass.requests
                .iter()
                .all(|request| request.font_revision == 7)
        );
        let overlay = pass
            .complete(vec![
                TextMeasurement {
                    key: "legend-label-0".to_owned(),
                    metrics: TextMetrics::new(58.0, 14.0, 10.0, 3.0, 58.0).unwrap(),
                },
                TextMeasurement {
                    key: "legend-label-1".to_owned(),
                    metrics: TextMetrics::new(46.0, 14.0, 10.0, 3.0, 46.0).unwrap(),
                },
            ])
            .unwrap();
        let legend = overlay.legend.unwrap();
        assert!(legend.bounds.right() <= viewport.right());
        assert!(legend.bounds.bottom() <= viewport.bottom());
        assert_eq!(legend.entries.len(), 2);
        assert!(legend.entries[0].swatch.start.x < legend.entries[0].swatch.end.x);
        assert_eq!(
            legend.entries[0].swatch.start.y,
            legend.entries[1].swatch.start.y
        );
        assert_eq!(
            overlay.text[0].text.code_units,
            "温度 🌡️".encode_utf16().collect::<Vec<_>>()
        );
    }
}
