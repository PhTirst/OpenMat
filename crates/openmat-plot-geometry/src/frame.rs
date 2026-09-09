use openmat_plot_mir::{
    AxesPoint, AxesRect, DrawOrder, GridLineBatch, Lighting3D, LineSegmentBatch3D, MarkerBatch,
    MarkerBatch3D, MarkerShape, MirCommand, OverlayPlan, PickingId, PlotFrame, RenderOperation,
    Rgba, RulerLineBatch3D, RulerSelection3D, ScreenLineBatch, StrokeStyle, SurfaceMesh3D,
    TickMarkBatch3D, TriangleMesh2D, ViewProjection3D, Viewport,
};

use crate::marker::MarkerStyle;
use crate::{
    GeometryError, LineRun, SeriesGeometryInput, axes_line_runs, clip_line_runs, marker_batch,
    marker_batch_rotated, marker_batch_rotated_at_indices, split_line_runs, tessellate_stroke,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatterStyle {
    pub shape: MarkerShape,
    pub size_css_px: openmat_plot_mir::CssPx,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub stroke_width_css_px: openmat_plot_mir::CssPx,
}

/// Pure-Rust orchestration from independent series inputs to one MIR frame.
pub struct MirFrameBuilder {
    viewport: Viewport,
    clip: AxesRect,
    overlay: OverlayPlan,
    draws: Vec<RenderOperation>,
    view_projection_3d: Option<ViewProjection3D>,
    lighting_3d: Option<Lighting3D>,
    ruler_selection_3d: Option<RulerSelection3D>,
}

impl MirFrameBuilder {
    #[must_use]
    pub fn new(viewport: Viewport, clip: AxesRect, overlay: OverlayPlan) -> Self {
        Self {
            viewport,
            clip,
            overlay,
            draws: Vec::new(),
            view_projection_3d: None,
            lighting_3d: None,
            ruler_selection_3d: None,
        }
    }

    /// Sets the single camera matrix used by subsequent 3D draw operations.
    pub fn set_view_projection_3d(&mut self, matrix: ViewProjection3D) {
        self.view_projection_3d = Some(matrix);
    }

    /// Sets one camera-following light used by subsequent 3D surfaces.
    pub fn set_lighting_3d(&mut self, lighting: Lighting3D) {
        self.lighting_3d = Some(lighting);
    }

    /// Sets the renderer-local candidate edge selected for each 3D ruler.
    pub fn set_ruler_selection_3d(&mut self, selection: RulerSelection3D) {
        self.ruler_selection_3d = Some(selection);
    }

    /// Adds one renderer-ready structured surface mesh.
    pub fn add_surface_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        mesh: SurfaceMesh3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::SurfaceMesh3D(mesh),
        });
    }

    /// Adds one filled per-vertex-colored 2D triangle mesh.
    pub fn add_triangle_mesh_2d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        mesh: TriangleMesh2D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::TriangleMesh2D(mesh),
        });
    }

    /// Adds one surface whose fill depth is offset before coplanar grid edges
    /// are overlaid. The distinct command keeps polygon offset out of ordinary
    /// filled surfaces and unrelated 3D lines.
    pub fn add_surface_with_edges_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        mesh: SurfaceMesh3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::SurfaceMeshWithEdges3D(mesh),
        });
    }

    /// Adds camera-independent 3D line segments such as Surface edges or Axes grids.
    pub fn add_line_segments_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: LineSegmentBatch3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::LineSegments3D(batch),
        });
    }

    /// Adds depth-tested, CSS-sized markers anchored in normalized 3D space.
    pub fn add_markers_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: MarkerBatch3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::MarkerBatch3D(batch),
        });
    }

    /// Adds renderer-ready CSS-sized markers anchored in normalized 2D Axes space.
    pub fn add_markers(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: MarkerBatch,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::MarkerBatch(batch),
        });
    }

    /// Adds coplanar Surface grid edges through the dedicated wireframe
    /// overlay pipeline without changing ordinary 3D line depth semantics.
    pub fn add_surface_edge_segments_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: LineSegmentBatch3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::SurfaceEdgeSegments3D(batch),
        });
    }

    /// Adds all camera-independent candidate edges for dynamic 3D rulers.
    pub fn add_ruler_lines_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: RulerLineBatch3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::RulerLines3D(batch),
        });
    }

    /// Adds camera-aware fixed-CSS-length 3D tick marks.
    pub fn add_tick_marks_3d(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: TickMarkBatch3D,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::TickMarks3D(batch),
        });
    }

    /// Adds canvas-space non-text overlay lines.
    pub fn add_screen_lines(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        batch: ScreenLineBatch,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::ScreenLines(batch),
        });
    }

    pub fn add_grid(
        &mut self,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        grid: GridLineBatch,
    ) {
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::GridLineBatch(grid),
        });
    }

    #[allow(clippy::too_many_arguments)]
    /// Adds one clipped, tessellated line series.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for invalid input, mapping, tessellation, or overflow.
    pub fn add_line(
        &mut self,
        input: SeriesGeometryInput<'_>,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        style: StrokeStyle,
        color: Rgba,
        round_segments: u16,
        mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
    ) -> Result<(), GeometryError> {
        let runs = split_line_runs(input)?;
        self.add_line_runs(
            &runs,
            order,
            picking_id,
            style,
            color,
            round_segments,
            mapper,
        )
    }

    #[allow(clippy::too_many_arguments)]
    /// Adds preselected finite line runs, such as a viewport-dependent LOD result.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for invalid mapping, clipping, tessellation, or overflow.
    pub fn add_line_runs(
        &mut self,
        runs: &[LineRun],
        order: DrawOrder,
        picking_id: Option<PickingId>,
        style: StrokeStyle,
        color: Rgba,
        round_segments: u16,
        mut mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
    ) -> Result<(), GeometryError> {
        let axes_runs = axes_line_runs(runs, &mut mapper)?;
        let clipped = clip_line_runs(&axes_runs, self.clip);
        let tessellation = tessellate_stroke(
            &clipped,
            self.viewport.css.size,
            style,
            color,
            round_segments,
        )?;
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::StrokeMesh(tessellation.mesh),
        });
        Ok(())
    }

    /// Adds one clipped scatter series.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for invalid input or coordinate mapping.
    pub fn add_scatter(
        &mut self,
        input: SeriesGeometryInput<'_>,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        style: ScatterStyle,
        mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
    ) -> Result<(), GeometryError> {
        let batch = marker_batch(
            input,
            Some(self.clip),
            MarkerStyle {
                shape: style.shape,
                size_css_px: style.size_css_px,
                fill: style.fill,
                stroke: style.stroke,
                stroke_width_css_px: style.stroke_width_css_px,
            },
            mapper,
        )?;
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::MarkerBatch(batch),
        });
        Ok(())
    }

    /// Adds one clipped scatter series with a shared marker rotation.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for invalid input, rotation, or coordinate mapping.
    pub fn add_scatter_rotated(
        &mut self,
        input: SeriesGeometryInput<'_>,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        style: ScatterStyle,
        rotation_radians: f32,
        mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
    ) -> Result<(), GeometryError> {
        let batch = marker_batch_rotated(
            input,
            Some(self.clip),
            MarkerStyle {
                shape: style.shape,
                size_css_px: style.size_css_px,
                fill: style.fill,
                stroke: style.stroke,
                stroke_width_css_px: style.stroke_width_css_px,
            },
            rotation_radians,
            mapper,
        )?;
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::MarkerBatch(batch),
        });
        Ok(())
    }

    /// Adds one clipped marker series selected by exact one-based source indices.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for invalid input, indices, rotation, or mapping.
    #[allow(clippy::too_many_arguments)]
    pub fn add_scatter_rotated_at_indices(
        &mut self,
        input: SeriesGeometryInput<'_>,
        source_indices_one_based: &[u64],
        order: DrawOrder,
        picking_id: Option<PickingId>,
        style: ScatterStyle,
        rotation_radians: f32,
        mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
    ) -> Result<(), GeometryError> {
        let batch = marker_batch_rotated_at_indices(
            input,
            source_indices_one_based,
            Some(self.clip),
            MarkerStyle {
                shape: style.shape,
                size_css_px: style.size_css_px,
                fill: style.fill,
                stroke: style.stroke,
                stroke_width_css_px: style.stroke_width_css_px,
            },
            rotation_radians,
            mapper,
        )?;
        self.draws.push(RenderOperation {
            order,
            picking_id,
            command: MirCommand::MarkerBatch(batch),
        });
        Ok(())
    }

    /// Sorts draw operations and closes the validated viewport command stream.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for operation-count overflow or invalid MIR.
    pub fn finish(mut self) -> Result<PlotFrame, GeometryError> {
        self.draws.sort_by_key(|operation| operation.order);
        let mut operations = Vec::with_capacity(
            self.draws
                .len()
                .checked_add(
                    3 + usize::from(self.view_projection_3d.is_some())
                        + usize::from(self.lighting_3d.is_some())
                        + usize::from(self.ruler_selection_3d.is_some()),
                )
                .ok_or(GeometryError::ArithmeticOverflow("MIR operation count"))?,
        );
        operations.push(RenderOperation {
            order: DrawOrder(i64::MIN),
            picking_id: None,
            command: MirCommand::BeginViewport(self.viewport),
        });
        operations.push(RenderOperation {
            order: DrawOrder(i64::MIN + 1),
            picking_id: None,
            command: MirCommand::SetClipRect(self.clip),
        });
        if let Some(matrix) = self.view_projection_3d {
            operations.push(RenderOperation {
                order: DrawOrder(i64::MIN + 2),
                picking_id: None,
                command: MirCommand::SetViewProjection3D(matrix),
            });
        }
        if let Some(lighting) = self.lighting_3d {
            operations.push(RenderOperation {
                order: DrawOrder(i64::MIN + 3),
                picking_id: None,
                command: MirCommand::SetLighting3D(lighting),
            });
        }
        if let Some(selection) = self.ruler_selection_3d {
            operations.push(RenderOperation {
                order: DrawOrder(i64::MIN + 4),
                picking_id: None,
                command: MirCommand::SetRulerSelection3D(selection),
            });
        }
        operations.extend(self.draws);
        operations.push(RenderOperation {
            order: DrawOrder(i64::MAX),
            picking_id: None,
            command: MirCommand::EndViewport,
        });
        PlotFrame::new(operations, self.overlay).map_err(GeometryError::from)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_truncation)]

    use super::*;
    use openmat_plot_mir::{CssPx, CssRect, DevicePixelRatio, DeviceRect, LineCap, LineJoin};

    use crate::NumericView;

    #[test]
    fn builds_line_and_scatter_frame_without_platform_types() {
        let viewport = Viewport {
            css: CssRect::new(0.0, 0.0, 640.0, 480.0).unwrap(),
            device: DeviceRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 960,
            },
            device_pixel_ratio: DevicePixelRatio::new(2.0).unwrap(),
        };
        let mut builder = MirFrameBuilder::new(viewport, AxesRect::unit(), OverlayPlan::default());
        let input = SeriesGeometryInput {
            x: None,
            y: NumericView::F64(&[0.0, 0.5, 1.0]),
        };
        builder
            .add_line(
                input,
                DrawOrder(10),
                None,
                StrokeStyle::solid(CssPx::new(2.0).unwrap(), LineCap::Butt, LineJoin::Miter)
                    .unwrap(),
                Rgba::new(0.0, 0.0, 1.0, 1.0).unwrap(),
                8,
                |x, y| {
                    AxesPoint::new(((x - 1.0) / 2.0) as f32, y as f32).map_err(GeometryError::from)
                },
            )
            .unwrap();
        builder
            .add_scatter(
                input,
                DrawOrder(20),
                None,
                ScatterStyle {
                    shape: MarkerShape::Circle,
                    size_css_px: CssPx::new(6.0).unwrap(),
                    fill: Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
                    stroke: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
                    stroke_width_css_px: CssPx::new(1.0).unwrap(),
                },
                |x, y| {
                    AxesPoint::new(((x - 1.0) / 2.0) as f32, y as f32).map_err(GeometryError::from)
                },
            )
            .unwrap();
        let frame = builder.finish().unwrap();
        assert!(matches!(
            frame.operations[0].command,
            MirCommand::BeginViewport(_)
        ));
        assert!(
            frame
                .operations
                .iter()
                .any(|operation| matches!(operation.command, MirCommand::StrokeMesh(_)))
        );
        assert!(
            frame
                .operations
                .iter()
                .any(|operation| matches!(operation.command, MirCommand::MarkerBatch(_)))
        );
        assert!(matches!(
            frame.operations.last().unwrap().command,
            MirCommand::EndViewport
        ));
    }

    #[test]
    fn adds_preselected_line_runs_without_reselecting_source_data() {
        let viewport = Viewport {
            css: CssRect::new(0.0, 0.0, 100.0, 100.0).unwrap(),
            device: DeviceRect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            device_pixel_ratio: DevicePixelRatio::new(1.0).unwrap(),
        };
        let mut builder = MirFrameBuilder::new(viewport, AxesRect::unit(), OverlayPlan::default());
        builder
            .add_line_runs(
                &[LineRun {
                    points: vec![
                        crate::DataPoint {
                            source_index: 0,
                            x: 0.0,
                            y: 0.0,
                        },
                        crate::DataPoint {
                            source_index: 1,
                            x: 1.0,
                            y: 1.0,
                        },
                    ],
                }],
                DrawOrder(1),
                None,
                StrokeStyle::solid(CssPx::new(1.0).unwrap(), LineCap::Butt, LineJoin::Miter)
                    .unwrap(),
                Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
                8,
                |x, y| AxesPoint::new(x as f32, y as f32).map_err(GeometryError::from),
            )
            .unwrap();
        let frame = builder.finish().unwrap();
        assert!(frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::StrokeMesh(mesh) if !mesh.indices.is_empty()
        )));
    }
}
