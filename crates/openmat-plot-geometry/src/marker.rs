use openmat_plot_mir::{
    AxesPoint, AxesRect, CssPx, MarkerBatch, MarkerInstance, MarkerShape, Rgba,
};

use crate::{GeometryError, SeriesGeometryInput};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerStyle {
    pub shape: MarkerShape,
    pub size_css_px: CssPx,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub stroke_width_css_px: CssPx,
}

/// Builds a renderer-independent marker batch from finite, optionally clipped points.
///
/// # Errors
/// Returns [`GeometryError`] for invalid views or coordinate-mapper failures.
pub fn marker_batch(
    input: SeriesGeometryInput<'_>,
    clip: Option<AxesRect>,
    style: MarkerStyle,
    mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
) -> Result<MarkerBatch, GeometryError> {
    marker_batch_rotated(input, clip, style, 0.0, mapper)
}

/// Builds a marker batch with one renderer-independent rotation applied to every instance.
///
/// # Errors
/// Returns [`GeometryError`] for invalid views, rotation, or coordinate-mapper failures.
pub fn marker_batch_rotated(
    input: SeriesGeometryInput<'_>,
    clip: Option<AxesRect>,
    style: MarkerStyle,
    rotation_radians: f32,
    mut mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
) -> Result<MarkerBatch, GeometryError> {
    input.validate()?;
    if !rotation_radians.is_finite() {
        return Err(GeometryError::NonFiniteCoordinate);
    }
    let mut instances = Vec::new();
    for index in 0..input.y.len() {
        let (x, y) = input.point(index)?;
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let center = mapper(x, y)?;
        if clip.is_none_or(|clip| clip.contains(center)) {
            instances.push(MarkerInstance {
                center,
                size_css_px: style.size_css_px,
                rotation_radians,
                fill: style.fill,
                stroke: style.stroke,
                stroke_width_css_px: style.stroke_width_css_px,
            });
        }
    }
    Ok(MarkerBatch {
        shape: style.shape,
        instances,
    })
}

/// Builds a marker batch from exact one-based source indices. Order and
/// duplicates are preserved; positive indices outside the current source data
/// and non-finite selected points are skipped.
///
/// # Errors
/// Returns [`GeometryError`] for zero indices, invalid views, rotation, or
/// coordinate-mapper failures.
pub fn marker_batch_at_indices(
    input: SeriesGeometryInput<'_>,
    source_indices_one_based: &[u64],
    clip: Option<AxesRect>,
    style: MarkerStyle,
    mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
) -> Result<MarkerBatch, GeometryError> {
    marker_batch_rotated_at_indices(input, source_indices_one_based, clip, style, 0.0, mapper)
}

/// Builds a rotated marker batch from exact one-based source indices.
///
/// # Errors
/// Returns [`GeometryError`] for zero indices, invalid views, rotation, or
/// coordinate-mapper failures.
pub fn marker_batch_rotated_at_indices(
    input: SeriesGeometryInput<'_>,
    source_indices_one_based: &[u64],
    clip: Option<AxesRect>,
    style: MarkerStyle,
    rotation_radians: f32,
    mut mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
) -> Result<MarkerBatch, GeometryError> {
    input.validate()?;
    if !rotation_radians.is_finite() {
        return Err(GeometryError::NonFiniteCoordinate);
    }
    let mut instances = Vec::with_capacity(source_indices_one_based.len().min(input.len()));
    for &one_based in source_indices_one_based {
        let Some(zero_based) = one_based.checked_sub(1) else {
            return Err(GeometryError::InvalidSourceIndex);
        };
        let Ok(index) = usize::try_from(zero_based) else {
            continue;
        };
        if index >= input.len() {
            continue;
        }
        let (x, y) = input.point(index)?;
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let center = mapper(x, y)?;
        if clip.is_none_or(|clip| clip.contains(center)) {
            instances.push(MarkerInstance {
                center,
                size_css_px: style.size_css_px,
                rotation_radians,
                fill: style.fill,
                stroke: style.stroke,
                stroke_width_css_px: style.stroke_width_css_px,
            });
        }
    }
    Ok(MarkerBatch {
        shape: style.shape,
        instances,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_truncation)]

    use super::*;
    use crate::NumericView;

    #[test]
    fn marker_batch_skips_non_finite_and_clipped_points() {
        let input = SeriesGeometryInput {
            x: None,
            y: NumericView::F64(&[0.5, f64::NAN, 2.0]),
        };
        let style = MarkerStyle {
            shape: MarkerShape::Circle,
            size_css_px: CssPx::new(6.0).unwrap(),
            fill: Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
            stroke: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            stroke_width_css_px: CssPx::new(1.0).unwrap(),
        };
        let batch = marker_batch(input, Some(AxesRect::unit()), style, |x, y| {
            AxesPoint::new((x / 3.0) as f32, y as f32).map_err(GeometryError::from)
        })
        .unwrap();
        assert_eq!(batch.instances.len(), 1);
    }

    #[test]
    fn rotated_batch_preserves_one_shared_finite_rotation() {
        let input = SeriesGeometryInput {
            x: None,
            y: NumericView::F64(&[0.25, 0.75]),
        };
        let style = MarkerStyle {
            shape: MarkerShape::UpTriangle,
            size_css_px: CssPx::new(6.0).unwrap(),
            fill: Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
            stroke: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            stroke_width_css_px: CssPx::new(1.0).unwrap(),
        };
        let batch =
            marker_batch_rotated(input, None, style, std::f32::consts::FRAC_PI_2, |x, y| {
                AxesPoint::new((x / 2.0) as f32, y as f32).map_err(GeometryError::from)
            })
            .unwrap();
        assert!(batch.instances.iter().all(|instance| {
            instance.rotation_radians.to_bits() == std::f32::consts::FRAC_PI_2.to_bits()
        }));
        assert!(
            marker_batch_rotated(input, None, style, f32::NAN, |x, y| {
                AxesPoint::new((x / 2.0) as f32, y as f32).map_err(GeometryError::from)
            })
            .is_err()
        );
    }

    #[test]
    fn indexed_markers_preserve_order_and_duplicates_while_skipping_missing_points() {
        let input = SeriesGeometryInput {
            x: Some(NumericView::F64(&[0.1, 0.2, 0.3])),
            y: NumericView::F64(&[1.0, f64::NAN, 3.0]),
        };
        let style = MarkerStyle {
            shape: MarkerShape::Circle,
            size_css_px: CssPx::new(6.0).unwrap(),
            fill: Rgba::TRANSPARENT,
            stroke: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            stroke_width_css_px: CssPx::new(1.0).unwrap(),
        };
        let batch = marker_batch_at_indices(input, &[3, 1, 3, 2, u64::MAX], None, style, |x, y| {
            AxesPoint::new(x as f32, y as f32).map_err(GeometryError::from)
        })
        .unwrap();
        assert_eq!(
            batch
                .instances
                .iter()
                .map(|instance| instance.center.x)
                .collect::<Vec<_>>(),
            vec![0.3, 0.1, 0.3]
        );
        assert_eq!(
            marker_batch_at_indices(input, &[0], None, style, |x, y| {
                AxesPoint::new(x as f32, y as f32).map_err(GeometryError::from)
            }),
            Err(GeometryError::InvalidSourceIndex)
        );
    }
}
