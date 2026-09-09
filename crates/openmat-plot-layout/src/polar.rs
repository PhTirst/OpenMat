use openmat_plot_mir::{
    AxesLineSegment, AxesPoint, CssPoint, GridLineBatch, HorizontalAlignment, OverlayPlan,
    OverlayTextRole, Utf16Text, VerticalAlignment,
};

use crate::{
    FirstLayoutPass, LayoutError, LinearAxesStyle, TextIntent, TickDomain, TickSet, ViewportMapping,
};

/// Direction in which polar theta values increase.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PolarThetaDirection {
    /// Counterclockwise.
    #[default]
    Counterclockwise,
    /// Clockwise.
    Clockwise,
}

/// Screen location of zero theta.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PolarZeroLocation {
    /// Three o'clock.
    #[default]
    Right,
    /// Twelve o'clock.
    Top,
    /// Nine o'clock.
    Left,
    /// Six o'clock.
    Bottom,
}

/// Inputs for one retained `PolarAxes` chrome layout.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct PolarAxesLayoutInput<'a> {
    pub viewport: ViewportMapping,
    pub theta_domain: TickDomain,
    pub radius_domain: TickDomain,
    pub theta_ticks: &'a TickSet,
    pub radius_ticks: &'a TickSet,
    /// Number of active theta units in one complete turn (`360` or `2*pi`).
    pub theta_period: f64,
    pub theta_direction: PolarThetaDirection,
    pub theta_zero_location: PolarZeroLocation,
    /// Radial ruler location in active theta units.
    pub r_axis_location: f64,
    pub theta_grid: bool,
    pub radius_grid: bool,
    pub theta_minor_grid: bool,
    pub radius_minor_grid: bool,
    pub font_revision: u64,
    pub style: LinearAxesStyle,
    pub additional_text: Vec<TextIntent>,
}

/// GPU line batches and first-pass text requests for `PolarAxes` chrome.
#[derive(Clone, Debug, PartialEq)]
pub struct PolarAxesFirstPass {
    pub grid_lines: GridLineBatch,
    pub axis_lines: GridLineBatch,
    pub text: FirstLayoutPass,
}

/// Builds circular grids, theta spokes, the outer ruler, and tick-label intents.
///
/// # Errors
/// Returns [`LayoutError`] when domains or generated CSS coordinates are invalid.
pub fn build_polar_axes_layout(
    mut input: PolarAxesLayoutInput<'_>,
) -> Result<PolarAxesFirstPass, LayoutError> {
    if !input.theta_period.is_finite() || input.theta_period <= 0.0 {
        return Err(LayoutError::NonFiniteInput("polar theta period"));
    }
    let viewport = input.viewport.viewport();
    if viewport.css.size.width.get() == 0.0 || viewport.css.size.height.get() == 0.0 {
        return Err(LayoutError::DegenerateViewport);
    }

    let mut grid = Vec::new();
    let mut axes = Vec::new();
    let mut text = std::mem::take(&mut input.additional_text);
    append_arc(
        &mut axes,
        input.theta_domain,
        input.theta_period,
        input.theta_direction,
        input.theta_zero_location,
        0.5,
    )?;

    for (index, tick) in input.theta_ticks.ticks.iter().enumerate() {
        let angle = polar_angle(
            tick.value,
            input.theta_period,
            input.theta_direction,
            input.theta_zero_location,
        )?;
        if input.theta_grid {
            grid.push(radial_segment(angle, 0.0, 0.5));
        }
        let anchor = AxesPoint {
            x: 0.5 + angle.cos() * 0.54,
            y: 0.5 + angle.sin() * 0.54,
        };
        text.push(polar_tick_text(
            format!("theta-tick-{index}-label"),
            tick.label_code_units.clone(),
            input.viewport.axes_to_css(anchor)?,
            angle,
            &input.style,
        ));
    }

    for (index, tick) in input.radius_ticks.ticks.iter().enumerate() {
        let normalized = input.radius_domain.normalize(tick.value)?;
        if normalized > 0.0 && normalized < 1.0 && input.radius_grid {
            append_arc(
                &mut grid,
                input.theta_domain,
                input.theta_period,
                input.theta_direction,
                input.theta_zero_location,
                normalized * 0.5,
            )?;
        }
        let angle = polar_angle(
            input.r_axis_location,
            input.theta_period,
            input.theta_direction,
            input.theta_zero_location,
        )?;
        let anchor = AxesPoint {
            x: 0.5 + angle.cos() * normalized * 0.5,
            y: 0.5 + angle.sin() * normalized * 0.5,
        };
        text.push(TextIntent {
            key: format!("r-tick-{index}-label"),
            role: OverlayTextRole::TickLabel,
            text: Utf16Text::from_code_units(tick.label_code_units.clone()),
            anchor: input.viewport.axes_to_css(anchor)?,
            horizontal_alignment: HorizontalAlignment::Center,
            vertical_alignment: VerticalAlignment::Bottom,
            rotation_radians: 0.0,
            color: input.style.text_color,
            font: input.style.tick_font.clone(),
            interpreter: input.style.tick_label_interpreter,
        });
    }

    append_minor_grids(&input, &mut grid)?;
    let overlay = OverlayPlan {
        viewport_css: viewport.css,
        ..OverlayPlan::default()
    };
    Ok(PolarAxesFirstPass {
        grid_lines: GridLineBatch {
            segments: grid,
            color: input.style.grid_color,
            width_css_px: input.style.grid_width_css_px,
        },
        axis_lines: GridLineBatch {
            segments: axes,
            color: input.style.axis_color,
            width_css_px: input.style.axis_width_css_px,
        },
        text: FirstLayoutPass::from_overlay(overlay, input.font_revision, text),
    })
}

fn append_minor_grids(
    input: &PolarAxesLayoutInput<'_>,
    grid: &mut Vec<AxesLineSegment>,
) -> Result<(), LayoutError> {
    if input.theta_minor_grid {
        for pair in input.theta_ticks.ticks.windows(2) {
            let value = (pair[0].value + pair[1].value) * 0.5;
            let angle = polar_angle(
                value,
                input.theta_period,
                input.theta_direction,
                input.theta_zero_location,
            )?;
            grid.push(radial_segment(angle, 0.0, 0.5));
        }
    }
    if input.radius_minor_grid {
        for pair in input.radius_ticks.ticks.windows(2) {
            let value = (pair[0].value + pair[1].value) * 0.5;
            let radius = input.radius_domain.normalize(value)? * 0.5;
            append_arc(
                grid,
                input.theta_domain,
                input.theta_period,
                input.theta_direction,
                input.theta_zero_location,
                radius,
            )?;
        }
    }
    Ok(())
}

fn append_arc(
    output: &mut Vec<AxesLineSegment>,
    domain: TickDomain,
    period: f64,
    direction: PolarThetaDirection,
    zero: PolarZeroLocation,
    radius: f32,
) -> Result<(), LayoutError> {
    let limits = domain.limits();
    let segments = 128_u32;
    let mut previous = None;
    for index in 0..=segments {
        let fraction = f64::from(index) / f64::from(segments);
        let theta = limits.lower() + fraction * (limits.upper() - limits.lower());
        let angle = polar_angle(theta, period, direction, zero)?;
        let point = AxesPoint {
            x: 0.5 + angle.cos() * radius,
            y: 0.5 + angle.sin() * radius,
        };
        if let Some(start) = previous {
            output.push(AxesLineSegment { start, end: point });
        }
        previous = Some(point);
    }
    Ok(())
}

fn radial_segment(angle: f32, inner: f32, outer: f32) -> AxesLineSegment {
    AxesLineSegment {
        start: AxesPoint {
            x: 0.5 + angle.cos() * inner,
            y: 0.5 + angle.sin() * inner,
        },
        end: AxesPoint {
            x: 0.5 + angle.cos() * outer,
            y: 0.5 + angle.sin() * outer,
        },
    }
}

fn polar_angle(
    theta: f64,
    period: f64,
    direction: PolarThetaDirection,
    zero: PolarZeroLocation,
) -> Result<f32, LayoutError> {
    let zero = match zero {
        PolarZeroLocation::Right => 0.0,
        PolarZeroLocation::Top => std::f64::consts::FRAC_PI_2,
        PolarZeroLocation::Left => std::f64::consts::PI,
        PolarZeroLocation::Bottom => -std::f64::consts::FRAC_PI_2,
    };
    let sign = match direction {
        PolarThetaDirection::Counterclockwise => 1.0,
        PolarThetaDirection::Clockwise => -1.0,
    };
    let angle = zero + sign * theta / period * std::f64::consts::TAU;
    if !angle.is_finite() || angle.abs() > f64::from(f32::MAX) {
        return Err(LayoutError::NonFiniteInput("polar angle"));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(angle as f32)
}

fn polar_tick_text(
    key: String,
    code_units: Vec<u16>,
    anchor: CssPoint,
    angle: f32,
    style: &LinearAxesStyle,
) -> TextIntent {
    let horizontal_alignment = if angle.cos() > 0.25 {
        HorizontalAlignment::Start
    } else if angle.cos() < -0.25 {
        HorizontalAlignment::End
    } else {
        HorizontalAlignment::Center
    };
    let vertical_alignment = if angle.sin() > 0.25 {
        VerticalAlignment::Bottom
    } else if angle.sin() < -0.25 {
        VerticalAlignment::Top
    } else {
        VerticalAlignment::Middle
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_plot_mir::{CssPx, CssRect, DevicePixelRatio, FontWeight, OverlayFontStyle, Rgba};

    use crate::{FontKey, Limits, SemanticLimits, TextMeasurement, TextMetrics, explicit_ticks};

    #[test]
    #[allow(clippy::too_many_lines)]
    fn builds_circular_gpu_chrome_and_polar_tick_text() {
        let mapping = ViewportMapping::new(
            CssRect::new(20.0, 10.0, 300.0, 300.0).unwrap(),
            DevicePixelRatio::new(2.0).unwrap(),
        )
        .unwrap();
        let theta_domain =
            TickDomain::Semantic(SemanticLimits::new(Limits::new(0.0, 360.0).unwrap()));
        let radius_domain =
            TickDomain::Semantic(SemanticLimits::new(Limits::new(0.0, 1.5).unwrap()));
        let theta_labels =
            ["0°", "90°", "180°", "270°"].map(|value| value.encode_utf16().collect::<Vec<_>>());
        let theta = explicit_ticks(
            theta_domain,
            &[0.0, 90.0, 180.0, 270.0, 360.0],
            Some(&theta_labels),
        )
        .unwrap();
        let radius_labels =
            ["0", "0.5", "1", "1.5"].map(|value| value.encode_utf16().collect::<Vec<_>>());
        let radius =
            explicit_ticks(radius_domain, &[0.0, 0.5, 1.0, 1.5], Some(&radius_labels)).unwrap();
        let font = FontKey {
            family_utf16: "sans-serif".encode_utf16().collect(),
            size_css_px: CssPx::new(12.0).unwrap(),
            weight: FontWeight::NORMAL,
            style: OverlayFontStyle::Normal,
        };
        let style = LinearAxesStyle {
            axis_color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            grid_color: Rgba::new(0.8, 0.8, 0.8, 1.0).unwrap(),
            text_color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            axis_width_css_px: CssPx::new(1.0).unwrap(),
            grid_width_css_px: CssPx::new(0.5).unwrap(),
            tick_length_css_px: CssPx::new(4.0).unwrap(),
            label_gap_css_px: CssPx::new(3.0).unwrap(),
            tick_font: font.clone(),
            tick_label_interpreter: openmat_plot_mir::TextInterpreter::Tex,
        };
        let title = TextIntent {
            key: "polar-title".to_owned(),
            role: OverlayTextRole::Title,
            text: Utf16Text::from_code_units("Polar Flower".encode_utf16().collect()),
            anchor: CssPoint::default(),
            horizontal_alignment: HorizontalAlignment::Center,
            vertical_alignment: VerticalAlignment::Bottom,
            rotation_radians: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            font,
            interpreter: openmat_plot_mir::TextInterpreter::None,
        };
        let layout = build_polar_axes_layout(PolarAxesLayoutInput {
            viewport: mapping,
            theta_domain,
            radius_domain,
            theta_ticks: &theta,
            radius_ticks: &radius,
            theta_period: 360.0,
            theta_direction: PolarThetaDirection::Counterclockwise,
            theta_zero_location: PolarZeroLocation::Right,
            r_axis_location: 80.0,
            theta_grid: true,
            radius_grid: true,
            theta_minor_grid: false,
            radius_minor_grid: false,
            font_revision: 7,
            style,
            additional_text: vec![title],
        })
        .unwrap();
        assert_eq!(layout.axis_lines.segments.len(), 128);
        assert_eq!(layout.grid_lines.segments.len(), 5 + 2 * 128);
        assert_eq!(layout.text.requests.len(), 10);
        let measurements = layout
            .text
            .requests
            .iter()
            .map(|request| TextMeasurement {
                key: request.key.clone(),
                metrics: TextMetrics::new(40.0, 15.0, 11.0, 4.0, 40.0).unwrap(),
            })
            .collect();
        let overlay = layout.text.complete(measurements).unwrap();
        let title = overlay
            .text
            .iter()
            .find(|text| text.key == "polar-title")
            .unwrap();
        let north_tick = overlay
            .text
            .iter()
            .find(|text| text.key == "theta-tick-1-label")
            .unwrap();
        assert!(title.anchor.y + title.measured_size.height.get() + 8.0 <= north_tick.anchor.y);
        assert!(
            layout
                .axis_lines
                .segments
                .iter()
                .all(|segment| (0.0..=1.0).contains(&segment.start.x)
                    && (0.0..=1.0).contains(&segment.start.y))
        );
    }
}
