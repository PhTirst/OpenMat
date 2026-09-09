use std::{error::Error, fmt, sync::Arc};

const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;

/// Public graphics classes implemented by the first Plot Engine tranche.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GraphicsClass {
    /// `matlab.ui.Figure`.
    Figure,
    /// `matlab.graphics.axis.Axes`.
    Axes2D,
    /// `matlab.graphics.axis.PolarAxes`.
    PolarAxes,
    /// `matlab.graphics.layout.TiledChartLayout`.
    TiledChartLayout,
    /// `matlab.graphics.chart.primitive.Line`.
    LineSeries,
    /// `matlab.graphics.chart.primitive.Scatter`.
    ScatterSeries,
    /// `matlab.graphics.chart.primitive.Surface`.
    SurfaceSeries,
    /// `matlab.graphics.primitive.Patch`.
    PatchSeries,
    /// `matlab.graphics.chart.primitive.Stair`.
    Stair,
    /// `matlab.graphics.chart.primitive.Stem`.
    Stem,
    /// `matlab.graphics.chart.primitive.ErrorBar`.
    ErrorBar,
    /// `matlab.graphics.chart.primitive.Area`.
    Area,
    /// `matlab.graphics.chart.primitive.Bar`.
    Bar,
    /// `matlab.graphics.chart.primitive.Histogram`.
    Histogram,
    /// `matlab.graphics.chart.primitive.Contour`.
    Contour,
    /// `matlab.graphics.primitive.Image`.
    Image,
    /// `matlab.graphics.primitive.Text`.
    Text,
    /// `matlab.graphics.illustration.Legend`.
    Legend,
    /// `matlab.graphics.illustration.ColorBar`.
    ColorBar,
}

impl GraphicsClass {
    /// Returns the measured MATLAB R2022b public class name.
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Figure => "matlab.ui.Figure",
            Self::Axes2D => "matlab.graphics.axis.Axes",
            Self::PolarAxes => "matlab.graphics.axis.PolarAxes",
            Self::TiledChartLayout => "matlab.graphics.layout.TiledChartLayout",
            Self::LineSeries => "matlab.graphics.chart.primitive.Line",
            Self::ScatterSeries => "matlab.graphics.chart.primitive.Scatter",
            Self::SurfaceSeries => "matlab.graphics.chart.primitive.Surface",
            Self::PatchSeries => "matlab.graphics.primitive.Patch",
            Self::Stair => "matlab.graphics.chart.primitive.Stair",
            Self::Stem => "matlab.graphics.chart.primitive.Stem",
            Self::ErrorBar => "matlab.graphics.chart.primitive.ErrorBar",
            Self::Area => "matlab.graphics.chart.primitive.Area",
            Self::Bar => "matlab.graphics.chart.primitive.Bar",
            Self::Histogram => "matlab.graphics.chart.primitive.Histogram",
            Self::Contour => "matlab.graphics.chart.primitive.Contour",
            Self::Image => "matlab.graphics.primitive.Image",
            Self::Text => "matlab.graphics.primitive.Text",
            Self::Legend => "matlab.graphics.illustration.Legend",
            Self::ColorBar => "matlab.graphics.illustration.ColorBar",
        }
    }
}

/// A session-local generational graphics handle.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphicsHandle {
    slot: u32,
    generation: u32,
    class: GraphicsClass,
}

impl GraphicsHandle {
    /// Constructs a checked handle from arena components.
    #[must_use]
    pub const fn new(slot: u32, generation: u32, class: GraphicsClass) -> Option<Self> {
        if slot == 0 || generation == 0 {
            None
        } else {
            Some(Self {
                slot,
                generation,
                class,
            })
        }
    }

    /// Returns the one-based arena slot.
    #[must_use]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// Returns the slot generation.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Returns the public graphics class carried by this handle.
    #[must_use]
    pub const fn class(self) -> GraphicsClass {
        self.class
    }
}

/// Immutable numerical resource element type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DataDType {
    /// IEEE-754 binary32.
    F32,
    /// IEEE-754 binary64.
    F64,
}

impl DataDType {
    /// Returns the element width in bytes.
    #[must_use]
    pub const fn byte_width(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// A session-local immutable data resource identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DataResourceId(u64);

impl DataResourceId {
    pub(crate) const fn new(identifier: u64) -> Option<Self> {
        if identifier == 0 || identifier > JSON_SAFE_INTEGER_MAX {
            None
        } else {
            Some(Self(identifier))
        }
    }

    /// Returns the session-local numeric identifier.
    #[must_use]
    pub const fn identifier(self) -> u64 {
        self.0
    }
}

/// A protocol-neutral reference to immutable column-major bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataRef {
    /// Resource identifier.
    pub id: DataResourceId,
    /// Exact source precision.
    pub dtype: DataDType,
    /// Canonical two-dimensional MATLAB shape.
    pub shape: [u64; 2],
    /// Payload byte length.
    pub byte_length: u64,
}

/// An immutable little-endian data resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataResource {
    descriptor: DataRef,
    bytes: Arc<[u8]>,
}

impl DataResource {
    pub(crate) fn from_numeric_with_shape(
        id: DataResourceId,
        data: &NumericData,
        shape: [u64; 2],
    ) -> Result<Self, GraphicsError> {
        let length = data.len();
        let length_u64 = u64::try_from(length).map_err(|_| {
            GraphicsError::limit("graphics data length cannot be represented by the protocol")
        })?;
        let shape_length = shape[0]
            .checked_mul(shape[1])
            .ok_or_else(|| GraphicsError::limit("graphics data shape overflowed"))?;
        if shape_length != length_u64 {
            return Err(GraphicsError::invalid_input(
                "graphics data shape does not match its element count",
            ));
        }
        let byte_length = length_u64
            .checked_mul(data.dtype().byte_width())
            .filter(|length| *length <= JSON_SAFE_INTEGER_MAX)
            .ok_or_else(|| GraphicsError::limit("graphics data byte length overflowed"))?;
        let capacity = usize::try_from(byte_length)
            .map_err(|_| GraphicsError::limit("graphics data byte length exceeds host limits"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| GraphicsError::limit("graphics data allocation failed"))?;
        match data {
            NumericData::F32(values) => {
                for value in values.iter() {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            NumericData::F64(values) => {
                for value in values.iter() {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(Self {
            descriptor: DataRef {
                id,
                dtype: data.dtype(),
                shape,
                byte_length,
            },
            bytes: Arc::from(bytes),
        })
    }

    /// Returns the immutable descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &DataRef {
        &self.descriptor
    }

    /// Returns immutable little-endian payload bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Clones the immutable shared byte allocation without copying payload data.
    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    /// Returns whether two snapshots share the immutable byte allocation.
    #[must_use]
    pub fn shares_bytes_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes)
    }
}

/// Prepared real numerical data for one logical series vector.
#[derive(Clone, Debug, PartialEq)]
pub enum NumericData {
    /// Exact binary32 values.
    F32(Arc<[f32]>),
    /// Exact binary64 values.
    F64(Arc<[f64]>),
}

impl NumericData {
    /// Creates copied binary32 graphics data.
    #[must_use]
    pub fn from_f32(values: impl Into<Arc<[f32]>>) -> Self {
        Self::F32(values.into())
    }

    /// Creates copied binary64 graphics data.
    #[must_use]
    pub fn from_f64(values: impl Into<Arc<[f64]>>) -> Self {
        Self::F64(values.into())
    }

    /// Returns the exact element type.
    #[must_use]
    pub const fn dtype(&self) -> DataDType {
        match self {
            Self::F32(_) => DataDType::F32,
            Self::F64(_) => DataDType::F64,
        }
    }

    /// Returns the vector length.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
        }
    }

    /// Returns whether this vector is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Visits values widened to `f64` without changing stored precision.
    pub fn visit_f64(&self, mut visitor: impl FnMut(f64)) {
        match self {
            Self::F32(values) => values.iter().for_each(|value| visitor(f64::from(*value))),
            Self::F64(values) => values.iter().for_each(|value| visitor(*value)),
        }
    }
}

/// One fully prepared line, with continuous independent X/Y and optional Z resources.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInput {
    /// X values in display order.
    pub x: NumericData,
    /// Y values in display order.
    pub y: NumericData,
    /// Optional Z values in display order. Present for `plot3`.
    pub z: Option<NumericData>,
}

/// One fully prepared line together with assignments specific to that series.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledLineInput {
    /// Independent numerical data for the series.
    pub line: LineInput,
    /// Assignments applied after automatic Axes color/style selection.
    pub properties: Vec<GraphicsPropertyUpdate>,
}

/// One fully prepared scatter series.
#[derive(Clone, Debug, PartialEq)]
pub struct ScatterInput {
    /// X values in display order.
    pub x: NumericData,
    /// Y values in display order.
    pub y: NumericData,
    /// Optional Z values in display order. Present for `scatter3`.
    pub z: Option<NumericData>,
    /// Optional scalar or one-value-per-point marker areas in points squared.
    pub size_data: Option<NumericData>,
    /// Optional one-value-per-point scalar color data resolved through Axes `CLim`.
    pub color_data: Option<NumericData>,
    /// Whether scalar color data colors marker faces instead of edges.
    pub filled: bool,
}

/// One prepared structured surface in MATLAB column-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceInput {
    /// X coordinates, either one value per Z column or one value per Z element.
    pub x: NumericData,
    /// Y coordinates, either one value per Z row or one value per Z element.
    pub y: NumericData,
    /// Z matrix values in column-major order.
    pub z: NumericData,
    /// `CData` matrix values in column-major order.
    pub c: NumericData,
    /// Matrix row count.
    pub rows: u64,
    /// Matrix column count.
    pub columns: u64,
}

/// One prepared irregular polygon mesh in MATLAB column-major order.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchInput {
    /// Face connectivity matrix. Each row is one face and NaN pads short rows.
    pub faces: NumericData,
    /// Vertex coordinate matrix with two or three columns.
    pub vertices: NumericData,
    /// Optional scalar face/vertex color data.
    pub face_vertex_cdata: NumericData,
    /// Number of face rows.
    pub face_rows: u64,
    /// Maximum number of vertices per face.
    pub face_columns: u64,
    /// Number of vertices.
    pub vertex_rows: u64,
    /// Coordinate columns, either two or three.
    pub vertex_columns: u64,
    /// Color-data rows.
    pub cdata_rows: u64,
    /// Color-data columns.
    pub cdata_columns: u64,
    /// Initial face color intent resolved from the constructor form.
    pub face_color: SurfaceColor,
    /// Initial edge color intent resolved from the constructor form.
    pub edge_color: SurfaceColor,
}

/// Scalar volume and optional rectilinear coordinates used by MATLAB `isonormals`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoNormalsInput {
    /// Full meshgrid-style X coordinate volume, or `None` for `1:columns`.
    pub x: Option<NumericData>,
    /// Full meshgrid-style Y coordinate volume, or `None` for `1:rows`.
    pub y: Option<NumericData>,
    /// Full meshgrid-style Z coordinate volume, or `None` for `1:pages`.
    pub z: Option<NumericData>,
    /// Scalar field values in MATLAB column-major order.
    pub values: NumericData,
    /// MATLAB volume shape `[rows, columns, pages]`.
    pub shape: [u64; 3],
}

/// MATLAB-visible high-level 2D chart type backed by retained Line/Patch children.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartGroupKind {
    /// Step plot.
    Stair,
    /// Stem plot.
    Stem,
    /// Error-bar plot.
    ErrorBar,
    /// Filled area plot.
    Area,
    /// Bar chart.
    Bar,
    /// Histogram chart.
    Histogram,
    /// Contour chart.
    Contour,
    /// Raster-like image chart.
    Image,
}

impl ChartGroupKind {
    /// Returns the public graphics class represented by this chart.
    #[must_use]
    pub const fn graphics_class(self) -> GraphicsClass {
        match self {
            Self::Stair => GraphicsClass::Stair,
            Self::Stem => GraphicsClass::Stem,
            Self::ErrorBar => GraphicsClass::ErrorBar,
            Self::Area => GraphicsClass::Area,
            Self::Bar => GraphicsClass::Bar,
            Self::Histogram => GraphicsClass::Histogram,
            Self::Contour => GraphicsClass::Contour,
            Self::Image => GraphicsClass::Image,
        }
    }
}

/// Retained primitive owned by one MATLAB-visible chart object.
#[derive(Clone, Debug, PartialEq)]
pub enum ChartPrimitiveInput {
    /// One line primitive. `automatic_color` shares the chart's `ColorOrder` entry.
    Line {
        /// Prepared source geometry.
        line: LineInput,
        /// Explicit line assignments.
        properties: Vec<GraphicsPropertyUpdate>,
        /// Whether the chart `ColorOrder` entry supplies the default line color.
        automatic_color: bool,
    },
    /// One patch primitive. `automatic_color` shares the chart's `ColorOrder` entry.
    Patch {
        /// Prepared polygon geometry.
        patch: PatchInput,
        /// Explicit patch assignments.
        properties: Vec<GraphicsPropertyUpdate>,
        /// Whether the chart `ColorOrder` entry supplies the default face color.
        automatic_color: bool,
    },
}

/// Fully prepared high-level chart creation input.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartGroupInput {
    /// MATLAB-visible chart type.
    pub kind: ChartGroupKind,
    /// Retained drawing primitives in stable paint order.
    pub primitives: Vec<ChartPrimitiveInput>,
    /// Chart-container assignments retained independently of drawing primitives.
    pub properties: Vec<GraphicsPropertyUpdate>,
    /// Axes assignments committed atomically with chart creation.
    pub axes_properties: Vec<GraphicsPropertyUpdate>,
}

/// Axes limit mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitMode {
    /// Limits are resolved from current series data.
    Auto,
    /// Limits were explicitly assigned.
    Manual,
}

/// MATLAB Cartesian axis scale.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AxisScale {
    /// Linear data coordinates.
    #[default]
    Linear,
    /// Base-10 logarithmic data coordinates.
    Log,
}

/// MATLAB Cartesian axis direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AxisDirection {
    /// Values increase left-to-right or bottom-to-top.
    #[default]
    Normal,
    /// Values increase in the opposite screen direction.
    Reverse,
}

/// Coordinate system owned by an Axes HIR object.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AxesCoordinateSystem {
    /// Ordinary Cartesian X/Y coordinates.
    #[default]
    Cartesian,
    /// MATLAB polar coordinates with theta data supplied in radians.
    Polar,
}

impl AxesCoordinateSystem {
    /// Returns the MATLAB-visible graphics class for this coordinate system.
    #[must_use]
    pub const fn graphics_class(self) -> GraphicsClass {
        match self {
            Self::Cartesian => GraphicsClass::Axes2D,
            Self::Polar => GraphicsClass::PolarAxes,
        }
    }
}

/// Display units used by `PolarAxes` limits, ticks, and labels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThetaAxisUnits {
    /// Degrees, matching the R2022b factory default.
    #[default]
    Degrees,
    /// Radians.
    Radians,
}

/// Direction in which `PolarAxes` theta values increase.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThetaDirection {
    /// Counterclockwise, matching the R2022b factory default.
    #[default]
    Counterclockwise,
    /// Clockwise.
    Clockwise,
}

/// Screen edge at which zero theta is placed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThetaZeroLocation {
    /// Three o'clock, matching the R2022b factory default.
    #[default]
    Right,
    /// Twelve o'clock.
    Top,
    /// Nine o'clock.
    Left,
    /// Six o'clock.
    Bottom,
}

/// MATLAB automatic limit expansion policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LimitMethod {
    /// Expand to suitable tick boundaries.
    #[default]
    TickAligned,
    /// Use exact finite data bounds.
    Tight,
    /// Add the R2022b seven-percent data-span margin.
    Padded,
}

/// Automatic/manual mode for one axis tick or tick-label property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickMode {
    /// Values follow the resolved axis limits.
    Auto,
    /// Values were explicitly assigned or frozen.
    Manual,
}

/// MATLAB text-interpreter intent retained independently of layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interpreter {
    /// MATLAB's lightweight TeX subset.
    Tex,
    /// LaTeX math/text interpretation.
    Latex,
    /// Literal text with no markup interpretation.
    None,
}

/// Language storage class retained by the `MarkerIndices` property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerIndicesClass {
    /// Factory-default MATLAB `uint64` storage.
    UInt64,
    /// Explicit double input.
    Double,
    /// Explicit single input.
    Single,
}

/// MATLAB-compatible replacement/addition state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NextPlot {
    /// A new plotting call replaces relevant existing children.
    Replace,
    /// A new plotting call adds children.
    Add,
}

/// Grid state accepted by the first tranche.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridMode {
    /// Grid disabled.
    Off,
    /// Grid enabled.
    On,
}

/// Axis selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    /// Horizontal axis.
    X,
    /// Vertical axis.
    Y,
    /// Depth axis.
    Z,
}

/// Text role in an Axes hierarchy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRole {
    /// Axes title.
    Title,
    /// X-axis label.
    XLabel,
    /// Y-axis label.
    YLabel,
    /// Z-axis label.
    ZLabel,
}

/// MATLAB Axes projection intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    /// Parallel projection.
    Orthographic,
    /// Perspective projection.
    Perspective,
}

/// MATLAB Surface face/edge color intent retained above tessellation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceColor {
    /// Resolve colors from `CData` and `CLim`.
    Flat,
    /// Resolve colors at vertices and interpolate them across each triangle.
    Interp,
    /// Do not render this component.
    None,
    /// One explicit unpremultiplied sRGB color.
    Rgba([f32; 4]),
}

/// MATLAB `shading` modes for Surface objects in an Axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadingMode {
    /// Flat faces without mesh edges.
    Flat,
    /// Interpolated faces without mesh edges.
    Interp,
    /// Flat faces with black mesh edges.
    Faceted,
}

/// MATLAB surface-lighting mode retained on Cartesian Axes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LightingMode {
    /// Disable lighting and display source colors directly.
    #[default]
    None,
    /// Use one face normal per triangle.
    Flat,
    /// Evaluate lighting at vertices and interpolate it across triangles.
    Gouraud,
}

/// Surface construction style selected by the plotting builtin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceStyle {
    /// Filled colored faces with black edges.
    Surf,
    /// White faces with CData-colored edges.
    Mesh,
}

/// Surface `CData` interpretation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CDataMapping {
    /// Scale through the Axes `CLim` interval.
    Scaled,
    /// Treat values as direct colormap indices.
    Direct,
}

/// First-tranche line style HIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineStyle {
    /// No stroke. Markers remain visible.
    None,
    /// Continuous stroke.
    Solid,
    /// Dashed stroke, reserved for a later style API.
    Dash,
    /// Dotted stroke, reserved for a later style API.
    Dot,
    /// Dash-dot stroke, reserved for a later style API.
    DashDot,
}

/// Marker shape HIR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Marker {
    /// No marker.
    None,
    /// Point marker.
    Point,
    /// Circle marker.
    Circle,
    /// Plus marker.
    Plus,
    /// Star marker.
    Star,
    /// Cross marker.
    Cross,
    /// Square marker.
    Square,
    /// Diamond marker.
    Diamond,
    /// Upward triangle marker.
    TriangleUp,
    /// Downward triangle marker.
    TriangleDown,
    /// Right triangle marker.
    TriangleRight,
    /// Left triangle marker.
    TriangleLeft,
    /// Horizontal line marker.
    HorizontalLine,
    /// Vertical line marker.
    VerticalLine,
}

/// Settable Plot Engine 2D property names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsProperty {
    /// Public parent handle.
    Parent,
    /// Stable direct children.
    Children,
    /// Axes normalized outer rectangle `[left, bottom, width, height]`.
    Position,
    /// Figure title-bar name.
    Name,
    /// Figure number prefix state.
    NumberTitle,
    /// Tiled layout `[rows columns]` grid size.
    GridSize,
    /// Tiled layout inter-tile spacing mode.
    TileSpacing,
    /// Tiled layout outer padding mode.
    Padding,
    /// Line `LineWidth`.
    LineWidth,
    /// Line `Color`.
    Color,
    /// Line `LineStyle`.
    LineStyle,
    /// Line or Scatter `Marker`.
    Marker,
    /// Line `MarkerSize`.
    MarkerSize,
    /// Line one-based source-point marker indices.
    MarkerIndices,
    /// Line or Scatter legend display name.
    DisplayName,
    /// Whether Axes clipping applies to a plotted series.
    Clipping,
    /// Line or Scatter `Visible`.
    Visible,
    /// Scalar Scatter `SizeData`.
    SizeData,
    /// Scalar RGB Scatter `CData`.
    CData,
    /// Scalar Scatter `MarkerFaceColor`.
    MarkerFaceColor,
    /// Scalar Scatter `MarkerEdgeColor`.
    MarkerEdgeColor,
    /// Chart baseline value.
    BaseValue,
    /// Bar chart relative bar width.
    BarWidth,
    /// Error-bar cap width in typographic points.
    CapSize,
    /// Axes color order matrix.
    ColorOrder,
    /// Axes line-style order.
    LineStyleOrder,
    /// Axes one-based color order index.
    ColorOrderIndex,
    /// Axes X limits.
    XLim,
    /// Axes Y limits.
    YLim,
    /// Axes Z limits.
    ZLim,
    /// Axes X limit mode.
    XLimMode,
    /// Axes Y limit mode.
    YLimMode,
    /// Axes Z limit mode.
    ZLimMode,
    /// `PolarAxes` theta limits in the active theta units.
    ThetaLim,
    /// `PolarAxes` radial limits.
    RLim,
    /// `PolarAxes` theta limit mode.
    ThetaLimMode,
    /// `PolarAxes` radial limit mode.
    RLimMode,
    /// Axes X scale.
    XScale,
    /// Axes Y scale.
    YScale,
    /// Axes Z scale.
    ZScale,
    /// Axes X automatic limit method.
    XLimitMethod,
    /// Axes Y automatic limit method.
    YLimitMethod,
    /// Axes Z automatic limit method.
    ZLimitMethod,
    /// Axes X direction.
    XDir,
    /// Axes Y direction.
    YDir,
    /// Axes Z direction.
    ZDir,
    /// Axes X tick positions.
    XTick,
    /// Axes Y tick positions.
    YTick,
    /// Axes Z tick positions.
    ZTick,
    /// Axes X tick-position mode.
    XTickMode,
    /// Axes Y tick-position mode.
    YTickMode,
    /// Axes Z tick-position mode.
    ZTickMode,
    /// Axes X tick labels.
    XTickLabel,
    /// Axes Y tick labels.
    YTickLabel,
    /// Axes Z tick labels.
    ZTickLabel,
    /// Axes X tick-label mode.
    XTickLabelMode,
    /// Axes Y tick-label mode.
    YTickLabelMode,
    /// Axes Z tick-label mode.
    ZTickLabelMode,
    /// `PolarAxes` theta tick positions.
    ThetaTick,
    /// `PolarAxes` radial tick positions.
    RTick,
    /// `PolarAxes` theta tick-position mode.
    ThetaTickMode,
    /// `PolarAxes` radial tick-position mode.
    RTickMode,
    /// `PolarAxes` theta tick labels.
    ThetaTickLabel,
    /// `PolarAxes` radial tick labels.
    RTickLabel,
    /// `PolarAxes` theta tick-label mode.
    ThetaTickLabelMode,
    /// `PolarAxes` radial tick-label mode.
    RTickLabelMode,
    /// `PolarAxes` angle display units.
    ThetaAxisUnits,
    /// `PolarAxes` theta direction.
    ThetaDir,
    /// `PolarAxes` zero-angle screen location.
    ThetaZeroLocation,
    /// `PolarAxes` radial ruler location.
    RAxisLocation,
    /// `PolarAxes` theta grid state.
    ThetaGrid,
    /// `PolarAxes` radial grid state.
    RGrid,
    /// `PolarAxes` theta minor-grid state.
    ThetaMinorGrid,
    /// `PolarAxes` radial minor-grid state.
    RMinorGrid,
    /// Colorbar tick positions.
    Ticks,
    /// Colorbar tick-position mode.
    TicksMode,
    /// Colorbar tick labels.
    TickLabels,
    /// Colorbar tick-label mode.
    TickLabelsMode,
    /// Axes tick-label interpreter.
    TickLabelInterpreter,
    /// Axes outline state.
    Box,
    /// Axes X-grid state.
    XGrid,
    /// Axes Y-grid state.
    YGrid,
    /// Axes Z-grid compatibility state.
    ZGrid,
    /// Axes X-minor-grid state.
    XMinorGrid,
    /// Axes Y-minor-grid state.
    YMinorGrid,
    /// Axes Z-minor-grid state.
    ZMinorGrid,
    /// Axes tick direction.
    TickDir,
    /// Line or Scatter X data.
    XData,
    /// Line or Scatter Y data.
    YData,
    /// Polar Line theta data, always supplied in radians.
    ThetaData,
    /// Polar Line radial data.
    RData,
    /// Surface Z data.
    ZData,
    /// Axes azimuth/elevation pair.
    View,
    /// Axes projection mode.
    Projection,
    /// Axes data-unit aspect ratio.
    DataAspectRatio,
    /// Axes data-unit aspect-ratio mode.
    DataAspectRatioMode,
    /// Axes plot-box aspect ratio.
    PlotBoxAspectRatio,
    /// Axes plot-box aspect-ratio mode.
    PlotBoxAspectRatioMode,
    /// Axes color limits.
    CLim,
    /// Axes color-limit mode.
    CLimMode,
    /// Surface face color intent.
    FaceColor,
    /// Surface edge color intent.
    EdgeColor,
    /// Surface `CData` mapping mode.
    CDataMapping,
    /// Surface face alpha.
    FaceAlpha,
    /// Patch edge alpha.
    EdgeAlpha,
    /// Patch connectivity matrix.
    Faces,
    /// Patch vertex coordinate matrix.
    Vertices,
    /// Patch per-face/per-vertex color data.
    FaceVertexCData,
    /// Text or Legend strings.
    String,
    /// Legend placement.
    Location,
    /// Text or Legend font size.
    FontSize,
    /// Axes, Text, or Legend font family.
    FontName,
    /// Text or Legend font weight.
    FontWeight,
    /// Text rotation in degrees.
    Rotation,
    /// Legend item flow direction.
    Orientation,
    /// Legend column count.
    NumColumns,
    /// Text or Legend interpreter.
    Interpreter,
}

/// One validated-intent graphics property assignment.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsPropertyUpdate {
    /// Axes normalized outer rectangle `[left, bottom, width, height]`.
    Position([f64; 4]),
    /// Exact UTF-16 Figure title-bar name.
    Name(Vec<u16>),
    /// Whether the Figure number prefixes its title-bar name.
    NumberTitle(bool),
    /// Positive finite line width in typographic points.
    LineWidth(f32),
    /// Unpremultiplied sRGB line color.
    Color([f32; 4]),
    /// Axes background color, or `None` for MATLAB `Color='none'`.
    AxesColor(Option<[f32; 4]>),
    /// Line stroke pattern.
    LineStyle(LineStyle),
    /// Line or Scatter marker.
    Marker(Marker),
    /// Positive finite line marker size in typographic points.
    MarkerSize(f32),
    /// One-based source-point indices at which markers are drawn.
    MarkerIndices {
        /// Positive one-based index values.
        values: Vec<u64>,
        /// MATLAB-visible numeric storage class.
        value_class: MarkerIndicesClass,
    },
    /// Exact UTF-16 legend display name.
    DisplayName(Vec<u16>),
    /// Whether Axes clipping applies.
    Clipping(bool),
    /// Line or Scatter visibility.
    Visible(bool),
    /// Nonnegative finite scalar Scatter marker area in points squared.
    SizeData(f32),
    /// Unpremultiplied scalar sRGB Scatter color data.
    CData([f32; 4]),
    /// Unpremultiplied scalar sRGB Scatter face color.
    MarkerFaceColor([f32; 4]),
    /// Unpremultiplied scalar sRGB Scatter edge color.
    MarkerEdgeColor([f32; 4]),
    /// High-level chart marker face color intent.
    ChartMarkerFaceColor(ChartColor),
    /// High-level chart marker edge color intent.
    ChartMarkerEdgeColor(ChartColor),
    /// Finite chart baseline value.
    BaseValue(f64),
    /// Finite Bar width in `(0,1]`.
    BarWidth(f64),
    /// Nonnegative finite `ErrorBar` cap width in typographic points.
    CapSize(f64),
    /// Axes automatic line colors.
    ColorOrder(Vec<[f32; 4]>),
    /// Axes automatic line styles.
    LineStyleOrder(Vec<LineStyle>),
    /// Axes one-based color order index.
    ColorOrderIndex(u32),
    /// Explicit Axes X limits.
    XLim([f64; 2]),
    /// Explicit Axes Y limits.
    YLim([f64; 2]),
    /// Explicit Axes Z limits.
    ZLim([f64; 2]),
    /// Axes X limit mode.
    XLimMode(LimitMode),
    /// Axes Y limit mode.
    YLimMode(LimitMode),
    /// Axes Z limit mode.
    ZLimMode(LimitMode),
    /// Axes X scale.
    XScale(AxisScale),
    /// Axes Y scale.
    YScale(AxisScale),
    /// Axes Z scale.
    ZScale(AxisScale),
    /// Axes X automatic limit method.
    XLimitMethod(LimitMethod),
    /// Axes Y automatic limit method.
    YLimitMethod(LimitMethod),
    /// Axes Z automatic limit method.
    ZLimitMethod(LimitMethod),
    /// Axes X direction.
    XDir(AxisDirection),
    /// Axes Y direction.
    YDir(AxisDirection),
    /// Axes Z direction.
    ZDir(AxisDirection),
    /// Explicit Axes X tick positions.
    XTick(Vec<f64>),
    /// Explicit Axes Y tick positions.
    YTick(Vec<f64>),
    /// Explicit Axes Z tick positions.
    ZTick(Vec<f64>),
    /// Axes X tick-position mode.
    XTickMode(TickMode),
    /// Axes Y tick-position mode.
    YTickMode(TickMode),
    /// Axes Z tick-position mode.
    ZTickMode(TickMode),
    /// Explicit Axes X tick labels.
    XTickLabel(Vec<Vec<u16>>),
    /// Explicit Axes Y tick labels.
    YTickLabel(Vec<Vec<u16>>),
    /// Explicit Axes Z tick labels.
    ZTickLabel(Vec<Vec<u16>>),
    /// Axes X tick-label mode.
    XTickLabelMode(TickMode),
    /// Axes Y tick-label mode.
    YTickLabelMode(TickMode),
    /// Axes Z tick-label mode.
    ZTickLabelMode(TickMode),
    /// Explicit Colorbar tick positions.
    Ticks(Vec<f64>),
    /// Colorbar tick-position mode.
    TicksMode(TickMode),
    /// Explicit Colorbar tick labels.
    TickLabels(Vec<Vec<u16>>),
    /// Colorbar tick-label mode.
    TickLabelsMode(TickMode),
    /// Axes tick-label interpreter.
    TickLabelInterpreter(Interpreter),
    /// Axes outline state.
    Box(bool),
    /// Axes X-minor-grid state.
    XMinorGrid(bool),
    /// Axes Y-minor-grid state.
    YMinorGrid(bool),
    /// Axes Z-minor-grid state.
    ZMinorGrid(bool),
    /// Axes X-grid state.
    XGrid(bool),
    /// Axes Y-grid state.
    YGrid(bool),
    /// Axes Z-grid state.
    ZGrid(bool),
    /// Axes tick direction.
    TickDir(TickDirection),
    /// `PolarAxes` angle display units.
    ThetaAxisUnits(ThetaAxisUnits),
    /// `PolarAxes` theta direction.
    ThetaDir(ThetaDirection),
    /// `PolarAxes` zero-angle screen location.
    ThetaZeroLocation(ThetaZeroLocation),
    /// `PolarAxes` radial ruler location in the active theta units.
    RAxisLocation(f64),
    /// Surface face color intent.
    FaceColor(SurfaceColor),
    /// Surface edge color intent.
    EdgeColor(SurfaceColor),
    /// High-level chart face color intent.
    ChartFaceColor(ChartColor),
    /// High-level chart edge color intent.
    ChartEdgeColor(ChartColor),
    /// Surface or Patch color-data interpretation.
    CDataMapping(CDataMapping),
    /// Replacement X series data.
    XData(NumericData),
    /// Replacement Y series data.
    YData(NumericData),
    /// Replacement Z series data.
    ZData(NumericData),
    /// Axes projection mode.
    Projection(ProjectionMode),
    /// Positive finite Axes data-unit aspect ratio.
    DataAspectRatio([f64; 3]),
    /// Axes data-unit aspect-ratio mode.
    DataAspectRatioMode(LimitMode),
    /// Positive finite Axes plot-box aspect ratio.
    PlotBoxAspectRatio([f64; 3]),
    /// Axes plot-box aspect-ratio mode.
    PlotBoxAspectRatioMode(LimitMode),
    /// Explicit Axes color limits.
    CLim([f64; 2]),
    /// Axes color-limit mode.
    CLimMode(LimitMode),
    /// Text or Legend font size.
    FontSize(f32),
    /// Exact UTF-16 font family.
    FontName(Vec<u16>),
    /// CSS-like font weight in `1..=1000`.
    FontWeight(u16),
    /// Text rotation in degrees.
    Rotation(f32),
    /// Legend item flow direction.
    Orientation(LegendOrientation),
    /// Positive legend column count.
    NumColumns(u32),
    /// Legend placement.
    Location(LegendLocation),
    /// Text or Legend interpreter.
    Interpreter(Interpreter),
    /// Surface or Patch face alpha.
    FaceAlpha(f32),
    /// Patch edge alpha.
    EdgeAlpha(f32),
}

/// MATLAB tiled-layout inter-tile spacing mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TileSpacing {
    /// R2022b default spacing.
    #[default]
    Loose,
    /// Reduced spacing.
    Compact,
    /// No explicit gap between tile rectangles.
    None,
}

/// MATLAB tiled-layout outer padding mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TilePadding {
    /// R2022b default outer padding.
    #[default]
    Loose,
    /// Reduced outer padding.
    Compact,
    /// Minimum outer padding.
    Tight,
}

/// One `nexttile` selection intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NextTileSelection {
    /// First unoccupied tile.
    Automatic,
    /// One explicit one-based tile number.
    Tile(u32),
    /// First free row/column span.
    Span { rows: u32, columns: u32 },
}

impl GraphicsPropertyUpdate {
    /// Returns the property selected by this assignment.
    #[must_use]
    pub const fn property(&self) -> GraphicsProperty {
        match self {
            Self::Position(_) => GraphicsProperty::Position,
            Self::Name(_) => GraphicsProperty::Name,
            Self::NumberTitle(_) => GraphicsProperty::NumberTitle,
            Self::LineWidth(_) => GraphicsProperty::LineWidth,
            Self::Color(_) | Self::AxesColor(_) => GraphicsProperty::Color,
            Self::LineStyle(_) => GraphicsProperty::LineStyle,
            Self::Marker(_) => GraphicsProperty::Marker,
            Self::MarkerSize(_) => GraphicsProperty::MarkerSize,
            Self::MarkerIndices { .. } => GraphicsProperty::MarkerIndices,
            Self::DisplayName(_) => GraphicsProperty::DisplayName,
            Self::Clipping(_) => GraphicsProperty::Clipping,
            Self::Visible(_) => GraphicsProperty::Visible,
            Self::SizeData(_) => GraphicsProperty::SizeData,
            Self::CData(_) => GraphicsProperty::CData,
            Self::MarkerFaceColor(_) | Self::ChartMarkerFaceColor(_) => {
                GraphicsProperty::MarkerFaceColor
            }
            Self::MarkerEdgeColor(_) | Self::ChartMarkerEdgeColor(_) => {
                GraphicsProperty::MarkerEdgeColor
            }
            Self::BaseValue(_) => GraphicsProperty::BaseValue,
            Self::BarWidth(_) => GraphicsProperty::BarWidth,
            Self::CapSize(_) => GraphicsProperty::CapSize,
            Self::ColorOrder(_) => GraphicsProperty::ColorOrder,
            Self::LineStyleOrder(_) => GraphicsProperty::LineStyleOrder,
            Self::ColorOrderIndex(_) => GraphicsProperty::ColorOrderIndex,
            Self::XLim(_) => GraphicsProperty::XLim,
            Self::YLim(_) => GraphicsProperty::YLim,
            Self::ZLim(_) => GraphicsProperty::ZLim,
            Self::XLimMode(_) => GraphicsProperty::XLimMode,
            Self::YLimMode(_) => GraphicsProperty::YLimMode,
            Self::ZLimMode(_) => GraphicsProperty::ZLimMode,
            Self::XScale(_) => GraphicsProperty::XScale,
            Self::YScale(_) => GraphicsProperty::YScale,
            Self::ZScale(_) => GraphicsProperty::ZScale,
            Self::XLimitMethod(_) => GraphicsProperty::XLimitMethod,
            Self::YLimitMethod(_) => GraphicsProperty::YLimitMethod,
            Self::ZLimitMethod(_) => GraphicsProperty::ZLimitMethod,
            Self::XDir(_) => GraphicsProperty::XDir,
            Self::YDir(_) => GraphicsProperty::YDir,
            Self::ZDir(_) => GraphicsProperty::ZDir,
            Self::XTick(_) => GraphicsProperty::XTick,
            Self::YTick(_) => GraphicsProperty::YTick,
            Self::ZTick(_) => GraphicsProperty::ZTick,
            Self::XTickMode(_) => GraphicsProperty::XTickMode,
            Self::YTickMode(_) => GraphicsProperty::YTickMode,
            Self::ZTickMode(_) => GraphicsProperty::ZTickMode,
            Self::XTickLabel(_) => GraphicsProperty::XTickLabel,
            Self::YTickLabel(_) => GraphicsProperty::YTickLabel,
            Self::ZTickLabel(_) => GraphicsProperty::ZTickLabel,
            Self::XTickLabelMode(_) => GraphicsProperty::XTickLabelMode,
            Self::YTickLabelMode(_) => GraphicsProperty::YTickLabelMode,
            Self::ZTickLabelMode(_) => GraphicsProperty::ZTickLabelMode,
            Self::Ticks(_) => GraphicsProperty::Ticks,
            Self::TicksMode(_) => GraphicsProperty::TicksMode,
            Self::TickLabels(_) => GraphicsProperty::TickLabels,
            Self::TickLabelsMode(_) => GraphicsProperty::TickLabelsMode,
            Self::TickLabelInterpreter(_) => GraphicsProperty::TickLabelInterpreter,
            Self::Box(_) => GraphicsProperty::Box,
            Self::XMinorGrid(_) => GraphicsProperty::XMinorGrid,
            Self::YMinorGrid(_) => GraphicsProperty::YMinorGrid,
            Self::ZMinorGrid(_) => GraphicsProperty::ZMinorGrid,
            Self::XGrid(_) => GraphicsProperty::XGrid,
            Self::YGrid(_) => GraphicsProperty::YGrid,
            Self::ZGrid(_) => GraphicsProperty::ZGrid,
            Self::TickDir(_) => GraphicsProperty::TickDir,
            Self::ThetaAxisUnits(_) => GraphicsProperty::ThetaAxisUnits,
            Self::ThetaDir(_) => GraphicsProperty::ThetaDir,
            Self::ThetaZeroLocation(_) => GraphicsProperty::ThetaZeroLocation,
            Self::RAxisLocation(_) => GraphicsProperty::RAxisLocation,
            Self::FaceColor(_) | Self::ChartFaceColor(_) => GraphicsProperty::FaceColor,
            Self::EdgeColor(_) | Self::ChartEdgeColor(_) => GraphicsProperty::EdgeColor,
            Self::CDataMapping(_) => GraphicsProperty::CDataMapping,
            Self::XData(_) => GraphicsProperty::XData,
            Self::YData(_) => GraphicsProperty::YData,
            Self::ZData(_) => GraphicsProperty::ZData,
            Self::Projection(_) => GraphicsProperty::Projection,
            Self::DataAspectRatio(_) => GraphicsProperty::DataAspectRatio,
            Self::DataAspectRatioMode(_) => GraphicsProperty::DataAspectRatioMode,
            Self::PlotBoxAspectRatio(_) => GraphicsProperty::PlotBoxAspectRatio,
            Self::PlotBoxAspectRatioMode(_) => GraphicsProperty::PlotBoxAspectRatioMode,
            Self::CLim(_) => GraphicsProperty::CLim,
            Self::CLimMode(_) => GraphicsProperty::CLimMode,
            Self::FontSize(_) => GraphicsProperty::FontSize,
            Self::FontName(_) => GraphicsProperty::FontName,
            Self::FontWeight(_) => GraphicsProperty::FontWeight,
            Self::Rotation(_) => GraphicsProperty::Rotation,
            Self::Orientation(_) => GraphicsProperty::Orientation,
            Self::NumColumns(_) => GraphicsProperty::NumColumns,
            Self::Location(_) => GraphicsProperty::Location,
            Self::Interpreter(_) => GraphicsProperty::Interpreter,
            Self::FaceAlpha(_) => GraphicsProperty::FaceAlpha,
            Self::EdgeAlpha(_) => GraphicsProperty::EdgeAlpha,
        }
    }
}

/// Strongly typed value returned by a graphics property query.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsPropertyValue {
    /// Real scalar value.
    Scalar(f64),
    /// Unpremultiplied sRGB color.
    Color([f32; 4]),
    /// Line stroke pattern.
    LineStyle(LineStyle),
    /// Marker shape.
    Marker(Marker),
    /// Visibility state.
    Visible(bool),
    /// One graphics handle.
    Handle(GraphicsHandle),
    /// Stable graphics handles, possibly heterogeneous.
    Handles(Vec<GraphicsHandle>),
    /// Real matrix in column-major order.
    NumericMatrix {
        /// MATLAB-visible two-dimensional shape.
        shape: [u64; 2],
        /// Column-major values.
        values: Vec<f64>,
    },
    /// Exact UTF-16 char row.
    Text(Vec<u16>),
    /// Exact UTF-16 cell row.
    TextList(Vec<Vec<u16>>),
    /// Exact UTF-16 cell column.
    TextColumn(Vec<Vec<u16>>),
    /// Exact marker-index row with its MATLAB-visible numeric class.
    MarkerIndices {
        /// Positive one-based index values.
        values: Vec<u64>,
        /// MATLAB-visible numeric storage class.
        value_class: MarkerIndicesClass,
    },
    /// Axes limit mode.
    LimitMode(LimitMode),
    /// Axes tick or tick-label mode.
    TickMode(TickMode),
    /// Text interpreter intent.
    Interpreter(Interpreter),
    /// Legend placement.
    LegendLocation(LegendLocation),
    /// Surface color mode.
    SurfaceColor(SurfaceColor),
    /// Surface `CData` mapping mode.
    CDataMapping(CDataMapping),
    /// Axes projection mode.
    Projection(ProjectionMode),
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HorizontalAlignment {
    /// Left aligned.
    Left,
    /// Center aligned.
    Center,
    /// Right aligned.
    Right,
}

/// Vertical text alignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalAlignment {
    /// Top aligned.
    Top,
    /// Middle aligned.
    Middle,
    /// Baseline aligned.
    Baseline,
    /// Bottom aligned.
    Bottom,
}

/// Font style intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontStyle {
    /// Normal font style.
    Normal,
    /// Italic font style.
    Italic,
}

/// First-tranche legend location.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegendLocation {
    /// Automatic best location.
    Best,
    /// Upper-right interior location and MATLAB R2022b default.
    Northeast,
    /// Upper-left interior location.
    Northwest,
    /// Centered below the Axes plot box.
    SouthOutside,
}

/// MATLAB legend item flow direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LegendOrientation {
    /// Items flow from top to bottom.
    #[default]
    Vertical,
    /// Items flow from left to right.
    Horizontal,
}

/// MATLAB Axes tick-mark direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TickDirection {
    /// Tick marks extend into the plot box.
    #[default]
    In,
    /// Tick marks extend away from the plot box.
    Out,
    /// Tick marks extend on both sides of the ruler.
    Both,
}

/// Figure HIR properties.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureProperties {
    /// MATLAB figure number.
    pub number: u32,
    /// Exact UTF-16 name.
    pub name_utf16: Vec<u16>,
    /// Whether the MATLAB Figure number prefixes the title-bar name.
    pub number_title: bool,
    /// Visibility.
    pub visible: bool,
    /// Unpremultiplied sRGB color.
    pub background_rgba: [f32; 4],
    /// Logical `[left, bottom, width, height]` rectangle in CSS pixels.
    pub position_css_pixels: [f64; 4],
    /// Figure-level `NextPlot`, retained for later `newplot` compatibility.
    pub next_plot: NextPlot,
}

/// Non-rendered controller state for one MATLAB-style tiled chart layout.
///
/// Axes remain direct Figure children in the wire HIR; this object owns the
/// language-visible layout handle and deterministic tile allocation state.
#[derive(Clone, Debug, PartialEq)]
pub struct TiledChartLayoutProperties {
    /// Owning Figure.
    pub parent: GraphicsHandle,
    /// Number of tile rows.
    pub rows: u32,
    /// Number of tile columns.
    pub columns: u32,
    /// Inter-tile spacing mode.
    pub tile_spacing: TileSpacing,
    /// Outer padding mode.
    pub padding: TilePadding,
    /// One Axes handle per occupied tile, repeated for spanning Axes.
    pub tile_axes: Vec<Option<GraphicsHandle>>,
}

/// Properties accepted while constructing a fresh auto-numbered Figure.
///
/// The first slice deliberately contains only values that the retained Figure
/// HIR and every current transport can represent without loss.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureCreationProperties {
    /// Exact UTF-16 Figure name.
    pub name_utf16: Vec<u16>,
    /// Whether the Figure number prefixes the title-bar name.
    pub number_title: bool,
    /// Visibility at creation time.
    pub visible: bool,
    /// Unpremultiplied sRGB background color.
    pub background_rgba: [f32; 4],
    /// Logical `[left, bottom, width, height]` rectangle in CSS pixels.
    pub position_css_pixels: [f64; 4],
}

impl Default for FigureCreationProperties {
    fn default() -> Self {
        Self {
            name_utf16: Vec::new(),
            number_title: true,
            visible: true,
            background_rgba: [0.94, 0.94, 0.94, 1.0],
            position_css_pixels: [100.0, 100.0, 560.0, 420.0],
        }
    }
}

/// Axes HIR properties.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Axes2DProperties {
    /// Cartesian or polar coordinate-system identity.
    pub coordinate_system: AxesCoordinateSystem,
    /// Normalized position.
    pub position_normalized: [f64; 4],
    /// Opaque Axes background, or `None` for MATLAB `Color='none'`.
    pub background_rgba: Option<[f32; 4]>,
    /// Polar theta limit/tick display units.
    pub theta_axis_units: ThetaAxisUnits,
    /// Polar theta increasing direction.
    pub theta_direction: ThetaDirection,
    /// Polar zero-angle screen location.
    pub theta_zero_location: ThetaZeroLocation,
    /// Polar radial ruler location in the active theta units.
    pub r_axis_location: f64,
    /// Resolved, finite, increasing X limits.
    pub x_limits: [f64; 2],
    /// Resolved, finite, increasing Y limits.
    pub y_limits: [f64; 2],
    /// Resolved, finite, increasing Z limits.
    pub z_limits: [f64; 2],
    /// X limit mode.
    pub x_limit_mode: LimitMode,
    /// Y limit mode.
    pub y_limit_mode: LimitMode,
    /// Z limit mode.
    pub z_limit_mode: LimitMode,
    /// X scale.
    pub x_scale: AxisScale,
    /// Y scale.
    pub y_scale: AxisScale,
    /// Z scale.
    pub z_scale: AxisScale,
    /// X automatic-limit method.
    pub x_limit_method: LimitMethod,
    /// Y automatic-limit method.
    pub y_limit_method: LimitMethod,
    /// Z automatic-limit method.
    pub z_limit_method: LimitMethod,
    /// X direction.
    pub x_direction: AxisDirection,
    /// Y direction.
    pub y_direction: AxisDirection,
    /// Z direction.
    pub z_direction: AxisDirection,
    /// Resolved X tick positions.
    pub x_ticks: Vec<f64>,
    /// Resolved Y tick positions.
    pub y_ticks: Vec<f64>,
    /// Resolved Z tick positions.
    pub z_ticks: Vec<f64>,
    /// X tick-position mode.
    pub x_tick_mode: TickMode,
    /// Y tick-position mode.
    pub y_tick_mode: TickMode,
    /// Z tick-position mode.
    pub z_tick_mode: TickMode,
    /// Resolved or explicit X tick labels as exact UTF-16.
    pub x_tick_labels_utf16: Vec<Vec<u16>>,
    /// Resolved or explicit Y tick labels as exact UTF-16.
    pub y_tick_labels_utf16: Vec<Vec<u16>>,
    /// Resolved or explicit Z tick labels as exact UTF-16.
    pub z_tick_labels_utf16: Vec<Vec<u16>>,
    /// X tick-label mode.
    pub x_tick_label_mode: TickMode,
    /// Y tick-label mode.
    pub y_tick_label_mode: TickMode,
    /// Z tick-label mode.
    pub z_tick_label_mode: TickMode,
    /// Tick-label interpreter intent.
    pub tick_label_interpreter: Interpreter,
    /// MATLAB Axes `FontSize`, in typographic points.
    pub font_size_points: f32,
    /// Exact UTF-16 Axes/tick-label font family.
    pub font_family_utf16: Vec<u16>,
    /// Tick-mark direction.
    pub tick_direction: TickDirection,
    /// MATLAB Axes `LineWidth`, in typographic points.
    pub line_width_points: f32,
    /// Whether all four Axes outline edges are enabled.
    pub box_enabled: bool,
    /// Whether Axes chrome and labels are visible.
    pub visible: bool,
    /// Hold state.
    pub next_plot: NextPlot,
    /// X grid state.
    pub grid_x: bool,
    /// Y grid state.
    pub grid_y: bool,
    /// Z grid state.
    pub grid_z: bool,
    /// X minor-grid state.
    pub minor_grid_x: bool,
    /// Y minor-grid state.
    pub minor_grid_y: bool,
    /// Z minor-grid state.
    pub minor_grid_z: bool,
    /// MATLAB azimuth in degrees.
    pub view_azimuth_degrees: f64,
    /// MATLAB elevation in degrees.
    pub view_elevation_degrees: f64,
    /// Projection mode.
    pub projection: ProjectionMode,
    /// Relative scale of one X/Y/Z data unit.
    pub data_aspect_ratio: [f64; 3],
    /// Whether the data aspect ratio follows automatic Axes layout.
    pub data_aspect_ratio_mode: LimitMode,
    /// Relative X/Y/Z extent of the rendered plot box.
    pub plot_box_aspect_ratio: [f64; 3],
    /// Whether the plot-box aspect ratio follows automatic Axes layout.
    pub plot_box_aspect_ratio_mode: LimitMode,
    /// Renderer camera scale relative to the MATLAB-visible default view.
    /// This is retained so browser zoom survives unrelated graphics deltas.
    pub camera_scale: f64,
    /// Whether a camera-following light exists for this Axes.
    pub headlight_enabled: bool,
    /// Lighting mode applied to current and subsequently created surfaces.
    pub lighting_mode: LightingMode,
    /// Resolved color limits.
    pub c_limits: [f64; 2],
    /// Color-limit mode.
    pub c_limit_mode: LimitMode,
    /// Active colormap as ordered sRGB-encoded RGB rows.
    pub colormap: Vec<[f64; 3]>,
    /// Whether an east-outside colorbar is attached to this Axes.
    pub colorbar_visible: bool,
    /// One-based color order index.
    pub color_order_index: u32,
    /// Ordered line colors used by automatic styling.
    pub color_order: Vec<[f32; 4]>,
    /// Ordered line styles used by automatic styling.
    pub line_style_order: Vec<LineStyle>,
    /// Current title object.
    pub title: Option<GraphicsHandle>,
    /// Current X-label object.
    pub x_label: Option<GraphicsHandle>,
    /// Current Y-label object.
    pub y_label: Option<GraphicsHandle>,
    /// Current Z-label object.
    pub z_label: Option<GraphicsHandle>,
}

/// Line HIR properties.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct LineSeriesProperties {
    /// Immutable X resource.
    pub x_data: DataRef,
    /// Immutable Y resource.
    pub y_data: DataRef,
    /// Optional immutable Z resource. Present for a three-dimensional Line.
    pub z_data: Option<DataRef>,
    /// Default line color.
    pub color_rgba: [f32; 4],
    /// MATLAB `LineWidth` value in typographic points.
    pub line_width_points: f32,
    /// Stroke pattern. Basic `plot` produces `Solid`.
    pub line_style: LineStyle,
    /// Marker shape. Basic `plot` produces `None`.
    pub marker: Marker,
    /// MATLAB `MarkerSize` value in typographic points.
    pub marker_size_points: f32,
    /// Resolved marker fill color. Transparent is MATLAB `none`.
    pub marker_face_color_rgba: [f32; 4],
    /// Resolved marker edge color.
    pub marker_edge_color_rgba: [f32; 4],
    /// Whether marker edge color follows the Line `Color`.
    pub marker_edge_color_automatic: bool,
    /// One-based original source-point indices used for marker placement.
    pub marker_indices: Vec<u64>,
    /// MATLAB-visible numeric storage class for `MarkerIndices` queries.
    pub marker_indices_class: MarkerIndicesClass,
    /// Whether data-length changes still regenerate the default `1..=N` indices.
    pub marker_indices_automatic: bool,
    /// Exact UTF-16 display name.
    pub display_name_utf16: Vec<u16>,
    /// Visibility.
    pub visible: bool,
    /// Whether axes clipping applies.
    pub clipping: bool,
}

/// Scatter HIR properties.
#[derive(Clone, Debug, PartialEq)]
pub struct ScatterSeriesProperties {
    /// Immutable X resource.
    pub x_data: DataRef,
    /// Immutable Y resource.
    pub y_data: DataRef,
    /// Optional immutable Z resource. Present for a three-dimensional Scatter.
    pub z_data: Option<DataRef>,
    /// Optional per-point marker sizes; absent in the basic API.
    pub size_data: Option<DataRef>,
    /// Optional per-point marker colors; absent in the basic API.
    pub color_data: Option<DataRef>,
    /// Component colored by scalar `color_data` through Axes `CLim`.
    pub color_data_target: ScatterColorTarget,
    /// Diameter derived from MATLAB scatter `SizeData`, in points.
    pub marker_size_points: f32,
    /// Marker edge width in typographic points.
    pub line_width_points: f32,
    /// Marker face color.
    pub face_color_rgba: [f32; 4],
    /// Marker edge color.
    pub edge_color_rgba: [f32; 4],
    /// Marker shape. Basic `scatter` produces `Circle`.
    pub marker: Marker,
    /// Exact UTF-16 display name.
    pub display_name_utf16: Vec<u16>,
    /// Visibility.
    pub visible: bool,
    /// Whether axes clipping applies.
    pub clipping: bool,
}

/// Scatter component resolved from scalar `CData` and Axes `CLim`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScatterColorTarget {
    /// No scalar color-data mapping is active.
    None,
    /// Marker faces use mapped scalar colors.
    Face,
    /// Marker edges use mapped scalar colors.
    Edge,
}

/// Structured Surface HIR properties.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSeriesProperties {
    /// X coordinates, shaped `1 x columns` or like `ZData`.
    pub x_data: DataRef,
    /// Y coordinates, shaped `rows x 1` or like `ZData`.
    pub y_data: DataRef,
    /// Z matrix in MATLAB column-major order.
    pub z_data: DataRef,
    /// `CData` matrix in MATLAB column-major order.
    pub c_data: DataRef,
    /// Face color intent.
    pub face_color: SurfaceColor,
    /// Edge color intent.
    pub edge_color: SurfaceColor,
    /// Edge width in typographic points.
    pub line_width_points: f32,
    /// Edge stroke pattern.
    pub line_style: LineStyle,
    /// `CData` interpretation.
    pub c_data_mapping: CDataMapping,
    /// Opaque first-tranche face alpha.
    pub face_alpha: f32,
    /// Whether camera-following diffuse lighting is active.
    pub lighting_enabled: bool,
    /// Visibility.
    pub visible: bool,
    /// Whether axes clipping applies.
    pub clipping: bool,
}

/// Irregular Patch HIR properties.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // These are independent MATLAB properties, not one state machine.
pub struct PatchSeriesProperties {
    /// Face connectivity matrix, one face per row with optional NaN padding.
    pub faces: DataRef,
    /// Vertex coordinate matrix with two or three columns.
    pub vertices: DataRef,
    /// Optional scalar face/vertex color values.
    pub face_vertex_cdata: DataRef,
    /// Optional MATLAB `VertexNormals`, one data-space vector per vertex.
    pub vertex_normals: Option<DataRef>,
    /// Face color intent.
    pub face_color: SurfaceColor,
    /// Edge color intent.
    pub edge_color: SurfaceColor,
    /// Edge width in typographic points.
    pub line_width_points: f32,
    /// Edge stroke pattern.
    pub line_style: LineStyle,
    /// Color-data interpretation.
    pub c_data_mapping: CDataMapping,
    /// Scalar face alpha.
    pub face_alpha: f32,
    /// Scalar edge alpha.
    pub edge_alpha: f32,
    /// Whether camera-following diffuse lighting is active.
    pub lighting_enabled: bool,
    /// Whether adjacent triangle normals are averaged per source vertex.
    pub smooth_normals: bool,
    /// Visibility.
    pub visible: bool,
    /// Whether Axes clipping applies.
    pub clipping: bool,
}

/// Text HIR properties. UTF-16 code units are the exact semantic truth.
#[derive(Clone, Debug, PartialEq)]
pub struct TextProperties {
    /// Exact UTF-16 text, including unpaired code units.
    pub code_units: Vec<u16>,
    /// Semantic role.
    pub role: TextRole,
    /// Normalized anchor.
    pub anchor_normalized: [f64; 2],
    /// Horizontal alignment.
    pub horizontal_alignment: HorizontalAlignment,
    /// Vertical alignment.
    pub vertical_alignment: VerticalAlignment,
    /// Unpremultiplied sRGB text color.
    pub color_rgba: [f32; 4],
    /// Exact UTF-16 font family.
    pub font_family_utf16: Vec<u16>,
    /// Font size in CSS pixels.
    pub font_size_css_px: f32,
    /// CSS-like font weight in `1..=1000`.
    pub font_weight: u16,
    /// Font style.
    pub font_style: FontStyle,
    /// Counter-clockwise rotation in degrees.
    pub rotation_degrees: f32,
    /// Text interpreter intent; glyph layout is a downstream concern.
    pub interpreter: Interpreter,
    /// Visibility.
    pub visible: bool,
}

/// Legend HIR properties.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendProperties {
    /// Ordered series membership.
    pub series: Vec<GraphicsHandle>,
    /// Exact UTF-16 labels.
    pub labels_utf16: Vec<Vec<u16>>,
    /// Location intent.
    pub location: LegendLocation,
    /// Visibility.
    pub visible: bool,
    /// Legend background.
    pub background_rgba: [f32; 4],
    /// Legend border.
    pub border_rgba: [f32; 4],
    /// Exact UTF-16 font family.
    pub font_family_utf16: Vec<u16>,
    /// Font size in CSS pixels.
    pub font_size_css_px: f32,
    /// Font weight in `1..=1000`.
    pub font_weight: u16,
    /// Legend label interpreter intent; glyph layout is downstream.
    pub interpreter: Interpreter,
    /// Item flow direction.
    pub orientation: LegendOrientation,
    /// Positive explicit column count.
    pub num_columns: u32,
    /// Whether the legend border is visible.
    pub box_enabled: bool,
}

/// Colorbar HIR properties. Color limits and colormap remain owned by its Axes.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBarProperties {
    /// Visibility of the east-outside colorbar.
    pub visible: bool,
    /// Resolved tick values for language-level property queries.
    pub ticks: Vec<f64>,
    /// Automatic or manually selected tick positions.
    pub tick_mode: TickMode,
    /// Exact tick labels as UTF-16 code-unit rows.
    pub tick_labels_utf16: Vec<Vec<u16>>,
    /// Automatic or manually selected tick labels.
    pub tick_label_mode: TickMode,
}

/// MATLAB chart color property retaining automatic and disabled intent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartColor {
    /// Resolve from the owning Axes color order.
    Auto,
    /// Do not paint this component.
    None,
    /// Explicit unpremultiplied sRGB color.
    Rgba([f32; 4]),
}

/// Non-rendered MATLAB-visible chart container properties.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartGroupProperties {
    /// Public chart type.
    pub kind: ChartGroupKind,
    /// Visibility inherited by retained primitive children.
    pub visible: bool,
    /// Line-chart color, absent for patch-backed charts.
    pub color: Option<ChartColor>,
    /// Common chart outline/stroke width in typographic points.
    pub line_width_points: f32,
    /// Common chart outline/stroke style.
    pub line_style: LineStyle,
    /// Line-chart marker, absent for patch-backed charts.
    pub marker: Option<Marker>,
    /// Line-chart marker size in typographic points.
    pub marker_size_points: Option<f32>,
    /// Line-chart marker face color intent.
    pub marker_face_color: Option<ChartColor>,
    /// Line-chart marker edge color intent.
    pub marker_edge_color: Option<ChartColor>,
    /// Patch-chart face color intent.
    pub face_color: Option<ChartColor>,
    /// Patch-chart edge color intent.
    pub edge_color: Option<ChartColor>,
    /// Resolved renderer color restored by patch `FaceColor='auto'`.
    pub automatic_face_color_rgba: Option<[f32; 4]>,
    /// Resolved renderer color restored by patch `EdgeColor='auto'`.
    pub automatic_edge_color_rgba: Option<[f32; 4]>,
    /// Patch-chart face alpha.
    pub face_alpha: Option<f32>,
    /// Patch-chart edge alpha.
    pub edge_alpha: Option<f32>,
    /// Baseline for Stem, Area, and Bar charts.
    pub base_value: Option<f64>,
    /// Relative width for Bar charts.
    pub bar_width: Option<f64>,
    /// `ErrorBar` cap width in typographic points.
    pub cap_size_points: Option<f64>,
}

/// Strongly typed graphics object payload.
#[allow(clippy::large_enum_variant)] // Public by-value graphics ABI keeps snapshots simple and stable.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsObject {
    /// Figure root.
    Figure(FigureProperties),
    /// 2D axes.
    Axes2D(Axes2DProperties),
    /// Non-rendered tiled chart layout controller.
    TiledChartLayout(TiledChartLayoutProperties),
    /// Line series.
    LineSeries(LineSeriesProperties),
    /// Scatter series.
    ScatterSeries(ScatterSeriesProperties),
    /// Structured surface series.
    SurfaceSeries(SurfaceSeriesProperties),
    /// Irregular polygon mesh.
    PatchSeries(PatchSeriesProperties),
    /// MATLAB-visible chart backed by retained primitive children.
    ChartGroup(ChartGroupProperties),
    /// Text.
    Text(TextProperties),
    /// Legend.
    Legend(LegendProperties),
    /// Colorbar.
    ColorBar(ColorBarProperties),
}

impl GraphicsObject {
    /// Returns the object's public class.
    #[must_use]
    pub const fn class(&self) -> GraphicsClass {
        match self {
            Self::Figure(_) => GraphicsClass::Figure,
            Self::Axes2D(properties) => properties.coordinate_system.graphics_class(),
            Self::TiledChartLayout(_) => GraphicsClass::TiledChartLayout,
            Self::LineSeries(_) => GraphicsClass::LineSeries,
            Self::ScatterSeries(_) => GraphicsClass::ScatterSeries,
            Self::SurfaceSeries(_) => GraphicsClass::SurfaceSeries,
            Self::PatchSeries(_) => GraphicsClass::PatchSeries,
            Self::ChartGroup(properties) => properties.kind.graphics_class(),
            Self::Text(_) => GraphicsClass::Text,
            Self::Legend(_) => GraphicsClass::Legend,
            Self::ColorBar(_) => GraphicsClass::ColorBar,
        }
    }
}

/// Strong HIR property payload used by snapshots and deltas.
#[allow(clippy::large_enum_variant)] // Mirrors GraphicsObject without protocol-visible boxing.
#[derive(Clone, Debug, PartialEq)]
pub enum HirProperties {
    /// Figure properties.
    Figure(FigureProperties),
    /// Axes properties.
    Axes2D(Axes2DProperties),
    /// Detached tiled layout controller; never emitted in Figure HIR.
    TiledChartLayout(TiledChartLayoutProperties),
    /// Line properties.
    LineSeries(LineSeriesProperties),
    /// Scatter properties.
    ScatterSeries(ScatterSeriesProperties),
    /// Surface properties.
    SurfaceSeries(SurfaceSeriesProperties),
    /// Patch properties.
    PatchSeries(PatchSeriesProperties),
    /// High-level chart-container properties.
    ChartGroup(ChartGroupProperties),
    /// Text properties.
    Text(TextProperties),
    /// Legend properties.
    Legend(LegendProperties),
    /// Colorbar properties.
    ColorBar(ColorBarProperties),
}

impl From<&GraphicsObject> for HirProperties {
    fn from(value: &GraphicsObject) -> Self {
        match value {
            GraphicsObject::Figure(properties) => Self::Figure(properties.clone()),
            GraphicsObject::Axes2D(properties) => Self::Axes2D(properties.clone()),
            GraphicsObject::TiledChartLayout(properties) => {
                Self::TiledChartLayout(properties.clone())
            }
            GraphicsObject::LineSeries(properties) => Self::LineSeries(properties.clone()),
            GraphicsObject::ScatterSeries(properties) => Self::ScatterSeries(properties.clone()),
            GraphicsObject::SurfaceSeries(properties) => Self::SurfaceSeries(properties.clone()),
            GraphicsObject::PatchSeries(properties) => Self::PatchSeries(properties.clone()),
            GraphicsObject::ChartGroup(properties) => Self::ChartGroup(properties.clone()),
            GraphicsObject::Text(properties) => Self::Text(properties.clone()),
            GraphicsObject::Legend(properties) => Self::Legend(properties.clone()),
            GraphicsObject::ColorBar(properties) => Self::ColorBar(properties.clone()),
        }
    }
}

/// One stable HIR object record.
#[derive(Clone, Debug, PartialEq)]
pub struct HirObject {
    /// Session-opaque public identifier.
    pub id: String,
    /// Arena generation.
    pub generation: u32,
    /// Last object revision.
    pub object_revision: u64,
    /// Public class.
    pub class: GraphicsClass,
    /// Parent identifier, absent for a Figure.
    pub parent_id: Option<String>,
    /// Stable display-order child identifiers.
    pub children: Vec<String>,
    /// Strong typed properties.
    pub properties: HirProperties,
}

/// A complete Figure HIR snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureSnapshot {
    /// Opaque Figure identifier.
    pub figure_id: String,
    /// Figure revision.
    pub revision: u64,
    /// Figure root identifier.
    pub root_id: String,
    /// Parent-before-child stable traversal.
    pub objects: Vec<HirObject>,
    /// Immutable resources referenced by the snapshot.
    pub referenced_data: Vec<DataResource>,
}

/// One first-tranche delta operation.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsDeltaOperation {
    /// Complete object upsert.
    UpsertObject(Box<HirObject>),
    /// Recursive object deletion.
    DeleteObject { id: String, generation: u32 },
    /// Stable child-order replacement.
    ReorderChildren {
        /// Parent identifier.
        parent_id: String,
        /// Complete child order.
        children: Vec<String>,
    },
    /// Root assignment for a new Figure.
    SetRoot { root_id: String },
}

/// An ordered atomic Figure delta.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsDelta {
    /// Opaque Figure identifier.
    pub figure_id: String,
    /// Exact base revision.
    pub base_revision: u64,
    /// Committed revision.
    pub revision: u64,
    /// Ordered operations.
    pub operations: Vec<GraphicsDeltaOperation>,
    /// Newly referenced immutable resources.
    pub added_data: Vec<DataRef>,
    /// Resources whose live Figure reference count reached zero.
    pub released_data_ids: Vec<DataResourceId>,
}

/// Internal notice kind. Server attachment discovery is deliberately separate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsNoticeKind {
    /// Figure was created or changed.
    Updated,
    /// Figure and descendants were closed.
    Closed,
}

/// A small runtime-internal graphics notification without numerical data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphicsNotice {
    /// Opaque Figure identifier.
    pub figure_id: String,
    /// Committed Figure revision.
    pub revision: u64,
    /// Change kind.
    pub kind: GraphicsNoticeKind,
    /// True only when the Server should publish first discovery for a Figure.
    pub discovery: bool,
}

/// Requests accepted by the narrow runtime graphics service.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsRequest {
    /// Create a fresh auto-numbered Figure, or select/create an explicit number.
    Figure { number: Option<u32> },
    /// Create a fresh auto-numbered Figure with constructor properties.
    CreateFigure {
        /// Fully prepared Figure properties.
        properties: FigureCreationProperties,
    },
    /// Return/create the current Figure.
    CurrentFigure,
    /// Return the current Figure without creating one; an empty handle vector means none.
    CurrentFigureOrEmpty,
    /// Create a new Axes, or select an existing one.
    Axes { select: Option<GraphicsHandle> },
    /// Create a new Axes in an explicit or current Figure with constructor properties.
    CreateAxes {
        /// Optional parent Figure.
        figure: Option<GraphicsHandle>,
        /// Fully prepared Axes properties.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create a new `PolarAxes` in an explicit or current Figure.
    CreatePolarAxes {
        /// Optional parent Figure.
        figure: Option<GraphicsHandle>,
        /// Fully prepared `PolarAxes` properties.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Select or create an Axes occupying one subplot rectangle.
    Subplot {
        /// Optional parent Figure.
        figure: Option<GraphicsHandle>,
        /// Normalized subplot rectangle.
        position: [f64; 4],
    },
    /// Replace the current Figure contents with a tiled chart layout.
    CreateTiledLayout {
        /// Optional explicit parent Figure.
        figure: Option<GraphicsHandle>,
        /// Positive row count.
        rows: u32,
        /// Positive column count.
        columns: u32,
        /// Inter-tile spacing mode.
        tile_spacing: TileSpacing,
        /// Outer padding mode.
        padding: TilePadding,
    },
    /// Select or create one Axes in the current/explicit tiled layout.
    NextTile {
        /// Optional explicit layout handle.
        layout: Option<GraphicsHandle>,
        /// Tile number, span, or automatic placement.
        selection: NextTileSelection,
    },
    /// Return/create the current Axes.
    CurrentAxes,
    /// Create zero or more Line children in one transaction.
    Plot { lines: Vec<LineInput> },
    /// Create zero or more styled Line children in one transaction.
    PlotStyled {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Fully prepared data series.
        lines: Vec<LineInput>,
        /// Assignments applied to every created Line, in order.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create Lines with independent per-series styles in one transaction.
    PlotSeries {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Fully prepared data and per-series assignments.
        series: Vec<StyledLineInput>,
        /// Assignments applied to every series after its `LineSpec`.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create polar Line children, replacing an incompatible Cartesian Axes.
    PolarPlotSeries {
        /// Optional target `PolarAxes`.
        axes: Option<GraphicsHandle>,
        /// Theta/radius data and per-series assignments.
        series: Vec<StyledLineInput>,
        /// Assignments applied to every series after its `LineSpec`.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create styled Lines while atomically updating their target Axes.
    PlotWithAxesProperties {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Fully prepared data series.
        lines: Vec<LineInput>,
        /// Assignments applied to every created Line, in order.
        properties: Vec<GraphicsPropertyUpdate>,
        /// Assignments applied to the target Axes in the same Figure revision.
        axes_properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create one Scatter child.
    Scatter {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Prepared numerical series.
        series: ScatterInput,
        /// Assignments applied before creation.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create one structured Surface child.
    Surface {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Prepared structured data.
        surface: SurfaceInput,
        /// MATLAB plotting style.
        style: SurfaceStyle,
        /// Assignments applied before creation.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create one irregular Patch child.
    Patch {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Prepared polygon topology, coordinates, and color data.
        patch: PatchInput,
        /// Assignments applied before creation.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create one MATLAB-visible high-level 2D chart with retained primitives.
    ChartGroup {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Fully prepared chart and child primitives.
        chart: ChartGroupInput,
    },
    /// Apply one MATLAB shading mode to every Surface in an Axes atomically.
    Shading {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Requested face/edge mode.
        mode: ShadingMode,
    },
    /// Create or remove the camera-following light for an Axes.
    SetHeadlight {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Whether the headlight exists.
        enabled: bool,
    },
    /// Set the surface-lighting mode for an Axes.
    SetLighting {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Requested lighting mode.
        mode: LightingMode,
    },
    /// Enable smooth per-vertex normals on one Patch.
    SetPatchSmoothNormals {
        /// Target Patch handle.
        patch: GraphicsHandle,
    },
    /// Compute MATLAB-compatible negative scalar-field gradients at Patch vertices.
    SetPatchIsoNormals {
        /// Target Patch handle.
        patch: GraphicsHandle,
        /// Scalar volume and optional meshgrid coordinates.
        volume: IsoNormalsInput,
    },
    /// Show or hide the current Axes east-outside colorbar.
    Colorbar {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Whether the colorbar is visible.
        visible: bool,
    },
    /// Replace the current or explicit Axes colormap atomically.
    SetColormap {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Ordered sRGB-encoded RGB rows.
        colors: Vec<[f64; 3]>,
    },
    /// Return the current or explicit Axes colormap.
    GetColormap {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
    },
    /// Set MATLAB azimuth/elevation on an explicit or current Axes.
    SetView {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Azimuth in degrees.
        azimuth_degrees: f64,
        /// Elevation in degrees.
        elevation_degrees: f64,
    },
    /// Atomically commit a browser 3D camera gesture for one Figure.
    SetAxesCamera {
        /// Figure that owns the target Axes.
        figure: GraphicsHandle,
        /// Explicit target Axes for multi-Axes clients; v2 callers omit it.
        axes: Option<GraphicsHandle>,
        /// MATLAB azimuth in degrees.
        azimuth_degrees: f64,
        /// MATLAB elevation in degrees.
        elevation_degrees: f64,
        /// Projection scale relative to the default camera.
        camera_scale: f64,
    },
    /// Set axes hold state.
    SetHold {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Add/replace state.
        enabled: bool,
    },
    /// Query axes hold state.
    IsHold { axes: Option<GraphicsHandle> },
    /// Return stable direct children for a graphics parent.
    GetChildren { parent: GraphicsHandle },
    /// Query exact live/stale state without rejecting stale handles.
    IsGraphics { handles: Vec<GraphicsHandle> },
    /// Atomically update one homogeneous Line or Scatter handle column.
    SetProperties {
        /// Nonempty targets must belong to one Figure and carry one class.
        handles: Vec<GraphicsHandle>,
        /// Assignments applied in order; later duplicates win.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Query one property from a homogeneous Line or Scatter handle column.
    GetProperties {
        /// Query targets in language-visible order.
        handles: Vec<GraphicsHandle>,
        /// Property to read from every target.
        property: GraphicsProperty,
    },
    /// Create or update title/X-label/Y-label.
    SetText {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Semantic text role.
        role: TextRole,
        /// Exact UTF-16 text.
        code_units: Vec<u16>,
        /// Assignments applied in the same transaction.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Create or update a Legend for current series.
    Legend {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Exact UTF-16 labels.
        labels_utf16: Vec<Vec<u16>>,
        /// Assignments applied in order after labels are resolved.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Atomically set properties on an explicit or current Axes.
    SetAxesProperties {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Assignments applied in order.
        properties: Vec<GraphicsPropertyUpdate>,
    },
    /// Query one property on an explicit or current Axes.
    GetAxesProperty {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Property to query.
        property: GraphicsProperty,
    },
    /// Toggle the current Axes outline state atomically.
    ToggleAxesBox {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
    },
    /// Set manual limits.
    SetLimits {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Axis selector.
        axis: Axis,
        /// Finite increasing limits.
        limits: [f64; 2],
    },
    /// Atomically set both manual limits on one Figure's target Axes.
    SetAxesLimits {
        /// Figure that owns the target Axes.
        figure: GraphicsHandle,
        /// Explicit target Axes for multi-Axes clients; v2 callers omit it.
        axes: Option<GraphicsHandle>,
        /// Finite, strictly increasing X limits.
        x_limits: [f64; 2],
        /// Finite, strictly increasing Y limits.
        y_limits: [f64; 2],
    },
    /// Query resolved limits.
    GetLimits {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Axis selector.
        axis: Axis,
    },
    /// Switch one target Axes limit mode.
    SetLimitMode {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Axis selector.
        axis: Axis,
        /// New limit mode.
        mode: LimitMode,
    },
    /// Set both axes grid state.
    SetGrid {
        /// Optional target Axes.
        axes: Option<GraphicsHandle>,
        /// Grid state.
        mode: GridMode,
    },
    /// Clear current Axes children while preserving the Axes.
    ClearAxes { axes: Option<GraphicsHandle> },
    /// Clear current Figure descendants while preserving the Figure.
    ClearFigure { figure: Option<GraphicsHandle> },
    /// Close a specified or current Figure.
    CloseFigure { figure: Option<GraphicsHandle> },
    /// Close every Figure visible to the retained graphics model atomically.
    CloseAllFigures,
}

/// Language-facing result of a graphics request.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsResponse {
    /// No language value.
    None,
    /// One graphics handle.
    Handle(GraphicsHandle),
    /// A homogeneous vector of graphics handles; the language boundary selects
    /// the MATLAB-compatible orientation.
    Handles(Vec<GraphicsHandle>),
    /// Logical result.
    Logical(bool),
    /// Logical results in request order.
    Logicals(Vec<bool>),
    /// Limit pair.
    Limits([f64; 2]),
    /// Ordered sRGB-encoded colormap rows.
    Colormap(Vec<[f64; 3]>),
    /// Property values in request handle order.
    PropertyValues(Vec<GraphicsPropertyValue>),
}

/// Result of one committed graphics builtin.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsExecution {
    /// Language-facing result.
    pub response: GraphicsResponse,
    /// Internal notification, absent for selection/query-only operations.
    pub notice: Option<GraphicsNotice>,
}

/// Stable graphics model error categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsErrorCategory {
    /// Request form is outside the first tranche.
    Unsupported,
    /// Handle is stale, unknown, or has the wrong class.
    InvalidHandle,
    /// Input shape or value domain is invalid.
    InvalidInput,
    /// A checked model/resource limit was exceeded.
    Limit,
    /// Session state is internally inconsistent.
    InvalidState,
}

/// A structured graphics operation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphicsError {
    /// Stable category.
    pub category: GraphicsErrorCategory,
    /// OpenMat-owned diagnostic detail.
    pub message: String,
}

impl GraphicsError {
    /// Creates an error.
    #[must_use]
    pub fn new(category: GraphicsErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_handle(message: impl Into<String>) -> Self {
        Self::new(GraphicsErrorCategory::InvalidHandle, message)
    }

    pub(crate) fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(GraphicsErrorCategory::InvalidInput, message)
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self::new(GraphicsErrorCategory::Limit, message)
    }

    pub(crate) fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(GraphicsErrorCategory::InvalidState, message)
    }
}

impl fmt::Display for GraphicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GraphicsError {}
