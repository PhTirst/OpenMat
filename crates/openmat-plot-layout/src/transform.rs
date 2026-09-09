use openmat_plot_mir::{AxesPoint, CssPoint, CssRect, DevicePixelRatio, DeviceRect, Viewport};

use crate::{LayoutError, LocalViewLimits, SemanticLimits};

/// Data-to-axes mapping applied before affine normalization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AxisScale {
    /// Identity mapping.
    #[default]
    Linear,
    /// Base-10 logarithm; nonpositive values are outside the domain.
    Log10,
}

/// Direction of increasing normalized coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AxisDirection {
    /// Lower maps to zero and upper maps to one.
    #[default]
    Normal,
    /// Lower maps to one and upper maps to zero.
    Reverse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisAffine {
    origin: f64,
    scale: f64,
    data_scale: AxisScale,
    direction: AxisDirection,
}

impl AxisAffine {
    fn new(
        limits: crate::Limits,
        data_scale: AxisScale,
        direction: AxisDirection,
    ) -> Result<Self, LayoutError> {
        let lower = map_axis_value(limits.lower(), data_scale)?;
        let upper = map_axis_value(limits.upper(), data_scale)?;
        let span = upper - lower;
        let scale = 1.0 / span;
        if !scale.is_finite() {
            return Err(LayoutError::ArithmeticOverflow("axes scale"));
        }
        Ok(Self {
            origin: lower,
            scale,
            data_scale,
            direction,
        })
    }

    fn map(self, value: f64) -> Result<f32, LayoutError> {
        let value = map_axis_value(value, self.data_scale)?;
        // Subtraction happens in f64 before scale and the final f32 conversion.
        let normalized = (value - self.origin) * self.scale;
        let normalized = if self.direction == AxisDirection::Reverse {
            1.0 - normalized
        } else {
            normalized
        };
        if !normalized.is_finite()
            || normalized > f64::from(f32::MAX)
            || normalized < f64::from(f32::MIN)
        {
            return Err(LayoutError::ArithmeticOverflow("axes coordinate"));
        }
        #[allow(clippy::cast_possible_truncation)]
        let normalized = normalized as f32;
        Ok(normalized)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxesTransform {
    x: AxisAffine,
    y: AxisAffine,
    renderer_local: bool,
}

impl AxesTransform {
    /// Creates the authoritative transform from Kernel-resolved semantic limits.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when a span produces an unrepresentable scale.
    pub fn from_semantic(x: SemanticLimits, y: SemanticLimits) -> Result<Self, LayoutError> {
        Self::from_scaled_semantic(
            x,
            y,
            AxisScale::Linear,
            AxisScale::Linear,
            AxisDirection::Normal,
            AxisDirection::Normal,
        )
    }

    /// Creates a semantic transform with independent scale and direction per axis.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when limits are outside a logarithmic domain or
    /// produce an unrepresentable affine transform.
    pub fn from_scaled_semantic(
        x: SemanticLimits,
        y: SemanticLimits,
        x_scale: AxisScale,
        y_scale: AxisScale,
        x_direction: AxisDirection,
        y_direction: AxisDirection,
    ) -> Result<Self, LayoutError> {
        Ok(Self {
            x: AxisAffine::new(x.get(), x_scale, x_direction)?,
            y: AxisAffine::new(y.get(), y_scale, y_direction)?,
            renderer_local: false,
        })
    }

    /// Constructs a non-authoritative transform used only for renderer-local navigation.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when a local span produces an unrepresentable scale.
    pub fn from_renderer_local(
        x: LocalViewLimits,
        y: LocalViewLimits,
    ) -> Result<Self, LayoutError> {
        Ok(Self {
            x: AxisAffine::new(x.get(), AxisScale::Linear, AxisDirection::Normal)?,
            y: AxisAffine::new(y.get(), AxisScale::Linear, AxisDirection::Normal)?,
            renderer_local: true,
        })
    }

    #[must_use]
    pub const fn is_renderer_local(self) -> bool {
        self.renderer_local
    }

    /// Maps one finite data coordinate after f64 origin subtraction and scaling.
    ///
    /// # Errors
    /// Returns [`LayoutError`] for non-finite input or unrepresentable output.
    pub fn map(self, x: f64, y: f64) -> Result<AxesPoint, LayoutError> {
        AxesPoint::new(self.x.map(x)?, self.y.map(y)?)
            .map_err(|_| LayoutError::ArithmeticOverflow("axes point"))
    }
}

fn map_axis_value(value: f64, scale: AxisScale) -> Result<f64, LayoutError> {
    if !value.is_finite() {
        return Err(LayoutError::NonFiniteInput("data coordinate"));
    }
    let mapped = match scale {
        AxisScale::Linear => value,
        AxisScale::Log10 if value > 0.0 => value.log10(),
        AxisScale::Log10 => return Err(LayoutError::NonFiniteInput("logarithmic coordinate")),
    };
    mapped
        .is_finite()
        .then_some(mapped)
        .ok_or(LayoutError::ArithmeticOverflow("axes coordinate"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportMapping {
    viewport: Viewport,
}

impl ViewportMapping {
    /// Creates a shared CSS/device viewport mapping.
    ///
    /// # Errors
    /// Returns [`LayoutError`] when DPR multiplication exceeds device-pixel limits.
    pub fn new(css: CssRect, dpr: DevicePixelRatio) -> Result<Self, LayoutError> {
        let device = DeviceRect {
            x: checked_device_pixel(f64::from(css.origin.x), dpr)?,
            y: checked_device_pixel(f64::from(css.origin.y), dpr)?,
            width: checked_device_pixel(f64::from(css.size.width.get()), dpr)?,
            height: checked_device_pixel(f64::from(css.size.height.get()), dpr)?,
        };
        Ok(Self {
            viewport: Viewport {
                css,
                device,
                device_pixel_ratio: dpr,
            },
        })
    }

    #[must_use]
    pub const fn viewport(self) -> Viewport {
        self.viewport
    }

    /// Maps an axes-local point into top-left-origin CSS coordinates.
    ///
    /// # Errors
    /// Returns [`LayoutError`] for a degenerate viewport or non-finite result.
    pub fn axes_to_css(self, point: AxesPoint) -> Result<CssPoint, LayoutError> {
        let css = self.viewport.css;
        let width = css.size.width.get();
        let height = css.size.height.get();
        if width == 0.0 || height == 0.0 {
            return Err(LayoutError::DegenerateViewport);
        }
        CssPoint::new(
            css.origin.x + point.x * width,
            css.origin.y + (1.0 - point.y) * height,
        )
        .map_err(|_| LayoutError::ArithmeticOverflow("CSS coordinate"))
    }
}

fn checked_device_pixel(css: f64, dpr: DevicePixelRatio) -> Result<u32, LayoutError> {
    let value = css * dpr.get();
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(LayoutError::ArithmeticOverflow("device viewport"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = value.round() as u32;
    Ok(value)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::Limits;

    #[test]
    fn subtracts_large_origin_before_f32_conversion() {
        let x = SemanticLimits::new(Limits::new(1.0e12, 1.0e12 + 4.0).unwrap());
        let y = SemanticLimits::new(Limits::new(0.0, 1.0).unwrap());
        let transform = AxesTransform::from_semantic(x, y).unwrap();
        let left = transform.map(1.0e12 + 1.0, 0.0).unwrap();
        let right = transform.map(1.0e12 + 2.0, 0.0).unwrap();
        assert_eq!(left.x, 0.25);
        assert_eq!(right.x, 0.5);
        assert_ne!(left.x, right.x);
    }

    #[test]
    fn logarithmic_and_reversed_axes_map_decades_deterministically() {
        let limits = SemanticLimits::new(Limits::new(1.0, 100.0).unwrap());
        let transform = AxesTransform::from_scaled_semantic(
            limits,
            limits,
            AxisScale::Log10,
            AxisScale::Log10,
            AxisDirection::Normal,
            AxisDirection::Reverse,
        )
        .unwrap();
        assert_eq!(
            transform.map(1.0, 1.0).unwrap(),
            AxesPoint { x: 0.0, y: 1.0 }
        );
        assert_eq!(
            transform.map(10.0, 10.0).unwrap(),
            AxesPoint { x: 0.5, y: 0.5 }
        );
        assert_eq!(
            transform.map(100.0, 100.0).unwrap(),
            AxesPoint { x: 1.0, y: 0.0 }
        );
        assert!(transform.map(0.0, 1.0).is_err());
    }

    #[test]
    fn maps_css_and_dpr_without_confusing_coordinate_spaces() {
        let css = CssRect::new(10.0, 20.0, 300.0, 200.0).unwrap();
        let mapping = ViewportMapping::new(css, DevicePixelRatio::new(1.5).unwrap()).unwrap();
        assert_eq!(
            mapping.viewport().device,
            DeviceRect {
                x: 15,
                y: 30,
                width: 450,
                height: 300
            }
        );
        assert_eq!(
            mapping.axes_to_css(AxesPoint { x: 0.5, y: 0.25 }).unwrap(),
            CssPoint { x: 160.0, y: 170.0 }
        );
    }

    #[test]
    fn reports_degenerate_and_overflowing_viewports() {
        let zero = CssRect::new(0.0, 0.0, 0.0, 100.0).unwrap();
        let mapping = ViewportMapping::new(zero, DevicePixelRatio::new(1.0).unwrap()).unwrap();
        assert_eq!(
            mapping.axes_to_css(AxesPoint::default()),
            Err(LayoutError::DegenerateViewport)
        );

        let huge = CssRect::new(0.0, 0.0, f32::MAX, 1.0).unwrap();
        assert!(ViewportMapping::new(huge, DevicePixelRatio::new(2.0).unwrap()).is_err());
    }

    #[test]
    fn handles_or_reports_subnormal_spans_before_geometry() {
        let ordinary_subnormal =
            SemanticLimits::new(Limits::new(0.0, f64::MIN_POSITIVE / 2.0).unwrap());
        let transform =
            AxesTransform::from_semantic(ordinary_subnormal, ordinary_subnormal).unwrap();
        let midpoint = f64::MIN_POSITIVE / 4.0;
        let mapped = transform.map(midpoint, midpoint).unwrap();
        assert!((mapped.x - 0.5).abs() <= f32::EPSILON);

        let minimum_span = SemanticLimits::new(Limits::new(0.0, f64::from_bits(1)).unwrap());
        assert_eq!(
            AxesTransform::from_semantic(minimum_span, minimum_span),
            Err(LayoutError::ArithmeticOverflow("axes scale"))
        );
    }
}
