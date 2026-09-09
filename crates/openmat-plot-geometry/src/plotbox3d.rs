use openmat_plot_mir::ViewProjection3D;

use crate::GeometryError;

const UNIT_CUBE_CORNERS: [[f64; 3]; 8] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
];

/// Screen-space policy used to place a projected 3D plot box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlotBoxFitMode3D {
    /// Independently scale projected X and Y to fill the available rectangle.
    StretchToFill,
    /// Use one screen-space scale so the projected shape is not distorted.
    PreserveAspect,
}

/// Renderer-derived target for one projected 3D plot box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotBoxFit3D {
    mode: PlotBoxFitMode3D,
    target_half_extent_ndc: [f64; 2],
    zoom: f64,
}

impl PlotBoxFit3D {
    /// Creates a centered fit target in normalized device coordinates.
    ///
    /// `target_half_extent_ndc` must be positive and no larger than one. A
    /// `zoom` of one fits the projected box exactly; larger values preserve a
    /// user-requested zoom-in after the automatic fit has been resolved.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for a non-finite or out-of-range target.
    pub fn new(
        mode: PlotBoxFitMode3D,
        target_half_extent_ndc: [f64; 2],
        zoom: f64,
    ) -> Result<Self, GeometryError> {
        if target_half_extent_ndc
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value > 1.0)
        {
            return Err(GeometryError::InvalidProjection("plot-box fit target"));
        }
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err(GeometryError::InvalidProjection("plot-box fit zoom"));
        }
        Ok(Self {
            mode,
            target_half_extent_ndc,
            zoom,
        })
    }

    #[must_use]
    pub const fn mode(self) -> PlotBoxFitMode3D {
        self.mode
    }

    /// Returns the same fit target with a different renderer-local zoom.
    ///
    /// # Errors
    /// Returns [`GeometryError`] when `zoom` is non-positive or non-finite.
    pub fn with_zoom(self, zoom: f64) -> Result<Self, GeometryError> {
        Self::new(self.mode, self.target_half_extent_ndc, zoom)
    }
}

/// Fits the projected unit-cube silhouette into a centered NDC rectangle.
///
/// The operation is a clip-space X/Y post-transform. WebGPU depth and `w`
/// remain unchanged, so depth testing and perspective interpolation retain the
/// original camera semantics.
///
/// # Errors
/// Returns [`GeometryError`] when a cube corner cannot be projected, the
/// projected silhouette degenerates, or the fitted matrix overflows `f32`.
pub fn fit_plot_box_3d(
    matrix: ViewProjection3D,
    fit: PlotBoxFit3D,
) -> Result<ViewProjection3D, GeometryError> {
    let [minimum_x, maximum_x, minimum_y, maximum_y] = projected_bounds(matrix)?;
    let width = maximum_x - minimum_x;
    let height = maximum_y - minimum_y;
    if !width.is_finite() || !height.is_finite() || width <= f64::EPSILON || height <= f64::EPSILON
    {
        return Err(GeometryError::InvalidProjection(
            "projected plot-box bounds",
        ));
    }

    let target_width = fit.target_half_extent_ndc[0] * 2.0;
    let target_height = fit.target_half_extent_ndc[1] * 2.0;
    let stretch_x = target_width / width;
    let stretch_y = target_height / height;
    let (scale_x, scale_y) = match fit.mode {
        PlotBoxFitMode3D::StretchToFill => (stretch_x, stretch_y),
        PlotBoxFitMode3D::PreserveAspect => {
            let scale = stretch_x.min(stretch_y);
            (scale, scale)
        }
    };
    let scale_x = scale_x * fit.zoom;
    let scale_y = scale_y * fit.zoom;
    let translate_x = -scale_x * (minimum_x + maximum_x) * 0.5;
    let translate_y = -scale_y * (minimum_y + maximum_y) * 0.5;
    if [scale_x, scale_y, translate_x, translate_y]
        .into_iter()
        .any(|value| !value.is_finite())
    {
        return Err(GeometryError::ArithmeticOverflow("plot-box fit"));
    }

    let original = matrix.columns();
    let mut columns = original;
    for column in 0..4 {
        columns[column][0] = finite_f32(scale_x.mul_add(
            f64::from(original[column][0]),
            translate_x * f64::from(original[column][3]),
        ))?;
        columns[column][1] = finite_f32(scale_y.mul_add(
            f64::from(original[column][1]),
            translate_y * f64::from(original[column][3]),
        ))?;
    }
    ViewProjection3D::new(columns).map_err(GeometryError::from)
}

fn projected_bounds(matrix: ViewProjection3D) -> Result<[f64; 4], GeometryError> {
    let columns = matrix.columns();
    let mut minimum_x = f64::INFINITY;
    let mut maximum_x = f64::NEG_INFINITY;
    let mut minimum_y = f64::INFINITY;
    let mut maximum_y = f64::NEG_INFINITY;
    for point in UNIT_CUBE_CORNERS {
        let input = [point[0], point[1], point[2], 1.0];
        let mut clip = [0.0_f64; 4];
        for (column, value) in input.into_iter().enumerate() {
            for (row, output) in clip.iter_mut().enumerate() {
                *output += f64::from(columns[column][row]) * value;
            }
        }
        if !clip.iter().all(|value| value.is_finite()) || clip[3] <= f64::EPSILON {
            return Err(GeometryError::InvalidProjection(
                "projected plot-box corner",
            ));
        }
        let x = clip[0] / clip[3];
        let y = clip[1] / clip[3];
        minimum_x = minimum_x.min(x);
        maximum_x = maximum_x.max(x);
        minimum_y = minimum_y.min(y);
        maximum_y = maximum_y.max(y);
    }
    Ok([minimum_x, maximum_x, minimum_y, maximum_y])
}

fn finite_f32(value: f64) -> Result<f32, GeometryError> {
    if !value.is_finite() || value < -f64::from(f32::MAX) || value > f64::from(f32::MAX) {
        return Err(GeometryError::ArithmeticOverflow("plot-box fit matrix"));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(matrix: ViewProjection3D) -> [f64; 4] {
        projected_bounds(matrix).unwrap()
    }

    #[test]
    fn stretch_to_fill_uses_both_target_dimensions() {
        let fit = PlotBoxFit3D::new(PlotBoxFitMode3D::StretchToFill, [0.9, 0.75], 1.0).unwrap();
        let fitted = fit_plot_box_3d(ViewProjection3D::identity(), fit).unwrap();
        let [minimum_x, maximum_x, minimum_y, maximum_y] = bounds(fitted);
        assert!((minimum_x + 0.9).abs() < 1.0e-6);
        assert!((maximum_x - 0.9).abs() < 1.0e-6);
        assert!((minimum_y + 0.75).abs() < 1.0e-6);
        assert!((maximum_y - 0.75).abs() < 1.0e-6);
    }

    #[test]
    fn aspect_preserving_fit_uses_the_limiting_dimension() {
        let fit = PlotBoxFit3D::new(PlotBoxFitMode3D::PreserveAspect, [0.9, 0.5], 1.0).unwrap();
        let fitted = fit_plot_box_3d(ViewProjection3D::identity(), fit).unwrap();
        let [minimum_x, maximum_x, minimum_y, maximum_y] = bounds(fitted);
        assert!((maximum_x - minimum_x - 1.0).abs() < 1.0e-6);
        assert!((maximum_y - minimum_y - 1.0).abs() < 1.0e-6);
        assert!(minimum_x >= -0.9 && maximum_x <= 0.9);
        assert!((minimum_y + 0.5).abs() < 1.0e-6);
        assert!((maximum_y - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn automatic_fit_preserves_explicit_user_zoom() {
        let fit = PlotBoxFit3D::new(PlotBoxFitMode3D::StretchToFill, [0.8, 0.7], 1.5).unwrap();
        let fitted = fit_plot_box_3d(ViewProjection3D::identity(), fit).unwrap();
        let [minimum_x, maximum_x, minimum_y, maximum_y] = bounds(fitted);
        assert!((minimum_x + 1.2).abs() < 1.0e-6);
        assert!((maximum_x - 1.2).abs() < 1.0e-6);
        assert!((minimum_y + 1.05).abs() < 1.0e-6);
        assert!((maximum_y - 1.05).abs() < 1.0e-6);
    }
}
