use crate::{InteractionError, MirError};

const MIN_VIEW_SPAN_FRACTION: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct CssPx(f32);

impl CssPx {
    /// Creates a non-negative finite CSS-pixel length.
    ///
    /// # Errors
    /// Returns [`MirError`] when `value` is negative or non-finite.
    pub fn new(value: f32) -> Result<Self, MirError> {
        if !value.is_finite() {
            return Err(MirError::NonFinite("CSS pixel length"));
        }
        if value < 0.0 {
            return Err(MirError::Negative("CSS pixel length"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct DevicePixelRatio(f64);

impl DevicePixelRatio {
    /// Creates a positive finite device-pixel ratio.
    ///
    /// # Errors
    /// Returns [`MirError`] when `value` is non-positive or non-finite.
    pub fn new(value: f64) -> Result<Self, MirError> {
        if !value.is_finite() {
            return Err(MirError::NonFinite("device pixel ratio"));
        }
        if value <= 0.0 {
            return Err(MirError::NonPositive("device pixel ratio"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxesPoint {
    pub x: f32,
    pub y: f32,
}

impl AxesPoint {
    /// Creates a finite axes-local point.
    ///
    /// # Errors
    /// Returns [`MirError`] when either coordinate is non-finite.
    pub fn new(x: f32, y: f32) -> Result<Self, MirError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(MirError::NonFinite("axes-local point"));
        }
        Ok(Self { x, y })
    }

    /// Validates a point assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] when either coordinate is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.x, self.y).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxesRect {
    pub min: AxesPoint,
    pub max: AxesPoint,
}

impl AxesRect {
    /// Creates an ordered axes-local rectangle.
    ///
    /// # Errors
    /// Returns [`MirError`] when either axis has reversed bounds.
    pub fn new(min: AxesPoint, max: AxesPoint) -> Result<Self, MirError> {
        min.validate()?;
        max.validate()?;
        if min.x > max.x || min.y > max.y {
            return Err(MirError::InvalidRectangle("axes rectangle"));
        }
        Ok(Self { min, max })
    }

    /// Validates a rectangle assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] for non-finite or reversed bounds.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.min, self.max).map(|_| ())
    }

    #[must_use]
    pub const fn unit() -> Self {
        Self {
            min: AxesPoint { x: 0.0, y: 0.0 },
            max: AxesPoint { x: 1.0, y: 1.0 },
        }
    }

    #[must_use]
    pub fn contains(self, point: AxesPoint) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CssPoint {
    pub x: f32,
    pub y: f32,
}

impl CssPoint {
    /// Creates a finite point in CSS pixels.
    ///
    /// # Errors
    /// Returns [`MirError`] when either coordinate is non-finite.
    pub fn new(x: f32, y: f32) -> Result<Self, MirError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(MirError::NonFinite("CSS point"));
        }
        Ok(Self { x, y })
    }

    /// Validates a point assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] when either coordinate is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.x, self.y).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CssSize {
    pub width: CssPx,
    pub height: CssPx,
}

impl CssSize {
    /// Creates a non-negative finite CSS-pixel size.
    ///
    /// # Errors
    /// Returns [`MirError`] when either extent is negative or non-finite.
    pub fn new(width: f32, height: f32) -> Result<Self, MirError> {
        Ok(Self {
            width: CssPx::new(width)?,
            height: CssPx::new(height)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CssRect {
    pub origin: CssPoint,
    pub size: CssSize,
}

impl CssRect {
    /// Creates a CSS-pixel rectangle with non-negative finite extents.
    ///
    /// # Errors
    /// Returns [`MirError`] when the origin or either extent is invalid.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, MirError> {
        let rectangle = Self {
            origin: CssPoint::new(x, y)?,
            size: CssSize::new(width, height)?,
        };
        if !rectangle.right().is_finite() || !rectangle.bottom().is_finite() {
            return Err(MirError::InvalidRectangle("CSS rectangle"));
        }
        Ok(rectangle)
    }

    #[must_use]
    pub fn right(self) -> f32 {
        self.origin.x + self.size.width.get()
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.origin.y + self.size.height.get()
    }

    /// Validates a rectangle assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] for a non-finite origin or checked extent overflow.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(
            self.origin.x,
            self.origin.y,
            self.size.width.get(),
            self.size.height.get(),
        )
        .map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub css: CssRect,
    pub device: DeviceRect,
    pub device_pixel_ratio: DevicePixelRatio,
}

impl Viewport {
    /// Validates the CSS rectangle and the independently rounded device rectangle.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid CSS coordinates or a zero device extent.
    pub fn validate(self) -> Result<(), MirError> {
        self.css.validate()?;
        if self.css.size.width.get() == 0.0 || self.css.size.height.get() == 0.0 {
            return Err(MirError::NonPositive("viewport CSS span"));
        }
        if self.device.width == 0 || self.device.height == 0 {
            return Err(MirError::NonPositive("viewport device span"));
        }
        Ok(())
    }

    /// Maps a canvas-local CSS point to axes coordinates (`x` right, `y` up).
    /// Coordinates outside the viewport are intentionally preserved for callers
    /// that need unclamped gestures.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid input or a degenerate viewport.
    pub fn css_to_axes(self, point: CssPoint) -> Result<AxesPoint, MirError> {
        self.validate()?;
        point.validate()?;
        let x = (point.x - self.css.origin.x) / self.css.size.width.get();
        let y = 1.0 - (point.y - self.css.origin.y) / self.css.size.height.get();
        AxesPoint::new(x, y)
    }

    /// Maps axes coordinates (`x` right, `y` up) to canvas-local CSS pixels.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid input, a degenerate viewport, or overflow.
    pub fn axes_to_css(self, point: AxesPoint) -> Result<CssPoint, MirError> {
        self.validate()?;
        point.validate()?;
        let x = self.css.origin.x + point.x * self.css.size.width.get();
        let y = self.css.origin.y + (1.0 - point.y) * self.css.size.height.get();
        CssPoint::new(x, y).map_err(|_| MirError::ArithmeticOverflow("axes-to-CSS mapping"))
    }

    /// Maps canvas-local CSS pixels to device pixels using the actual rounded
    /// backing viewport, rather than assuming `css * DPR` is integral.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid input, a degenerate viewport, or overflow.
    pub fn css_to_device(self, point: CssPoint) -> Result<(f64, f64), MirError> {
        let axes = self.css_to_axes(point)?;
        let x = f64::from(self.device.x) + f64::from(axes.x) * f64::from(self.device.width);
        let y =
            f64::from(self.device.y) + (1.0 - f64::from(axes.y)) * f64::from(self.device.height);
        if x.is_finite() && y.is_finite() {
            Ok((x, y))
        } else {
            Err(MirError::ArithmeticOverflow("CSS-to-device mapping"))
        }
    }

    /// Clamps a CSS point to the axes viewport before a box/pick operation.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid input or viewport state.
    pub fn clamp_css_point(self, point: CssPoint) -> Result<CssPoint, MirError> {
        self.validate()?;
        point.validate()?;
        CssPoint::new(
            point.x.clamp(self.css.origin.x, self.css.right()),
            point.y.clamp(self.css.origin.y, self.css.bottom()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataPoint {
    pub x: f64,
    pub y: f64,
}

impl DataPoint {
    /// Creates a finite semantic data point.
    ///
    /// # Errors
    /// Returns [`InteractionError`] when either coordinate is non-finite.
    pub fn new(x: f64, y: f64) -> Result<Self, InteractionError> {
        if x.is_finite() && y.is_finite() {
            Ok(Self { x, y })
        } else {
            Err(InteractionError::NonFinite("data point"))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataRect {
    pub min: DataPoint,
    pub max: DataPoint,
}

impl DataRect {
    /// Creates finite, strictly increasing X/Y semantic limits.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for non-finite, reversed, zero-span, or overflowing limits.
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> Result<Self, InteractionError> {
        let min = DataPoint::new(x_min, y_min)?;
        let max = DataPoint::new(x_max, y_max)?;
        let x_span = x_max - x_min;
        let y_span = y_max - y_min;
        if !(x_span.is_finite() && y_span.is_finite()) {
            return Err(InteractionError::Overflow("data limits span"));
        }
        if x_span <= 0.0 || y_span <= 0.0 {
            return Err(InteractionError::ZeroSpan("data limits"));
        }
        Ok(Self { min, max })
    }

    #[must_use]
    pub const fn x_limits(self) -> [f64; 2] {
        [self.min.x, self.max.x]
    }

    #[must_use]
    pub const fn y_limits(self) -> [f64; 2] {
        [self.min.y, self.max.y]
    }

    #[must_use]
    pub fn x_span(self) -> f64 {
        self.max.x - self.min.x
    }

    #[must_use]
    pub fn y_span(self) -> f64 {
        self.max.y - self.min.y
    }
}

/// Backend-neutral renderer-local 2D view. Source geometry remains normalized
/// against `home`; gestures only replace this small affine state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTransform2D {
    viewport: Viewport,
    home: DataRect,
    current: DataRect,
}

impl ViewTransform2D {
    /// Creates a home view for one axes viewport.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for a zero CSS/device span or invalid limits.
    pub fn new(viewport: Viewport, home: DataRect) -> Result<Self, InteractionError> {
        validate_interaction_viewport(viewport)?;
        let home = DataRect::new(home.min.x, home.max.x, home.min.y, home.max.y)?;
        Ok(Self {
            viewport,
            home,
            current: home,
        })
    }

    #[must_use]
    pub const fn viewport(self) -> Viewport {
        self.viewport
    }

    #[must_use]
    pub const fn home_limits(self) -> DataRect {
        self.home
    }

    #[must_use]
    pub const fn current_limits(self) -> DataRect {
        self.current
    }

    /// Updates pixel metadata while preserving semantic limits.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for a degenerate viewport.
    pub fn set_viewport(&mut self, viewport: Viewport) -> Result<(), InteractionError> {
        validate_interaction_viewport(viewport)?;
        self.viewport = viewport;
        Ok(())
    }

    pub fn home(&mut self) {
        self.current = self.home;
    }

    /// Pans by a CSS-pixel drag delta. Positive X drags content right; positive
    /// Y drags content down. Home is an explicit reset target, not a pan boundary.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for non-finite input or overflow.
    pub fn pan_css(&mut self, delta_x: f64, delta_y: f64) -> Result<(), InteractionError> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Err(InteractionError::NonFinite("pan delta"));
        }
        let width = f64::from(self.viewport.css.size.width.get());
        let height = f64::from(self.viewport.css.size.height.get());
        let dx = -delta_x / width * self.current.x_span();
        let dy = delta_y / height * self.current.y_span();
        let next = DataRect::new(
            checked_data_add(self.current.min.x, dx, "pan X")?,
            checked_data_add(self.current.max.x, dx, "pan X")?,
            checked_data_add(self.current.min.y, dy, "pan Y")?,
            checked_data_add(self.current.max.y, dy, "pan Y")?,
        )?;
        self.current = next;
        Ok(())
    }

    /// Zooms around a canvas-local CSS anchor. Factors below one zoom in.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid factor, point, or overflow.
    pub fn zoom_css(&mut self, anchor: CssPoint, factor: f64) -> Result<(), InteractionError> {
        if !factor.is_finite() {
            return Err(InteractionError::NonFinite("zoom factor"));
        }
        if factor <= 0.0 {
            return Err(InteractionError::NonPositive("zoom factor"));
        }
        let anchor = self.clamp_css(anchor)?;
        let anchor_axes = self.css_to_axes(anchor)?;
        let anchor_data = self.axes_to_data(anchor_axes)?;
        let requested_spans = (
            self.current.x_span() * factor,
            self.current.y_span() * factor,
        );
        if !requested_spans.0.is_finite() || !requested_spans.1.is_finite() {
            return Err(InteractionError::Overflow("zoom span"));
        }
        let x_span = requested_spans.0.max(minimum_view_span(self.home.x_span()));
        let y_span = requested_spans.1.max(minimum_view_span(self.home.y_span()));
        let x_fraction = (anchor_data.x - self.current.min.x) / self.current.x_span();
        let y_fraction = (anchor_data.y - self.current.min.y) / self.current.y_span();
        let next = DataRect::new(
            checked_data_add(anchor_data.x, -x_fraction * x_span, "zoom X")?,
            checked_data_add(anchor_data.x, (1.0 - x_fraction) * x_span, "zoom X")?,
            checked_data_add(anchor_data.y, -y_fraction * y_span, "zoom Y")?,
            checked_data_add(anchor_data.y, (1.0 - y_fraction) * y_span, "zoom Y")?,
        )?;
        self.current = next;
        Ok(())
    }

    /// Replaces the current view with a viewport-clamped CSS selection rectangle.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid input, a zero-area box, or overflow.
    pub fn box_zoom_css(
        &mut self,
        first: CssPoint,
        second: CssPoint,
    ) -> Result<(), InteractionError> {
        let first = self.clamp_css(first)?;
        let second = self.clamp_css(second)?;
        if (first.x - second.x).abs() <= f32::EPSILON || (first.y - second.y).abs() <= f32::EPSILON
        {
            return Err(InteractionError::BoxTooSmall);
        }
        let a = self.css_to_data(first)?;
        let b = self.css_to_data(second)?;
        let next = DataRect::new(a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y))?;
        if next.x_span() < minimum_view_span(self.home.x_span())
            || next.y_span() < minimum_view_span(self.home.y_span())
        {
            return Err(InteractionError::BoxTooSmall);
        }
        self.current = next;
        Ok(())
    }

    /// Maps CSS pixels to current semantic data coordinates.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn css_to_data(self, point: CssPoint) -> Result<DataPoint, InteractionError> {
        self.axes_to_data(self.css_to_axes(point)?)
    }

    /// Maps current semantic data coordinates to CSS pixels.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn data_to_css(self, point: DataPoint) -> Result<CssPoint, InteractionError> {
        let axes = self.data_to_axes(point)?;
        self.viewport
            .axes_to_css(axes)
            .map_err(|_| InteractionError::Overflow("data-to-CSS mapping"))
    }

    /// Maps CSS pixels to y-up axes coordinates in the current view.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn css_to_axes(self, point: CssPoint) -> Result<AxesPoint, InteractionError> {
        self.viewport
            .css_to_axes(point)
            .map_err(|_| InteractionError::Overflow("CSS-to-axes mapping"))
    }

    /// Maps current axes coordinates to semantic data coordinates.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn axes_to_data(self, point: AxesPoint) -> Result<DataPoint, InteractionError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(InteractionError::NonFinite("axes point"));
        }
        DataPoint::new(
            checked_data_add(
                self.current.min.x,
                f64::from(point.x) * self.current.x_span(),
                "axes-to-data X",
            )?,
            checked_data_add(
                self.current.min.y,
                f64::from(point.y) * self.current.y_span(),
                "axes-to-data Y",
            )?,
        )
    }

    /// Maps semantic data coordinates to current y-up axes coordinates.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn data_to_axes(self, point: DataPoint) -> Result<AxesPoint, InteractionError> {
        let point = DataPoint::new(point.x, point.y)?;
        f64_pair_to_axes(
            (point.x - self.current.min.x) / self.current.x_span(),
            (point.y - self.current.min.y) / self.current.y_span(),
        )
    }

    /// Maps immutable source axes (normalized against home) to current axes.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn source_axes_to_view_axes(self, point: AxesPoint) -> Result<AxesPoint, InteractionError> {
        let data = DataPoint::new(
            checked_data_add(
                self.home.min.x,
                f64::from(point.x) * self.home.x_span(),
                "source X",
            )?,
            checked_data_add(
                self.home.min.y,
                f64::from(point.y) * self.home.y_span(),
                "source Y",
            )?,
        )?;
        self.data_to_axes(data)
    }

    /// Maps immutable source axes directly to CSS pixels.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid coordinates or overflow.
    pub fn source_axes_to_css(self, point: AxesPoint) -> Result<CssPoint, InteractionError> {
        let axes = self.source_axes_to_view_axes(point)?;
        self.viewport
            .axes_to_css(axes)
            .map_err(|_| InteractionError::Overflow("source-to-CSS mapping"))
    }

    /// Returns `(scale_x, scale_y, offset_x, offset_y)` for renderer uniforms.
    ///
    /// # Errors
    /// Returns [`InteractionError`] if the affine cannot be represented by f32.
    pub fn source_to_view_affine(self) -> Result<[f32; 4], InteractionError> {
        let scale = f64_pair_to_axes(
            self.home.x_span() / self.current.x_span(),
            self.home.y_span() / self.current.y_span(),
        )?;
        let offset = f64_pair_to_axes(
            (self.home.min.x - self.current.min.x) / self.current.x_span(),
            (self.home.min.y - self.current.min.y) / self.current.y_span(),
        )?;
        Ok([scale.x, scale.y, offset.x, offset.y])
    }

    fn clamp_css(self, point: CssPoint) -> Result<CssPoint, InteractionError> {
        self.viewport
            .clamp_css_point(point)
            .map_err(|_| InteractionError::NonFinite("CSS point"))
    }
}

fn validate_interaction_viewport(viewport: Viewport) -> Result<(), InteractionError> {
    viewport
        .validate()
        .map_err(|_| InteractionError::InvalidViewport)
}

fn checked_data_add(left: f64, right: f64, field: &'static str) -> Result<f64, InteractionError> {
    let result = left + right;
    result
        .is_finite()
        .then_some(result)
        .ok_or(InteractionError::Overflow(field))
}

fn minimum_view_span(home_span: f64) -> f64 {
    (home_span * MIN_VIEW_SPAN_FRACTION).max(f64::MIN_POSITIVE)
}

#[allow(clippy::cast_possible_truncation)]
fn f64_pair_to_axes(x: f64, y: f64) -> Result<AxesPoint, InteractionError> {
    if !x.is_finite()
        || !y.is_finite()
        || x.abs() > f64::from(f32::MAX)
        || y.abs() > f64::from(f32::MAX)
    {
        return Err(InteractionError::Overflow("f64-to-f32 coordinate"));
    }
    AxesPoint::new(x as f32, y as f32)
        .map_err(|_| InteractionError::Overflow("f64-to-f32 coordinate"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Rgba {
    /// Creates an unpremultiplied sRGB color.
    ///
    /// # Errors
    /// Returns [`MirError`] unless every component is finite and in `0..=1`.
    pub fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Result<Self, MirError> {
        if [red, green, blue, alpha]
            .into_iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
        {
            Ok(Self {
                red,
                green,
                blue,
                alpha,
            })
        } else {
            Err(MirError::InvalidColorComponent)
        }
    }

    /// Validates a color assembled through a public structure literal.
    ///
    /// # Errors
    /// Returns [`MirError`] unless every component is finite and in `0..=1`.
    pub fn validate(self) -> Result<(), MirError> {
        Self::new(self.red, self.green, self.blue, self.alpha).map(|_| ())
    }

    pub const TRANSPARENT: Self = Self {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 0.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_coordinate_and_color_values() {
        assert_eq!(
            CssPx::new(f32::NAN),
            Err(MirError::NonFinite("CSS pixel length"))
        );
        assert_eq!(
            DevicePixelRatio::new(0.0),
            Err(MirError::NonPositive("device pixel ratio"))
        );
        assert_eq!(
            Rgba::new(1.0, -0.01, 0.0, 1.0),
            Err(MirError::InvalidColorComponent)
        );
        assert!(AxesRect::new(AxesPoint { x: 1.0, y: 0.0 }, AxesPoint { x: 0.0, y: 1.0 }).is_err());
        assert!(CssRect::new(f32::MAX, 0.0, f32::MAX, 1.0).is_err());
    }

    #[test]
    fn viewport_coordinate_round_trip_uses_css_and_actual_device_extents() {
        let viewport = Viewport {
            css: CssRect::new(10.0, 20.0, 320.25, 200.5).unwrap(),
            device: DeviceRect {
                x: 20,
                y: 40,
                width: 641,
                height: 401,
            },
            device_pixel_ratio: DevicePixelRatio::new(2.0).unwrap(),
        };
        let axes = AxesPoint::new(0.25, 0.75).unwrap();
        let css = viewport.axes_to_css(axes).unwrap();
        let round_trip = viewport.css_to_axes(css).unwrap();
        assert!((round_trip.x - axes.x).abs() < 2.0 * f32::EPSILON);
        assert!((round_trip.y - axes.y).abs() < 2.0 * f32::EPSILON);
        assert_eq!(viewport.css_to_device(css).unwrap(), (180.25, 140.25));
    }

    #[test]
    fn viewport_rejects_zero_spans_and_clamps_boundary_points() {
        let mut viewport = Viewport {
            css: CssRect::new(10.0, 20.0, 100.0, 80.0).unwrap(),
            device: DeviceRect {
                x: 10,
                y: 20,
                width: 100,
                height: 80,
            },
            device_pixel_ratio: DevicePixelRatio::new(1.0).unwrap(),
        };
        assert_eq!(
            viewport
                .clamp_css_point(CssPoint::new(-20.0, 200.0).unwrap())
                .unwrap(),
            CssPoint::new(10.0, 100.0).unwrap()
        );
        viewport.device.width = 0;
        assert_eq!(
            viewport.validate(),
            Err(MirError::NonPositive("viewport device span"))
        );
    }
}
