use std::cmp::Ordering;

use openmat_plot_mir::{
    AxesLineSegment, AxesPoint, CssPoint, DrawOrder, MarkerBatch, PickingId, StrokeMesh,
};

use crate::{InteractionError, ViewTransform2D};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickTolerance(f64);

impl PickTolerance {
    /// Creates a non-negative finite CSS-pixel tolerance.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for negative or non-finite input.
    pub fn new(css_px: f64) -> Result<Self, InteractionError> {
        if !css_px.is_finite() {
            return Err(InteractionError::NonFinite("pick tolerance"));
        }
        if css_px < 0.0 {
            return Err(InteractionError::NonPositive("pick tolerance"));
        }
        Ok(Self(css_px))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    LineSegment,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickHit {
    pub picking_id: PickingId,
    pub kind: PickKind,
    pub primitive_index: u32,
    pub distance_css_px: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PickGeometry {
    Line {
        start: AxesPoint,
        end: AxesPoint,
        half_width_css_px: f64,
    },
    Marker {
        center: AxesPoint,
        radius_css_px: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PickCandidate {
    picking_id: PickingId,
    order: DrawOrder,
    kind: PickKind,
    primitive_index: u32,
    geometry: PickGeometry,
}

/// Deterministic CPU picking index retained alongside immutable GPU buffers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickingIndex {
    candidates: Vec<PickCandidate>,
}

impl PickingIndex {
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Returns the nearest candidate in CSS pixels. Equal-distance hits prefer
    /// later draw order and then stable identifier/primitive order.
    #[must_use]
    pub fn pick(
        &self,
        view: ViewTransform2D,
        pointer: CssPoint,
        tolerance: PickTolerance,
    ) -> Option<PickHit> {
        let clamped = view.viewport().clamp_css_point(pointer).ok()?;
        if clamped != pointer {
            return None;
        }
        let pointer = (f64::from(pointer.x), f64::from(pointer.y));
        let mut best: Option<(PickHit, DrawOrder)> = None;
        for candidate in &self.candidates {
            let distance = match candidate.geometry {
                PickGeometry::Line {
                    start,
                    end,
                    half_width_css_px,
                } => {
                    let start = view.source_axes_to_css(start).ok()?;
                    let end = view.source_axes_to_css(end).ok()?;
                    point_segment_distance(
                        pointer,
                        (f64::from(start.x), f64::from(start.y)),
                        (f64::from(end.x), f64::from(end.y)),
                    ) - half_width_css_px
                }
                PickGeometry::Marker {
                    center,
                    radius_css_px,
                } => {
                    let center = view.source_axes_to_css(center).ok()?;
                    (pointer.0 - f64::from(center.x)).hypot(pointer.1 - f64::from(center.y))
                        - radius_css_px
                }
            }
            .max(0.0);
            if distance > tolerance.get() {
                continue;
            }
            let hit = PickHit {
                picking_id: candidate.picking_id,
                kind: candidate.kind,
                primitive_index: candidate.primitive_index,
                distance_css_px: distance,
            };
            let replace = best.as_ref().is_none_or(|(current, current_order)| {
                match distance.total_cmp(&current.distance_css_px) {
                    Ordering::Less => true,
                    Ordering::Greater => false,
                    Ordering::Equal => {
                        candidate.order > *current_order
                            || (candidate.order == *current_order
                                && (candidate.picking_id.get(), candidate.primitive_index)
                                    < (current.picking_id.get(), current.primitive_index))
                    }
                }
            });
            if replace {
                best = Some((hit, candidate.order));
            }
        }
        best.map(|(hit, _)| hit)
    }

    pub(crate) fn add_line_segments(
        &mut self,
        picking_id: Option<PickingId>,
        order: DrawOrder,
        segments: impl IntoIterator<Item = AxesLineSegment>,
        width_css_px: f32,
    ) {
        let Some(picking_id) = picking_id else {
            return;
        };
        for (index, segment) in segments.into_iter().enumerate() {
            let Ok(primitive_index) = u32::try_from(index) else {
                break;
            };
            self.candidates.push(PickCandidate {
                picking_id,
                order,
                kind: PickKind::LineSegment,
                primitive_index,
                geometry: PickGeometry::Line {
                    start: segment.start,
                    end: segment.end,
                    half_width_css_px: f64::from(width_css_px) * 0.5,
                },
            });
        }
    }

    pub(crate) fn add_stroke_mesh(
        &mut self,
        picking_id: Option<PickingId>,
        order: DrawOrder,
        mesh: &StrokeMesh,
    ) {
        let segments = mesh_centerline_segments(mesh);
        self.add_line_segments(picking_id, order, segments, mesh.style.width_css_px.get());
    }

    pub(crate) fn add_markers(
        &mut self,
        picking_id: Option<PickingId>,
        order: DrawOrder,
        batch: &MarkerBatch,
    ) {
        let Some(picking_id) = picking_id else {
            return;
        };
        for (index, marker) in batch.instances.iter().enumerate() {
            let Ok(primitive_index) = u32::try_from(index) else {
                break;
            };
            self.candidates.push(PickCandidate {
                picking_id,
                order,
                kind: PickKind::Marker,
                primitive_index,
                geometry: PickGeometry::Marker {
                    center: marker.center,
                    radius_css_px: f64::from(marker.size_css_px.get()) * 0.5,
                },
            });
        }
    }
}

fn mesh_centerline_segments(mesh: &StrokeMesh) -> Vec<AxesLineSegment> {
    let mut segments = Vec::new();
    for indices in mesh.indices.chunks_exact(6) {
        let Some(vertices) = indices
            .iter()
            .map(|&index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| mesh.vertices.get(index))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        // Geometry tessellation emits each segment quad as two triangles with
        // signed opposite stroke edges. Joins/caps do not satisfy this layout.
        let [
            left_start,
            right_start,
            left_end,
            repeated_right_start,
            right_end,
            repeated_left_end,
        ] = vertices.as_slice()
        else {
            continue;
        };
        if right_start.position != repeated_right_start.position
            || left_end.position != repeated_left_end.position
            || left_start.edge_distance_css_px.signum() == right_start.edge_distance_css_px.signum()
            || right_end.edge_distance_css_px.signum()
                == repeated_left_end.edge_distance_css_px.signum()
        {
            continue;
        }
        let start = AxesPoint {
            x: (left_start.position.x + right_start.position.x) * 0.5,
            y: (left_start.position.y + right_start.position.y) * 0.5,
        };
        let end = AxesPoint {
            x: (right_end.position.x + repeated_left_end.position.x) * 0.5,
            y: (right_end.position.y + repeated_left_end.position.y) * 0.5,
        };
        segments.push(AxesLineSegment { start, end });
    }
    segments
}

fn point_segment_distance(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length_squared = dx.mul_add(dx, dy * dy);
    if length_squared <= f64::EPSILON {
        return (point.0 - start.0).hypot(point.1 - start.1);
    }
    let projection = ((point.0 - start.0).mul_add(dx, (point.1 - start.1) * dy) / length_squared)
        .clamp(0.0, 1.0);
    let nearest = (start.0 + projection * dx, start.1 + projection * dy);
    (point.0 - nearest.0).hypot(point.1 - nearest.1)
}

#[cfg(test)]
mod tests {
    use openmat_plot_mir::{
        CssPx, CssRect, DevicePixelRatio, DeviceRect, MarkerInstance, MarkerShape, Rgba,
    };

    use super::*;
    use crate::DataRect;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn view(dpr: f64) -> ViewTransform2D {
        ViewTransform2D::new(
            openmat_plot_mir::Viewport {
                css: CssRect::new(0.0, 0.0, 200.0, 100.0).unwrap(),
                device: DeviceRect {
                    x: 0,
                    y: 0,
                    width: (200.0 * dpr).round() as u32,
                    height: (100.0 * dpr).round() as u32,
                },
                device_pixel_ratio: DevicePixelRatio::new(dpr).unwrap(),
            },
            DataRect::new(0.0, 10.0, 0.0, 10.0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn nearest_line_and_marker_pick_is_stable_across_dpr() {
        let mut index = PickingIndex::default();
        index.add_line_segments(
            Some(PickingId::from_u64(10).unwrap()),
            DrawOrder(1),
            [AxesLineSegment {
                start: AxesPoint { x: 0.0, y: 0.5 },
                end: AxesPoint { x: 1.0, y: 0.5 },
            }],
            2.0,
        );
        index.add_markers(
            Some(PickingId::from_u64(20).unwrap()),
            DrawOrder(2),
            &MarkerBatch {
                shape: MarkerShape::Circle,
                instances: vec![MarkerInstance {
                    center: AxesPoint { x: 0.5, y: 0.5 },
                    size_css_px: CssPx::new(8.0).unwrap(),
                    rotation_radians: 0.0,
                    fill: Rgba::TRANSPARENT,
                    stroke: Rgba::TRANSPARENT,
                    stroke_width_css_px: CssPx::new(1.0).unwrap(),
                }],
            },
        );
        for dpr in [1.0, 1.25, 2.0, 3.0] {
            let hit = index
                .pick(
                    view(dpr),
                    CssPoint::new(100.0, 50.0).unwrap(),
                    PickTolerance::new(5.0).unwrap(),
                )
                .unwrap();
            assert_eq!(hit.picking_id.get(), 20);
            assert_eq!(hit.kind, PickKind::Marker);
            assert!(hit.distance_css_px.abs() < f64::EPSILON);
        }
    }

    #[test]
    fn tolerance_and_outside_viewport_are_enforced_in_css_pixels() {
        let mut index = PickingIndex::default();
        index.add_line_segments(
            Some(PickingId::from_u64(7).unwrap()),
            DrawOrder(1),
            [AxesLineSegment {
                start: AxesPoint { x: 0.0, y: 0.5 },
                end: AxesPoint { x: 1.0, y: 0.5 },
            }],
            0.0,
        );
        assert!(
            index
                .pick(
                    view(2.0),
                    CssPoint::new(50.0, 56.0).unwrap(),
                    PickTolerance::new(5.0).unwrap(),
                )
                .is_none()
        );
        assert!(
            index
                .pick(
                    view(2.0),
                    CssPoint::new(-1.0, 50.0).unwrap(),
                    PickTolerance::new(10.0).unwrap(),
                )
                .is_none()
        );
    }
}
