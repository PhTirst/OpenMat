use openmat_plot_mir::{
    AxesLineSegment, AxesPoint, AxisDimension, CssLineSegment, CssPoint, CssPx, GridLineBatch,
    HorizontalAlignment, OverlayAxisLine, OverlayPlan, OverlayTextRole, OverlayTick, Rgba,
    TextInterpreter, Utf16Text, VerticalAlignment,
};

use crate::{
    FirstLayoutPass, FontKey, LayoutError, TextIntent, TickDomain, TickSet, ViewportMapping,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LinearAxesStyle {
    pub axis_color: Rgba,
    pub grid_color: Rgba,
    pub text_color: Rgba,
    pub axis_width_css_px: CssPx,
    pub grid_width_css_px: CssPx,
    pub tick_length_css_px: CssPx,
    pub label_gap_css_px: CssPx,
    pub tick_font: FontKey,
    pub tick_label_interpreter: TextInterpreter,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TickDirection {
    #[default]
    In,
    Out,
    Both,
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct LinearAxesLayoutInput<'a> {
    pub viewport: ViewportMapping,
    pub x_domain: TickDomain,
    pub y_domain: TickDomain,
    pub x_ticks: &'a TickSet,
    pub y_ticks: &'a TickSet,
    pub grid_x: bool,
    pub grid_y: bool,
    pub minor_grid_x: bool,
    pub minor_grid_y: bool,
    pub box_enabled: bool,
    pub tick_direction: TickDirection,
    pub font_revision: u64,
    pub style: LinearAxesStyle,
    /// Title, axis-label, legend-label, or annotation intents appended to tick labels.
    pub additional_text: Vec<TextIntent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearAxesFirstPass {
    pub grid_lines: GridLineBatch,
    pub text: FirstLayoutPass,
}

/// Builds deterministic axis lines, CSS-pixel ticks, axes-local grids, and text requests.
///
/// # Errors
/// Returns [`LayoutError`] for invalid tick values, transforms, or CSS arithmetic overflow.
pub fn build_linear_axes_layout(
    mut input: LinearAxesLayoutInput<'_>,
) -> Result<LinearAxesFirstPass, LayoutError> {
    let viewport = input.viewport.viewport();
    if viewport.css.size.width.get() == 0.0 || viewport.css.size.height.get() == 0.0 {
        return Err(LayoutError::DegenerateViewport);
    }
    let left = viewport.css.origin.x;
    let right = viewport.css.right();
    let top = viewport.css.origin.y;
    let bottom = viewport.css.bottom();
    let mut axis_lines = vec![
        OverlayAxisLine {
            segment: css_line(left, bottom, right, bottom)?,
            color: input.style.axis_color,
            width_css_px: input.style.axis_width_css_px,
        },
        OverlayAxisLine {
            segment: css_line(left, top, left, bottom)?,
            color: input.style.axis_color,
            width_css_px: input.style.axis_width_css_px,
        },
    ];
    if input.box_enabled {
        axis_lines.extend([
            OverlayAxisLine {
                segment: css_line(left, top, right, top)?,
                color: input.style.axis_color,
                width_css_px: input.style.axis_width_css_px,
            },
            OverlayAxisLine {
                segment: css_line(right, top, right, bottom)?,
                color: input.style.axis_color,
                width_css_px: input.style.axis_width_css_px,
            },
        ]);
    }
    let mut overlay = OverlayPlan {
        viewport_css: viewport.css,
        axis_lines,
        ..OverlayPlan::default()
    };
    let mut grid_segments = Vec::new();
    let mut text = std::mem::take(&mut input.additional_text);
    append_x_ticks(&input, bottom, &mut overlay, &mut grid_segments, &mut text)?;
    append_y_ticks(&input, left, &mut overlay, &mut grid_segments, &mut text)?;
    append_minor_grid(&input, &mut grid_segments)?;
    Ok(LinearAxesFirstPass {
        grid_lines: GridLineBatch {
            segments: grid_segments,
            color: input.style.grid_color,
            width_css_px: input.style.grid_width_css_px,
        },
        text: FirstLayoutPass::from_overlay(overlay, input.font_revision, text),
    })
}

fn append_x_ticks(
    input: &LinearAxesLayoutInput<'_>,
    bottom: f32,
    overlay: &mut OverlayPlan,
    grid: &mut Vec<AxesLineSegment>,
    text: &mut Vec<TextIntent>,
) -> Result<(), LayoutError> {
    let length = input.style.tick_length_css_px.get();
    let (tick_start, tick_end) = match input.tick_direction {
        TickDirection::In => (bottom, checked_sub(bottom, length)?),
        TickDirection::Out => (bottom, checked_add(bottom, length)?),
        TickDirection::Both => (checked_sub(bottom, length)?, checked_add(bottom, length)?),
    };
    let label_y = checked_add(
        checked_add(bottom, length)?,
        input.style.label_gap_css_px.get(),
    )?;
    for (index, tick) in input.x_ticks.ticks.iter().enumerate() {
        let position = normalize_tick(tick.value, input.x_domain)?;
        let css = input.viewport.axes_to_css(AxesPoint {
            x: position,
            y: 0.0,
        })?;
        let key = format!("x-tick-{index}");
        let label_key = format!("{key}-label");
        overlay.ticks.push(OverlayTick {
            key,
            axis: AxisDimension::X,
            value: tick.value,
            mark: css_line(css.x, tick_start, css.x, tick_end)?,
            label_key: label_key.clone(),
        });
        text.push(tick_text(
            label_key,
            tick.label_code_units.clone(),
            CssPoint::new(css.x, label_y)
                .map_err(|_| LayoutError::ArithmeticOverflow("x tick label coordinate"))?,
            HorizontalAlignment::Center,
            VerticalAlignment::Top,
            &input.style,
        ));
        if input.grid_x {
            grid.push(AxesLineSegment {
                start: AxesPoint {
                    x: position,
                    y: 0.0,
                },
                end: AxesPoint {
                    x: position,
                    y: 1.0,
                },
            });
        }
    }
    Ok(())
}

fn append_y_ticks(
    input: &LinearAxesLayoutInput<'_>,
    left: f32,
    overlay: &mut OverlayPlan,
    grid: &mut Vec<AxesLineSegment>,
    text: &mut Vec<TextIntent>,
) -> Result<(), LayoutError> {
    let length = input.style.tick_length_css_px.get();
    let (tick_start, tick_end) = match input.tick_direction {
        TickDirection::In => (left, checked_add(left, length)?),
        TickDirection::Out => (left, checked_sub(left, length)?),
        TickDirection::Both => (checked_add(left, length)?, checked_sub(left, length)?),
    };
    let label_x = checked_sub(
        checked_sub(left, length)?,
        input.style.label_gap_css_px.get(),
    )?;
    for (index, tick) in input.y_ticks.ticks.iter().enumerate() {
        let position = normalize_tick(tick.value, input.y_domain)?;
        let css = input.viewport.axes_to_css(AxesPoint {
            x: 0.0,
            y: position,
        })?;
        let key = format!("y-tick-{index}");
        let label_key = format!("{key}-label");
        overlay.ticks.push(OverlayTick {
            key,
            axis: AxisDimension::Y,
            value: tick.value,
            mark: css_line(tick_start, css.y, tick_end, css.y)?,
            label_key: label_key.clone(),
        });
        text.push(tick_text(
            label_key,
            tick.label_code_units.clone(),
            CssPoint::new(label_x, css.y)
                .map_err(|_| LayoutError::ArithmeticOverflow("y tick label coordinate"))?,
            HorizontalAlignment::End,
            VerticalAlignment::Middle,
            &input.style,
        ));
        if input.grid_y {
            grid.push(AxesLineSegment {
                start: AxesPoint {
                    x: 0.0,
                    y: position,
                },
                end: AxesPoint {
                    x: 1.0,
                    y: position,
                },
            });
        }
    }
    Ok(())
}

fn append_minor_grid(
    input: &LinearAxesLayoutInput<'_>,
    grid: &mut Vec<AxesLineSegment>,
) -> Result<(), LayoutError> {
    if input.minor_grid_x {
        for pair in input.x_ticks.ticks.windows(2) {
            let left = normalize_tick(pair[0].value, input.x_domain)?;
            let right = normalize_tick(pair[1].value, input.x_domain)?;
            let x = (left + right) * 0.5;
            grid.push(AxesLineSegment {
                start: AxesPoint { x, y: 0.0 },
                end: AxesPoint { x, y: 1.0 },
            });
        }
    }
    if input.minor_grid_y {
        for pair in input.y_ticks.ticks.windows(2) {
            let bottom = normalize_tick(pair[0].value, input.y_domain)?;
            let top = normalize_tick(pair[1].value, input.y_domain)?;
            let y = (bottom + top) * 0.5;
            grid.push(AxesLineSegment {
                start: AxesPoint { x: 0.0, y },
                end: AxesPoint { x: 1.0, y },
            });
        }
    }
    Ok(())
}

fn tick_text(
    key: String,
    code_units: Vec<u16>,
    anchor: CssPoint,
    horizontal_alignment: HorizontalAlignment,
    vertical_alignment: VerticalAlignment,
    style: &LinearAxesStyle,
) -> TextIntent {
    TextIntent {
        key,
        role: OverlayTextRole::TickLabel,
        text: Utf16Text::from_code_units(code_units),
        anchor,
        horizontal_alignment,
        vertical_alignment,
        rotation_radians: 0.0,
        color: style.text_color,
        font: style.tick_font.clone(),
        interpreter: style.tick_label_interpreter,
    }
}

fn normalize_tick(value: f64, domain: TickDomain) -> Result<f32, LayoutError> {
    domain.normalize(value)
}

fn css_line(
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
) -> Result<CssLineSegment, LayoutError> {
    Ok(CssLineSegment {
        start: CssPoint::new(start_x, start_y)
            .map_err(|_| LayoutError::ArithmeticOverflow("axis line"))?,
        end: CssPoint::new(end_x, end_y)
            .map_err(|_| LayoutError::ArithmeticOverflow("axis line"))?,
    })
}

fn checked_add(left: f32, right: f32) -> Result<f32, LayoutError> {
    let result = left + right;
    result
        .is_finite()
        .then_some(result)
        .ok_or(LayoutError::ArithmeticOverflow("CSS coordinate"))
}

fn checked_sub(left: f32, right: f32) -> Result<f32, LayoutError> {
    checked_add(left, -right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_plot_mir::{DevicePixelRatio, FontWeight, OverlayFontStyle};

    use crate::{Limits, SemanticLimits, TextMeasurement, TextMetrics, linear_ticks};

    #[test]
    #[allow(clippy::too_many_lines)]
    fn builds_shared_grid_tick_and_two_pass_overlay_coordinates() {
        let viewport = ViewportMapping::new(
            openmat_plot_mir::CssRect::new(20.0, 10.0, 400.0, 200.0).unwrap(),
            DevicePixelRatio::new(2.0).unwrap(),
        )
        .unwrap();
        let x_domain = TickDomain::Semantic(SemanticLimits::new(
            Limits::new(1.0e12, 1.0e12 + 2.0).unwrap(),
        ));
        let y_domain = TickDomain::Semantic(SemanticLimits::new(Limits::new(-1.0, 1.0).unwrap()));
        let x_ticks = linear_ticks(x_domain, 3).unwrap();
        let y_ticks = linear_ticks(y_domain, 3).unwrap();
        let label_font = FontKey {
            family_utf16: "sans-serif".encode_utf16().collect(),
            size_css_px: CssPx::new(12.0).unwrap(),
            weight: FontWeight::NORMAL,
            style: OverlayFontStyle::Normal,
        };
        let additional_text = [
            ("title", OverlayTextRole::Title, 0.0),
            ("xlabel", OverlayTextRole::XLabel, 0.0),
            (
                "ylabel",
                OverlayTextRole::YLabel,
                -std::f32::consts::FRAC_PI_2,
            ),
        ]
        .into_iter()
        .map(|(key, role, rotation_radians)| TextIntent {
            key: key.to_owned(),
            role,
            text: Utf16Text::from_code_units(key.encode_utf16().collect()),
            anchor: CssPoint::default(),
            horizontal_alignment: HorizontalAlignment::Center,
            vertical_alignment: VerticalAlignment::Middle,
            rotation_radians,
            color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            font: label_font.clone(),
            interpreter: TextInterpreter::Tex,
        })
        .collect();
        let first = build_linear_axes_layout(LinearAxesLayoutInput {
            viewport,
            x_domain,
            y_domain,
            x_ticks: &x_ticks,
            y_ticks: &y_ticks,
            grid_x: true,
            grid_y: true,
            minor_grid_x: true,
            minor_grid_y: false,
            box_enabled: true,
            tick_direction: TickDirection::In,
            font_revision: 9,
            style: LinearAxesStyle {
                axis_color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
                grid_color: Rgba::new(0.8, 0.8, 0.8, 1.0).unwrap(),
                text_color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
                axis_width_css_px: CssPx::new(1.0).unwrap(),
                grid_width_css_px: CssPx::new(1.0).unwrap(),
                tick_length_css_px: CssPx::new(5.0).unwrap(),
                label_gap_css_px: CssPx::new(3.0).unwrap(),
                tick_font: label_font,
                tick_label_interpreter: TextInterpreter::Tex,
            },
            additional_text,
        })
        .unwrap();
        assert_eq!(
            first.grid_lines.segments.len(),
            x_ticks.ticks.len() + y_ticks.ticks.len() + x_ticks.ticks.len() - 1
        );
        let overlay = first.text.base_overlay();
        let x_mark = &overlay
            .ticks
            .iter()
            .find(|tick| tick.axis == AxisDimension::X)
            .unwrap()
            .mark;
        let y_mark = &overlay
            .ticks
            .iter()
            .find(|tick| tick.axis == AxisDimension::Y)
            .unwrap()
            .mark;
        assert!(x_mark.end.y < x_mark.start.y);
        assert!(y_mark.end.x > y_mark.start.x);
        assert!(
            first
                .text
                .requests
                .iter()
                .all(|request| request.font_revision == 9)
        );
        let measurements = first
            .text
            .requests
            .iter()
            .map(|request| TextMeasurement {
                key: request.key.clone(),
                metrics: TextMetrics::new(20.0, 12.0, 8.0, 3.0, 20.0).unwrap(),
            })
            .collect();
        let overlay = first.text.complete(measurements).unwrap();
        assert_eq!(overlay.axis_lines.len(), 4);
        assert_eq!(overlay.ticks.len() + 3, overlay.text.len());
        let title = overlay
            .text
            .iter()
            .find(|text| text.key == "title")
            .unwrap();
        let xlabel = overlay
            .text
            .iter()
            .find(|text| text.key == "xlabel")
            .unwrap();
        let ylabel = overlay
            .text
            .iter()
            .find(|text| text.key == "ylabel")
            .unwrap();
        assert!(title.anchor.y < viewport.viewport().css.origin.y);
        assert!(xlabel.anchor.y > viewport.viewport().css.bottom());
        assert!(ylabel.anchor.x < viewport.viewport().css.origin.x);
        assert!((ylabel.rotation_radians + std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);
        overlay.validate().unwrap();
    }
}
