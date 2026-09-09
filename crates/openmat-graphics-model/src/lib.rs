#![doc = "Session-owned graphics objects, generational handles, and Plot HIR state."]
#![forbid(unsafe_code)]

mod colormap;
mod isonormals;
mod model;
mod session;

pub use colormap::{
    DEFAULT_COLORMAP_LENGTH, PARULA_R2022B, PredefinedColormap, TURBO_R2022B, parula_r2022b,
    predefined_colormap_r2022b,
};
pub use model::{
    Axes2DProperties, AxesCoordinateSystem, Axis, AxisDirection, AxisScale, CDataMapping,
    ChartColor, ChartGroupInput, ChartGroupKind, ChartGroupProperties, ChartPrimitiveInput,
    ColorBarProperties, DataDType, DataRef, DataResource, DataResourceId, FigureCreationProperties,
    FigureProperties, FigureSnapshot, FontStyle, GraphicsClass, GraphicsDelta,
    GraphicsDeltaOperation, GraphicsError, GraphicsErrorCategory, GraphicsExecution,
    GraphicsHandle, GraphicsNotice, GraphicsNoticeKind, GraphicsObject, GraphicsProperty,
    GraphicsPropertyUpdate, GraphicsPropertyValue, GraphicsRequest, GraphicsResponse, GridMode,
    HirObject, HirProperties, HorizontalAlignment, Interpreter, IsoNormalsInput, LegendLocation,
    LegendOrientation, LegendProperties, LightingMode, LimitMethod, LimitMode, LineInput,
    LineSeriesProperties, LineStyle, Marker, MarkerIndicesClass, NextPlot, NextTileSelection,
    NumericData, PatchInput, PatchSeriesProperties, ProjectionMode, ScatterColorTarget,
    ScatterInput, ScatterSeriesProperties, ShadingMode, StyledLineInput, SurfaceColor,
    SurfaceInput, SurfaceSeriesProperties, SurfaceStyle, TextProperties, TextRole, ThetaAxisUnits,
    ThetaDirection, ThetaZeroLocation, TickDirection, TickMode, TilePadding, TileSpacing,
    TiledChartLayoutProperties, VerticalAlignment,
};
pub use session::{AutoLimitPlanner, DefaultAutoLimitPlanner, GraphicsSession};
