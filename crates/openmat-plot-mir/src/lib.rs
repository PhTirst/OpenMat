#![doc = "Backend-neutral Plot Engine render and overlay intermediate representation."]
#![forbid(unsafe_code)]

mod coordinates;
mod coordinates3d;
mod error;
mod overlay;
mod render;

pub use coordinates::{
    AxesPoint, AxesRect, CssPoint, CssPx, CssRect, CssSize, DataPoint, DataRect, DevicePixelRatio,
    DeviceRect, DeviceSize, Rgba, ViewTransform2D, Viewport,
};
pub use coordinates3d::{AxesPoint3D, AxesVector3D, ViewProjection3D};
pub use error::{InteractionError, MirError};
pub use overlay::{
    AccessibilityNode, AxisDimension, CssLineSegment, FontWeight, HorizontalAlignment, LegendEntry,
    LegendPlan, OverlayAxisLine, OverlayFont, OverlayFontStyle, OverlayPlan, OverlayText,
    OverlayTextRole, OverlayTick, TextInterpreter, Utf16Text, VerticalAlignment,
};
pub use render::{
    AxesLineSegment, DrawOrder, GridLineBatch, Lighting3D, LineCap, LineJoin, LinePattern3D,
    LineSegment3D, LineSegmentBatch3D, MarkerBatch, MarkerBatch3D, MarkerInstance,
    MarkerInstance3D, MarkerShape, MirCommand, PickingId, PlotFrame, RenderOperation, RulerLine3D,
    RulerLineBatch3D, RulerSelection3D, ScreenLine, ScreenLineBatch, StrokeMesh, StrokeStyle,
    StrokeVertex, SurfaceColorInterpolation, SurfaceMesh3D, SurfaceVertex3D, TickMark3D,
    TickMarkBatch3D, TriangleMesh2D, TriangleVertex2D,
};
