use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::limits::validate_positive_safe;
use crate::message::GraphicsProtocol;
use crate::{DataRef, ErrorCategory, GraphicsLimits, ValidationError};

/// Required nullable wire member. Unlike `Option<T>`, omitting the containing
/// field is a serde error; JSON `null` maps to `None`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Nullable<T>(pub Option<T>);

impl<T> Default for Nullable<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<'de, T> Deserialize<'de> for Nullable<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Self)
    }
}

impl<T> Nullable<T> {
    /// Returns the contained reference, if non-null.
    #[must_use]
    pub const fn as_ref(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl Nullable<String> {
    /// Returns the contained string slice, if non-null.
    #[must_use]
    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Nullable<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Nullable::deserialize(deserializer)
}

/// Unpremultiplied sRGB plus alpha.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Rgba(pub [f64; 4]);

impl Rgba {
    fn validate(self, field: &str) -> Result<(), ValidationError> {
        if self
            .0
            .iter()
            .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
        {
            Ok(())
        } else {
            Err(invalid_scene(format!(
                "{field} must contain four finite channels in [0,1]"
            )))
        }
    }
}

/// Common wire fields carried by every graphics object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectFields {
    /// Opaque object identifier.
    pub id: String,
    /// Positive generational-arena generation.
    pub generation: u64,
    /// Positive revision of this complete object property set.
    pub object_revision: u64,
    /// Parent identifier, null only for the Figure root.
    pub parent_id: Option<String>,
    /// Ordered child identifiers.
    pub children: Vec<String>,
}

/// Figure property schema. All fields are required; UTF-16 arrays preserve
/// exact JavaScript/MATLAB code units without a UTF-8 round trip.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureProperties {
    /// Positive MATLAB Figure number.
    pub number: u64,
    /// Exact title-bar name code units. Empty is a valid name.
    pub name_code_units: Vec<u16>,
    /// Whether the title bar includes the MATLAB Figure number.
    pub number_title: bool,
    /// Visibility.
    pub visible: bool,
    /// Background color.
    pub background_rgba: Rgba,
    /// Width and height in CSS pixels.
    pub initial_logical_size_css_pixels: [f64; 2],
    /// MATLAB Position rectangle `[left, bottom, width, height]` in CSS pixels.
    pub position_css_pixels: [f64; 4],
    /// Figure-level creation/add intent.
    pub next_plot: FigureNextPlot,
}

/// Cartesian axis scale.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    /// Linear scale.
    #[default]
    Linear,
    /// Base-10 logarithmic scale, introduced in graphics-v4.
    Log,
}

impl Scale {
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if requires &T.
    const fn is_linear(value: &Self) -> bool {
        matches!(value, Self::Linear)
    }
}

/// Cartesian axis direction.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxisDirection {
    /// Increasing values follow the conventional screen direction.
    #[default]
    Normal,
    /// Increasing values follow the opposite screen direction.
    Reverse,
}

impl AxisDirection {
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if requires &T.
    const fn is_normal(value: &Self) -> bool {
        matches!(value, Self::Normal)
    }
}

/// Automatic or explicitly selected axes limits.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxesLimitMode {
    /// Deterministic auto-limit output.
    #[default]
    Auto,
    /// User-selected limits.
    Manual,
}

/// Automatic or explicitly selected tick/tick-label values.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxesTickMode {
    /// Renderer uses resolved automatic values, or generates them for a legacy snapshot.
    #[default]
    Auto,
    /// Values were explicitly selected by the language-level property.
    Manual,
}

/// One extended-real tick value. JSON uses exact strings for infinities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickValue(f64);

impl TickValue {
    /// Creates a tick value, rejecting only NaN.
    #[must_use]
    pub fn new(value: f64) -> Option<Self> {
        (!value.is_nan()).then_some(Self(value))
    }

    /// Returns the represented extended-real value.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for TickValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0 == f64::INFINITY {
            serializer.serialize_str("Infinity")
        } else if self.0 == f64::NEG_INFINITY {
            serializer.serialize_str("-Infinity")
        } else if self.0.is_finite() {
            serializer.serialize_f64(self.0)
        } else {
            Err(serde::ser::Error::custom("tick value cannot be NaN"))
        }
    }
}

impl<'de> Deserialize<'de> for TickValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(number) => number
                .as_f64()
                .and_then(Self::new)
                .ok_or_else(|| serde::de::Error::custom("tick value must not be NaN")),
            serde_json::Value::String(value) if value == "Infinity" => Ok(Self(f64::INFINITY)),
            serde_json::Value::String(value) if value == "-Infinity" => Ok(Self(f64::NEG_INFINITY)),
            _ => Err(serde::de::Error::custom(
                "tick value must be a JSON number, Infinity, or -Infinity",
            )),
        }
    }
}

/// Axes replacement/hold behavior.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NextPlot {
    /// Replace plot children on the next plotting call.
    Replace,
    /// Preserve plot children (`hold on`).
    Add,
}

/// Figure-level `nextPlot` intent supported by the first tranche.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FigureNextPlot {
    /// Add content to the existing Figure.
    Add,
    /// Create a new Figure target.
    New,
}

/// Axes coordinate system. Missing values identify legacy Cartesian snapshots.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AxesCoordinateSystem {
    /// Ordinary Cartesian coordinates.
    #[default]
    Cartesian,
    /// Theta/radius coordinates rendered by a `PolarAxes`.
    Polar,
}

impl AxesCoordinateSystem {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    const fn is_cartesian(&self) -> bool {
        matches!(self, Self::Cartesian)
    }
}

/// `PolarAxes` limit/tick units. Polar line data remains radians.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThetaAxisUnits {
    /// Degrees, matching the R2022b factory default.
    #[default]
    Degrees,
    /// Radians.
    Radians,
}

/// `PolarAxes` theta direction.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThetaDirection {
    /// Counterclockwise, matching the R2022b factory default.
    #[default]
    Counterclockwise,
    /// Clockwise.
    Clockwise,
}

/// `PolarAxes` zero-angle location.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThetaZeroLocation {
    /// Three o'clock.
    #[default]
    Right,
    /// Twelve o'clock.
    Top,
    /// Nine o'clock.
    Left,
    /// Six o'clock.
    Bottom,
}

/// `Axes2D` property schema. Label identifiers and fields introduced after v1
/// retain safe wire defaults so older retained snapshots remain readable.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct Axes2DProperties {
    /// Cartesian or polar coordinate system.
    #[serde(default, skip_serializing_if = "AxesCoordinateSystem::is_cartesian")]
    pub coordinate_system: AxesCoordinateSystem,
    /// Normalized x/y/width/height.
    pub position_normalized: [f64; 4],
    /// Axes background, or null for MATLAB `Color='none'`.
    #[serde(default = "default_axes_background_rgba")]
    pub background_rgba: Nullable<Rgba>,
    /// Polar limit/tick display units.
    #[serde(default)]
    pub theta_axis_units: ThetaAxisUnits,
    /// Polar theta increasing direction.
    #[serde(default)]
    pub theta_direction: ThetaDirection,
    /// Polar zero-angle screen location.
    #[serde(default)]
    pub theta_zero_location: ThetaZeroLocation,
    /// Polar radial ruler location in active theta units.
    #[serde(default = "default_r_axis_location")]
    pub r_axis_location: f64,
    /// X scale, exactly linear in v1.
    pub x_scale: Scale,
    /// Y scale, exactly linear in v1.
    pub y_scale: Scale,
    /// Z scale, introduced in graphics-v4.
    #[serde(default, skip_serializing_if = "Scale::is_linear")]
    pub z_scale: Scale,
    /// X direction.
    #[serde(default, skip_serializing_if = "AxisDirection::is_normal")]
    pub x_direction: AxisDirection,
    /// Y direction.
    #[serde(default, skip_serializing_if = "AxisDirection::is_normal")]
    pub y_direction: AxisDirection,
    /// Z direction.
    #[serde(default, skip_serializing_if = "AxisDirection::is_normal")]
    pub z_direction: AxisDirection,
    /// Axes chrome visibility. Plot children remain independently visible.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// Resolved finite increasing X limits.
    pub x_limits: [f64; 2],
    /// Resolved finite increasing Y limits.
    pub y_limits: [f64; 2],
    /// Resolved finite increasing Z limits for a 3D scene.
    #[serde(default = "default_axis_limits")]
    pub z_limits: [f64; 2],
    /// X limit mode.
    pub x_limits_mode: AxesLimitMode,
    /// Y limit mode.
    pub y_limits_mode: AxesLimitMode,
    /// Z limit mode.
    #[serde(default)]
    pub z_limits_mode: AxesLimitMode,
    /// Hold/replacement mode.
    pub next_plot: NextPlot,
    /// X grid visibility.
    pub grid_x: bool,
    /// Y grid visibility.
    pub grid_y: bool,
    /// Z grid visibility.
    #[serde(default)]
    pub grid_z: bool,
    /// X minor-grid visibility.
    #[serde(default)]
    pub minor_grid_x: bool,
    /// Y minor-grid visibility.
    #[serde(default)]
    pub minor_grid_y: bool,
    /// Z minor-grid visibility.
    #[serde(default)]
    pub minor_grid_z: bool,
    /// Positive one-based color-order index.
    pub color_order_index: u64,
    /// Resolved X tick values. Empty automatic values ask the renderer to generate them.
    #[serde(default)]
    pub x_tick: Vec<TickValue>,
    /// Resolved Y tick values.
    #[serde(default)]
    pub y_tick: Vec<TickValue>,
    /// Resolved Z tick values.
    #[serde(default)]
    pub z_tick: Vec<TickValue>,
    /// Exact X tick-label UTF-16 strings; layout handles unequal counts.
    #[serde(default)]
    pub x_tick_label_code_units: Vec<Vec<u16>>,
    /// Exact Y tick-label UTF-16 strings.
    #[serde(default)]
    pub y_tick_label_code_units: Vec<Vec<u16>>,
    /// Exact Z tick-label UTF-16 strings.
    #[serde(default)]
    pub z_tick_label_code_units: Vec<Vec<u16>>,
    /// X tick mode.
    #[serde(default)]
    pub x_tick_mode: AxesTickMode,
    /// Y tick mode.
    #[serde(default)]
    pub y_tick_mode: AxesTickMode,
    /// Z tick mode.
    #[serde(default)]
    pub z_tick_mode: AxesTickMode,
    /// X tick-label mode.
    #[serde(default)]
    pub x_tick_label_mode: AxesTickMode,
    /// Y tick-label mode.
    #[serde(default)]
    pub y_tick_label_mode: AxesTickMode,
    /// Z tick-label mode.
    #[serde(default)]
    pub z_tick_label_mode: AxesTickMode,
    /// MATLAB-compatible azimuth/elevation pair in degrees.
    #[serde(default = "default_axes_view")]
    pub view: [f64; 2],
    /// Projection mode.
    #[serde(default)]
    pub projection: Projection,
    /// Renderer camera scale retained across browser interactions.
    #[serde(default = "default_camera_scale")]
    pub camera_scale: f64,
    /// Relative scale of one X/Y/Z data unit.
    #[serde(default = "default_aspect_ratio")]
    pub data_aspect_ratio: [f64; 3],
    /// Automatic/manual data-aspect mode.
    #[serde(default)]
    pub data_aspect_ratio_mode: AxesLimitMode,
    /// Relative X/Y/Z extent of the plot box.
    #[serde(default = "default_aspect_ratio")]
    pub plot_box_aspect_ratio: [f64; 3],
    /// Automatic/manual plot-box aspect mode.
    #[serde(default)]
    pub plot_box_aspect_ratio_mode: AxesLimitMode,
    /// Resolved color limits.
    #[serde(default = "default_axis_limits")]
    pub c_limits: [f64; 2],
    /// Color-limit mode.
    #[serde(default)]
    pub c_limits_mode: AxesLimitMode,
    /// Active sRGB-encoded RGB rows. Missing only for legacy snapshots.
    #[serde(default)]
    pub colormap: Option<Vec<[f64; 3]>>,
    /// Whether an east-outside colorbar should be rendered.
    #[serde(default)]
    pub colorbar_visible: bool,
    /// Whether the top and right Axes borders are visible.
    #[serde(default, rename = "box")]
    pub box_enabled: bool,
    /// Axes/tick-label font size in CSS pixels.
    #[serde(default = "default_axes_font_size_css_px")]
    pub font_size_css_px: f64,
    /// Exact Axes/tick-label font-family code units.
    #[serde(default)]
    pub font_family_code_units: Vec<u16>,
    /// Tick-mark direction.
    #[serde(default)]
    pub tick_direction: TickDirection,
    /// Axes line width in CSS pixels.
    #[serde(default = "default_axes_line_width_css_px")]
    pub line_width_css_px: f64,
    /// Interpreter shared by X/Y tick labels.
    #[serde(default)]
    pub tick_label_interpreter: TextInterpreter,
    /// Optional title Text identifier.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub title_id: Nullable<String>,
    /// Optional x-label Text identifier.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub x_label_id: Nullable<String>,
    /// Optional y-label Text identifier.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub y_label_id: Nullable<String>,
    /// Optional z-label Text identifier.
    #[serde(default)]
    pub z_label_id: Nullable<String>,
}

const fn default_axis_limits() -> [f64; 2] {
    [0.0, 1.0]
}

const fn default_r_axis_location() -> f64 {
    80.0
}

const fn default_true() -> bool {
    true
}

const fn default_axes_background_rgba() -> Nullable<Rgba> {
    Nullable(Some(Rgba([1.0, 1.0, 1.0, 1.0])))
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if requires &T.
const fn is_true(value: &bool) -> bool {
    *value
}

const fn default_axes_view() -> [f64; 2] {
    [0.0, 90.0]
}

const fn default_camera_scale() -> f64 {
    1.0
}

const fn default_aspect_ratio() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}

/// Axes projection mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Projection {
    /// Parallel projection.
    #[default]
    Orthographic,
    /// Perspective projection.
    Perspective,
}

/// V1 line style.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LineStyle {
    /// No stroke; marker rendering remains active.
    None,
    /// Solid line.
    Solid,
    /// Dashed line.
    #[serde(rename = "dash")]
    Dash,
    /// Dotted line.
    #[serde(rename = "dot")]
    Dot,
    /// Dash-dot line.
    DashDot,
}

impl Default for LineStyle {
    fn default() -> Self {
        Self::Solid
    }
}

/// V1 marker style.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Marker {
    /// No marker.
    None,
    /// Point.
    Point,
    /// Circle.
    Circle,
    /// Plus.
    Plus,
    /// Star.
    Star,
    /// Cross.
    Cross,
    /// Square.
    Square,
    /// Diamond.
    Diamond,
    /// Upward triangle.
    TriangleUp,
    /// Downward triangle.
    TriangleDown,
    /// Right triangle.
    TriangleRight,
    /// Left triangle.
    TriangleLeft,
    /// Horizontal line marker.
    HorizontalLine,
    /// Vertical line marker.
    VerticalLine,
    /// Pentagram.
    Pentagram,
    /// Hexagram.
    Hexagram,
}

/// Exact one-based MATLAB source index, serialized as a canonical decimal string.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MarkerIndex(u64);

impl MarkerIndex {
    /// Creates a positive one-based source index.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Returns the exact one-based source index.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Serialize for MarkerIndex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0 == 0 {
            return Err(serde::ser::Error::custom(
                "marker index must be a positive integer",
            ));
        }
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for MarkerIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.is_empty()
            || encoded.starts_with('0')
            || !encoded.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(serde::de::Error::custom(
                "marker index must be a canonical positive decimal string",
            ));
        }
        encoded
            .parse::<u64>()
            .ok()
            .and_then(Self::new)
            .ok_or_else(|| serde::de::Error::custom("marker index does not fit u64"))
    }
}

/// `LineSeries` property schema. Every field is required and there are no wire
/// defaults.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineSeriesProperties {
    /// Independent contiguous 1-by-N X resource.
    pub x_data: DataRef,
    /// Independent contiguous 1-by-N Y resource.
    pub y_data: DataRef,
    /// Optional contiguous 1-by-N Z resource for a three-dimensional Line.
    #[serde(default)]
    pub z_data: Nullable<DataRef>,
    /// Stroke color.
    pub color_rgba: Rgba,
    /// Positive width in CSS pixels.
    pub line_width_css_px: f64,
    /// Stroke pattern.
    pub line_style: LineStyle,
    /// Marker shape.
    pub marker: Marker,
    /// Nonnegative marker size in CSS pixels.
    pub marker_size_css_px: f64,
    /// Optional resolved marker fill color. Missing preserves the legacy transparent fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_face_rgba: Option<Rgba>,
    /// Optional resolved marker edge color. Missing preserves the legacy Line color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_edge_rgba: Option<Rgba>,
    /// Exact source indices for markers. `None` is the legacy snapshot default.
    #[serde(default)]
    pub marker_indices: Option<Vec<MarkerIndex>>,
    /// Exact legend-display name code units.
    pub display_name_code_units: Vec<u16>,
    /// Visibility.
    pub visible: bool,
    /// Axes clipping flag.
    pub clipping: bool,
}

/// `ScatterSeries` property schema. Nullable resource members are required on
/// the wire and contain either a resource or explicit null.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScatterSeriesProperties {
    /// Independent contiguous 1-by-N X resource.
    pub x_data: DataRef,
    /// Independent contiguous 1-by-N Y resource.
    pub y_data: DataRef,
    /// Optional contiguous 1-by-N Z resource for a three-dimensional Scatter.
    #[serde(default)]
    pub z_data: Nullable<DataRef>,
    /// Nullable scalar or 1-by-N marker-size resource.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub size_data: Nullable<DataRef>,
    /// Nullable scalar or 1-by-N marker-color resource.
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub color_data: Nullable<DataRef>,
    /// Component resolved from scalar `CData` through Axes `CLim`.
    #[serde(default)]
    pub color_data_target: ScatterColorTarget,
    /// Marker shape.
    pub marker: Marker,
    /// Nonnegative default marker size in CSS pixels.
    pub marker_size_css_px: f64,
    /// Positive marker edge width in CSS pixels.
    #[serde(default = "default_axes_line_width_css_px")]
    pub marker_edge_width_css_px: f64,
    /// Required fill color.
    pub marker_face_rgba: Rgba,
    /// Required stroke color.
    pub marker_edge_rgba: Rgba,
    /// Exact legend-display name code units.
    pub display_name_code_units: Vec<u16>,
    /// Visibility.
    pub visible: bool,
    /// Axes clipping flag.
    pub clipping: bool,
}

/// Scatter component resolved from scalar `CData` through Axes `CLim`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScatterColorTarget {
    /// Legacy snapshots and fixed-color Scatter objects.
    #[default]
    None,
    /// Marker faces use mapped scalar colors.
    Face,
    /// Marker edges use mapped scalar colors.
    Edge,
}

/// Surface color source intent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceColorMode {
    /// Resolve from `CData` and Axes `CLim`.
    Flat,
    /// Resolve at vertices and interpolate across each triangle.
    Interp,
    /// Do not draw this component.
    None,
    /// Use the accompanying explicit RGBA value.
    Uniform,
}

/// Surface `CData` interpretation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CDataMapping {
    /// Scale through Axes `CLim`.
    Scaled,
    /// Use direct colormap indices.
    Direct,
}

/// Structured Surface property schema used by graphics-v2 3D scenes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceSeriesProperties {
    pub x_data: DataRef,
    pub y_data: DataRef,
    pub z_data: DataRef,
    pub c_data: DataRef,
    pub face_color: SurfaceColorMode,
    #[serde(default)]
    pub face_rgba: Nullable<Rgba>,
    pub edge_color: SurfaceColorMode,
    #[serde(default)]
    pub edge_rgba: Nullable<Rgba>,
    pub line_width_css_px: f64,
    #[serde(default)]
    pub line_style: LineStyle,
    pub c_data_mapping: CDataMapping,
    pub face_alpha: f64,
    #[serde(default)]
    pub lighting_enabled: bool,
    pub visible: bool,
    pub clipping: bool,
}

/// Irregular Patch property schema used by graphics-v2 scenes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // Independent retained graphics properties map one-to-one.
pub struct PatchSeriesProperties {
    /// Face connectivity matrix, one face per row with optional NaN padding.
    pub faces: DataRef,
    /// Vertex coordinate matrix with two or three columns.
    pub vertices: DataRef,
    /// Optional scalar face/vertex color values.
    pub face_vertex_cdata: DataRef,
    /// Optional MATLAB `VertexNormals`, one N-by-3 vector per source vertex.
    #[serde(default)]
    pub vertex_normals: Nullable<DataRef>,
    pub face_color: SurfaceColorMode,
    #[serde(default)]
    pub face_rgba: Nullable<Rgba>,
    pub edge_color: SurfaceColorMode,
    #[serde(default)]
    pub edge_rgba: Nullable<Rgba>,
    pub line_width_css_px: f64,
    #[serde(default)]
    pub line_style: LineStyle,
    pub c_data_mapping: CDataMapping,
    pub face_alpha: f64,
    pub edge_alpha: f64,
    #[serde(default)]
    pub lighting_enabled: bool,
    #[serde(default)]
    pub smooth_normals: bool,
    pub visible: bool,
    pub clipping: bool,
}

/// Text semantic role.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TextRole {
    /// Axes title.
    Title,
    /// X label.
    XLabel,
    /// Y label.
    YLabel,
    /// Z-axis label.
    ZLabel,
    /// Free annotation.
    Annotation,
}

/// MATLAB text interpreter intent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextInterpreter {
    /// MATLAB TeX subset. Plain strings retain the fast text path.
    #[default]
    Tex,
    /// LaTeX math rendering.
    Latex,
    /// Literal text with no markup interpretation.
    None,
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HorizontalAlignment {
    /// Left aligned.
    Left,
    /// Center aligned.
    Center,
    /// Right aligned.
    Right,
}

/// Vertical text alignment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VerticalAlignment {
    /// Bottom aligned.
    Bottom,
    /// Middle aligned.
    Middle,
    /// Top aligned.
    Top,
    /// Font baseline aligned.
    Baseline,
}

/// Text property schema. Text and font-family values are exact UTF-16 code
/// unit arrays. Every field is required.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextProperties {
    /// Exact display code units.
    pub code_units: Vec<u16>,
    /// Semantic role.
    pub role: TextRole,
    /// Normalized anchor.
    pub anchor_normalized: [f64; 2],
    /// Horizontal alignment.
    pub horizontal_alignment: HorizontalAlignment,
    /// Vertical alignment.
    pub vertical_alignment: VerticalAlignment,
    /// Text color.
    pub color_rgba: Rgba,
    /// Exact font-family code units; empty selects host default.
    pub font_family_code_units: Vec<u16>,
    /// Positive font size in CSS pixels.
    pub font_size_css_px: f64,
    /// Font weight in `1..=1000`.
    pub font_weight: u16,
    /// Font style.
    pub font_style: FontStyle,
    /// Text interpreter.
    #[serde(default)]
    pub interpreter: TextInterpreter,
    /// Counter-clockwise MATLAB text rotation in degrees.
    #[serde(default)]
    pub rotation_degrees: f64,
    /// Visibility.
    pub visible: bool,
}

/// V1 legend placement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendLocation {
    /// Automatic placement.
    Best,
    /// North.
    North,
    /// South.
    South,
    /// East.
    East,
    /// West.
    West,
    /// Northeast.
    NorthEast,
    /// Northwest.
    NorthWest,
    /// Southeast.
    SouthEast,
    /// Southwest.
    SouthWest,
    /// Centered below the Axes plot box.
    SouthOutside,
}

/// Legend item flow direction.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendOrientation {
    /// Items flow from top to bottom.
    #[default]
    Vertical,
    /// Items flow from left to right.
    Horizontal,
}

/// Axes tick-mark direction.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TickDirection {
    /// Marks extend into the plot box.
    #[default]
    In,
    /// Marks extend away from the plot box.
    Out,
    /// Marks extend on both sides of the ruler.
    Both,
}

/// V1 text font style.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    /// Upright text.
    Normal,
    /// Italic text.
    Italic,
}

/// Legend property schema. Membership and label arrays are required, ordered,
/// and have equal lengths.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegendProperties {
    /// Ordered line/scatter identifiers.
    pub series_ids: Vec<String>,
    /// Ordered exact UTF-16 labels.
    pub label_code_units: Vec<Vec<u16>>,
    /// Placement.
    pub location: LegendLocation,
    /// Visibility.
    pub visible: bool,
    /// Background color.
    pub background_rgba: Rgba,
    /// Border color.
    pub border_rgba: Rgba,
    /// Exact font-family code units; empty selects host default.
    pub font_family_code_units: Vec<u16>,
    /// Positive font size in CSS pixels.
    pub font_size_css_px: f64,
    /// Font weight in `1..=1000`.
    pub font_weight: u16,
    /// Interpreter shared by legend labels.
    #[serde(default)]
    pub interpreter: TextInterpreter,
    /// Item flow direction.
    #[serde(default)]
    pub orientation: LegendOrientation,
    /// Positive requested column count.
    #[serde(default = "default_legend_columns")]
    pub num_columns: u32,
}

const fn default_legend_columns() -> u32 {
    1
}

/// Colorbar object schema. Its Axes owns color limits and colormap state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorBarProperties {
    /// Visibility.
    pub visible: bool,
    /// Explicit tick values. Empty automatic values ask the renderer to generate them.
    #[serde(default)]
    pub ticks: Vec<TickValue>,
    /// Tick-position mode.
    #[serde(default)]
    pub ticks_mode: AxesTickMode,
    /// Exact tick-label UTF-16 strings.
    #[serde(default)]
    pub tick_label_code_units: Vec<Vec<u16>>,
    /// Tick-label mode.
    #[serde(default)]
    pub tick_labels_mode: AxesTickMode,
}

/// MATLAB-visible high-level chart type backed by retained primitive children.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartType {
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

/// Semantic color selection retained by a MATLAB chart handle.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartColorMode {
    /// Resolve from the owning axes or current primitive style.
    Auto,
    /// Disable the corresponding stroke or fill.
    None,
    /// Use the accompanying uniform RGBA value.
    Uniform,
}

/// Chart color intent plus an optional resolved uniform color.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartColorProperties {
    /// Semantic color mode.
    pub mode: ChartColorMode,
    /// Present exactly when `mode` is `uniform`.
    pub rgba: Nullable<Rgba>,
}

/// Non-rendered chart container properties.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartGroupProperties {
    /// MATLAB-visible chart type.
    pub chart_type: ChartType,
    /// Group visibility inherited by all retained children.
    pub visible: bool,
    /// Nullable line-chart Color intent.
    pub color: Nullable<ChartColorProperties>,
    /// Positive stroke width in CSS pixels.
    pub line_width_css_px: f64,
    /// Stroke pattern.
    pub line_style: LineStyle,
    /// Nullable line-chart marker.
    pub marker: Nullable<Marker>,
    /// Nullable positive marker size in CSS pixels.
    pub marker_size_css_px: Nullable<f64>,
    /// Nullable marker fill intent.
    pub marker_face_color: Nullable<ChartColorProperties>,
    /// Nullable marker edge intent.
    pub marker_edge_color: Nullable<ChartColorProperties>,
    /// Nullable patch-chart face intent.
    pub face_color: Nullable<ChartColorProperties>,
    /// Nullable patch-chart edge intent.
    pub edge_color: Nullable<ChartColorProperties>,
    /// Nullable patch face alpha.
    pub face_alpha: Nullable<f64>,
    /// Nullable patch edge alpha.
    pub edge_alpha: Nullable<f64>,
    /// Nullable Stem/Area/Bar baseline.
    pub base_value: Nullable<f64>,
    /// Nullable Bar width fraction.
    pub bar_width: Nullable<f64>,
    /// Nullable `ErrorBar` cap size in CSS pixels.
    pub cap_size_css_px: Nullable<f64>,
}

const fn default_axes_font_size_css_px() -> f64 {
    40.0 / 3.0
}

const fn default_axes_line_width_css_px() -> f64 {
    2.0 / 3.0
}

/// Typed object/property DTO. Unknown kinds fail deserialization rather than
/// being silently interpreted as a known v1 object.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GraphicsObject {
    /// Figure root.
    Figure {
        /// Common fields flattened into the wire object.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete Figure properties.
        properties: FigureProperties,
    },
    /// 2D axes.
    Axes2d {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete `Axes2D` properties.
        properties: Axes2DProperties,
    },
    /// Line series.
    LineSeries {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete line properties.
        properties: LineSeriesProperties,
    },
    /// Scatter series.
    ScatterSeries {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete scatter properties.
        properties: ScatterSeriesProperties,
    },
    /// Structured surface series.
    SurfaceSeries {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete Surface properties.
        properties: SurfaceSeriesProperties,
    },
    /// Irregular polygon mesh.
    PatchSeries {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete Patch properties.
        properties: PatchSeriesProperties,
    },
    /// MATLAB-visible chart backed by retained Line/Patch children.
    ChartGroup {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete chart-container properties.
        properties: ChartGroupProperties,
    },
    /// Text object.
    Text {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete text properties.
        properties: TextProperties,
    },
    /// Legend object.
    Legend {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete legend properties.
        properties: LegendProperties,
    },
    /// East-outside colorbar object.
    ColorBar {
        /// Common fields.
        #[serde(flatten)]
        fields: ObjectFields,
        /// Complete colorbar properties.
        properties: ColorBarProperties,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObjectKind {
    Figure,
    Axes2d,
    LineSeries,
    ScatterSeries,
    SurfaceSeries,
    PatchSeries,
    ChartGroup,
    Text,
    Legend,
    ColorBar,
}

impl GraphicsObject {
    /// Returns common identity/hierarchy fields.
    #[must_use]
    pub const fn fields(&self) -> &ObjectFields {
        match self {
            Self::Figure { fields, .. }
            | Self::Axes2d { fields, .. }
            | Self::LineSeries { fields, .. }
            | Self::ScatterSeries { fields, .. }
            | Self::SurfaceSeries { fields, .. }
            | Self::PatchSeries { fields, .. }
            | Self::ChartGroup { fields, .. }
            | Self::Text { fields, .. }
            | Self::Legend { fields, .. }
            | Self::ColorBar { fields, .. } => fields,
        }
    }

    /// Returns mutable common fields for transactional delta application.
    #[must_use]
    pub fn fields_mut(&mut self) -> &mut ObjectFields {
        match self {
            Self::Figure { fields, .. }
            | Self::Axes2d { fields, .. }
            | Self::LineSeries { fields, .. }
            | Self::ScatterSeries { fields, .. }
            | Self::SurfaceSeries { fields, .. }
            | Self::PatchSeries { fields, .. }
            | Self::ChartGroup { fields, .. }
            | Self::Text { fields, .. }
            | Self::Legend { fields, .. }
            | Self::ColorBar { fields, .. } => fields,
        }
    }

    const fn kind(&self) -> ObjectKind {
        match self {
            Self::Figure { .. } => ObjectKind::Figure,
            Self::Axes2d { .. } => ObjectKind::Axes2d,
            Self::LineSeries { .. } => ObjectKind::LineSeries,
            Self::ScatterSeries { .. } => ObjectKind::ScatterSeries,
            Self::SurfaceSeries { .. } => ObjectKind::SurfaceSeries,
            Self::PatchSeries { .. } => ObjectKind::PatchSeries,
            Self::ChartGroup { .. } => ObjectKind::ChartGroup,
            Self::Text { .. } => ObjectKind::Text,
            Self::Legend { .. } => ObjectKind::Legend,
            Self::ColorBar { .. } => ObjectKind::ColorBar,
        }
    }

    fn data_refs(&self) -> Vec<&DataRef> {
        match self {
            Self::LineSeries { properties, .. } => {
                let mut refs = vec![&properties.x_data, &properties.y_data];
                refs.extend(properties.z_data.as_ref());
                refs
            }
            Self::ScatterSeries { properties, .. } => {
                let mut refs = vec![&properties.x_data, &properties.y_data];
                refs.extend(properties.z_data.as_ref());
                refs.extend(properties.size_data.as_ref());
                refs.extend(properties.color_data.as_ref());
                refs
            }
            Self::SurfaceSeries { properties, .. } => vec![
                &properties.x_data,
                &properties.y_data,
                &properties.z_data,
                &properties.c_data,
            ],
            Self::PatchSeries { properties, .. } => {
                let mut refs = vec![
                    &properties.faces,
                    &properties.vertices,
                    &properties.face_vertex_cdata,
                ];
                refs.extend(properties.vertex_normals.as_ref());
                refs
            }
            Self::Figure { .. }
            | Self::Axes2d { .. }
            | Self::ChartGroup { .. }
            | Self::Text { .. }
            | Self::Legend { .. }
            | Self::ColorBar { .. } => Vec::new(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn validate_properties(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        match self {
            Self::Figure { properties, .. } => {
                validate_positive_safe(properties.number, "figure number")
                    .map_err(|_| invalid_scene("Figure number must be a positive safe integer"))?;
                properties
                    .background_rgba
                    .validate("Figure backgroundRgba")?;
                validate_finite_positive(
                    &properties.initial_logical_size_css_pixels,
                    "Figure initialLogicalSizeCssPixels",
                )?;
                validate_figure_position(properties.position_css_pixels)?;
                if properties.initial_logical_size_css_pixels[0].to_bits()
                    != properties.position_css_pixels[2].to_bits()
                    || properties.initial_logical_size_css_pixels[1].to_bits()
                        != properties.position_css_pixels[3].to_bits()
                {
                    return Err(invalid_scene(
                        "Figure initialLogicalSizeCssPixels must match Position width and height",
                    ));
                }
                Ok(())
            }
            Self::Axes2d { properties, .. } => validate_axes_properties(properties),
            Self::LineSeries { properties, .. } => validate_line_properties(properties, limits),
            Self::ScatterSeries { properties, .. } => {
                properties.x_data.validate_series(limits)?;
                properties.y_data.validate_series(limits)?;
                let count = properties.x_data.element_count()?;
                if count != properties.y_data.element_count()? {
                    return Err(invalid_scene(
                        "ScatterSeries XData and YData lengths differ",
                    ));
                }
                if let Some(z_data) = properties.z_data.as_ref() {
                    z_data.validate_series(limits)?;
                    if count != z_data.element_count()? {
                        return Err(invalid_scene(
                            "ScatterSeries XData and ZData lengths differ",
                        ));
                    }
                }
                for data in [
                    properties.size_data.as_ref(),
                    properties.color_data.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    data.validate(limits)?;
                    if data.shape.len() != 2 || (data.shape[0] != 1 && data.shape[1] != 1) {
                        return Err(invalid_scene(
                            "ScatterSeries optional data must be a two-dimensional vector",
                        ));
                    }
                    let optional_count = data.element_count()?;
                    if optional_count != 1 && optional_count != count {
                        return Err(invalid_scene(
                            "ScatterSeries optional data must be scalar or match XData length",
                        ));
                    }
                }
                if properties.marker == Marker::None {
                    return Err(invalid_scene("ScatterSeries marker cannot be none"));
                }
                properties
                    .marker_face_rgba
                    .validate("ScatterSeries markerFaceRgba")?;
                properties
                    .marker_edge_rgba
                    .validate("ScatterSeries markerEdgeRgba")?;
                validate_positive(
                    properties.marker_size_css_px,
                    "ScatterSeries markerSizeCssPx",
                )?;
                validate_positive(
                    properties.marker_edge_width_css_px,
                    "ScatterSeries markerEdgeWidthCssPx",
                )
            }
            Self::SurfaceSeries { properties, .. } => {
                validate_surface_properties(properties, limits)
            }
            Self::PatchSeries { properties, .. } => validate_patch_properties(properties, limits),
            Self::ChartGroup { properties, .. } => validate_chart_group_properties(properties),
            Self::Text { properties, .. } => {
                validate_finite(&properties.anchor_normalized, "Text anchorNormalized")?;
                properties.color_rgba.validate("Text colorRgba")?;
                validate_positive(properties.font_size_css_px, "Text fontSizeCssPx")?;
                if !properties.rotation_degrees.is_finite() {
                    return Err(invalid_scene("Text rotationDegrees must be finite"));
                }
                validate_font_weight(properties.font_weight, "Text fontWeight")
            }
            Self::Legend { properties, .. } => {
                if properties.series_ids.len() != properties.label_code_units.len() {
                    return Err(invalid_scene(
                        "Legend seriesIds and labelsCodeUnits lengths differ",
                    ));
                }
                if properties.series_ids.iter().any(String::is_empty) {
                    return Err(invalid_scene("Legend seriesIds must be non-empty"));
                }
                properties
                    .background_rgba
                    .validate("Legend backgroundRgba")?;
                properties.border_rgba.validate("Legend borderRgba")?;
                validate_positive(properties.font_size_css_px, "Legend fontSizeCssPx")?;
                if properties.num_columns == 0 {
                    return Err(invalid_scene("Legend numColumns must be positive"));
                }
                validate_font_weight(properties.font_weight, "Legend fontWeight")
            }
            Self::ColorBar { properties, .. } => {
                validate_tick_values(&properties.ticks, "ColorBar ticks")
            }
        }
    }
}

fn validate_axes_properties(properties: &Axes2DProperties) -> Result<(), ValidationError> {
    validate_normalized_position(properties.position_normalized)?;
    if let Some(background) = properties.background_rgba.as_ref() {
        background.validate("Axes backgroundRgba")?;
    }
    if !properties.r_axis_location.is_finite() {
        return Err(invalid_scene("Axes rAxisLocation must be finite"));
    }
    if properties.coordinate_system == AxesCoordinateSystem::Polar
        && (properties.x_scale != Scale::Linear || properties.y_scale != Scale::Linear)
    {
        return Err(invalid_scene(
            "PolarAxes supports only linear theta and radial scales",
        ));
    }
    validate_limits(properties.x_limits, "Axes xLimits")?;
    validate_limits(properties.y_limits, "Axes yLimits")?;
    validate_limits(properties.z_limits, "Axes zLimits")?;
    validate_limits(properties.c_limits, "Axes cLimits")?;
    for (scale, limits, name) in [
        (properties.x_scale, properties.x_limits, "Axes xLimits"),
        (properties.y_scale, properties.y_limits, "Axes yLimits"),
        (properties.z_scale, properties.z_limits, "Axes zLimits"),
    ] {
        if scale == Scale::Log && limits[0] <= 0.0 {
            return Err(invalid_scene(format!(
                "{name} must be positive for a logarithmic axis"
            )));
        }
    }
    if let Some(colormap) = &properties.colormap
        && (colormap.is_empty()
            || colormap
                .iter()
                .flatten()
                .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component)))
    {
        return Err(invalid_scene(
            "Axes colormap must contain finite RGB rows in [0, 1]",
        ));
    }
    validate_finite(&properties.view, "Axes view")?;
    for (values, name) in [
        (properties.data_aspect_ratio, "Axes dataAspectRatio"),
        (properties.plot_box_aspect_ratio, "Axes plotBoxAspectRatio"),
    ] {
        if values
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(invalid_scene(format!(
                "{name} must contain positive finite values"
            )));
        }
    }
    if !properties.camera_scale.is_finite()
        || !(1.0 / 45.0..=170.0 / 45.0).contains(&properties.camera_scale)
    {
        return Err(invalid_scene(
            "Axes cameraScale is outside the supported range",
        ));
    }
    validate_tick_values(&properties.x_tick, "Axes xTick")?;
    validate_tick_values(&properties.y_tick, "Axes yTick")?;
    validate_tick_values(&properties.z_tick, "Axes zTick")?;
    validate_positive(properties.font_size_css_px, "Axes fontSizeCssPx")?;
    validate_positive(properties.line_width_css_px, "Axes lineWidthCssPx")?;
    validate_positive_safe(properties.color_order_index, "Axes colorOrderIndex")
        .map_err(|_| invalid_scene("Axes colorOrderIndex must be a positive safe integer"))
}

fn validate_surface_properties(
    properties: &SurfaceSeriesProperties,
    limits: GraphicsLimits,
) -> Result<(), ValidationError> {
    for data in [
        &properties.x_data,
        &properties.y_data,
        &properties.z_data,
        &properties.c_data,
    ] {
        data.validate_matrix(limits)?;
    }
    let rows = properties.z_data.shape[0];
    let columns = properties.z_data.shape[1];
    if rows < 2 || columns < 2 {
        return Err(invalid_scene(
            "SurfaceSeries ZData must contain at least two rows and columns",
        ));
    }
    let vector_coordinates =
        properties.x_data.shape == [1, columns] && properties.y_data.shape == [rows, 1];
    let matrix_coordinates =
        properties.x_data.shape == [rows, columns] && properties.y_data.shape == [rows, columns];
    if (!vector_coordinates && !matrix_coordinates) || properties.c_data.shape != [rows, columns] {
        return Err(invalid_scene(
            "SurfaceSeries X/Y/Z/CData shapes are inconsistent",
        ));
    }
    validate_surface_color(
        properties.face_color,
        properties.face_rgba.as_ref(),
        "SurfaceSeries face",
    )?;
    validate_surface_color(
        properties.edge_color,
        properties.edge_rgba.as_ref(),
        "SurfaceSeries edge",
    )?;
    validate_positive(properties.line_width_css_px, "SurfaceSeries lineWidthCssPx")?;
    if properties.face_alpha.to_bits() != 1.0_f64.to_bits() {
        return Err(invalid_scene(
            "the first SurfaceSeries pipeline requires opaque faceAlpha",
        ));
    }
    Ok(())
}

fn validate_patch_properties(
    properties: &PatchSeriesProperties,
    limits: GraphicsLimits,
) -> Result<(), ValidationError> {
    properties.faces.validate_matrix(limits)?;
    properties.vertices.validate_matrix(limits)?;
    properties.face_vertex_cdata.validate_matrix(limits)?;
    if let Some(vertex_normals) = properties.vertex_normals.as_ref() {
        vertex_normals.validate_matrix(limits)?;
        if properties.vertices.shape[1] != 3
            || vertex_normals.shape != [properties.vertices.shape[0], 3]
        {
            return Err(invalid_scene(
                "PatchSeries VertexNormals must be N-by-3 and match Vertices",
            ));
        }
    }
    if properties.faces.shape[0] == 0 || properties.faces.shape[1] < 3 {
        return Err(invalid_scene(
            "PatchSeries Faces must contain at least one row and three columns",
        ));
    }
    if properties.vertices.shape[0] < 3 || !matches!(properties.vertices.shape[1], 2 | 3) {
        return Err(invalid_scene(
            "PatchSeries Vertices must be N-by-2 or N-by-3",
        ));
    }
    let cdata_rows = properties.face_vertex_cdata.shape[0];
    let cdata_columns = properties.face_vertex_cdata.shape[1];
    if cdata_rows != 0
        && (cdata_columns != 1
            || (cdata_rows != properties.faces.shape[0]
                && cdata_rows != properties.vertices.shape[0]))
    {
        return Err(invalid_scene(
            "PatchSeries FaceVertexCData must contain one scalar per face or vertex",
        ));
    }
    validate_surface_color(
        properties.face_color,
        properties.face_rgba.as_ref(),
        "PatchSeries face",
    )?;
    validate_surface_color(
        properties.edge_color,
        properties.edge_rgba.as_ref(),
        "PatchSeries edge",
    )?;
    validate_positive(properties.line_width_css_px, "PatchSeries lineWidthCssPx")?;
    let needs_cdata = matches!(
        properties.face_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    ) || matches!(
        properties.edge_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    );
    if needs_cdata && cdata_rows == 0 {
        return Err(invalid_scene(
            "mapped PatchSeries colors require FaceVertexCData",
        ));
    }
    if (properties.face_color == SurfaceColorMode::Interp
        || properties.edge_color == SurfaceColorMode::Interp)
        && cdata_rows != properties.vertices.shape[0]
    {
        return Err(invalid_scene(
            "interpolated PatchSeries colors require one value per vertex",
        ));
    }
    if !properties.face_alpha.is_finite()
        || !(0.0..=1.0).contains(&properties.face_alpha)
        || !properties.edge_alpha.is_finite()
        || !(0.0..=1.0).contains(&properties.edge_alpha)
    {
        return Err(invalid_scene(
            "PatchSeries faceAlpha and edgeAlpha must be finite values in [0,1]",
        ));
    }
    if properties.vertices.shape[1] == 3
        && (properties.face_alpha.to_bits() != 1.0_f64.to_bits()
            || properties.edge_alpha.to_bits() != 1.0_f64.to_bits())
    {
        return Err(invalid_scene(
            "the current 3D PatchSeries pipeline requires opaque faceAlpha and edgeAlpha",
        ));
    }
    Ok(())
}

fn validate_surface_color(
    mode: SurfaceColorMode,
    rgba: Option<&Rgba>,
    field: &str,
) -> Result<(), ValidationError> {
    match (mode, rgba) {
        (SurfaceColorMode::Uniform, Some(color)) => color.validate(field),
        (SurfaceColorMode::Flat | SurfaceColorMode::Interp | SurfaceColorMode::None, None) => {
            Ok(())
        }
        _ => Err(invalid_scene(format!(
            "{field} color mode and rgba value are inconsistent"
        ))),
    }
}

fn validate_chart_color(color: &ChartColorProperties, field: &str) -> Result<(), ValidationError> {
    match (color.mode, color.rgba.as_ref()) {
        (ChartColorMode::Uniform, Some(rgba)) => (*rgba).validate(field),
        (ChartColorMode::Auto | ChartColorMode::None, None) => Ok(()),
        _ => Err(invalid_scene(format!(
            "{field} mode and rgba value are inconsistent"
        ))),
    }
}

fn validate_chart_group_properties(
    properties: &ChartGroupProperties,
) -> Result<(), ValidationError> {
    validate_positive(properties.line_width_css_px, "ChartGroup lineWidthCssPx")?;
    for (color, field) in [
        (properties.color.as_ref(), "ChartGroup color"),
        (
            properties.marker_face_color.as_ref(),
            "ChartGroup markerFaceColor",
        ),
        (
            properties.marker_edge_color.as_ref(),
            "ChartGroup markerEdgeColor",
        ),
        (properties.face_color.as_ref(), "ChartGroup faceColor"),
        (properties.edge_color.as_ref(), "ChartGroup edgeColor"),
    ] {
        if let Some(color) = color {
            validate_chart_color(color, field)?;
        }
    }
    if let Some(value) = properties.marker_size_css_px.as_ref() {
        validate_positive(*value, "ChartGroup markerSizeCssPx")?;
    }
    for (alpha, field) in [
        (properties.face_alpha.as_ref(), "ChartGroup faceAlpha"),
        (properties.edge_alpha.as_ref(), "ChartGroup edgeAlpha"),
    ] {
        if alpha.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err(invalid_scene(format!(
                "{field} must be a finite value in [0,1]"
            )));
        }
    }
    if properties
        .base_value
        .as_ref()
        .is_some_and(|value| !value.is_finite())
    {
        return Err(invalid_scene("ChartGroup baseValue must be finite"));
    }
    if properties
        .bar_width
        .as_ref()
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(value) || *value == 0.0)
    {
        return Err(invalid_scene(
            "ChartGroup barWidth must be a finite value in (0,1]",
        ));
    }
    if properties
        .cap_size_css_px
        .as_ref()
        .is_some_and(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(invalid_scene(
            "ChartGroup capSizeCssPx must be finite and nonnegative",
        ));
    }
    Ok(())
}

fn validate_line_properties(
    properties: &LineSeriesProperties,
    limits: GraphicsLimits,
) -> Result<(), ValidationError> {
    properties.x_data.validate_series(limits)?;
    properties.y_data.validate_series(limits)?;
    if let Some(z_data) = properties.z_data.as_ref() {
        z_data.validate_series(limits)?;
        if z_data.element_count()? != properties.x_data.element_count()? {
            return Err(invalid_scene("LineSeries XData and ZData lengths differ"));
        }
    }
    if properties.x_data.element_count()? != properties.y_data.element_count()? {
        return Err(invalid_scene("LineSeries XData and YData lengths differ"));
    }
    properties.color_rgba.validate("LineSeries colorRgba")?;
    if let Some(color) = properties.marker_face_rgba {
        color.validate("LineSeries markerFaceRgba")?;
    }
    if let Some(color) = properties.marker_edge_rgba {
        color.validate("LineSeries markerEdgeRgba")?;
    }
    validate_positive(properties.line_width_css_px, "LineSeries lineWidthCssPx")?;
    validate_positive(properties.marker_size_css_px, "LineSeries markerSizeCssPx")?;
    if properties
        .marker_indices
        .as_ref()
        .is_some_and(|indices| indices.iter().any(|index| index.get() == 0))
    {
        return Err(invalid_scene(
            "LineSeries markerIndices must contain positive integers",
        ));
    }
    Ok(())
}

/// Complete Figure HIR wire snapshot.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureSnapshot {
    /// Exactly `figureSnapshot`.
    #[serde(rename = "type")]
    pub snapshot_type: String,
    /// Opaque Figure identifier.
    pub figure_id: String,
    /// Positive Figure revision.
    pub revision: u64,
    /// Root object identifier.
    pub root_id: String,
    /// Complete object collection in stable display order.
    pub objects: Vec<GraphicsObject>,
    /// Every resource descriptor used by `objects`, exactly once.
    pub referenced_buffers: Vec<DataRef>,
}

impl FigureSnapshot {
    /// Validates complete hierarchy, property, resource, and revision invariants.
    pub fn validate(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        limits.validate()?;
        if self.snapshot_type != "figureSnapshot" {
            return Err(invalid_scene("snapshot type must be figureSnapshot"));
        }
        validate_nonempty(&self.figure_id, "snapshot figureId")?;
        validate_nonempty(&self.root_id, "snapshot rootId")?;
        validate_scene_revision(self.revision, "snapshot revision")?;
        let object_count = u64::try_from(self.objects.len())
            .map_err(|_| invalid_scene("snapshot object count does not fit u64"))?;
        if object_count > limits.max_objects {
            return Err(ValidationError::new(
                ErrorCategory::PayloadLimit,
                "snapshot exceeds negotiated live-object limit",
            ));
        }

        let mut indexes = HashMap::with_capacity(self.objects.len());
        for (index, object) in self.objects.iter().enumerate() {
            let fields = object.fields();
            validate_nonempty(&fields.id, "object id")?;
            validate_scene_revision(fields.generation, "object generation")?;
            validate_scene_revision(fields.object_revision, "objectRevision")?;
            if fields.object_revision > self.revision {
                return Err(invalid_scene("objectRevision exceeds Figure revision"));
            }
            if indexes.insert(fields.id.as_str(), index).is_some() {
                return Err(invalid_scene("snapshot contains duplicate object ids"));
            }
            let mut child_ids = HashSet::with_capacity(fields.children.len());
            for child in &fields.children {
                validate_nonempty(child, "child id")?;
                if !child_ids.insert(child.as_str()) {
                    return Err(invalid_scene("object contains duplicate child ids"));
                }
            }
            object.validate_properties(limits)?;
        }

        let root_index = *indexes
            .get(self.root_id.as_str())
            .ok_or_else(|| invalid_scene("rootId does not resolve"))?;
        let root = &self.objects[root_index];
        if root.kind() != ObjectKind::Figure || root.fields().parent_id.is_some() {
            return Err(invalid_scene(
                "snapshot root must be a parentless Figure object",
            ));
        }
        if self
            .objects
            .iter()
            .filter(|object| object.kind() == ObjectKind::Figure)
            .count()
            != 1
        {
            return Err(invalid_scene("snapshot must contain exactly one Figure"));
        }

        validate_hierarchy(&self.objects, &indexes, root_index)?;
        validate_cross_object_properties(&self.objects, &indexes)?;
        validate_referenced_buffers(&self.objects, &self.referenced_buffers, limits)
    }

    /// Validates the scene and rejects semantics unavailable on older endpoints.
    pub fn validate_for(
        &self,
        limits: GraphicsLimits,
        protocol: GraphicsProtocol,
    ) -> Result<(), ValidationError> {
        self.validate(limits)?;
        for object in &self.objects {
            validate_object_protocol(object, protocol)?;
        }
        Ok(())
    }
}

/// Ordered Figure-delta operation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(clippy::large_enum_variant)] // Wire DTO keeps the complete upsert object inline.
pub enum DeltaOperation {
    /// Create or replace one complete typed object.
    UpsertObject {
        /// Complete replacement object.
        object: GraphicsObject,
    },
    /// Recursively delete one exact generational object.
    DeleteObject {
        /// Object identifier.
        id: String,
        /// Expected generation.
        generation: u64,
    },
    /// Replace one parent's complete child ordering.
    ReorderChildren {
        /// Parent identifier.
        parent_id: String,
        /// Complete ordered children.
        children: Vec<String>,
    },
    /// Select a new root identifier.
    SetRoot {
        /// New root identifier.
        root_id: String,
    },
}

/// One exact `baseRevision -> baseRevision + 1` atomic transaction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureDelta {
    /// Opaque Figure identifier.
    pub figure_id: String,
    /// Exact client/base revision.
    pub base_revision: u64,
    /// Must equal `baseRevision + 1`.
    pub revision: u64,
    /// Ordered object operations.
    pub operations: Vec<DeltaOperation>,
    /// Complete descriptors for every newly referenced immutable resource.
    pub added_buffers: Vec<DataRef>,
    /// Every resource no longer referenced by the resulting live Figure.
    pub released_buffer_ids: Vec<String>,
}

impl FigureDelta {
    /// Validates revision arithmetic and operation-local structure. Use
    /// [`SceneState::apply_delta`] for resulting-scene validation.
    pub fn validate_structure(&self, limits: GraphicsLimits) -> Result<(), ValidationError> {
        validate_nonempty(&self.figure_id, "delta figureId")?;
        validate_scene_revision(self.base_revision, "delta baseRevision")?;
        let expected = self
            .base_revision
            .checked_add(1)
            .filter(|revision| *revision <= crate::MAX_SAFE_INTEGER)
            .ok_or_else(|| invalid_scene("Figure revision safe integer exhausted"))?;
        if self.revision != expected {
            return Err(invalid_scene(
                "delta revision must equal baseRevision plus one",
            ));
        }
        let mut added = HashSet::with_capacity(self.added_buffers.len());
        for descriptor in &self.added_buffers {
            descriptor.validate(limits)?;
            if !added.insert(descriptor.buffer_id.as_str()) {
                return Err(invalid_scene("delta contains duplicate addedBuffers"));
            }
        }
        let mut released = HashSet::with_capacity(self.released_buffer_ids.len());
        for id in &self.released_buffer_ids {
            validate_nonempty(id, "released buffer id")?;
            if !released.insert(id.as_str()) {
                return Err(invalid_scene("delta contains duplicate releasedBufferIds"));
            }
            if added.contains(id.as_str()) {
                return Err(invalid_scene(
                    "one delta cannot add and release the same bufferId",
                ));
            }
        }
        for operation in &self.operations {
            match operation {
                DeltaOperation::UpsertObject { object } => object.validate_properties(limits)?,
                DeltaOperation::DeleteObject { id, generation } => {
                    validate_nonempty(id, "deleteObject id")?;
                    validate_scene_revision(*generation, "deleteObject generation")?;
                }
                DeltaOperation::ReorderChildren {
                    parent_id,
                    children,
                } => {
                    validate_nonempty(parent_id, "reorderChildren parentId")?;
                    let mut unique = HashSet::new();
                    for child in children {
                        validate_nonempty(child, "reorderChildren child id")?;
                        if !unique.insert(child) {
                            return Err(invalid_scene(
                                "reorderChildren contains duplicate child ids",
                            ));
                        }
                    }
                }
                DeltaOperation::SetRoot { root_id } => {
                    validate_nonempty(root_id, "setRoot rootId")?;
                }
            }
        }
        for operation in &self.operations {
            let DeltaOperation::ReorderChildren {
                parent_id,
                children,
            } = operation
            else {
                continue;
            };
            let matching_upsert = self.operations.iter().any(|candidate| {
                matches!(
                    candidate,
                    DeltaOperation::UpsertObject { object }
                        if object.fields().id == *parent_id
                            && object.fields().children == *children
                )
            });
            if !matching_upsert {
                return Err(invalid_scene(
                    "reorderChildren requires a matching parent upsertObject",
                ));
            }
        }
        Ok(())
    }

    /// Validates delta structure and endpoint-specific scene semantics.
    pub fn validate_structure_for(
        &self,
        limits: GraphicsLimits,
        protocol: GraphicsProtocol,
    ) -> Result<(), ValidationError> {
        self.validate_structure(limits)?;
        for operation in &self.operations {
            if let DeltaOperation::UpsertObject { object } = operation {
                validate_object_protocol(object, protocol)?;
            }
        }
        Ok(())
    }
}

fn validate_object_protocol(
    object: &GraphicsObject,
    protocol: GraphicsProtocol,
) -> Result<(), ValidationError> {
    let GraphicsObject::Axes2d { properties, .. } = object else {
        return Ok(());
    };
    let needs_v4 = properties.x_scale != Scale::Linear
        || properties.y_scale != Scale::Linear
        || properties.z_scale != Scale::Linear
        || properties.x_direction != AxisDirection::Normal
        || properties.y_direction != AxisDirection::Normal
        || properties.z_direction != AxisDirection::Normal
        || !properties.visible;
    if needs_v4 && protocol != GraphicsProtocol::V4 {
        Err(invalid_scene(
            "logarithmic, reversed, or hidden Axes require graphics-v4",
        ))
    } else {
        Ok(())
    }
}

/// Validated client-side scene mirror. Delta application is clone-validate-
/// commit so an invalid transaction cannot expose partial state.
#[derive(Clone, Debug)]
pub struct SceneState {
    snapshot: FigureSnapshot,
    limits: GraphicsLimits,
}

impl SceneState {
    /// Creates a state from one complete validated snapshot.
    pub fn new(snapshot: FigureSnapshot, limits: GraphicsLimits) -> Result<Self, ValidationError> {
        snapshot.validate(limits)?;
        Ok(Self { snapshot, limits })
    }

    /// Returns the last completely validated scene.
    #[must_use]
    pub const fn snapshot(&self) -> &FigureSnapshot {
        &self.snapshot
    }

    /// Applies one transaction atomically.
    pub fn apply_delta(&mut self, delta: &FigureDelta) -> Result<(), ValidationError> {
        delta.validate_structure(self.limits)?;
        if delta.figure_id != self.snapshot.figure_id {
            return Err(invalid_scene("delta figureId does not match scene"));
        }
        if delta.base_revision != self.snapshot.revision {
            return Err(ValidationError::new(
                ErrorCategory::RevisionConflict,
                "delta baseRevision does not match the current Figure revision",
            ));
        }

        let old_buffers = descriptor_map(&self.snapshot.referenced_buffers)?;
        let mut candidate = self.snapshot.clone();
        for operation in &delta.operations {
            apply_operation(&mut candidate, operation)?;
        }
        candidate.revision = delta.revision;

        let mut descriptors = old_buffers.clone();
        for descriptor in &delta.added_buffers {
            if descriptors
                .insert(descriptor.buffer_id.clone(), descriptor.clone())
                .is_some()
            {
                return Err(invalid_scene(
                    "addedBuffers cannot reuse a live immutable bufferId",
                ));
            }
        }
        let used = collect_data_refs(&candidate.objects)?;
        let old_ids = old_buffers.keys().cloned().collect::<HashSet<_>>();
        let new_ids = used.keys().cloned().collect::<HashSet<_>>();
        let expected_added = new_ids
            .difference(&old_ids)
            .cloned()
            .collect::<HashSet<_>>();
        let supplied_added = delta
            .added_buffers
            .iter()
            .map(|descriptor| descriptor.buffer_id.clone())
            .collect::<HashSet<_>>();
        if expected_added != supplied_added {
            return Err(invalid_scene(
                "addedBuffers does not exactly describe newly referenced resources",
            ));
        }
        let expected_released = old_ids
            .difference(&new_ids)
            .cloned()
            .collect::<HashSet<_>>();
        let supplied_released = delta
            .released_buffer_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if !supplied_released.is_subset(&expected_released) {
            return Err(invalid_scene(
                "releasedBufferIds contains a resource still referenced by this Figure",
            ));
        }

        let mut referenced_buffers = Vec::with_capacity(new_ids.len());
        let mut ordered_ids = new_ids.into_iter().collect::<Vec<_>>();
        ordered_ids.sort_unstable();
        for id in ordered_ids {
            let descriptor = descriptors
                .get(&id)
                .ok_or_else(|| invalid_scene("object references an unknown bufferId"))?;
            if used.get(&id) != Some(descriptor) {
                return Err(invalid_scene(
                    "immutable bufferId descriptor changed without a new identifier",
                ));
            }
            referenced_buffers.push(descriptor.clone());
        }
        candidate.referenced_buffers = referenced_buffers;
        candidate.validate(self.limits)?;
        self.snapshot = candidate;
        Ok(())
    }
}

fn validate_hierarchy(
    objects: &[GraphicsObject],
    indexes: &HashMap<&str, usize>,
    root_index: usize,
) -> Result<(), ValidationError> {
    for object in objects {
        let fields = object.fields();
        if object.kind() != ObjectKind::Figure && fields.parent_id.is_none() {
            return Err(invalid_scene("non-root object is missing parentId"));
        }
        if matches!(
            object.kind(),
            ObjectKind::LineSeries
                | ObjectKind::ScatterSeries
                | ObjectKind::SurfaceSeries
                | ObjectKind::PatchSeries
                | ObjectKind::Text
                | ObjectKind::Legend
                | ObjectKind::ColorBar
        ) && !fields.children.is_empty()
        {
            return Err(invalid_scene("leaf object contains children"));
        }
        for child_id in &fields.children {
            let child = &objects[*indexes
                .get(child_id.as_str())
                .ok_or_else(|| invalid_scene("child id does not resolve"))?];
            if child.fields().parent_id.as_deref() != Some(fields.id.as_str()) {
                return Err(invalid_scene(
                    "child parentId does not point back to parent",
                ));
            }
            if !allowed_child(object.kind(), child.kind()) {
                return Err(invalid_scene("object hierarchy kind is not allowed in v1"));
            }
        }
        if let Some(parent_id) = fields.parent_id.as_deref() {
            let parent = &objects[*indexes
                .get(parent_id)
                .ok_or_else(|| invalid_scene("parentId does not resolve"))?];
            if parent
                .fields()
                .children
                .iter()
                .filter(|child| child.as_str() == fields.id)
                .count()
                != 1
            {
                return Err(invalid_scene(
                    "non-root object must occur exactly once in its parent children",
                ));
            }
        }
    }

    let mut visited = HashSet::with_capacity(objects.len());
    let mut stack = vec![root_index];
    while let Some(index) = stack.pop() {
        if !visited.insert(index) {
            return Err(invalid_scene("snapshot hierarchy contains a cycle"));
        }
        for child in &objects[index].fields().children {
            stack.push(indexes[child.as_str()]);
        }
    }
    if visited.len() != objects.len() {
        return Err(invalid_scene(
            "every snapshot object must be reachable from the Figure root",
        ));
    }
    Ok(())
}

fn validate_cross_object_properties(
    objects: &[GraphicsObject],
    indexes: &HashMap<&str, usize>,
) -> Result<(), ValidationError> {
    for object in objects {
        match object {
            GraphicsObject::Axes2d { fields, properties } => {
                for (id, role) in [
                    (properties.title_id.as_deref(), TextRole::Title),
                    (properties.x_label_id.as_deref(), TextRole::XLabel),
                    (properties.y_label_id.as_deref(), TextRole::YLabel),
                    (properties.z_label_id.as_deref(), TextRole::ZLabel),
                ] {
                    let Some(id) = id else { continue };
                    let referenced = &objects[*indexes
                        .get(id)
                        .ok_or_else(|| invalid_scene("Axes text identifier does not resolve"))?];
                    let GraphicsObject::Text {
                        fields: text_fields,
                        properties: text,
                    } = referenced
                    else {
                        return Err(invalid_scene("Axes label identifier must refer to Text"));
                    };
                    if text.role != role || text_fields.parent_id.as_deref() != Some(&fields.id) {
                        return Err(invalid_scene(
                            "Axes label role or parent does not match its property",
                        ));
                    }
                }
            }
            GraphicsObject::Legend { fields, properties } => {
                for id in &properties.series_ids {
                    let series = &objects[*indexes.get(id.as_str()).ok_or_else(|| {
                        invalid_scene("Legend series identifier does not resolve")
                    })?];
                    if !matches!(
                        series.kind(),
                        ObjectKind::LineSeries
                            | ObjectKind::ScatterSeries
                            | ObjectKind::SurfaceSeries
                            | ObjectKind::PatchSeries
                            | ObjectKind::ChartGroup
                    ) || series.fields().parent_id != fields.parent_id
                    {
                        return Err(invalid_scene(
                            "Legend membership must refer to a sibling plottable series",
                        ));
                    }
                }
            }
            GraphicsObject::Figure { .. }
            | GraphicsObject::ChartGroup { .. }
            | GraphicsObject::LineSeries { .. }
            | GraphicsObject::ScatterSeries { .. }
            | GraphicsObject::SurfaceSeries { .. }
            | GraphicsObject::PatchSeries { .. }
            | GraphicsObject::Text { .. }
            | GraphicsObject::ColorBar { .. } => {}
        }
    }
    Ok(())
}

fn validate_referenced_buffers(
    objects: &[GraphicsObject],
    listed: &[DataRef],
    limits: GraphicsLimits,
) -> Result<(), ValidationError> {
    let used = collect_data_refs(objects)?;
    let listed = descriptor_map(listed)?;
    for descriptor in listed.values() {
        descriptor.validate(limits)?;
    }
    if used != listed {
        return Err(invalid_scene(
            "referencedBuffers must contain every used DataRef exactly once",
        ));
    }
    Ok(())
}

fn collect_data_refs(
    objects: &[GraphicsObject],
) -> Result<HashMap<String, DataRef>, ValidationError> {
    let mut refs = HashMap::new();
    for object in objects {
        for descriptor in object.data_refs() {
            if refs
                .insert(descriptor.buffer_id.clone(), descriptor.clone())
                .is_some()
            {
                return Err(invalid_scene(
                    "first-tranche series resources must use independent bufferIds",
                ));
            }
        }
    }
    Ok(refs)
}

fn descriptor_map(listed: &[DataRef]) -> Result<HashMap<String, DataRef>, ValidationError> {
    let mut refs = HashMap::with_capacity(listed.len());
    for descriptor in listed {
        if refs
            .insert(descriptor.buffer_id.clone(), descriptor.clone())
            .is_some()
        {
            return Err(invalid_scene("duplicate bufferId descriptor"));
        }
    }
    Ok(refs)
}

fn apply_operation(
    snapshot: &mut FigureSnapshot,
    operation: &DeltaOperation,
) -> Result<(), ValidationError> {
    match operation {
        DeltaOperation::UpsertObject { object } => {
            if let Some(index) = snapshot
                .objects
                .iter()
                .position(|existing| existing.fields().id == object.fields().id)
            {
                let previous = snapshot.objects[index].fields();
                if previous.generation != object.fields().generation {
                    return Err(invalid_scene(
                        "upsertObject cannot change an existing object's generation",
                    ));
                }
                let expected = previous
                    .object_revision
                    .checked_add(1)
                    .filter(|revision| *revision <= crate::MAX_SAFE_INTEGER)
                    .ok_or_else(|| invalid_scene("objectRevision safe integer exhausted"))?;
                if object.fields().object_revision != expected {
                    return Err(invalid_scene(
                        "upsertObject objectRevision must increase by exactly one",
                    ));
                }
                snapshot.objects[index] = object.clone();
            } else {
                if object.fields().object_revision != 1 {
                    return Err(invalid_scene(
                        "new objects must start at objectRevision one",
                    ));
                }
                snapshot.objects.push(object.clone());
            }
        }
        DeltaOperation::DeleteObject { id, generation } => {
            let object = snapshot
                .objects
                .iter()
                .find(|object| object.fields().id == *id)
                .ok_or_else(|| invalid_scene("deleteObject id does not resolve"))?;
            if object.fields().generation != *generation {
                return Err(invalid_scene("deleteObject generation does not match"));
            }
            let mut deleted = HashSet::new();
            let mut stack = vec![id.clone()];
            while let Some(current) = stack.pop() {
                if deleted.insert(current.clone())
                    && let Some(object) = snapshot
                        .objects
                        .iter()
                        .find(|object| object.fields().id == current)
                {
                    stack.extend(object.fields().children.iter().cloned());
                }
            }
            snapshot
                .objects
                .retain(|object| !deleted.contains(&object.fields().id));
            for object in &mut snapshot.objects {
                object
                    .fields_mut()
                    .children
                    .retain(|child| !deleted.contains(child));
            }
        }
        DeltaOperation::ReorderChildren {
            parent_id,
            children,
        } => {
            let parent = snapshot
                .objects
                .iter_mut()
                .find(|object| object.fields().id == *parent_id)
                .ok_or_else(|| invalid_scene("reorderChildren parentId does not resolve"))?;
            parent.fields_mut().children.clone_from(children);
        }
        DeltaOperation::SetRoot { root_id } => snapshot.root_id.clone_from(root_id),
    }
    Ok(())
}

const fn allowed_child(parent: ObjectKind, child: ObjectKind) -> bool {
    matches!(
        (parent, child),
        (ObjectKind::Figure, ObjectKind::Axes2d)
            | (
                ObjectKind::Axes2d,
                ObjectKind::ChartGroup
                    | ObjectKind::LineSeries
                    | ObjectKind::ScatterSeries
                    | ObjectKind::SurfaceSeries
                    | ObjectKind::PatchSeries
                    | ObjectKind::Text
                    | ObjectKind::Legend
                    | ObjectKind::ColorBar
            )
            | (
                ObjectKind::ChartGroup,
                ObjectKind::LineSeries | ObjectKind::PatchSeries
            )
    )
}

fn validate_scene_revision(value: u64, field: &str) -> Result<(), ValidationError> {
    validate_positive_safe(value, field)
        .map_err(|_| invalid_scene(format!("{field} must be a positive JSON-safe integer")))
}

fn validate_nonempty(value: &str, field: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        Err(invalid_scene(format!("{field} must be non-empty")))
    } else {
        Ok(())
    }
}

fn validate_finite<const N: usize>(values: &[f64; N], field: &str) -> Result<(), ValidationError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(invalid_scene(format!("{field} must be finite")))
    }
}

fn validate_finite_positive<const N: usize>(
    values: &[f64; N],
    field: &str,
) -> Result<(), ValidationError> {
    if values.iter().all(|value| value.is_finite() && *value > 0.0) {
        Ok(())
    } else {
        Err(invalid_scene(format!(
            "{field} must be finite and positive"
        )))
    }
}

fn validate_normalized_position(position: [f64; 4]) -> Result<(), ValidationError> {
    validate_finite(&position, "Axes positionNormalized")?;
    let [x, y, width, height] = position;
    let right = x + width;
    let top = y + height;
    if x >= 0.0 && y >= 0.0 && width > 0.0 && height > 0.0 && right <= 1.0 && top <= 1.0 {
        Ok(())
    } else {
        Err(invalid_scene(
            "Axes positionNormalized must be a positive rectangle inside [0,1]^2",
        ))
    }
}

fn validate_figure_position(position: [f64; 4]) -> Result<(), ValidationError> {
    validate_finite(&position, "Figure positionCssPixels")?;
    if position[2] > 0.0 && position[3] > 0.0 {
        Ok(())
    } else {
        Err(invalid_scene(
            "Figure positionCssPixels width and height must be positive",
        ))
    }
}

fn validate_font_weight(weight: u16, field: &str) -> Result<(), ValidationError> {
    if (1..=1000).contains(&weight) {
        Ok(())
    } else {
        Err(invalid_scene(format!("{field} must be in 1..=1000")))
    }
}

fn validate_limits(values: [f64; 2], field: &str) -> Result<(), ValidationError> {
    validate_finite(&values, field)?;
    if values[0] < values[1] {
        Ok(())
    } else {
        Err(invalid_scene(format!(
            "{field} must be strictly increasing"
        )))
    }
}

fn validate_tick_values(values: &[TickValue], field: &str) -> Result<(), ValidationError> {
    if values.iter().any(|value| value.get().is_nan()) {
        return Err(invalid_scene(format!("{field} must not contain NaN")));
    }
    if values.windows(2).any(|pair| pair[0].get() >= pair[1].get()) {
        return Err(invalid_scene(format!(
            "{field} must be strictly increasing without duplicates"
        )));
    }
    Ok(())
}

fn validate_positive(value: f64, field: &str) -> Result<(), ValidationError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(invalid_scene(format!(
            "{field} must be finite and positive"
        )))
    }
}

fn invalid_scene(message: impl Into<String>) -> ValidationError {
    ValidationError::new(ErrorCategory::InvalidScene, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataType, Endianness, StorageOrder};

    fn data_ref(id: &str, elements: u64) -> DataRef {
        DataRef {
            buffer_id: id.to_owned(),
            dtype: DataType::F64,
            shape: vec![1, elements],
            order: StorageOrder::ColumnMajor,
            endianness: Endianness::Little,
            byte_offset: 0,
            byte_length: elements * 8,
        }
    }

    fn object_fields(id: &str, parent_id: Option<&str>, children: &[&str]) -> ObjectFields {
        ObjectFields {
            id: id.to_owned(),
            generation: 1,
            object_revision: 1,
            parent_id: parent_id.map(str::to_owned),
            children: children.iter().map(ToString::to_string).collect(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn valid_snapshot() -> FigureSnapshot {
        let x = data_ref("x", 3);
        let y = data_ref("y", 3);
        FigureSnapshot {
            snapshot_type: "figureSnapshot".to_owned(),
            figure_id: "figure-1".to_owned(),
            revision: 1,
            root_id: "figure-object".to_owned(),
            objects: vec![
                GraphicsObject::Figure {
                    fields: object_fields("figure-object", None, &["axes"]),
                    properties: FigureProperties {
                        number: 1,
                        name_code_units: vec![],
                        number_title: true,
                        visible: true,
                        background_rgba: Rgba([1.0, 1.0, 1.0, 1.0]),
                        initial_logical_size_css_pixels: [640.0, 480.0],
                        position_css_pixels: [100.0, 100.0, 640.0, 480.0],
                        next_plot: FigureNextPlot::Add,
                    },
                },
                GraphicsObject::Axes2d {
                    fields: object_fields("axes", Some("figure-object"), &["line"]),
                    properties: Axes2DProperties {
                        coordinate_system: AxesCoordinateSystem::Cartesian,
                        position_normalized: [0.1, 0.1, 0.8, 0.8],
                        background_rgba: default_axes_background_rgba(),
                        theta_axis_units: ThetaAxisUnits::Degrees,
                        theta_direction: ThetaDirection::Counterclockwise,
                        theta_zero_location: ThetaZeroLocation::Right,
                        r_axis_location: 80.0,
                        x_scale: Scale::Linear,
                        y_scale: Scale::Linear,
                        z_scale: Scale::Linear,
                        x_direction: AxisDirection::Normal,
                        y_direction: AxisDirection::Normal,
                        z_direction: AxisDirection::Normal,
                        visible: true,
                        x_limits: [0.0, 2.0],
                        y_limits: [1.0, 3.0],
                        z_limits: [0.0, 1.0],
                        x_limits_mode: AxesLimitMode::Auto,
                        y_limits_mode: AxesLimitMode::Auto,
                        z_limits_mode: AxesLimitMode::Auto,
                        next_plot: NextPlot::Replace,
                        grid_x: false,
                        grid_y: false,
                        grid_z: false,
                        minor_grid_x: false,
                        minor_grid_y: false,
                        minor_grid_z: false,
                        color_order_index: 1,
                        x_tick: Vec::new(),
                        y_tick: Vec::new(),
                        z_tick: Vec::new(),
                        x_tick_label_code_units: Vec::new(),
                        y_tick_label_code_units: Vec::new(),
                        z_tick_label_code_units: Vec::new(),
                        x_tick_mode: AxesTickMode::Auto,
                        y_tick_mode: AxesTickMode::Auto,
                        z_tick_mode: AxesTickMode::Auto,
                        x_tick_label_mode: AxesTickMode::Auto,
                        y_tick_label_mode: AxesTickMode::Auto,
                        z_tick_label_mode: AxesTickMode::Auto,
                        view: [0.0, 90.0],
                        camera_scale: 1.0,
                        projection: Projection::Orthographic,
                        data_aspect_ratio: [1.0, 1.0, 1.0],
                        data_aspect_ratio_mode: AxesLimitMode::Auto,
                        plot_box_aspect_ratio: [1.0, 1.0, 1.0],
                        plot_box_aspect_ratio_mode: AxesLimitMode::Auto,
                        c_limits: [0.0, 1.0],
                        c_limits_mode: AxesLimitMode::Auto,
                        colormap: None,
                        colorbar_visible: false,
                        box_enabled: false,
                        font_size_css_px: 40.0 / 3.0,
                        font_family_code_units: Vec::new(),
                        tick_direction: TickDirection::In,
                        line_width_css_px: 2.0 / 3.0,
                        tick_label_interpreter: TextInterpreter::Tex,
                        title_id: Nullable(None),
                        x_label_id: Nullable(None),
                        y_label_id: Nullable(None),
                        z_label_id: Nullable(None),
                    },
                },
                GraphicsObject::LineSeries {
                    fields: object_fields("line", Some("axes"), &[]),
                    properties: LineSeriesProperties {
                        x_data: x.clone(),
                        y_data: y.clone(),
                        z_data: Nullable(None),
                        color_rgba: Rgba([0.0, 0.45, 0.74, 1.0]),
                        line_width_css_px: 1.5,
                        line_style: LineStyle::Solid,
                        marker: Marker::None,
                        marker_size_css_px: 6.0,
                        marker_face_rgba: None,
                        marker_edge_rgba: None,
                        marker_indices: None,
                        display_name_code_units: vec![0x03b1],
                        visible: true,
                        clipping: true,
                    },
                },
            ],
            referenced_buffers: vec![x, y],
        }
    }

    #[test]
    fn validates_typed_snapshot_and_utf16_wire_schema() {
        let snapshot = valid_snapshot();
        snapshot.validate(GraphicsLimits::default()).unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            encoded["objects"][2]["properties"]["displayNameCodeUnits"],
            serde_json::json!([945])
        );
        assert_eq!(encoded["objects"][2]["kind"], "lineSeries");
    }

    #[test]
    fn chart_group_retains_public_identity_and_primitive_hierarchy() {
        let mut snapshot = valid_snapshot();
        let GraphicsObject::Axes2d { fields, .. } = &mut snapshot.objects[1] else {
            panic!()
        };
        fields.children = vec!["stairs".to_owned()];
        let GraphicsObject::LineSeries { fields, .. } = &mut snapshot.objects[2] else {
            panic!()
        };
        fields.parent_id = Some("stairs".to_owned());
        snapshot.objects.insert(
            2,
            GraphicsObject::ChartGroup {
                fields: object_fields("stairs", Some("axes"), &["line"]),
                properties: ChartGroupProperties {
                    chart_type: ChartType::Stair,
                    visible: true,
                    color: Nullable(Some(ChartColorProperties {
                        mode: ChartColorMode::Auto,
                        rgba: Nullable(None),
                    })),
                    line_width_css_px: 2.0 / 3.0,
                    line_style: LineStyle::Solid,
                    marker: Nullable(Some(Marker::None)),
                    marker_size_css_px: Nullable(Some(8.0)),
                    marker_face_color: Nullable(Some(ChartColorProperties {
                        mode: ChartColorMode::None,
                        rgba: Nullable(None),
                    })),
                    marker_edge_color: Nullable(Some(ChartColorProperties {
                        mode: ChartColorMode::Auto,
                        rgba: Nullable(None),
                    })),
                    face_color: Nullable(None),
                    edge_color: Nullable(None),
                    face_alpha: Nullable(None),
                    edge_alpha: Nullable(None),
                    base_value: Nullable(None),
                    bar_width: Nullable(None),
                    cap_size_css_px: Nullable(None),
                },
            },
        );
        snapshot.validate(GraphicsLimits::default()).unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(encoded["objects"][2]["kind"], "chartGroup");
        assert_eq!(encoded["objects"][2]["properties"]["chartType"], "stair");
    }

    #[test]
    fn graphics_v4_gates_logarithmic_reversed_and_hidden_axes() {
        let mut snapshot = valid_snapshot();
        let legacy = serde_json::to_value(&snapshot).unwrap();
        let legacy_axes = &legacy["objects"][1]["properties"];
        assert!(legacy_axes.get("zScale").is_none());
        assert!(legacy_axes.get("xDirection").is_none());
        assert!(legacy_axes.get("visible").is_none());
        let GraphicsObject::Axes2d { properties, .. } = &mut snapshot.objects[1] else {
            unreachable!();
        };
        properties.x_scale = Scale::Log;
        properties.x_limits = [1.0, 100.0];
        properties.y_direction = AxisDirection::Reverse;
        properties.visible = false;
        snapshot
            .validate_for(GraphicsLimits::default(), GraphicsProtocol::V4)
            .unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        let axes = &encoded["objects"][1]["properties"];
        assert_eq!(axes["xScale"], "log");
        assert_eq!(axes["yDirection"], "reverse");
        assert_eq!(axes["visible"], false);
        assert!(
            snapshot
                .validate_for(GraphicsLimits::default(), GraphicsProtocol::V3)
                .is_err()
        );
    }

    #[test]
    fn additive_visual_fields_have_legacy_defaults_and_exact_extended_values() {
        let mut encoded = serde_json::to_value(valid_snapshot()).unwrap();
        for field in [
            "backgroundRgba",
            "xTick",
            "yTick",
            "xTickLabelCodeUnits",
            "yTickLabelCodeUnits",
            "xTickMode",
            "yTickMode",
            "xTickLabelMode",
            "yTickLabelMode",
            "box",
            "fontSizeCssPx",
            "fontFamilyCodeUnits",
            "tickDirection",
            "minorGridX",
            "minorGridY",
            "minorGridZ",
            "lineWidthCssPx",
            "tickLabelInterpreter",
            "colormap",
        ] {
            encoded["objects"][1]["properties"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        encoded["objects"][2]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("markerIndices");
        let decoded: FigureSnapshot = serde_json::from_value(encoded).unwrap();
        let GraphicsObject::Axes2d { properties, .. } = &decoded.objects[1] else {
            unreachable!();
        };
        assert!(properties.x_tick.is_empty());
        assert_eq!(
            properties.background_rgba,
            Nullable(Some(Rgba([1.0, 1.0, 1.0, 1.0])))
        );
        assert_eq!(properties.x_tick_mode, AxesTickMode::Auto);
        assert_eq!(
            properties.font_size_css_px.to_bits(),
            (40.0_f64 / 3.0).to_bits()
        );
        assert_eq!(
            properties.line_width_css_px.to_bits(),
            (2.0_f64 / 3.0).to_bits()
        );
        assert_eq!(properties.tick_label_interpreter, TextInterpreter::Tex);
        assert!(properties.font_family_code_units.is_empty());
        assert_eq!(properties.tick_direction, TickDirection::In);
        assert!(!properties.minor_grid_x);
        assert!(properties.colormap.is_none());
        let GraphicsObject::LineSeries { properties, .. } = &decoded.objects[2] else {
            unreachable!();
        };
        assert!(properties.marker_indices.is_none());

        let mut exact = valid_snapshot();
        let GraphicsObject::Axes2d { properties, .. } = &mut exact.objects[1] else {
            unreachable!();
        };
        properties.x_tick = vec![
            TickValue::new(f64::NEG_INFINITY).unwrap(),
            TickValue::new(0.0).unwrap(),
            TickValue::new(f64::INFINITY).unwrap(),
        ];
        properties.x_tick_label_code_units = vec!["A".encode_utf16().collect()];
        let GraphicsObject::LineSeries { properties, .. } = &mut exact.objects[2] else {
            unreachable!();
        };
        properties.marker_indices = Some(vec![
            MarkerIndex::new(u64::MAX).unwrap(),
            MarkerIndex::new(1).unwrap(),
            MarkerIndex::new(1).unwrap(),
        ]);
        exact.validate(GraphicsLimits::default()).unwrap();
        let encoded = serde_json::to_value(exact).unwrap();
        assert_eq!(
            encoded["objects"][1]["properties"]["xTick"],
            serde_json::json!(["-Infinity", 0.0, "Infinity"])
        );
        assert_eq!(
            encoded["objects"][2]["properties"]["markerIndices"],
            serde_json::json!([u64::MAX.to_string(), "1", "1"])
        );
    }

    #[test]
    fn polar_axes_fields_are_additive_typed_and_validated() {
        let mut legacy = serde_json::to_value(valid_snapshot()).unwrap();
        for field in [
            "coordinateSystem",
            "thetaAxisUnits",
            "thetaDirection",
            "thetaZeroLocation",
            "rAxisLocation",
        ] {
            legacy["objects"][1]["properties"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        let decoded: FigureSnapshot = serde_json::from_value(legacy).unwrap();
        let GraphicsObject::Axes2d { properties, .. } = &decoded.objects[1] else {
            unreachable!();
        };
        assert_eq!(
            properties.coordinate_system,
            AxesCoordinateSystem::Cartesian
        );
        assert_eq!(properties.theta_axis_units, ThetaAxisUnits::Degrees);
        assert_eq!(properties.theta_direction, ThetaDirection::Counterclockwise);
        assert_eq!(properties.theta_zero_location, ThetaZeroLocation::Right);
        assert!((properties.r_axis_location - 80.0).abs() <= f64::EPSILON);

        let mut polar = valid_snapshot();
        let GraphicsObject::Axes2d { properties, .. } = &mut polar.objects[1] else {
            unreachable!();
        };
        properties.coordinate_system = AxesCoordinateSystem::Polar;
        properties.theta_axis_units = ThetaAxisUnits::Radians;
        properties.theta_direction = ThetaDirection::Clockwise;
        properties.theta_zero_location = ThetaZeroLocation::Top;
        properties.r_axis_location = std::f64::consts::FRAC_PI_2;
        polar.validate(GraphicsLimits::default()).unwrap();
        let encoded = serde_json::to_value(&polar).unwrap();
        let axes = &encoded["objects"][1]["properties"];
        assert_eq!(axes["coordinateSystem"], "polar");
        assert_eq!(axes["thetaAxisUnits"], "radians");
        assert_eq!(axes["thetaDirection"], "clockwise");
        assert_eq!(axes["thetaZeroLocation"], "top");

        let GraphicsObject::Axes2d { properties, .. } = &mut polar.objects[1] else {
            unreachable!();
        };
        properties.r_axis_location = f64::NAN;
        assert!(polar.validate(GraphicsLimits::default()).is_err());
    }

    #[test]
    fn visual_field_validation_rejects_bad_ticks_and_marker_index_encodings() {
        let mut snapshot = valid_snapshot();
        let GraphicsObject::Axes2d { properties, .. } = &mut snapshot.objects[1] else {
            unreachable!();
        };
        properties.x_tick = vec![TickValue(1.0), TickValue(1.0)];
        assert_eq!(
            snapshot
                .validate(GraphicsLimits::default())
                .unwrap_err()
                .category(),
            ErrorCategory::InvalidScene
        );

        let mut snapshot = valid_snapshot();
        let GraphicsObject::Axes2d { properties, .. } = &mut snapshot.objects[1] else {
            unreachable!();
        };
        properties.colormap = Some(vec![[0.0, 1.25, 0.0]]);
        assert_eq!(
            snapshot
                .validate(GraphicsLimits::default())
                .unwrap_err()
                .category(),
            ErrorCategory::InvalidScene
        );

        let encoded = serde_json::to_value(valid_snapshot()).unwrap();
        for invalid in [
            serde_json::json!(["0"]),
            serde_json::json!([1]),
            serde_json::json!(["01"]),
        ] {
            let mut candidate = encoded.clone();
            candidate["objects"][2]["properties"]["markerIndices"] = invalid;
            assert!(serde_json::from_value::<FigureSnapshot>(candidate).is_err());
        }
    }

    #[test]
    fn rejects_non_contiguous_series_shape_and_missing_resource() {
        let mut snapshot = valid_snapshot();
        let GraphicsObject::LineSeries { properties, .. } = &mut snapshot.objects[2] else {
            unreachable!();
        };
        properties.x_data.shape = vec![3, 1];
        assert_eq!(
            snapshot
                .validate(GraphicsLimits::default())
                .unwrap_err()
                .category(),
            ErrorCategory::InvalidScene
        );

        let mut snapshot = valid_snapshot();
        snapshot.referenced_buffers.pop();
        assert!(snapshot.validate(GraphicsLimits::default()).is_err());
    }

    #[test]
    fn rejects_cycles_duplicates_bad_color_and_unknown_kind() {
        let mut cycle = valid_snapshot();
        cycle.objects[0].fields_mut().parent_id = Some("line".to_owned());
        assert!(cycle.validate(GraphicsLimits::default()).is_err());

        let mut duplicate = valid_snapshot();
        duplicate.objects[0]
            .fields_mut()
            .children
            .push("axes".to_owned());
        assert!(duplicate.validate(GraphicsLimits::default()).is_err());

        let mut bad_color = valid_snapshot();
        let GraphicsObject::LineSeries { properties, .. } = &mut bad_color.objects[2] else {
            unreachable!();
        };
        properties.color_rgba = Rgba([2.0, 0.0, 0.0, 1.0]);
        assert!(bad_color.validate(GraphicsLimits::default()).is_err());

        let unknown = serde_json::json!({
            "id": "future",
            "generation": 1,
            "objectRevision": 1,
            "kind": "surfaceSeries",
            "parentId": null,
            "children": [],
            "properties": {}
        });
        assert!(serde_json::from_value::<GraphicsObject>(unknown).is_err());
    }

    #[test]
    fn required_nullable_and_property_members_cannot_be_omitted() {
        let encoded = serde_json::to_value(valid_snapshot()).unwrap();
        assert!(encoded["objects"][1]["properties"]["titleId"].is_null());

        let mut missing_null = encoded.clone();
        missing_null["objects"][1]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("titleId");
        assert!(serde_json::from_value::<FigureSnapshot>(missing_null).is_err());

        let mut missing_required = encoded;
        missing_required["objects"][0]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("nextPlot");
        assert!(serde_json::from_value::<FigureSnapshot>(missing_required).is_err());
    }

    #[test]
    fn exact_v1_enums_reject_matlab_alias_spellings() {
        let encoded = serde_json::to_value(valid_snapshot()).unwrap();
        let mut bad_line_style = encoded.clone();
        bad_line_style["objects"][2]["properties"]["lineStyle"] = serde_json::json!("dashed");
        assert!(serde_json::from_value::<FigureSnapshot>(bad_line_style).is_err());

        let mut bad_marker = encoded;
        bad_marker["objects"][2]["properties"]["marker"] = serde_json::json!("asterisk");
        assert!(serde_json::from_value::<FigureSnapshot>(bad_marker).is_err());
    }

    #[test]
    fn applies_exact_revision_delta_atomically() {
        let snapshot = valid_snapshot();
        let old = snapshot.clone();
        let mut state = SceneState::new(snapshot, GraphicsLimits::default()).unwrap();
        let replacement = data_ref("y-2", 3);
        let mut line = state.snapshot.objects[2].clone();
        line.fields_mut().object_revision = 2;
        let GraphicsObject::LineSeries { properties, .. } = &mut line else {
            unreachable!();
        };
        properties.y_data = replacement.clone();
        let delta = FigureDelta {
            figure_id: "figure-1".to_owned(),
            base_revision: 1,
            revision: 2,
            operations: vec![DeltaOperation::UpsertObject { object: line }],
            added_buffers: vec![replacement],
            released_buffer_ids: vec!["y".to_owned()],
        };
        state.apply_delta(&delta).unwrap();
        assert_eq!(state.snapshot.revision, 2);

        let mut invalid = delta;
        invalid.base_revision = 2;
        invalid.revision = 4;
        assert!(state.apply_delta(&invalid).is_err());
        assert_eq!(state.snapshot.revision, 2);
        assert_ne!(state.snapshot(), &old);
    }

    #[test]
    fn revision_safe_integer_exhaustion_changes_nothing() {
        let mut snapshot = valid_snapshot();
        snapshot.revision = crate::MAX_SAFE_INTEGER;
        for object in &mut snapshot.objects {
            object.fields_mut().object_revision = crate::MAX_SAFE_INTEGER;
        }
        let mut state = SceneState::new(snapshot, GraphicsLimits::default()).unwrap();
        let before = state.snapshot.clone();
        let delta = FigureDelta {
            figure_id: "figure-1".to_owned(),
            base_revision: crate::MAX_SAFE_INTEGER,
            revision: crate::MAX_SAFE_INTEGER,
            operations: vec![],
            added_buffers: vec![],
            released_buffer_ids: vec![],
        };
        assert!(state.apply_delta(&delta).is_err());
        assert_eq!(state.snapshot, before);
    }

    #[test]
    fn reorder_requires_parent_upsert_and_advances_object_revision() {
        let mut state = SceneState::new(valid_snapshot(), GraphicsLimits::default()).unwrap();
        let standalone = FigureDelta {
            figure_id: "figure-1".to_owned(),
            base_revision: 1,
            revision: 2,
            operations: vec![DeltaOperation::ReorderChildren {
                parent_id: "axes".to_owned(),
                children: vec!["line".to_owned()],
            }],
            added_buffers: vec![],
            released_buffer_ids: vec![],
        };
        assert!(state.apply_delta(&standalone).is_err());
        assert_eq!(state.snapshot().revision, 1);

        let mut axes = state.snapshot().objects[1].clone();
        axes.fields_mut().object_revision = 2;
        let valid = FigureDelta {
            operations: vec![
                DeltaOperation::UpsertObject { object: axes },
                DeltaOperation::ReorderChildren {
                    parent_id: "axes".to_owned(),
                    children: vec!["line".to_owned()],
                },
            ],
            ..standalone
        };
        state.apply_delta(&valid).unwrap();
        assert_eq!(state.snapshot().objects[1].fields().object_revision, 2);
    }
}
