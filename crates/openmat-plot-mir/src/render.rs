use std::num::NonZeroU64;

use crate::{
    AxesPoint, AxesPoint3D, AxesRect, AxesVector3D, AxisDimension, CssPx, MirError, OverlayPlan,
    Rgba, ViewProjection3D, Viewport,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DrawOrder(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PickingId(NonZeroU64);

impl PickingId {
    #[must_use]
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Creates a stable picking identifier without exposing `NonZeroU64` at
    /// serialization and browser boundaries.
    ///
    /// # Errors
    /// Returns [`MirError`] because zero is reserved for "no hit".
    pub fn from_u64(value: u64) -> Result<Self, MirError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(MirError::NonPositive("picking identifier"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxesLineSegment {
    pub start: AxesPoint,
    pub end: AxesPoint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridLineBatch {
    pub segments: Vec<AxesLineSegment>,
    pub color: Rgba,
    pub width_css_px: CssPx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Bevel,
    Round,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width_css_px: CssPx,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash_pattern_css_px: Vec<CssPx>,
    pub dash_phase_css_px: CssPx,
}

impl StrokeStyle {
    /// Creates a solid stroke style.
    ///
    /// # Errors
    /// Returns [`MirError`] if the zero dash phase cannot be represented.
    pub fn solid(width_css_px: CssPx, cap: LineCap, join: LineJoin) -> Result<Self, MirError> {
        Ok(Self {
            width_css_px,
            cap,
            join,
            miter_limit: 10.0,
            dash_pattern_css_px: Vec::new(),
            dash_phase_css_px: CssPx::new(0.0)?,
        })
    }

    /// Validates miter and dash invariants.
    ///
    /// # Errors
    /// Returns [`MirError`] for a non-positive miter limit or an all-zero dash pattern.
    pub fn validate(&self) -> Result<(), MirError> {
        if !self.miter_limit.is_finite() {
            return Err(MirError::NonFinite("miter limit"));
        }
        if self.miter_limit <= 0.0 {
            return Err(MirError::NonPositive("miter limit"));
        }
        if !self.dash_pattern_css_px.is_empty()
            && self
                .dash_pattern_css_px
                .iter()
                .all(|length| length.get() == 0.0)
        {
            return Err(MirError::NonPositive("dash pattern total length"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeVertex {
    pub position: AxesPoint,
    /// Signed distance from the stroke centerline in CSS pixels. Tessellated
    /// geometry extends beyond the nominal half width so the renderer can
    /// convert this distance into subpixel fragment coverage.
    pub edge_distance_css_px: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrokeMesh {
    pub vertices: Vec<StrokeVertex>,
    pub indices: Vec<u32>,
    pub color: Rgba,
    pub style: StrokeStyle,
}

/// One per-vertex-colored triangle vertex in normalized 2D Axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleVertex2D {
    pub position: AxesPoint,
    pub color: Rgba,
}

/// Backend-neutral filled 2D triangle mesh used by Patch and future area primitives.
#[derive(Clone, Debug, PartialEq)]
pub struct TriangleMesh2D {
    pub vertices: Vec<TriangleVertex2D>,
    pub indices: Vec<u32>,
}

/// One render-ready 3D surface vertex in normalized axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceVertex3D {
    pub position: AxesPoint3D,
    pub normal: AxesVector3D,
    pub color: Rgba,
}

/// Fragment color interpolation selected by the Surface HIR.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfaceColorInterpolation {
    /// Hold the first triangle vertex color across the primitive.
    #[default]
    Flat,
    /// Perspective-correctly interpolate vertex colors across the primitive.
    Smooth,
}

impl SurfaceVertex3D {
    /// Validates position, normal, and color components.
    ///
    /// # Errors
    /// Returns [`MirError`] when any component is non-finite or outside the
    /// accepted RGBA range.
    pub fn validate(self) -> Result<(), MirError> {
        self.position.validate()?;
        self.normal.validate()?;
        self.color.validate()?;
        if self.color.alpha.to_bits() != 1.0_f32.to_bits() {
            return Err(MirError::InvalidCommandStream(
                "the first 3D surface pipeline accepts opaque colors only",
            ));
        }
        Ok(())
    }
}

/// A backend-neutral indexed triangle mesh for opaque scientific surfaces.
///
/// Indices use counter-clockwise winding when viewed from the front. The first
/// 3D tranche renders both faces; retaining winding now allows a later material
/// layer to add culling without changing mesh resources.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceMesh3D {
    pub vertices: Vec<SurfaceVertex3D>,
    pub indices: Vec<u32>,
    pub color_interpolation: SurfaceColorInterpolation,
}

/// One camera-following local light in normalized Axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting3D {
    /// Point-light position. A MATLAB `camlight headlight` follows the camera eye.
    pub position: AxesPoint3D,
    /// Whether material lighting is enabled.
    pub enabled: bool,
}

impl Lighting3D {
    /// Validates the finite local-light position.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid lighting data.
    pub fn validate(self) -> Result<(), MirError> {
        self.position.validate()
    }
}

/// One camera-independent 3D line segment in normalized axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineSegment3D {
    pub start: AxesPoint3D,
    pub end: AxesPoint3D,
    pub color: Rgba,
}

/// Fixed MATLAB line patterns understood by the 3D screen-space stroke shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinePattern3D {
    #[default]
    Solid,
    Dash,
    Dot,
    DashDot,
}

/// A set of 3D segments rendered as antialiased screen-space triangle strips.
#[derive(Clone, Debug, PartialEq)]
pub struct LineSegmentBatch3D {
    pub segments: Vec<LineSegment3D>,
    pub width_css_px: CssPx,
    pub pattern: LinePattern3D,
}

/// The selected projected-cube edge for each MATLAB-style 3D ruler.
///
/// Every axis has four parallel cube-edge candidates. Keeping the selected
/// candidate in renderer-local state lets orbit gestures move rulers without
/// rebuilding or uploading the Surface mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RulerSelection3D {
    pub x: u8,
    pub y: u8,
    pub z: u8,
}

impl RulerSelection3D {
    pub const CANDIDATE_COUNT: u8 = 4;
    pub const HIDDEN_CANDIDATE: u8 = Self::CANDIDATE_COUNT;

    /// Returns the selected candidate for one axis.
    #[must_use]
    pub const fn candidate(self, axis: AxisDimension) -> u8 {
        match axis {
            AxisDimension::X => self.x,
            AxisDimension::Y => self.y,
            AxisDimension::Z => self.z,
        }
    }

    /// Validates the fixed four-edge candidate domain.
    ///
    /// # Errors
    /// Returns [`MirError`] when any candidate is outside `0..4`.
    pub fn validate(self) -> Result<(), MirError> {
        if [self.x, self.y, self.z]
            .into_iter()
            .all(|candidate| candidate <= Self::HIDDEN_CANDIDATE)
        {
            Ok(())
        } else {
            Err(MirError::InvalidCommandStream(
                "3D ruler candidate must be in 0..=4",
            ))
        }
    }
}

/// One candidate cube edge for a dynamically selected 3D ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerLine3D {
    pub segment: LineSegment3D,
    pub axis: AxisDimension,
    pub candidate: u8,
}

/// All candidate edges used by the dynamic MATLAB-style 3D rulers.
#[derive(Clone, Debug, PartialEq)]
pub struct RulerLineBatch3D {
    pub lines: Vec<RulerLine3D>,
    pub width_css_px: CssPx,
}

impl RulerLineBatch3D {
    /// Validates candidate metadata, endpoints, and colors.
    ///
    /// # Errors
    /// Returns [`MirError`] when a line is invalid.
    pub fn validate(&self) -> Result<(), MirError> {
        for line in &self.lines {
            if line.candidate >= RulerSelection3D::CANDIDATE_COUNT {
                return Err(MirError::InvalidCommandStream(
                    "3D ruler line candidate must be in 0..4",
                ));
            }
            line.segment.start.validate()?;
            line.segment.end.validate()?;
            line.segment.color.validate()?;
        }
        Ok(())
    }
}

/// One fixed-length candidate 3D tick mark. The renderer derives the shared
/// outward normal from the candidate ruler edge so every mark remains parallel
/// and outside the projected axes box while the camera moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickMark3D {
    pub anchor: AxesPoint3D,
    pub color: Rgba,
    pub axis: AxisDimension,
    pub candidate: u8,
}

/// Camera-aware 3D ticks rendered with stable CSS-pixel dimensions.
#[derive(Clone, Debug, PartialEq)]
pub struct TickMarkBatch3D {
    pub marks: Vec<TickMark3D>,
    pub length_css_px: CssPx,
    pub width_css_px: CssPx,
}

impl TickMarkBatch3D {
    /// Validates finite anchors, candidate metadata, and colors.
    ///
    /// # Errors
    /// Returns [`MirError`] when a mark contains invalid geometry or color.
    pub fn validate(&self) -> Result<(), MirError> {
        for mark in &self.marks {
            mark.anchor.validate()?;
            mark.color.validate()?;
            if mark.candidate >= RulerSelection3D::CANDIDATE_COUNT {
                return Err(MirError::InvalidCommandStream(
                    "3D tick candidate must be in 0..4",
                ));
            }
        }
        Ok(())
    }
}

/// One canvas-CSS-pixel line independent of the current data transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLine {
    pub segment: crate::CssLineSegment,
    pub color: Rgba,
}

/// Screen-space line geometry for axes chrome and other non-text overlays.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenLineBatch {
    pub lines: Vec<ScreenLine>,
    pub width_css_px: CssPx,
    pub pattern: LinePattern3D,
}

impl ScreenLineBatch {
    /// Validates finite endpoints and colors.
    ///
    /// # Errors
    /// Returns [`MirError`] when a line contains invalid geometry or color.
    pub fn validate(&self) -> Result<(), MirError> {
        for line in &self.lines {
            line.segment.start.validate()?;
            line.segment.end.validate()?;
            line.color.validate()?;
        }
        Ok(())
    }
}

impl LineSegmentBatch3D {
    /// Validates finite endpoints and colors.
    ///
    /// # Errors
    /// Returns [`MirError`] when a segment contains invalid geometry or color.
    pub fn validate(&self) -> Result<(), MirError> {
        for segment in &self.segments {
            segment.start.validate()?;
            segment.end.validate()?;
            segment.color.validate()?;
        }
        Ok(())
    }
}

impl SurfaceMesh3D {
    /// Validates triangle, vertex, and index invariants.
    ///
    /// # Errors
    /// Returns [`MirError`] for a non-triangle index stream, invalid vertex,
    /// or out-of-range index.
    pub fn validate(&self) -> Result<(), MirError> {
        if !self.indices.len().is_multiple_of(3) {
            return Err(MirError::InvalidCommandStream(
                "3D surface indices must describe triangles",
            ));
        }
        for &vertex in &self.vertices {
            vertex.validate()?;
        }
        for &index in &self.indices {
            if usize::try_from(index).map_or(true, |index| index >= self.vertices.len()) {
                return Err(MirError::IndexOutOfBounds {
                    index,
                    vertex_count: self.vertices.len(),
                });
            }
        }
        Ok(())
    }
}

impl StrokeMesh {
    /// Validates style, triangle, and index invariants.
    ///
    /// # Errors
    /// Returns [`MirError`] for an invalid style, index count, or index value.
    pub fn validate(&self) -> Result<(), MirError> {
        self.style.validate()?;
        self.color.validate()?;
        for vertex in &self.vertices {
            vertex.position.validate()?;
            if !vertex.edge_distance_css_px.is_finite() {
                return Err(MirError::NonFinite("stroke edge distance"));
            }
        }
        for &index in &self.indices {
            if usize::try_from(index).map_or(true, |index| index >= self.vertices.len()) {
                return Err(MirError::IndexOutOfBounds {
                    index,
                    vertex_count: self.vertices.len(),
                });
            }
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(MirError::InvalidCommandStream(
                "stroke mesh indices must describe triangles",
            ));
        }
        Ok(())
    }
}

impl TriangleMesh2D {
    /// Validates coordinates, colors, and triangle indices.
    ///
    /// # Errors
    /// Returns [`MirError`] for invalid vertices or connectivity.
    pub fn validate(&self) -> Result<(), MirError> {
        for vertex in &self.vertices {
            vertex.position.validate()?;
            vertex.color.validate()?;
        }
        for &index in &self.indices {
            if usize::try_from(index).map_or(true, |index| index >= self.vertices.len()) {
                return Err(MirError::IndexOutOfBounds {
                    index,
                    vertex_count: self.vertices.len(),
                });
            }
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(MirError::InvalidCommandStream(
                "2D triangle mesh indices must describe triangles",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerShape {
    Circle,
    Square,
    Diamond,
    UpTriangle,
    DownTriangle,
    Plus,
    Cross,
    HorizontalLine,
    VerticalLine,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerInstance {
    pub center: AxesPoint,
    pub size_css_px: CssPx,
    pub rotation_radians: f32,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub stroke_width_css_px: CssPx,
}

impl MarkerInstance {
    /// Validates renderer-independent marker values.
    ///
    /// # Errors
    /// Returns [`MirError`] when the rotation is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        if !self.rotation_radians.is_finite() {
            return Err(MirError::NonFinite("marker rotation"));
        }
        self.fill.validate()?;
        self.stroke.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerBatch {
    pub shape: MarkerShape,
    pub instances: Vec<MarkerInstance>,
}

/// One screen-sized marker anchored in normalized 3D Axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerInstance3D {
    pub center: AxesPoint3D,
    pub size_css_px: CssPx,
    pub rotation_radians: f32,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub stroke_width_css_px: CssPx,
}

impl MarkerInstance3D {
    /// Validates renderer-independent marker values.
    ///
    /// # Errors
    /// Returns [`MirError`] when the center or rotation is non-finite.
    pub fn validate(self) -> Result<(), MirError> {
        self.center.validate()?;
        if !self.rotation_radians.is_finite() {
            return Err(MirError::NonFinite("3D marker rotation"));
        }
        self.fill.validate()?;
        self.stroke.validate()?;
        Ok(())
    }
}

/// A batch of depth-tested, screen-sized 3D markers.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerBatch3D {
    pub shape: MarkerShape,
    pub instances: Vec<MarkerInstance3D>,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MirCommand {
    BeginViewport(Viewport),
    SetClipRect(AxesRect),
    SetViewProjection3D(ViewProjection3D),
    SetLighting3D(Lighting3D),
    SetRulerSelection3D(RulerSelection3D),
    GridLineBatch(GridLineBatch),
    StrokeMesh(StrokeMesh),
    TriangleMesh2D(TriangleMesh2D),
    MarkerBatch(MarkerBatch),
    MarkerBatch3D(MarkerBatch3D),
    SurfaceMesh3D(SurfaceMesh3D),
    SurfaceMeshWithEdges3D(SurfaceMesh3D),
    LineSegments3D(LineSegmentBatch3D),
    SurfaceEdgeSegments3D(LineSegmentBatch3D),
    RulerLines3D(RulerLineBatch3D),
    TickMarks3D(TickMarkBatch3D),
    ScreenLines(ScreenLineBatch),
    EndViewport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOperation {
    pub order: DrawOrder,
    pub picking_id: Option<PickingId>,
    pub command: MirCommand,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlotFrame {
    pub operations: Vec<RenderOperation>,
    pub overlay: OverlayPlan,
}

impl PlotFrame {
    /// Creates a validated frame from a command stream and overlay plan.
    ///
    /// # Errors
    /// Returns [`MirError`] when viewport boundaries, order, or draw data is invalid.
    pub fn new(operations: Vec<RenderOperation>, overlay: OverlayPlan) -> Result<Self, MirError> {
        validate_command_stream(&operations)?;
        overlay.validate()?;
        Ok(Self {
            operations,
            overlay,
        })
    }

    /// Revalidates the frame command stream.
    ///
    /// # Errors
    /// Returns [`MirError`] when viewport boundaries, order, or draw data is invalid.
    pub fn validate(&self) -> Result<(), MirError> {
        validate_command_stream(&self.operations)?;
        self.overlay.validate()
    }
}

#[allow(clippy::too_many_lines)]
fn validate_command_stream(operations: &[RenderOperation]) -> Result<(), MirError> {
    if operations.is_empty() {
        return Err(MirError::InvalidCommandStream("command stream is empty"));
    }
    if !matches!(
        operations.first().map(|operation| &operation.command),
        Some(MirCommand::BeginViewport(_))
    ) {
        return Err(MirError::InvalidCommandStream(
            "first command must begin a viewport",
        ));
    }
    if !matches!(
        operations.last().map(|operation| &operation.command),
        Some(MirCommand::EndViewport)
    ) {
        return Err(MirError::InvalidCommandStream(
            "last command must end the viewport",
        ));
    }
    if operations
        .windows(2)
        .any(|pair| pair[0].order > pair[1].order)
    {
        return Err(MirError::InvalidCommandStream(
            "draw order must be non-decreasing",
        ));
    }
    let mut view_projection_3d = false;
    let mut lighting_3d = false;
    let mut ruler_selection_3d = false;
    let mut drew_3d = false;
    for (index, operation) in operations.iter().enumerate() {
        match &operation.command {
            MirCommand::BeginViewport(_) if index != 0 => {
                return Err(MirError::InvalidCommandStream(
                    "nested viewports are not supported",
                ));
            }
            MirCommand::EndViewport if index + 1 != operations.len() => {
                return Err(MirError::InvalidCommandStream(
                    "commands after EndViewport are not supported",
                ));
            }
            MirCommand::BeginViewport(viewport) => viewport.validate()?,
            MirCommand::SetClipRect(clip) => clip.validate()?,
            MirCommand::SetViewProjection3D(matrix) => {
                if view_projection_3d || drew_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D view-projection must be set once before 3D drawing",
                    ));
                }
                matrix.validate()?;
                view_projection_3d = true;
            }
            MirCommand::SetLighting3D(lighting) => {
                if !view_projection_3d || lighting_3d || drew_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D lighting must be set once after the camera and before 3D drawing",
                    ));
                }
                lighting.validate()?;
                lighting_3d = true;
            }
            MirCommand::SetRulerSelection3D(selection) => {
                if !view_projection_3d || ruler_selection_3d || drew_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D ruler selection must be set once after the camera and before 3D drawing",
                    ));
                }
                selection.validate()?;
                ruler_selection_3d = true;
            }
            MirCommand::GridLineBatch(batch) => {
                batch.color.validate()?;
                for segment in &batch.segments {
                    segment.start.validate()?;
                    segment.end.validate()?;
                }
            }
            MirCommand::StrokeMesh(mesh) => mesh.validate()?,
            MirCommand::TriangleMesh2D(mesh) => mesh.validate()?,
            MirCommand::MarkerBatch(batch) => {
                for &instance in &batch.instances {
                    instance.validate()?;
                    instance.center.validate()?;
                }
            }
            MirCommand::MarkerBatch3D(batch) => {
                if !view_projection_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D markers require a preceding view-projection matrix",
                    ));
                }
                for &instance in &batch.instances {
                    instance.validate()?;
                }
                drew_3d = true;
            }
            MirCommand::SurfaceMesh3D(mesh) | MirCommand::SurfaceMeshWithEdges3D(mesh) => {
                if !view_projection_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D surface requires a preceding view-projection matrix",
                    ));
                }
                mesh.validate()?;
                drew_3d = true;
            }
            MirCommand::LineSegments3D(batch) | MirCommand::SurfaceEdgeSegments3D(batch) => {
                if !view_projection_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D line segments require a preceding view-projection matrix",
                    ));
                }
                batch.validate()?;
                drew_3d = true;
            }
            MirCommand::RulerLines3D(batch) => {
                if !ruler_selection_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D ruler lines require a preceding ruler selection",
                    ));
                }
                batch.validate()?;
                drew_3d = true;
            }
            MirCommand::TickMarks3D(batch) => {
                if !ruler_selection_3d {
                    return Err(MirError::InvalidCommandStream(
                        "3D tick marks require a preceding ruler selection",
                    ));
                }
                batch.validate()?;
                drew_3d = true;
            }
            MirCommand::ScreenLines(batch) => batch.validate()?,
            MirCommand::EndViewport => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CssRect, DevicePixelRatio, DeviceRect};

    fn viewport() -> Viewport {
        Viewport {
            css: CssRect::new(10.0, 20.0, 400.0, 300.0).unwrap(),
            device: DeviceRect {
                x: 20,
                y: 40,
                width: 800,
                height: 600,
            },
            device_pixel_ratio: DevicePixelRatio::new(2.0).unwrap(),
        }
    }

    #[test]
    fn command_stream_requires_viewport_boundaries_and_order() {
        let begin = RenderOperation {
            order: DrawOrder(0),
            picking_id: None,
            command: MirCommand::BeginViewport(viewport()),
        };
        let end = RenderOperation {
            order: DrawOrder(2),
            picking_id: None,
            command: MirCommand::EndViewport,
        };
        assert!(PlotFrame::new(vec![begin.clone(), end.clone()], OverlayPlan::default()).is_ok());
        let reversed = vec![
            RenderOperation {
                order: DrawOrder(3),
                ..begin
            },
            RenderOperation {
                order: DrawOrder(2),
                ..end
            },
        ];
        assert!(matches!(
            PlotFrame::new(reversed, OverlayPlan::default()),
            Err(MirError::InvalidCommandStream(_))
        ));
    }

    #[test]
    fn mesh_validation_checks_triangle_indices() {
        let style =
            StrokeStyle::solid(CssPx::new(1.0).unwrap(), LineCap::Butt, LineJoin::Miter).unwrap();
        let mesh = StrokeMesh {
            vertices: vec![StrokeVertex {
                position: AxesPoint { x: 0.0, y: 0.0 },
                edge_distance_css_px: 0.0,
            }],
            indices: vec![0, 1, 0],
            color: Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            style,
        };
        assert_eq!(
            mesh.validate(),
            Err(MirError::IndexOutOfBounds {
                index: 1,
                vertex_count: 1,
            })
        );
    }

    #[test]
    fn picking_identifier_reserves_zero_for_no_hit() {
        assert_eq!(PickingId::from_u64(7).unwrap().get(), 7);
        assert_eq!(
            PickingId::from_u64(0),
            Err(MirError::NonPositive("picking identifier"))
        );
    }

    #[test]
    fn surface_requires_one_camera_before_valid_triangle_geometry() {
        let mesh = SurfaceMesh3D {
            vertices: vec![
                SurfaceVertex3D {
                    position: AxesPoint3D::new(0.0, 0.0, 0.0).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color: Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
                },
                SurfaceVertex3D {
                    position: AxesPoint3D::new(1.0, 0.0, 0.0).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color: Rgba::new(0.0, 1.0, 0.0, 1.0).unwrap(),
                },
                SurfaceVertex3D {
                    position: AxesPoint3D::new(0.0, 1.0, 0.0).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color: Rgba::new(0.0, 0.0, 1.0, 1.0).unwrap(),
                },
            ],
            indices: vec![0, 1, 2],
            color_interpolation: SurfaceColorInterpolation::Flat,
        };
        let begin = RenderOperation {
            order: DrawOrder(0),
            picking_id: None,
            command: MirCommand::BeginViewport(viewport()),
        };
        let draw = RenderOperation {
            order: DrawOrder(2),
            picking_id: None,
            command: MirCommand::SurfaceMesh3D(mesh),
        };
        let end = RenderOperation {
            order: DrawOrder(3),
            picking_id: None,
            command: MirCommand::EndViewport,
        };
        assert!(matches!(
            PlotFrame::new(
                vec![begin.clone(), draw.clone(), end.clone()],
                OverlayPlan::default()
            ),
            Err(MirError::InvalidCommandStream(_))
        ));
        let camera = RenderOperation {
            order: DrawOrder(1),
            picking_id: None,
            command: MirCommand::SetViewProjection3D(ViewProjection3D::identity()),
        };
        assert!(PlotFrame::new(vec![begin, camera, draw, end], OverlayPlan::default()).is_ok());
    }
}
