#![doc = "Scientific axes limits, ticks, transforms, and overlay layout planning."]
#![forbid(unsafe_code)]

mod axes;
mod bounds;
mod error;
mod limits;
mod polar;
mod text;
mod ticks;
mod transform;

pub use axes::{
    LinearAxesFirstPass, LinearAxesLayoutInput, LinearAxesStyle, TickDirection,
    build_linear_axes_layout,
};
pub use bounds::{BoundsSummary, NumericView, SeriesLayoutInput};
pub use error::LayoutError;
pub use limits::{
    AutoLimitOptions, AutoLimitReason, LimitMode, Limits, LocalViewLimits, ResolvedSemanticAxes,
    ResolvedSemanticAxis, SemanticAxesRequest, SemanticAxisRequest, SemanticLimits,
    resolve_semantic_axes, resolve_semantic_axis,
};
pub use polar::{
    PolarAxesFirstPass, PolarAxesLayoutInput, PolarThetaDirection, PolarZeroLocation,
    build_polar_axes_layout,
};
pub use text::{
    FirstLayoutPass, FontKey, LegendEntryIntent, LegendIntent, LegendPlacement, TextIntent,
    TextMeasureRequest, TextMeasurement, TextMetrics,
};
pub use ticks::{
    Tick, TickDomain, TickSet, explicit_ticks, linear_ticks, linear_ticks_r2022b, log_ticks,
};
pub use transform::{AxesTransform, AxisDirection, AxisScale, ViewportMapping};
