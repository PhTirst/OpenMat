use std::num::NonZeroU64;
use std::ops::Range;

use openmat_plot_mir::{
    AxesLineSegment, AxesPoint, AxesRect, AxisDimension, DeviceRect, DrawOrder, GridLineBatch,
    Lighting3D, LinePattern3D, LineSegment3D, LineSegmentBatch3D, MarkerBatch, MarkerBatch3D,
    MarkerShape, MirCommand, PickingId, PlotFrame, RenderOperation, Rgba, RulerLine3D,
    RulerLineBatch3D, RulerSelection3D, ScreenLine, ScreenLineBatch, StrokeMesh, StrokeVertex,
    SurfaceMesh3D, SurfaceVertex3D, TickMark3D, TickMarkBatch3D, TriangleMesh2D, ViewProjection3D,
    Viewport,
};

use crate::{BufferKind, CompileError, PickingIndex, REQUIRED_MAX_BUFFER_SIZE};

pub const MESH_VERTEX_STRIDE: u64 = 40;
pub const MARKER_INSTANCE_STRIDE: u64 = 64;
pub const SURFACE_VERTEX_3D_STRIDE: u64 = 40;
pub const LINE_SEGMENT_3D_STRIDE: u64 = 56;
pub const SCREEN_LINE_STRIDE: u64 = 48;
pub const INDEX_STRIDE: u64 = 4;

const ANTIALIAS_FRINGE_CSS_PX: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameCompileOptions {
    pub clear_color: Rgba,
    pub max_buffer_bytes: u64,
}

impl Default for FrameCompileOptions {
    fn default() -> Self {
        Self {
            clear_color: Rgba::TRANSPARENT,
            max_buffer_bytes: REQUIRED_MAX_BUFFER_SIZE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawKind {
    Triangles {
        indices: Range<u32>,
    },
    Markers {
        shape: MarkerShape,
        instances: Range<u32>,
    },
    SurfaceTriangles3D {
        indices: Range<u32>,
        interpolation: openmat_plot_mir::SurfaceColorInterpolation,
        edge_overlay: bool,
    },
    LineSegments3D {
        instances: Range<u32>,
    },
    SurfaceEdgeSegments3D {
        instances: Range<u32>,
    },
    RulerLines3D {
        instances: Range<u32>,
    },
    TickMarks3D {
        instances: Range<u32>,
    },
    ScreenLines {
        instances: Range<u32>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawCommand {
    pub order: DrawOrder,
    pub picking_id: Option<NonZeroU64>,
    pub scissor: DeviceRect,
    pub kind: DrawKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledFrame {
    viewport: Viewport,
    clear_color: Rgba,
    mesh_vertices: Vec<u8>,
    mesh_indices: Vec<u8>,
    marker_instances: Vec<u8>,
    surface_vertices_3d: Vec<u8>,
    surface_indices_3d: Vec<u8>,
    line_segments_3d: Vec<u8>,
    screen_lines: Vec<u8>,
    view_projection_3d: Option<ViewProjection3D>,
    lighting_3d: Option<Lighting3D>,
    ruler_selection_3d: Option<RulerSelection3D>,
    draws: Vec<DrawCommand>,
    picking: PickingIndex,
}

impl CompiledFrame {
    #[must_use]
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    #[must_use]
    pub const fn clear_color(&self) -> Rgba {
        self.clear_color
    }

    #[must_use]
    pub fn mesh_vertex_bytes(&self) -> &[u8] {
        &self.mesh_vertices
    }

    #[must_use]
    pub fn mesh_index_bytes(&self) -> &[u8] {
        &self.mesh_indices
    }

    #[must_use]
    pub fn marker_instance_bytes(&self) -> &[u8] {
        &self.marker_instances
    }

    #[must_use]
    pub fn surface_vertex_3d_bytes(&self) -> &[u8] {
        &self.surface_vertices_3d
    }

    #[must_use]
    pub fn surface_index_3d_bytes(&self) -> &[u8] {
        &self.surface_indices_3d
    }

    #[must_use]
    pub fn line_segment_3d_bytes(&self) -> &[u8] {
        &self.line_segments_3d
    }

    #[must_use]
    pub fn screen_line_bytes(&self) -> &[u8] {
        &self.screen_lines
    }

    #[must_use]
    pub const fn view_projection_3d(&self) -> Option<ViewProjection3D> {
        self.view_projection_3d
    }

    #[must_use]
    pub const fn lighting_3d(&self) -> Option<Lighting3D> {
        self.lighting_3d
    }

    #[must_use]
    pub const fn ruler_selection_3d(&self) -> Option<RulerSelection3D> {
        self.ruler_selection_3d
    }

    #[must_use]
    pub fn draws(&self) -> &[DrawCommand] {
        &self.draws
    }

    #[must_use]
    pub const fn picking_index(&self) -> &PickingIndex {
        &self.picking
    }
}

pub struct DrawListCompiler {
    options: FrameCompileOptions,
}

impl DrawListCompiler {
    #[must_use]
    pub const fn new(options: FrameCompileOptions) -> Self {
        Self { options }
    }

    /// Compiles one validated MIR viewport into deterministic GPU buffer payloads.
    ///
    /// # Errors
    /// Returns [`CompileError`] for invalid MIR, unsupported markers, overflow, or buffer limits.
    pub fn compile(&self, frame: &PlotFrame) -> Result<CompiledFrame, CompileError> {
        frame.validate()?;
        self.options.clear_color.validate()?;

        let viewport = match frame.operations.first().map(|operation| &operation.command) {
            Some(MirCommand::BeginViewport(viewport)) => *viewport,
            _ => {
                return Err(openmat_plot_mir::MirError::InvalidCommandStream(
                    "first command must begin a viewport",
                )
                .into());
            }
        };
        let mut output = CompileOutput::new(viewport, self.options);
        let mut scissor = axes_rect_to_scissor(AxesRect::unit(), viewport);

        for operation in &frame.operations {
            compile_operation(&mut output, operation, viewport, &mut scissor)?;
        }

        Ok(output.finish())
    }
}

fn compile_operation(
    output: &mut CompileOutput,
    operation: &RenderOperation,
    viewport: Viewport,
    scissor: &mut DeviceRect,
) -> Result<(), CompileError> {
    match &operation.command {
        MirCommand::BeginViewport(_) | MirCommand::EndViewport => {}
        MirCommand::SetClipRect(clip) => {
            *scissor = axes_rect_to_scissor(*clip, viewport);
        }
        MirCommand::SetViewProjection3D(matrix) => {
            output.frame.view_projection_3d = Some(*matrix);
        }
        MirCommand::SetLighting3D(lighting) => {
            output.frame.lighting_3d = Some(*lighting);
        }
        MirCommand::SetRulerSelection3D(selection) => {
            output.frame.ruler_selection_3d = Some(*selection);
        }
        MirCommand::GridLineBatch(batch) => {
            output.append_grid_lines(batch, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::StrokeMesh(mesh) => {
            output.append_mesh(mesh, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::TriangleMesh2D(mesh) => {
            output.append_triangle_mesh_2d(
                mesh,
                operation.order,
                operation.picking_id,
                *scissor,
            )?;
        }
        MirCommand::MarkerBatch(batch) => {
            output.append_markers(batch, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::MarkerBatch3D(batch) => {
            output.append_markers_3d(batch, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::SurfaceMesh3D(mesh) => output.append_surface_3d(
            mesh,
            operation.order,
            operation.picking_id,
            *scissor,
            false,
        )?,
        MirCommand::SurfaceMeshWithEdges3D(mesh) => {
            output.append_surface_3d(
                mesh,
                operation.order,
                operation.picking_id,
                *scissor,
                true,
            )?;
        }
        MirCommand::LineSegments3D(batch) => output.append_line_segments_3d(
            batch,
            operation.order,
            operation.picking_id,
            *scissor,
            false,
        )?,
        MirCommand::SurfaceEdgeSegments3D(batch) => output.append_line_segments_3d(
            batch,
            operation.order,
            operation.picking_id,
            *scissor,
            true,
        )?,
        MirCommand::RulerLines3D(batch) => {
            output.append_ruler_lines_3d(batch, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::TickMarks3D(batch) => {
            output.append_tick_marks_3d(batch, operation.order, operation.picking_id, *scissor)?;
        }
        MirCommand::ScreenLines(batch) => {
            output.append_screen_lines(batch, operation.order, operation.picking_id)?;
        }
        _ => return Err(CompileError::UnsupportedMirCommand),
    }
    Ok(())
}

struct CompileOutput {
    frame: CompiledFrame,
    options: FrameCompileOptions,
    mesh_vertex_count: u32,
    mesh_index_count: u32,
    marker_instance_count: u32,
    surface_vertex_3d_count: u32,
    surface_index_3d_count: u32,
    line_segment_3d_count: u32,
    screen_line_count: u32,
}

impl CompileOutput {
    fn new(viewport: Viewport, options: FrameCompileOptions) -> Self {
        Self {
            frame: CompiledFrame {
                viewport,
                clear_color: options.clear_color,
                mesh_vertices: Vec::new(),
                mesh_indices: Vec::new(),
                marker_instances: Vec::new(),
                surface_vertices_3d: Vec::new(),
                surface_indices_3d: Vec::new(),
                line_segments_3d: Vec::new(),
                screen_lines: Vec::new(),
                view_projection_3d: None,
                lighting_3d: None,
                ruler_selection_3d: None,
                draws: Vec::new(),
                picking: PickingIndex::default(),
            },
            options,
            mesh_vertex_count: 0,
            mesh_index_count: 0,
            marker_instance_count: 0,
            surface_vertex_3d_count: 0,
            surface_index_3d_count: 0,
            line_segment_3d_count: 0,
            screen_line_count: 0,
        }
    }

    fn finish(self) -> CompiledFrame {
        self.frame
    }

    fn append_mesh(
        &mut self,
        mesh: &StrokeMesh,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        self.frame.picking.add_stroke_mesh(picking_id, order, mesh);
        if mesh.indices.is_empty() {
            return Ok(());
        }

        self.ensure_append(
            BufferKind::MeshVertex,
            self.frame.mesh_vertices.len(),
            mesh.vertices.len(),
            MESH_VERTEX_STRIDE,
        )?;
        self.ensure_append(
            BufferKind::MeshIndex,
            self.frame.mesh_indices.len(),
            mesh.indices.len(),
            INDEX_STRIDE,
        )?;

        let base_vertex = self.mesh_vertex_count;
        let vertex_increment =
            u32::try_from(mesh.vertices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.mesh_vertex_count = self
            .mesh_vertex_count
            .checked_add(vertex_increment)
            .ok_or(CompileError::TooManyVertices)?;

        for vertex in &mesh.vertices {
            push_mesh_vertex(
                &mut self.frame.mesh_vertices,
                vertex.position,
                mesh.color,
                vertex.edge_distance_css_px,
                mesh.style.width_css_px.get() * 0.5,
            );
        }

        let first_index = self.mesh_index_count;
        for &index in &mesh.indices {
            let absolute = base_vertex
                .checked_add(index)
                .ok_or(CompileError::TooManyVertices)?;
            push_u32(&mut self.frame.mesh_indices, absolute);
        }
        let index_increment =
            u32::try_from(mesh.indices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.mesh_index_count = self
            .mesh_index_count
            .checked_add(index_increment)
            .ok_or(CompileError::TooManyVertices)?;
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::Triangles {
                indices: first_index..self.mesh_index_count,
            },
        });
        Ok(())
    }

    fn append_triangle_mesh_2d(
        &mut self,
        mesh: &TriangleMesh2D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        if mesh.indices.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::MeshVertex,
            self.frame.mesh_vertices.len(),
            mesh.vertices.len(),
            MESH_VERTEX_STRIDE,
        )?;
        self.ensure_append(
            BufferKind::MeshIndex,
            self.frame.mesh_indices.len(),
            mesh.indices.len(),
            INDEX_STRIDE,
        )?;
        let base_vertex = self.mesh_vertex_count;
        let vertex_increment =
            u32::try_from(mesh.vertices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.mesh_vertex_count = self
            .mesh_vertex_count
            .checked_add(vertex_increment)
            .ok_or(CompileError::TooManyVertices)?;
        for vertex in &mesh.vertices {
            push_mesh_vertex(
                &mut self.frame.mesh_vertices,
                vertex.position,
                vertex.color,
                0.0,
                1.0,
            );
        }
        let first_index = self.mesh_index_count;
        for &index in &mesh.indices {
            push_u32(
                &mut self.frame.mesh_indices,
                base_vertex
                    .checked_add(index)
                    .ok_or(CompileError::TooManyVertices)?,
            );
        }
        let index_increment =
            u32::try_from(mesh.indices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.mesh_index_count = self
            .mesh_index_count
            .checked_add(index_increment)
            .ok_or(CompileError::TooManyVertices)?;
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::Triangles {
                indices: first_index..self.mesh_index_count,
            },
        });
        Ok(())
    }

    fn append_grid_lines(
        &mut self,
        batch: &GridLineBatch,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        self.frame.picking.add_line_segments(
            picking_id,
            order,
            batch.segments.iter().copied(),
            batch.width_css_px.get(),
        );
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for segment in &batch.segments {
            if let Some(quad) =
                tessellate_segment(*segment, batch.width_css_px.get(), self.frame.viewport)
            {
                let base =
                    u32::try_from(vertices.len()).map_err(|_| CompileError::TooManyVertices)?;
                vertices.extend(quad);
                let second = base.checked_add(1).ok_or(CompileError::TooManyVertices)?;
                let third = base.checked_add(2).ok_or(CompileError::TooManyVertices)?;
                let fourth = base.checked_add(3).ok_or(CompileError::TooManyVertices)?;
                indices.extend([base, second, third, base, third, fourth]);
            }
        }
        if indices.is_empty() {
            return Ok(());
        }

        self.ensure_append(
            BufferKind::MeshVertex,
            self.frame.mesh_vertices.len(),
            vertices.len(),
            MESH_VERTEX_STRIDE,
        )?;
        self.ensure_append(
            BufferKind::MeshIndex,
            self.frame.mesh_indices.len(),
            indices.len(),
            INDEX_STRIDE,
        )?;

        let base_vertex = self.mesh_vertex_count;
        let vertex_increment =
            u32::try_from(vertices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.mesh_vertex_count = self
            .mesh_vertex_count
            .checked_add(vertex_increment)
            .ok_or(CompileError::TooManyVertices)?;
        let effective_width_css_px = effective_hairline_width_css_px(
            batch.width_css_px.get(),
            self.frame.viewport.device_pixel_ratio.get(),
        );
        for vertex in vertices {
            push_mesh_vertex(
                &mut self.frame.mesh_vertices,
                vertex.position,
                batch.color,
                vertex.edge_distance_css_px,
                effective_width_css_px * 0.5,
            );
        }

        let first_index = self.mesh_index_count;
        let index_increment =
            u32::try_from(indices.len()).map_err(|_| CompileError::TooManyVertices)?;
        for index in indices {
            let absolute = base_vertex
                .checked_add(index)
                .ok_or(CompileError::TooManyVertices)?;
            push_u32(&mut self.frame.mesh_indices, absolute);
        }
        self.mesh_index_count = self
            .mesh_index_count
            .checked_add(index_increment)
            .ok_or(CompileError::TooManyVertices)?;
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::Triangles {
                indices: first_index..self.mesh_index_count,
            },
        });
        Ok(())
    }

    fn append_markers(
        &mut self,
        batch: &MarkerBatch,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        if batch.instances.is_empty() {
            return Ok(());
        }
        self.frame.picking.add_markers(picking_id, order, batch);
        self.ensure_append(
            BufferKind::MarkerInstance,
            self.frame.marker_instances.len(),
            batch.instances.len(),
            MARKER_INSTANCE_STRIDE,
        )?;

        let first_instance = self.marker_instance_count;
        let instance_increment = u32::try_from(batch.instances.len())
            .map_err(|_| CompileError::TooManyMarkerInstances)?;
        self.marker_instance_count = self
            .marker_instance_count
            .checked_add(instance_increment)
            .ok_or(CompileError::TooManyMarkerInstances)?;
        for instance in &batch.instances {
            push_marker_instance(
                &mut self.frame.marker_instances,
                instance.center,
                instance.size_css_px.get(),
                instance.rotation_radians,
                instance.fill,
                instance.stroke,
                instance.stroke_width_css_px.get(),
            );
        }
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::Markers {
                shape: batch.shape,
                instances: first_instance..self.marker_instance_count,
            },
        });
        Ok(())
    }

    fn append_markers_3d(
        &mut self,
        batch: &MarkerBatch3D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        if batch.instances.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::MarkerInstance,
            self.frame.marker_instances.len(),
            batch.instances.len(),
            MARKER_INSTANCE_STRIDE,
        )?;
        let first_instance = self.marker_instance_count;
        let instance_increment = u32::try_from(batch.instances.len())
            .map_err(|_| CompileError::TooManyMarkerInstances)?;
        self.marker_instance_count = self
            .marker_instance_count
            .checked_add(instance_increment)
            .ok_or(CompileError::TooManyMarkerInstances)?;
        for instance in &batch.instances {
            push_marker_instance_3d(
                &mut self.frame.marker_instances,
                instance.center,
                instance.size_css_px.get(),
                instance.rotation_radians,
                instance.fill,
                instance.stroke,
                instance.stroke_width_css_px.get(),
            );
        }
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::Markers {
                shape: batch.shape,
                instances: first_instance..self.marker_instance_count,
            },
        });
        Ok(())
    }

    fn append_surface_3d(
        &mut self,
        mesh: &SurfaceMesh3D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
        edge_overlay: bool,
    ) -> Result<(), CompileError> {
        if mesh.indices.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::SurfaceVertex3D,
            self.frame.surface_vertices_3d.len(),
            mesh.vertices.len(),
            SURFACE_VERTEX_3D_STRIDE,
        )?;
        self.ensure_append(
            BufferKind::SurfaceIndex3D,
            self.frame.surface_indices_3d.len(),
            mesh.indices.len(),
            INDEX_STRIDE,
        )?;

        let base_vertex = self.surface_vertex_3d_count;
        let vertex_increment =
            u32::try_from(mesh.vertices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.surface_vertex_3d_count = self
            .surface_vertex_3d_count
            .checked_add(vertex_increment)
            .ok_or(CompileError::TooManyVertices)?;
        for &vertex in &mesh.vertices {
            push_surface_vertex_3d(&mut self.frame.surface_vertices_3d, vertex);
        }

        let first_index = self.surface_index_3d_count;
        for &index in &mesh.indices {
            let absolute = base_vertex
                .checked_add(index)
                .ok_or(CompileError::TooManyVertices)?;
            push_u32(&mut self.frame.surface_indices_3d, absolute);
        }
        let index_increment =
            u32::try_from(mesh.indices.len()).map_err(|_| CompileError::TooManyVertices)?;
        self.surface_index_3d_count = self
            .surface_index_3d_count
            .checked_add(index_increment)
            .ok_or(CompileError::TooManyVertices)?;
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind: DrawKind::SurfaceTriangles3D {
                indices: first_index..self.surface_index_3d_count,
                interpolation: mesh.color_interpolation,
                edge_overlay,
            },
        });
        Ok(())
    }

    fn append_line_segments_3d(
        &mut self,
        batch: &LineSegmentBatch3D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        scissor: DeviceRect,
        surface_edge: bool,
    ) -> Result<(), CompileError> {
        if batch.segments.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::LineSegment3D,
            self.frame.line_segments_3d.len(),
            batch.segments.len(),
            LINE_SEGMENT_3D_STRIDE,
        )?;
        let first_instance = self.line_segment_3d_count;
        let increment =
            u32::try_from(batch.segments.len()).map_err(|_| CompileError::TooManyLineSegments)?;
        self.line_segment_3d_count = self
            .line_segment_3d_count
            .checked_add(increment)
            .ok_or(CompileError::TooManyLineSegments)?;
        for &segment in &batch.segments {
            push_line_segment_3d(
                &mut self.frame.line_segments_3d,
                segment,
                batch.width_css_px.get(),
                batch.pattern,
            );
        }
        let instances = first_instance..self.line_segment_3d_count;
        let kind = if surface_edge {
            DrawKind::SurfaceEdgeSegments3D { instances }
        } else {
            DrawKind::LineSegments3D { instances }
        };
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor,
            kind,
        });
        Ok(())
    }

    fn append_ruler_lines_3d(
        &mut self,
        batch: &RulerLineBatch3D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        _scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        if batch.lines.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::LineSegment3D,
            self.frame.line_segments_3d.len(),
            batch.lines.len(),
            LINE_SEGMENT_3D_STRIDE,
        )?;
        let first_instance = self.line_segment_3d_count;
        let increment =
            u32::try_from(batch.lines.len()).map_err(|_| CompileError::TooManyLineSegments)?;
        self.line_segment_3d_count = self
            .line_segment_3d_count
            .checked_add(increment)
            .ok_or(CompileError::TooManyLineSegments)?;
        for &line in &batch.lines {
            push_ruler_line_3d(
                &mut self.frame.line_segments_3d,
                line,
                batch.width_css_px.get(),
            );
        }
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor: full_surface_scissor(),
            kind: DrawKind::RulerLines3D {
                instances: first_instance..self.line_segment_3d_count,
            },
        });
        Ok(())
    }

    fn append_tick_marks_3d(
        &mut self,
        batch: &TickMarkBatch3D,
        order: DrawOrder,
        picking_id: Option<PickingId>,
        _scissor: DeviceRect,
    ) -> Result<(), CompileError> {
        if batch.marks.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::LineSegment3D,
            self.frame.line_segments_3d.len(),
            batch.marks.len(),
            LINE_SEGMENT_3D_STRIDE,
        )?;
        let first_instance = self.line_segment_3d_count;
        let increment =
            u32::try_from(batch.marks.len()).map_err(|_| CompileError::TooManyLineSegments)?;
        self.line_segment_3d_count = self
            .line_segment_3d_count
            .checked_add(increment)
            .ok_or(CompileError::TooManyLineSegments)?;
        for &mark in &batch.marks {
            push_tick_mark_3d(
                &mut self.frame.line_segments_3d,
                mark,
                batch.width_css_px.get(),
                batch.length_css_px.get(),
            );
        }
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor: full_surface_scissor(),
            kind: DrawKind::TickMarks3D {
                instances: first_instance..self.line_segment_3d_count,
            },
        });
        Ok(())
    }

    fn append_screen_lines(
        &mut self,
        batch: &ScreenLineBatch,
        order: DrawOrder,
        picking_id: Option<PickingId>,
    ) -> Result<(), CompileError> {
        if batch.lines.is_empty() {
            return Ok(());
        }
        self.ensure_append(
            BufferKind::ScreenLine,
            self.frame.screen_lines.len(),
            batch.lines.len(),
            SCREEN_LINE_STRIDE,
        )?;
        let first_instance = self.screen_line_count;
        let increment =
            u32::try_from(batch.lines.len()).map_err(|_| CompileError::TooManyLineSegments)?;
        self.screen_line_count = self
            .screen_line_count
            .checked_add(increment)
            .ok_or(CompileError::TooManyLineSegments)?;
        for &line in &batch.lines {
            push_screen_line(
                &mut self.frame.screen_lines,
                line,
                batch.width_css_px.get(),
                batch.pattern,
            );
        }
        self.frame.draws.push(DrawCommand {
            order,
            picking_id: picking_id.map(PickingId::get).and_then(NonZeroU64::new),
            scissor: full_surface_scissor(),
            kind: DrawKind::ScreenLines {
                instances: first_instance..self.screen_line_count,
            },
        });
        Ok(())
    }

    fn ensure_append(
        &self,
        kind: BufferKind,
        current_bytes: usize,
        element_count: usize,
        stride: u64,
    ) -> Result<(), CompileError> {
        let current =
            u64::try_from(current_bytes).map_err(|_| CompileError::ArithmeticOverflow(kind))?;
        let count =
            u64::try_from(element_count).map_err(|_| CompileError::ArithmeticOverflow(kind))?;
        let additional = count
            .checked_mul(stride)
            .ok_or(CompileError::ArithmeticOverflow(kind))?;
        let requested = current
            .checked_add(additional)
            .ok_or(CompileError::ArithmeticOverflow(kind))?;
        if requested > self.options.max_buffer_bytes {
            return Err(CompileError::BufferLimitExceeded {
                kind,
                requested_bytes: requested,
                maximum_bytes: self.options.max_buffer_bytes,
            });
        }
        Ok(())
    }
}

fn push_mesh_vertex(
    bytes: &mut Vec<u8>,
    position: AxesPoint,
    color: Rgba,
    edge_distance_css_px: f32,
    half_width_css_px: f32,
) {
    push_f32(bytes, position.x);
    push_f32(bytes, position.y);
    push_f32(bytes, 0.0);
    push_f32(bytes, 0.0);
    push_color(bytes, color);
    push_f32(bytes, edge_distance_css_px);
    push_f32(bytes, half_width_css_px);
}

#[allow(clippy::too_many_arguments)]
fn push_marker_instance(
    bytes: &mut Vec<u8>,
    center: AxesPoint,
    size_css_px: f32,
    rotation_radians: f32,
    fill: Rgba,
    stroke: Rgba,
    stroke_width_css_px: f32,
) {
    push_f32(bytes, center.x);
    push_f32(bytes, center.y);
    push_f32(bytes, 0.0);
    push_f32(bytes, 0.0);
    push_f32(bytes, size_css_px);
    push_f32(bytes, rotation_radians);
    push_color(bytes, fill);
    push_color(bytes, stroke);
    push_f32(bytes, stroke_width_css_px);
    push_f32(bytes, 0.0);
}

fn push_marker_instance_3d(
    bytes: &mut Vec<u8>,
    center: openmat_plot_mir::AxesPoint3D,
    size_css_px: f32,
    rotation_radians: f32,
    fill: Rgba,
    stroke: Rgba,
    stroke_width_css_px: f32,
) {
    push_f32(bytes, center.x);
    push_f32(bytes, center.y);
    push_f32(bytes, center.z);
    push_f32(bytes, 0.0);
    push_f32(bytes, size_css_px);
    push_f32(bytes, rotation_radians);
    push_color(bytes, fill);
    push_color(bytes, stroke);
    push_f32(bytes, stroke_width_css_px);
    push_f32(bytes, 1.0);
}

fn push_surface_vertex_3d(bytes: &mut Vec<u8>, vertex: SurfaceVertex3D) {
    push_f32(bytes, vertex.position.x);
    push_f32(bytes, vertex.position.y);
    push_f32(bytes, vertex.position.z);
    push_f32(bytes, vertex.normal.x);
    push_f32(bytes, vertex.normal.y);
    push_f32(bytes, vertex.normal.z);
    push_color(bytes, vertex.color);
}

fn push_line_segment_3d(
    bytes: &mut Vec<u8>,
    segment: LineSegment3D,
    width_css_px: f32,
    pattern: LinePattern3D,
) {
    for value in [segment.start.x, segment.start.y, segment.start.z] {
        push_f32(bytes, value);
    }
    for value in [segment.end.x, segment.end.y, segment.end.z] {
        push_f32(bytes, value);
    }
    push_color(bytes, segment.color);
    push_f32(bytes, width_css_px);
    push_f32(
        bytes,
        match pattern {
            LinePattern3D::Solid => 0.0,
            LinePattern3D::Dash => 1.0,
            LinePattern3D::Dot => 2.0,
            LinePattern3D::DashDot => 3.0,
        },
    );
    push_f32(bytes, 0.0);
    push_f32(bytes, 0.0);
}

fn push_ruler_line_3d(bytes: &mut Vec<u8>, line: RulerLine3D, width_css_px: f32) {
    for value in [
        line.segment.start.x,
        line.segment.start.y,
        line.segment.start.z,
    ] {
        push_f32(bytes, value);
    }
    for value in [line.segment.end.x, line.segment.end.y, line.segment.end.z] {
        push_f32(bytes, value);
    }
    push_color(bytes, line.segment.color);
    push_f32(bytes, width_css_px);
    push_f32(bytes, 0.0);
    push_f32(bytes, ruler_selector_code(line.axis, line.candidate));
    push_f32(bytes, 0.0);
}

fn push_tick_mark_3d(bytes: &mut Vec<u8>, mark: TickMark3D, width_css_px: f32, length_css_px: f32) {
    for value in [mark.anchor.x, mark.anchor.y, mark.anchor.z] {
        push_f32(bytes, value);
    }
    for value in [0.0, 0.0, 0.0] {
        push_f32(bytes, value);
    }
    push_color(bytes, mark.color);
    push_f32(bytes, width_css_px);
    push_f32(bytes, length_css_px);
    push_f32(bytes, ruler_selector_code(mark.axis, mark.candidate));
    push_f32(bytes, 0.0);
}

const fn ruler_selector_code(axis: AxisDimension, candidate: u8) -> f32 {
    let dimension = match axis {
        AxisDimension::X => 0,
        AxisDimension::Y => 1,
        AxisDimension::Z => 2,
    };
    (dimension * 4 + candidate) as f32
}

fn push_screen_line(
    bytes: &mut Vec<u8>,
    line: ScreenLine,
    width_css_px: f32,
    pattern: LinePattern3D,
) {
    for value in [
        line.segment.start.x,
        line.segment.start.y,
        line.segment.end.x,
        line.segment.end.y,
    ] {
        push_f32(bytes, value);
    }
    push_color(bytes, line.color);
    push_f32(bytes, width_css_px);
    push_f32(
        bytes,
        match pattern {
            LinePattern3D::Solid => 0.0,
            LinePattern3D::Dash => 1.0,
            LinePattern3D::Dot => 2.0,
            LinePattern3D::DashDot => 3.0,
        },
    );
    push_f32(bytes, 0.0);
    push_f32(bytes, 0.0);
}

fn push_color(bytes: &mut Vec<u8>, color: Rgba) {
    push_f32(bytes, color.red);
    push_f32(bytes, color.green);
    push_f32(bytes, color.blue);
    push_f32(bytes, color.alpha);
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::similar_names
)]
fn tessellate_segment(
    segment: AxesLineSegment,
    width_css_px: f32,
    viewport: Viewport,
) -> Option<[StrokeVertex; 4]> {
    let viewport_width = viewport.device.width as f32;
    let viewport_height = viewport.device.height as f32;
    if viewport_width == 0.0 || viewport_height == 0.0 || width_css_px == 0.0 {
        return None;
    }

    let dx_device = (segment.end.x - segment.start.x) * viewport_width;
    let dy_device = -(segment.end.y - segment.start.y) * viewport_height;
    let length = dx_device.hypot(dy_device);
    if !length.is_finite() || length <= f32::EPSILON {
        return None;
    }

    let dpr = viewport.device_pixel_ratio.get() as f32;
    let effective_width_css_px =
        effective_hairline_width_css_px(width_css_px, viewport.device_pixel_ratio.get());
    let outer_half_width_css_px = effective_width_css_px * 0.5 + ANTIALIAS_FRINGE_CSS_PX;
    let outer_half_width_device = outer_half_width_css_px * dpr;
    let offset_device_x = -dy_device / length * outer_half_width_device;
    let offset_device_y = dx_device / length * outer_half_width_device;
    let offset_axes_x = offset_device_x / viewport_width;
    let offset_axes_y = -offset_device_y / viewport_height;
    Some([
        StrokeVertex {
            position: AxesPoint {
                x: segment.start.x + offset_axes_x,
                y: segment.start.y + offset_axes_y,
            },
            edge_distance_css_px: outer_half_width_css_px,
        },
        StrokeVertex {
            position: AxesPoint {
                x: segment.start.x - offset_axes_x,
                y: segment.start.y - offset_axes_y,
            },
            edge_distance_css_px: -outer_half_width_css_px,
        },
        StrokeVertex {
            position: AxesPoint {
                x: segment.end.x - offset_axes_x,
                y: segment.end.y - offset_axes_y,
            },
            edge_distance_css_px: -outer_half_width_css_px,
        },
        StrokeVertex {
            position: AxesPoint {
                x: segment.end.x + offset_axes_x,
                y: segment.end.y + offset_axes_y,
            },
            edge_distance_css_px: outer_half_width_css_px,
        },
    ])
}

#[allow(clippy::cast_possible_truncation)]
fn effective_hairline_width_css_px(width_css_px: f32, device_pixel_ratio: f64) -> f32 {
    width_css_px.max((1.0 / device_pixel_ratio) as f32)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn axes_rect_to_scissor(clip: AxesRect, viewport: Viewport) -> DeviceRect {
    let min_x = f64::from(clip.min.x.clamp(0.0, 1.0));
    let max_x = f64::from(clip.max.x.clamp(0.0, 1.0));
    let min_y = f64::from(clip.min.y.clamp(0.0, 1.0));
    let max_y = f64::from(clip.max.y.clamp(0.0, 1.0));
    let viewport_x = f64::from(viewport.device.x);
    let viewport_y = f64::from(viewport.device.y);
    let viewport_width = f64::from(viewport.device.width);
    let viewport_height = f64::from(viewport.device.height);

    let left = (viewport_x + min_x * viewport_width).floor();
    let right = (viewport_x + max_x * viewport_width).ceil();
    let top = (viewport_y + (1.0 - max_y) * viewport_height).floor();
    let bottom = (viewport_y + (1.0 - min_y) * viewport_height).ceil();
    let x = left.clamp(0.0, f64::from(u32::MAX)) as u32;
    let y = top.clamp(0.0, f64::from(u32::MAX)) as u32;
    let right = right.clamp(f64::from(x), f64::from(u32::MAX)) as u32;
    let bottom = bottom.clamp(f64::from(y), f64::from(u32::MAX)) as u32;

    DeviceRect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

const fn full_surface_scissor() -> DeviceRect {
    DeviceRect {
        x: 0,
        y: 0,
        width: u32::MAX,
        height: u32::MAX,
    }
}

#[cfg(test)]
mod tests {
    use openmat_plot_mir::{
        AxesPoint3D, AxesVector3D, CssLineSegment, CssPoint, CssPx, CssRect, DevicePixelRatio,
        DrawOrder, LineCap, LineJoin, LinePattern3D, LineSegment3D, LineSegmentBatch3D,
        MarkerInstance, OverlayPlan, RenderOperation, ScreenLine, ScreenLineBatch, StrokeStyle,
        StrokeVertex, SurfaceMesh3D, SurfaceVertex3D, TickMark3D, TickMarkBatch3D,
        ViewProjection3D,
    };

    use super::*;

    fn viewport() -> Viewport {
        Viewport {
            css: CssRect::new(0.0, 0.0, 320.0, 200.0).unwrap(),
            device: DeviceRect {
                x: 10,
                y: 20,
                width: 640,
                height: 400,
            },
            device_pixel_ratio: DevicePixelRatio::new(2.0).unwrap(),
        }
    }

    fn operation(order: i64, command: MirCommand) -> RenderOperation {
        RenderOperation {
            order: DrawOrder(order),
            picking_id: None,
            command,
        }
    }

    fn triangle(color: Rgba) -> StrokeMesh {
        StrokeMesh {
            vertices: vec![
                StrokeVertex {
                    position: AxesPoint { x: 0.0, y: 0.0 },
                    edge_distance_css_px: 0.0,
                },
                StrokeVertex {
                    position: AxesPoint { x: 1.0, y: 0.0 },
                    edge_distance_css_px: 0.0,
                },
                StrokeVertex {
                    position: AxesPoint { x: 0.0, y: 1.0 },
                    edge_distance_css_px: 0.0,
                },
            ],
            indices: vec![0, 1, 2],
            color,
            style: StrokeStyle::solid(CssPx::new(1.0).unwrap(), LineCap::Butt, LineJoin::Miter)
                .unwrap(),
        }
    }

    fn surface_triangle(color: Rgba) -> SurfaceMesh3D {
        SurfaceMesh3D {
            vertices: vec![
                SurfaceVertex3D {
                    position: AxesPoint3D::new(0.0, 0.0, 0.0).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color,
                },
                SurfaceVertex3D {
                    position: AxesPoint3D::new(1.0, 0.0, 0.0).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color,
                },
                SurfaceVertex3D {
                    position: AxesPoint3D::new(0.0, 1.0, 0.5).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color,
                },
            ],
            indices: vec![0, 1, 2],
            color_interpolation: openmat_plot_mir::SurfaceColorInterpolation::Flat,
        }
    }

    #[test]
    fn packs_mesh_high_low_positions_color_and_u32_indices() {
        let red = Rgba::new(1.0, 0.0, 0.0, 0.5).unwrap();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::StrokeMesh(triangle(red))),
                operation(2, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(compiled.mesh_vertex_bytes().len(), 3 * 40);
        assert_eq!(
            compiled.mesh_index_bytes(),
            &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(&compiled.mesh_vertex_bytes()[8..16], &[0; 8]);
        assert_eq!(
            f32::from_le_bytes(compiled.mesh_vertex_bytes()[28..32].try_into().unwrap()).to_bits(),
            0.5_f32.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.mesh_vertex_bytes()[36..40].try_into().unwrap()).to_bits(),
            0.5_f32.to_bits()
        );
    }

    #[test]
    fn packs_surface_vertices_indices_and_camera_without_2d_reencoding() {
        let color = Rgba::new(0.25, 0.5, 0.75, 1.0).unwrap();
        let matrix = ViewProjection3D::identity();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::SetViewProjection3D(matrix)),
                operation(2, MirCommand::SurfaceMesh3D(surface_triangle(color))),
                operation(3, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(compiled.view_projection_3d(), Some(matrix));
        assert_eq!(
            compiled.surface_vertex_3d_bytes().len(),
            3 * usize::try_from(SURFACE_VERTEX_3D_STRIDE).unwrap()
        );
        assert_eq!(
            compiled.surface_index_3d_bytes(),
            &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(
            f32::from_le_bytes(
                compiled.surface_vertex_3d_bytes()[8..12]
                    .try_into()
                    .unwrap()
            )
            .to_bits(),
            0.0_f32.to_bits()
        );
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::SurfaceTriangles3D { ref indices, .. } if indices == &(0..3)
        ));
        assert!(compiled.mesh_vertex_bytes().is_empty());
    }

    #[test]
    fn keeps_surface_wireframe_fill_and_edge_draw_roles_distinct() {
        let color = Rgba::new(0.2, 0.3, 0.4, 1.0).unwrap();
        let edges = LineSegmentBatch3D {
            segments: vec![LineSegment3D {
                start: AxesPoint3D::new(0.0, 0.0, 0.0).unwrap(),
                end: AxesPoint3D::new(1.0, 0.0, 0.0).unwrap(),
                color,
            }],
            width_css_px: CssPx::new(2.0 / 3.0).unwrap(),
            pattern: LinePattern3D::Solid,
        };
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(
                    1,
                    MirCommand::SetViewProjection3D(ViewProjection3D::identity()),
                ),
                operation(
                    2,
                    MirCommand::SurfaceMeshWithEdges3D(surface_triangle(color)),
                ),
                operation(2, MirCommand::SurfaceEdgeSegments3D(edges)),
                operation(3, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::SurfaceTriangles3D {
                edge_overlay: true,
                ..
            }
        ));
        assert!(matches!(
            compiled.draws()[1].kind,
            DrawKind::SurfaceEdgeSegments3D { ref instances } if instances == &(0..1)
        ));
    }

    #[test]
    fn packs_camera_independent_3d_lines_as_instanced_screen_space_strokes() {
        let color = Rgba::new(0.1, 0.2, 0.3, 0.4).unwrap();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(
                    1,
                    MirCommand::SetViewProjection3D(ViewProjection3D::identity()),
                ),
                operation(
                    2,
                    MirCommand::LineSegments3D(LineSegmentBatch3D {
                        segments: vec![LineSegment3D {
                            start: AxesPoint3D::new(0.0, 0.25, 0.5).unwrap(),
                            end: AxesPoint3D::new(1.0, 0.75, 0.5).unwrap(),
                            color,
                        }],
                        width_css_px: CssPx::new(1.5).unwrap(),
                        pattern: LinePattern3D::DashDot,
                    }),
                ),
                operation(3, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();
        assert_eq!(
            compiled.line_segment_3d_bytes().len(),
            usize::try_from(LINE_SEGMENT_3D_STRIDE).unwrap()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.line_segment_3d_bytes()[40..44].try_into().unwrap())
                .to_bits(),
            1.5_f32.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.line_segment_3d_bytes()[44..48].try_into().unwrap())
                .to_bits(),
            3.0_f32.to_bits()
        );
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::LineSegments3D { ref instances } if instances == &(0..1)
        ));
    }

    #[test]
    fn packs_camera_aware_ticks_and_canvas_lines_into_separate_draws() {
        let color = Rgba::new(0.2, 0.3, 0.4, 1.0).unwrap();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(
                    1,
                    MirCommand::SetViewProjection3D(ViewProjection3D::identity()),
                ),
                operation(
                    2,
                    MirCommand::SetRulerSelection3D(RulerSelection3D { x: 1, y: 2, z: 3 }),
                ),
                operation(
                    3,
                    MirCommand::TickMarks3D(TickMarkBatch3D {
                        marks: vec![TickMark3D {
                            anchor: AxesPoint3D::new(1.0, 0.0, 0.0).unwrap(),
                            color,
                            axis: AxisDimension::X,
                            candidate: 1,
                        }],
                        length_css_px: CssPx::new(5.0).unwrap(),
                        width_css_px: CssPx::new(0.75).unwrap(),
                    }),
                ),
                operation(
                    4,
                    MirCommand::ScreenLines(ScreenLineBatch {
                        lines: vec![ScreenLine {
                            segment: CssLineSegment {
                                start: CssPoint::new(10.0, 20.0).unwrap(),
                                end: CssPoint::new(310.0, 20.0).unwrap(),
                            },
                            color,
                        }],
                        width_css_px: CssPx::new(1.25).unwrap(),
                        pattern: LinePattern3D::Solid,
                    }),
                ),
                operation(5, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(
            compiled.line_segment_3d_bytes().len(),
            usize::try_from(LINE_SEGMENT_3D_STRIDE).unwrap()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.line_segment_3d_bytes()[40..44].try_into().unwrap())
                .to_bits(),
            0.75_f32.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.line_segment_3d_bytes()[44..48].try_into().unwrap())
                .to_bits(),
            5.0_f32.to_bits()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.line_segment_3d_bytes()[48..52].try_into().unwrap())
                .to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(
            compiled.screen_line_bytes().len(),
            usize::try_from(SCREEN_LINE_STRIDE).unwrap()
        );
        assert_eq!(
            f32::from_le_bytes(compiled.screen_line_bytes()[8..12].try_into().unwrap()).to_bits(),
            310.0_f32.to_bits()
        );
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::TickMarks3D { ref instances } if instances == &(0..1)
        ));
        assert!(matches!(
            compiled.draws()[1].kind,
            DrawKind::ScreenLines { ref instances } if instances == &(0..1)
        ));
        assert_eq!(compiled.draws()[0].scissor, full_surface_scissor());
        assert_eq!(compiled.draws()[1].scissor, full_surface_scissor());
    }

    #[test]
    fn preserves_draw_order_across_mesh_and_marker_pipelines() {
        let color = Rgba::new(0.2, 0.3, 0.4, 0.6).unwrap();
        let markers = MarkerBatch {
            shape: MarkerShape::Circle,
            instances: vec![MarkerInstance {
                center: AxesPoint { x: 0.5, y: 0.5 },
                size_css_px: CssPx::new(8.0).unwrap(),
                rotation_radians: 0.0,
                fill: color,
                stroke: color,
                stroke_width_css_px: CssPx::new(1.0).unwrap(),
            }],
        };
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(10, MirCommand::StrokeMesh(triangle(color))),
                operation(11, MirCommand::MarkerBatch(markers)),
                operation(12, MirCommand::StrokeMesh(triangle(color))),
                operation(13, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(compiled.draws().len(), 3);
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::Triangles { .. }
        ));
        assert!(matches!(
            compiled.draws()[1].kind,
            DrawKind::Markers {
                shape: MarkerShape::Circle,
                ..
            }
        ));
        assert!(matches!(
            compiled.draws()[2].kind,
            DrawKind::Triangles { .. }
        ));
    }

    #[test]
    fn set_clip_rect_maps_axes_y_up_to_device_y_down() {
        let color = Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap();
        let clip = AxesRect::new(
            AxesPoint { x: 0.25, y: 0.25 },
            AxesPoint { x: 0.75, y: 0.75 },
        )
        .unwrap();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::SetClipRect(clip)),
                operation(2, MirCommand::StrokeMesh(triangle(color))),
                operation(3, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(
            compiled.draws()[0].scissor,
            DeviceRect {
                x: 170,
                y: 120,
                width: 320,
                height: 200,
            }
        );
    }

    #[test]
    fn enforces_each_packed_buffer_limit_before_upload() {
        let color = Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap();
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::StrokeMesh(triangle(color))),
                operation(2, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        let options = FrameCompileOptions {
            max_buffer_bytes: 31,
            ..FrameCompileOptions::default()
        };

        assert!(matches!(
            DrawListCompiler::new(options).compile(&frame),
            Err(CompileError::BufferLimitExceeded {
                kind: BufferKind::MeshVertex,
                ..
            })
        ));
    }

    #[test]
    fn grid_segments_are_tessellated_into_triangles() {
        let color = Rgba::new(0.5, 0.5, 0.5, 0.25).unwrap();
        let grid = GridLineBatch {
            segments: vec![AxesLineSegment {
                start: AxesPoint { x: 0.0, y: 0.5 },
                end: AxesPoint { x: 1.0, y: 0.5 },
            }],
            color,
            width_css_px: CssPx::new(1.0).unwrap(),
        };
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::GridLineBatch(grid)),
                operation(2, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();

        let compiled = DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap();

        assert_eq!(compiled.mesh_vertex_bytes().len(), 4 * 40);
        assert_eq!(compiled.mesh_index_bytes().len(), 6 * 4);
        let first_edge =
            f32::from_le_bytes(compiled.mesh_vertex_bytes()[32..36].try_into().unwrap());
        let first_half_width =
            f32::from_le_bytes(compiled.mesh_vertex_bytes()[36..40].try_into().unwrap());
        assert_eq!(first_edge.to_bits(), 1.5_f32.to_bits());
        assert_eq!(first_half_width.to_bits(), 0.5_f32.to_bits());
    }

    #[test]
    fn circle_and_square_marker_batches_use_separate_ordered_instance_ranges() {
        let color = Rgba::new(0.1, 0.2, 0.3, 0.4).unwrap();
        let marker = |shape| MarkerBatch {
            shape,
            instances: vec![MarkerInstance {
                center: AxesPoint { x: 0.5, y: 0.5 },
                size_css_px: CssPx::new(6.0).unwrap(),
                rotation_radians: 0.25,
                fill: color,
                stroke: color,
                stroke_width_css_px: CssPx::new(0.5).unwrap(),
            }],
        };
        let frame = PlotFrame::new(
            vec![
                operation(0, MirCommand::BeginViewport(viewport())),
                operation(1, MirCommand::MarkerBatch(marker(MarkerShape::Circle))),
                operation(2, MirCommand::MarkerBatch(marker(MarkerShape::Square))),
                operation(3, MirCommand::EndViewport),
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        let options = FrameCompileOptions {
            clear_color: color,
            ..FrameCompileOptions::default()
        };

        let compiled = DrawListCompiler::new(options).compile(&frame).unwrap();

        assert_eq!(compiled.marker_instance_bytes().len(), 2 * 64);
        assert_eq!(compiled.clear_color(), color);
        assert!(matches!(
            compiled.draws()[0].kind,
            DrawKind::Markers {
                shape: MarkerShape::Circle,
                ref instances,
            } if instances == &(0..1)
        ));
        assert!(matches!(
            compiled.draws()[1].kind,
            DrawKind::Markers {
                shape: MarkerShape::Square,
                ref instances,
            } if instances == &(1..2)
        ));
    }

    #[test]
    fn every_mir_marker_shape_compiles_to_an_ordered_instance_draw() {
        let color = Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap();
        for shape in [
            MarkerShape::Circle,
            MarkerShape::Square,
            MarkerShape::Diamond,
            MarkerShape::UpTriangle,
            MarkerShape::DownTriangle,
            MarkerShape::Plus,
            MarkerShape::Cross,
        ] {
            let batch = MarkerBatch {
                shape,
                instances: vec![MarkerInstance {
                    center: AxesPoint { x: 0.5, y: 0.5 },
                    size_css_px: CssPx::new(8.0).unwrap(),
                    rotation_radians: 0.0,
                    fill: color,
                    stroke: color,
                    stroke_width_css_px: CssPx::new(1.0).unwrap(),
                }],
            };
            let frame = PlotFrame::new(
                vec![
                    operation(0, MirCommand::BeginViewport(viewport())),
                    operation(1, MirCommand::MarkerBatch(batch)),
                    operation(2, MirCommand::EndViewport),
                ],
                OverlayPlan::default(),
            )
            .unwrap();
            let compiled = DrawListCompiler::new(FrameCompileOptions::default())
                .compile(&frame)
                .unwrap();
            assert!(matches!(
                compiled.draws()[0].kind,
                DrawKind::Markers { shape: compiled_shape, .. } if compiled_shape == shape
            ));
        }
    }
}
