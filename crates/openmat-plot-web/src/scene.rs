use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use openmat_graphics_model::PARULA_R2022B;
use openmat_plot_geometry::{
    DataBounds3D, DataPoint as GeometryDataPoint, GeometryError, LineRun, LinearLodView,
    LodCacheKey, LodResourceRevision, LodSourceKey, MirFrameBuilder, NumericView, OrbitCamera3D,
    PatchColors, PatchMeshView, PlotBoxFit3D, PlotBoxFitMode3D, Point3D, Projection3D,
    ScatterStyle, SeriesGeometryInput, SurfaceColors, SurfaceGridView, fit_plot_box_3d,
    line_runs_from_lod, min_max_lod, patch_mesh_edges, patch_mesh_edges_2d, ruler_edge_3d,
    select_projected_rulers_3d, surface_grid_edges, tessellate_patch_mesh,
    tessellate_patch_mesh_2d, tessellate_surface_grid,
};
use openmat_plot_layout::{
    AxesTransform, AxisDirection as LayoutAxisDirection, AxisScale as LayoutAxisScale,
    FirstLayoutPass, FontKey, LegendEntryIntent, LegendIntent, LegendPlacement, Limits,
    LinearAxesLayoutInput, LinearAxesStyle, PolarAxesLayoutInput, PolarThetaDirection,
    PolarZeroLocation, SemanticLimits, TextIntent, TextMeasurement, TextMetrics,
    TickDirection as LayoutTickDirection, TickDomain, TickSet, ViewportMapping,
    build_linear_axes_layout, build_polar_axes_layout, explicit_ticks, linear_ticks,
    linear_ticks_r2022b, log_ticks,
};
use openmat_plot_mir::{
    AxesPoint, AxesPoint3D, AxesRect, AxisDimension, CssLineSegment, CssPoint, CssPx, CssRect,
    DevicePixelRatio, DrawOrder, FontWeight, HorizontalAlignment, Lighting3D, LineCap, LineJoin,
    LinePattern3D, LineSegment3D, LineSegmentBatch3D, MarkerBatch, MarkerBatch3D, MarkerInstance,
    MarkerInstance3D, MarkerShape, OverlayAxisLine, OverlayFontStyle, OverlayPlan, OverlayTextRole,
    OverlayTick, PlotFrame, Rgba, RulerLine3D, RulerLineBatch3D, RulerSelection3D, ScreenLine,
    ScreenLineBatch, StrokeStyle, TextInterpreter as MirTextInterpreter, TickMark3D,
    TickMarkBatch3D, TriangleMesh2D, TriangleVertex2D, Utf16Text, VerticalAlignment,
    ViewProjection3D,
};
use openmat_plot_protocol::{
    Axes2DProperties, AxesCoordinateSystem, AxesTickMode, AxisDirection as WireAxisDirection,
    ColorBarProperties, DataRef, DataType, FigureSnapshot, FontStyle, GraphicsObject,
    HorizontalAlignment as WireHorizontalAlignment, LegendLocation,
    LegendOrientation as WireLegendOrientation, LineSeriesProperties, LineStyle, Marker,
    PatchSeriesProperties, Projection as WireProjection, Rgba as WireRgba, Scale as WireScale,
    ScatterColorTarget, ScatterSeriesProperties, SurfaceColorMode, SurfaceSeriesProperties,
    TextInterpreter as WireTextInterpreter, TextProperties, TextRole, ThetaAxisUnits,
    ThetaDirection, ThetaZeroLocation, TickDirection as WireTickDirection, TickValue,
    VerticalAlignment as WireVerticalAlignment,
};
use serde::Serialize;

use crate::{CanvasSize, PlotWebError, PlotWebErrorKind};

#[cfg(test)]
use openmat_plot_protocol::LegendProperties;

const MAX_LOD_CACHE_ENTRIES: usize = 16;
const DIRECT_POINTS_PER_DEVICE_COLUMN: usize = 4;
const MIN_DIRECT_POINT_BUDGET: usize = 2_048;
const PLOT_BOX_INSET_CSS_PX: f64 = 8.0;
const COLORBAR_IDEAL_TICK_SPACING_CSS_PX: f32 = 26.0;
const COLORBAR_MAX_TICK_INTERVALS: u32 = 10;
const COLORBAR_TICK_LENGTH_CSS_PX: f32 = 4.0;

#[derive(Debug, Default)]
pub(crate) struct SceneCache {
    lod_runs: HashMap<LodCacheKey, Arc<Vec<LineRun>>>,
    lod_order: VecDeque<LodCacheKey>,
}

impl SceneCache {
    fn get_or_build_lod(
        &mut self,
        key: LodCacheKey,
        input: SeriesGeometryInput<'_>,
        view: LinearLodView,
    ) -> Result<Arc<Vec<LineRun>>, GeometryError> {
        if let Some(runs) = self.lod_runs.get(&key) {
            return Ok(Arc::clone(runs));
        }
        let output = min_max_lod(input, view)?;
        let runs = Arc::new(line_runs_from_lod(&output.samples));
        if self.lod_runs.len() >= MAX_LOD_CACHE_ENTRIES
            && let Some(oldest) = self.lod_order.pop_front()
        {
            self.lod_runs.remove(&oldest);
        }
        self.lod_order.push_back(key.clone());
        self.lod_runs.insert(key, Arc::clone(&runs));
        Ok(runs)
    }
}

#[derive(Debug)]
pub(crate) struct PreparedScene {
    pub(crate) axes_id: Option<String>,
    pub(crate) frame: PlotFrame,
    pub(crate) clear_color: Rgba,
    pub(crate) camera_3d: Option<(OrbitCamera3D, f64, [f64; 3], PlotBoxFit3D)>,
    pub(crate) additional_axes: Vec<PreparedAxesScene>,
    canvas_width_css_px: f64,
    canvas_height_css_px: f64,
    text_pass: Option<FirstLayoutPass>,
}

#[derive(Debug)]
pub(crate) struct PreparedAxesScene {
    pub(crate) axes_id: String,
    pub(crate) frame: PlotFrame,
    pub(crate) camera_3d: Option<(OrbitCamera3D, f64, [f64; 3], PlotBoxFit3D)>,
    text_pass: FirstLayoutPass,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct TextLayoutState {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pass: FirstLayoutPass,
    passes: Vec<TextLayoutPassState>,
    canvas_width_css_px: f64,
    canvas_height_css_px: f64,
}

#[derive(Clone, Debug)]
struct TextLayoutPassState {
    key_prefix: String,
    pass: FirstLayoutPass,
}

#[derive(Debug)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct ProjectedOverlay {
    pub(crate) overlay: OverlayDto,
    pub(crate) text_layout: TextLayoutState,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayDto {
    canvas_width_css_px: f64,
    canvas_height_css_px: f64,
    axis_lines: Vec<LineDto>,
    ticks: Vec<LineDto>,
    text: Vec<TextDto>,
    legend: Option<LegendDto>,
    legends: Vec<LegendDto>,
    font_cache_revision: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LineDto {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: [f32; 4],
    width_css_px: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TextDto {
    key: String,
    code_units: Vec<u16>,
    x: f32,
    y: f32,
    horizontal_alignment: &'static str,
    vertical_alignment: &'static str,
    rotation_radians: f32,
    color: [f32; 4],
    font_family_code_units: Vec<u16>,
    font_size_css_px: f32,
    font_weight: u16,
    font_style: &'static str,
    interpreter: &'static str,
    measured_width_css_px: f32,
    measured_height_css_px: f32,
    role: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegendDto {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    background: [f32; 4],
    border: [f32; 4],
    entries: Vec<LineDto>,
}

impl PreparedScene {
    pub(crate) fn overlay_dto(&self) -> OverlayDto {
        let multiple = !self.additional_axes.is_empty();
        let mut overlays = Vec::with_capacity(1 + self.additional_axes.len());
        let mut first = overlay_dto(
            &self.frame.overlay,
            self.canvas_width_css_px,
            self.canvas_height_css_px,
            self.text_pass
                .as_ref()
                .map_or(1, |pass| pass.font_cache_revision),
        );
        if multiple {
            prefix_overlay_keys(&mut first, self.axes_id.as_deref().unwrap_or("axes"));
        }
        overlays.push(first);
        for axes in &self.additional_axes {
            let mut overlay = overlay_dto(
                &axes.frame.overlay,
                self.canvas_width_css_px,
                self.canvas_height_css_px,
                axes.text_pass.font_cache_revision,
            );
            prefix_overlay_keys(&mut overlay, &axes.axes_id);
            overlays.push(overlay);
        }
        merge_overlay_dtos(overlays)
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn text_layout_state(&self) -> Option<TextLayoutState> {
        let first = self.text_pass.as_ref()?;
        let multiple = !self.additional_axes.is_empty();
        let mut passes = Vec::with_capacity(1 + self.additional_axes.len());
        passes.push(TextLayoutPassState {
            key_prefix: if multiple {
                self.axes_id
                    .as_deref()
                    .map(|id| format!("{id}::"))
                    .unwrap_or_default()
            } else {
                String::new()
            },
            pass: first.clone(),
        });
        passes.extend(self.additional_axes.iter().map(|axes| TextLayoutPassState {
            key_prefix: format!("{}::", axes.axes_id),
            pass: axes.text_pass.clone(),
        }));
        Some(TextLayoutState {
            pass: first.clone(),
            passes,
            canvas_width_css_px: self.canvas_width_css_px,
            canvas_height_css_px: self.canvas_height_css_px,
        })
    }
}

impl TextLayoutState {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn complete(
        &self,
        font_revision: u64,
        measurements: Vec<TextMeasurement>,
    ) -> Result<OverlayDto, PlotWebError> {
        if self
            .passes
            .iter()
            .any(|state| font_revision != state.pass.font_cache_revision)
        {
            return Err(invalid_scene("The browser font revision is stale."));
        }
        let mut expected = HashMap::new();
        for (pass_index, state) in self.passes.iter().enumerate() {
            for request in &state.pass.requests {
                expected.insert(
                    format!("{}{}", state.key_prefix, request.key),
                    (pass_index, request.key.clone()),
                );
            }
        }
        let mut by_pass = vec![Vec::new(); self.passes.len()];
        for measurement in measurements {
            let Some((pass_index, original_key)) = expected.get(&measurement.key) else {
                return Err(invalid_scene(
                    "Browser text measurements are incomplete or invalid.",
                ));
            };
            by_pass[*pass_index].push(TextMeasurement {
                key: original_key.clone(),
                metrics: measurement.metrics,
            });
        }
        let mut overlays = Vec::with_capacity(self.passes.len());
        for (state, measurements) in self.passes.iter().zip(by_pass) {
            let overlay = state.pass.clone().complete(measurements).map_err(|_| {
                invalid_scene("Browser text measurements are incomplete or invalid.")
            })?;
            let mut dto = overlay_dto(
                &overlay,
                self.canvas_width_css_px,
                self.canvas_height_css_px,
                font_revision,
            );
            if !state.key_prefix.is_empty() {
                prefix_overlay_keys(&mut dto, state.key_prefix.trim_end_matches("::"));
            }
            overlays.push(dto);
        }
        Ok(merge_overlay_dtos(overlays))
    }
}

fn overlay_dto(
    overlay: &OverlayPlan,
    canvas_width_css_px: f64,
    canvas_height_css_px: f64,
    font_cache_revision: u64,
) -> OverlayDto {
    let legend = overlay.legend.as_ref().map(legend_dto);
    OverlayDto {
        canvas_width_css_px,
        canvas_height_css_px,
        // Axes and ticks are retained in OverlayPlan for text layout, but their
        // visual geometry is rendered by WebGPU rather than duplicated in SVG.
        axis_lines: Vec::new(),
        ticks: Vec::new(),
        text: overlay.text.iter().map(text_dto).collect(),
        legend: legend.clone(),
        legends: legend.into_iter().collect(),
        font_cache_revision,
    }
}

fn legend_dto(legend: &openmat_plot_mir::LegendPlan) -> LegendDto {
    LegendDto {
        x: legend.bounds.origin.x,
        y: legend.bounds.origin.y,
        width: legend.bounds.size.width.get(),
        height: legend.bounds.size.height.get(),
        background: rgba_array(legend.background),
        border: rgba_array(legend.border),
        entries: legend
            .entries
            .iter()
            .map(|entry| LineDto {
                x1: entry.swatch.start.x,
                y1: entry.swatch.start.y,
                x2: entry.swatch.end.x,
                y2: entry.swatch.end.y,
                color: rgba_array(entry.color),
                width_css_px: 2.0,
            })
            .collect(),
    }
}

fn prefix_overlay_keys(overlay: &mut OverlayDto, axes_id: &str) {
    for text in &mut overlay.text {
        text.key = format!("{axes_id}::{}", text.key);
    }
}

fn merge_overlay_dtos(mut overlays: Vec<OverlayDto>) -> OverlayDto {
    if overlays.is_empty() {
        return OverlayDto {
            canvas_width_css_px: 0.0,
            canvas_height_css_px: 0.0,
            axis_lines: Vec::new(),
            ticks: Vec::new(),
            text: Vec::new(),
            legend: None,
            legends: Vec::new(),
            font_cache_revision: 1,
        };
    }
    let mut merged = overlays.remove(0);
    for mut overlay in overlays {
        merged.axis_lines.append(&mut overlay.axis_lines);
        merged.ticks.append(&mut overlay.ticks);
        merged.text.append(&mut overlay.text);
        merged.legends.append(&mut overlay.legends);
    }
    merged.legend = merged.legends.first().cloned();
    merged
}

fn text_dto(text: &openmat_plot_mir::OverlayText) -> TextDto {
    TextDto {
        key: text.key.clone(),
        code_units: text.text.code_units.clone(),
        x: text.anchor.x,
        y: text.anchor.y,
        horizontal_alignment: match text.horizontal_alignment {
            HorizontalAlignment::Start => "start",
            HorizontalAlignment::Center => "center",
            HorizontalAlignment::End => "end",
        },
        vertical_alignment: match text.vertical_alignment {
            VerticalAlignment::Top => "top",
            VerticalAlignment::Middle => "middle",
            VerticalAlignment::Baseline => "baseline",
            VerticalAlignment::Bottom => "bottom",
        },
        rotation_radians: text.rotation_radians,
        color: rgba_array(text.color),
        font_family_code_units: text.font.family_utf16.clone(),
        font_size_css_px: text.font.size_css_px.get(),
        font_weight: text.font.weight.get(),
        font_style: match text.font.style {
            OverlayFontStyle::Normal => "normal",
            OverlayFontStyle::Italic => "italic",
        },
        interpreter: match text.interpreter {
            MirTextInterpreter::Tex => "tex",
            MirTextInterpreter::Latex => "latex",
            MirTextInterpreter::None => "none",
        },
        measured_width_css_px: text.measured_size.width.get(),
        measured_height_css_px: text.measured_size.height.get(),
        role: match text.role {
            OverlayTextRole::TickLabel => "tickLabel",
            OverlayTextRole::Title => "title",
            OverlayTextRole::XLabel => "xLabel",
            OverlayTextRole::YLabel => "yLabel",
            OverlayTextRole::ZLabel => "zLabel",
            OverlayTextRole::LegendLabel => "legendLabel",
            OverlayTextRole::Annotation => "annotation",
        },
    }
}

enum NumericBuffer {
    F32(Vec<f32>),
    F64(Vec<f64>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct SourcePoint {
    pub(crate) source_index: usize,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourcePoint3D {
    pub(crate) object_id: String,
    pub(crate) kind: &'static str,
    pub(crate) source_index: usize,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    pub(crate) normalized: AxesPoint3D,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[allow(clippy::too_many_lines)] // One traversal keeps source identity aligned across every 3D object kind.
pub(crate) fn source_points_3d(
    snapshot: &FigureSnapshot,
    buffers: &HashMap<String, Vec<u8>>,
    axes_id: Option<&str>,
) -> Result<Vec<SourcePoint3D>, PlotWebError> {
    let (axes_fields, axes) = snapshot
        .objects
        .iter()
        .find_map(|object| match object {
            GraphicsObject::Axes2d { fields, properties }
                if axes_id.is_none_or(|axes_id| fields.id == axes_id) =>
            {
                Some((fields, properties))
            }
            _ => None,
        })
        .ok_or_else(|| invalid_scene("A 3D Figure requires one Axes."))?;
    let bounds = transformed_bounds_3d(axes)?;
    let mut output = Vec::new();
    for object in &snapshot.objects {
        if object.fields().parent_id.as_deref() != Some(axes_fields.id.as_str()) {
            continue;
        }
        match object {
            GraphicsObject::LineSeries { fields, .. }
            | GraphicsObject::ScatterSeries { fields, .. } => {
                let (x_ref, y_ref, z_ref, kind, visible) = match object {
                    GraphicsObject::LineSeries { properties, .. } => (
                        &properties.x_data,
                        &properties.y_data,
                        properties.z_data.as_ref(),
                        "line3",
                        properties.visible,
                    ),
                    GraphicsObject::ScatterSeries { properties, .. } => (
                        &properties.x_data,
                        &properties.y_data,
                        properties.z_data.as_ref(),
                        "scatter3",
                        properties.visible,
                    ),
                    _ => unreachable!(),
                };
                let Some(z_ref) = z_ref.filter(|_| visible) else {
                    continue;
                };
                let x = decode_buffer(x_ref, buffers)?.values_f64();
                let y = decode_buffer(y_ref, buffers)?.values_f64();
                let z = decode_buffer(z_ref, buffers)?.values_f64();
                append_source_points_3d(&mut output, &fields.id, kind, &x, &y, &z, axes, bounds)?;
            }
            GraphicsObject::SurfaceSeries { fields, properties } if properties.visible => {
                let x = decode_buffer(&properties.x_data, buffers)?.values_f64();
                let y = decode_buffer(&properties.y_data, buffers)?.values_f64();
                let z = decode_buffer(&properties.z_data, buffers)?.values_f64();
                let rows = usize::try_from(properties.z_data.shape[0])
                    .map_err(|_| invalid_scene("Surface row count exceeds browser limits."))?;
                let columns = usize::try_from(properties.z_data.shape[1])
                    .map_err(|_| invalid_scene("Surface column count exceeds browser limits."))?;
                for column in 0..columns {
                    for row in 0..rows {
                        let index = column * rows + row;
                        let x_value = if x.len() == columns {
                            x[column]
                        } else {
                            x[index]
                        };
                        let y_value = if y.len() == rows { y[row] } else { y[index] };
                        let z_value = z[index];
                        push_source_point_3d(
                            &mut output,
                            &fields.id,
                            "surface",
                            index,
                            x_value,
                            y_value,
                            z_value,
                            axes,
                            bounds,
                        )?;
                    }
                }
            }
            GraphicsObject::PatchSeries { fields, properties } if properties.visible => {
                let vertices = decode_buffer(&properties.vertices, buffers)?.values_f64();
                let rows = usize::try_from(properties.vertices.shape[0])
                    .map_err(|_| invalid_scene("Patch vertex count exceeds browser limits."))?;
                let columns = usize::try_from(properties.vertices.shape[1])
                    .map_err(|_| invalid_scene("Patch coordinate count exceeds browser limits."))?;
                for index in 0..rows {
                    let x = vertices[index];
                    let y = vertices[index + rows];
                    let z = if columns == 3 {
                        vertices[index + rows * 2]
                    } else {
                        0.0
                    };
                    push_source_point_3d(
                        &mut output,
                        &fields.id,
                        "patch",
                        index,
                        x,
                        y,
                        z,
                        axes,
                        bounds,
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)] // Source values and transformed Axes state are intentionally kept distinct.
fn append_source_points_3d(
    output: &mut Vec<SourcePoint3D>,
    object_id: &str,
    kind: &'static str,
    x: &[f64],
    y: &[f64],
    z: &[f64],
    axes: &Axes2DProperties,
    bounds: DataBounds3D,
) -> Result<(), PlotWebError> {
    if x.len() != y.len() || x.len() != z.len() {
        return Err(invalid_scene("3D source data lengths differ."));
    }
    for index in 0..x.len() {
        push_source_point_3d(
            output, object_id, kind, index, x[index], y[index], z[index], axes, bounds,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_source_point_3d(
    output: &mut Vec<SourcePoint3D>,
    object_id: &str,
    kind: &'static str,
    source_index: usize,
    x: f64,
    y: f64,
    z: f64,
    axes: &Axes2DProperties,
    bounds: DataBounds3D,
) -> Result<(), PlotWebError> {
    if let Some(normalized) = transform_point_3d(x, y, z, axes, bounds)? {
        output.push(SourcePoint3D {
            object_id: object_id.to_owned(),
            kind,
            source_index,
            x,
            y,
            z,
            normalized,
        });
    }
    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn source_points(
    snapshot: &FigureSnapshot,
    buffers: &HashMap<String, Vec<u8>>,
    object_id: &str,
) -> Result<Vec<SourcePoint>, PlotWebError> {
    let (x_data, y_data) = snapshot
        .objects
        .iter()
        .find_map(|object| match object {
            GraphicsObject::LineSeries { fields, properties } if fields.id == object_id => {
                Some((&properties.x_data, &properties.y_data))
            }
            GraphicsObject::ScatterSeries { fields, properties } if fields.id == object_id => {
                Some((&properties.x_data, &properties.y_data))
            }
            _ => None,
        })
        .ok_or_else(|| invalid_scene("Picked graphics object is not a 2D data series."))?;
    let x = decode_buffer(x_data, buffers)?;
    let y = decode_buffer(y_data, buffers)?;
    let input = SeriesGeometryInput {
        x: Some(x.view()),
        y: y.view(),
    };
    input
        .validate()
        .map_err(|_| invalid_scene("Picked series data is invalid."))?;
    let mut points = Vec::with_capacity(input.y.len());
    for source_index in 0..input.y.len() {
        let x = input
            .x
            .and_then(|values| values.get(source_index))
            .ok_or_else(|| invalid_scene("Picked X data could not be indexed."))?;
        let y = input
            .y
            .get(source_index)
            .ok_or_else(|| invalid_scene("Picked Y data could not be indexed."))?;
        if x.is_finite() && y.is_finite() {
            points.push(SourcePoint { source_index, x, y });
        }
    }
    Ok(points)
}

impl NumericBuffer {
    fn view(&self) -> NumericView<'_> {
        match self {
            Self::F32(values) => NumericView::F32(values),
            Self::F64(values) => NumericView::F64(values),
        }
    }

    fn values_f64(&self) -> Vec<f64> {
        match self {
            Self::F32(values) => values.iter().map(|value| f64::from(*value)).collect(),
            Self::F64(values) => values.clone(),
        }
    }
}

#[allow(clippy::too_many_lines)] // Keeps one transactional HIR-to-MIR lowering path together.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn prepare_scene(
    snapshot: &FigureSnapshot,
    buffers: &HashMap<String, Vec<u8>>,
    size: CanvasSize,
) -> Result<PreparedScene, PlotWebError> {
    prepare_scene_cached(snapshot, buffers, size, &mut SceneCache::default())
}

/// Reprojects the text/legend overlay of a 3D Axes after a renderer-local orbit
/// or zoom. Surface, axes, grid, and tick geometry stay resident on the GPU;
/// only inexpensive text anchors and legend layout are rebuilt.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn projected_overlay_for_camera(
    snapshot: &FigureSnapshot,
    size: CanvasSize,
    axes_id: &str,
    camera: OrbitCamera3D,
    prepared: &PreparedScene,
) -> Result<ProjectedOverlay, PlotWebError> {
    let clear_color = figure_clear_color(snapshot)?;
    let (axes_fields, axes) = snapshot
        .objects
        .iter()
        .find_map(|object| match object {
            GraphicsObject::Axes2d { fields, properties } if fields.id == axes_id => {
                Some((fields, properties))
            }
            _ => None,
        })
        .ok_or_else(|| invalid_scene("A 3D Figure requires one Axes."))?;
    let mapping = axes_mapping(size, axes_render_position(axes, size))?;
    let viewport = mapping.viewport();
    let domains = scaled_tick_domains_3d(axes)?;
    let mut x_ticks = axes_ticks(
        domains[0],
        &axes.x_tick,
        axes.x_tick_mode,
        &axes.x_tick_label_code_units,
        axes.x_tick_label_mode,
        responsive_tick_target(viewport.css.size.width.get(), 72.0, 9),
    )
    .map_err(|_| invalid_scene("Axes X ticks cannot be generated."))?;
    let mut y_ticks = axes_ticks(
        domains[1],
        &axes.y_tick,
        axes.y_tick_mode,
        &axes.y_tick_label_code_units,
        axes.y_tick_label_mode,
        responsive_tick_target(viewport.css.size.height.get(), 52.0, 8),
    )
    .map_err(|_| invalid_scene("Axes Y ticks cannot be generated."))?;
    let mut z_ticks = axes_ticks(
        domains[2],
        &axes.z_tick,
        axes.z_tick_mode,
        &axes.z_tick_label_code_units,
        axes.z_tick_label_mode,
        responsive_tick_target(viewport.css.size.height.get(), 52.0, 8),
    )
    .map_err(|_| invalid_scene("Axes Z ticks cannot be generated."))?;
    if !axes.visible {
        x_ticks.ticks.clear();
        y_ticks.ticks.clear();
        z_ticks.ticks.clear();
    }
    let object_by_id: HashMap<_, _> = snapshot
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect();
    let colorbar = axes_fields.children.iter().find_map(|child_id| {
        match object_by_id.get(child_id.as_str()).copied() {
            Some(GraphicsObject::ColorBar { properties, .. }) => Some(properties),
            _ => None,
        }
    });
    let mut additional_text = axes_text_intents(axes, &object_by_id, mapping)?;
    additional_text.extend(colorbar_text_intents(axes, colorbar, size)?);
    let legend = legend_intent(&axes_fields.children, &object_by_id, axes)?;
    let aspect_ratio =
        f64::from(viewport.css.size.width.get()) / f64::from(viewport.css.size.height.get());
    let axes_scale = resolved_axes_scale(axes)?;
    let plot_box_fit = plot_box_fit_target_3d(axes, viewport.css)?;
    let view_projection =
        resolved_view_projection_3d(camera, aspect_ratio, axes_scale, plot_box_fit)?;
    let style_background = axes_background_color(axes)?.unwrap_or(clear_color);
    let layout = build_projected_axes_3d(
        viewport.css,
        view_projection,
        camera,
        domains[0],
        domains[1],
        domains[2],
        &x_ticks,
        &y_ticks,
        &z_ticks,
        axes,
        &axes_style(style_background, axes)?,
        additional_text,
    )?;
    let pass = match legend {
        Some(legend) => layout.text.with_legend(legend),
        None => layout.text,
    };
    let target_overlay = complete_with_provisional_metrics(pass.clone())?;
    compose_projected_overlay(size, axes_id, &pass, target_overlay, prepared)
}

fn scaled_tick_domains_3d(axes: &Axes2DProperties) -> Result<[TickDomain; 3], PlotWebError> {
    let domain = |limits: [f64; 2], scale: WireScale, direction: WireAxisDirection| {
        Limits::new(limits[0], limits[1])
            .map(SemanticLimits::new)
            .map(|limits| semantic_tick_domain(limits, scale, direction))
            .map_err(|_| invalid_scene("3D Axes limits cannot be rendered."))
    };
    Ok([
        domain(axes.x_limits, axes.x_scale, axes.x_direction)?,
        domain(axes.y_limits, axes.y_scale, axes.y_direction)?,
        domain(axes.z_limits, axes.z_scale, axes.z_direction)?,
    ])
}

fn compose_projected_overlay(
    size: CanvasSize,
    axes_id: &str,
    pass: &FirstLayoutPass,
    target_overlay: OverlayPlan,
    prepared: &PreparedScene,
) -> Result<ProjectedOverlay, PlotWebError> {
    let mut axes_entries = Vec::with_capacity(1 + prepared.additional_axes.len());
    if let (Some(first_id), Some(first_pass)) =
        (prepared.axes_id.as_deref(), prepared.text_pass.as_ref())
    {
        axes_entries.push((first_id, &prepared.frame, first_pass));
    }
    axes_entries.extend(
        prepared
            .additional_axes
            .iter()
            .map(|axes| (axes.axes_id.as_str(), &axes.frame, &axes.text_pass)),
    );
    let multiple = axes_entries.len() > 1;
    let mut target_overlay = Some(target_overlay);
    let mut overlays = Vec::with_capacity(axes_entries.len());
    let mut passes = Vec::with_capacity(axes_entries.len());
    for (entry_id, frame, entry_pass) in axes_entries {
        let selected_pass = if entry_id == axes_id {
            pass
        } else {
            entry_pass
        };
        let selected_overlay = if entry_id == axes_id {
            target_overlay
                .take()
                .ok_or_else(|| invalid_scene("Projected Axes appeared more than once."))?
        } else {
            frame.overlay.clone()
        };
        let key_prefix = if multiple {
            format!("{entry_id}::")
        } else {
            String::new()
        };
        let mut dto = overlay_dto(
            &selected_overlay,
            size.css_width,
            size.css_height,
            selected_pass.font_cache_revision,
        );
        if multiple {
            prefix_overlay_keys(&mut dto, entry_id);
        }
        overlays.push(dto);
        passes.push(TextLayoutPassState {
            key_prefix,
            pass: selected_pass.clone(),
        });
    }
    let first_pass = passes
        .first()
        .ok_or_else(|| invalid_scene("A projected Figure requires an Axes text pass."))?
        .pass
        .clone();
    let text_layout = TextLayoutState {
        pass: first_pass,
        passes,
        canvas_width_css_px: size.css_width,
        canvas_height_css_px: size.css_height,
    };
    Ok(ProjectedOverlay {
        overlay: merge_overlay_dtos(overlays),
        text_layout,
    })
}

fn resolved_axes_scale(axes: &Axes2DProperties) -> Result<[f64; 3], PlotWebError> {
    let raw = if axes.plot_box_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Manual {
        axes.plot_box_aspect_ratio
    } else if axes.data_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Manual {
        let x_limits = transformed_axis_limits(axes.x_limits, axes.x_scale)?;
        let y_limits = transformed_axis_limits(axes.y_limits, axes.y_scale)?;
        let z_limits = transformed_axis_limits(axes.z_limits, axes.z_scale)?;
        [
            (x_limits[1] - x_limits[0]) / axes.data_aspect_ratio[0],
            (y_limits[1] - y_limits[0]) / axes.data_aspect_ratio[1],
            (z_limits[1] - z_limits[0]) / axes.data_aspect_ratio[2],
        ]
    } else {
        [1.0, 1.0, 1.0]
    };
    if raw.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err(invalid_scene("Axes aspect ratio cannot be represented."));
    }
    let maximum = raw.into_iter().fold(f64::NEG_INFINITY, f64::max);
    Ok(raw.map(|value| value / maximum))
}

fn plot_box_fit_target_3d(
    axes: &Axes2DProperties,
    viewport: CssRect,
) -> Result<PlotBoxFit3D, PlotWebError> {
    let mode = if axes.plot_box_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Auto
        && axes.data_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Auto
    {
        PlotBoxFitMode3D::StretchToFill
    } else {
        PlotBoxFitMode3D::PreserveAspect
    };
    let target_half_extent_ndc =
        [viewport.size.width.get(), viewport.size.height.get()].map(|length| {
            let length = f64::from(length);
            let inset = PLOT_BOX_INSET_CSS_PX.min(length * 0.1);
            1.0 - inset * 2.0 / length
        });
    PlotBoxFit3D::new(mode, target_half_extent_ndc, 1.0)
        .map_err(|_| invalid_scene("Axes 3D plot-box fit cannot be represented."))
}

fn camera_zoom_factor_3d(camera: OrbitCamera3D) -> Result<f64, PlotWebError> {
    let zoom = match camera.projection() {
        Projection3D::Orthographic { vertical_span, .. } => 1.8 / vertical_span,
        Projection3D::Perspective {
            vertical_fov_radians,
            ..
        } => 22.5_f64.to_radians().tan() / (vertical_fov_radians * 0.5).tan(),
    };
    if zoom.is_finite() && zoom > 0.0 {
        Ok(zoom)
    } else {
        Err(invalid_scene("Axes 3D camera zoom cannot be represented."))
    }
}

pub(crate) fn headlight_position_3d(camera: OrbitCamera3D) -> Result<AxesPoint3D, PlotWebError> {
    let eye = camera.eye();
    AxesPoint3D::new(f64_to_f32(eye.x)?, f64_to_f32(eye.y)?, f64_to_f32(eye.z)?)
        .map_err(|_| invalid_scene("Axes headlight position is invalid."))
}

pub(crate) fn resolved_view_projection_3d(
    camera: OrbitCamera3D,
    aspect_ratio: f64,
    axes_scale: [f64; 3],
    plot_box_fit: PlotBoxFit3D,
) -> Result<ViewProjection3D, PlotWebError> {
    let matrix = camera
        .view_projection(aspect_ratio)
        .map_err(|_| invalid_scene("Axes 3D camera cannot be represented."))?;
    let matrix = scale_view_projection_3d(matrix, axes_scale)?;
    let fit = plot_box_fit
        .with_zoom(camera_zoom_factor_3d(camera)?)
        .map_err(|_| invalid_scene("Axes 3D plot-box fit cannot be represented."))?;
    fit_plot_box_3d(matrix, fit)
        .map_err(|_| invalid_scene("Axes 3D plot-box fit cannot be represented."))
}

pub(crate) fn scale_view_projection_3d(
    matrix: ViewProjection3D,
    scale: [f64; 3],
) -> Result<ViewProjection3D, PlotWebError> {
    if scale
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(invalid_scene("Axes 3D scale cannot be represented."));
    }
    let mut columns = matrix.columns();
    let original = columns;
    for axis in 0..3 {
        let scale = f64_to_f32(scale[axis])?;
        for row in 0..4 {
            columns[axis][row] = original[axis][row] * scale;
        }
    }
    let translation = scale.map(|value| (1.0 - value) * 0.5);
    for row in 0..4 {
        columns[3][row] = original[3][row]
            + original[0][row] * f64_to_f32(translation[0])?
            + original[1][row] * f64_to_f32(translation[1])?
            + original[2][row] * f64_to_f32(translation[2])?;
    }
    ViewProjection3D::new(columns)
        .map_err(|_| invalid_scene("Scaled 3D camera matrix cannot be represented."))
}

#[allow(clippy::too_many_lines)] // Keeps one transactional HIR-to-MIR lowering path together.
pub(crate) fn prepare_scene_cached(
    snapshot: &FigureSnapshot,
    buffers: &HashMap<String, Vec<u8>>,
    size: CanvasSize,
    cache: &mut SceneCache,
) -> Result<PreparedScene, PlotWebError> {
    let clear_color = figure_clear_color(snapshot)?;

    let axes: Vec<_> = snapshot
        .objects
        .iter()
        .filter_map(|object| match object {
            GraphicsObject::Axes2d { fields, properties } => Some((fields, properties)),
            _ => None,
        })
        .collect();
    if axes.is_empty() {
        return Ok(PreparedScene {
            axes_id: None,
            frame: empty_frame(size)?,
            clear_color,
            camera_3d: None,
            additional_axes: Vec::new(),
            canvas_width_css_px: size.css_width,
            canvas_height_css_px: size.css_height,
            text_pass: None,
        });
    }

    let mut prepared = axes
        .into_iter()
        .map(|(fields, properties)| {
            prepare_axes_scene(
                snapshot,
                buffers,
                size,
                cache,
                clear_color,
                fields,
                properties,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first = prepared.remove(0);
    Ok(PreparedScene {
        axes_id: Some(first.axes_id),
        frame: first.frame,
        clear_color,
        camera_3d: first.camera_3d,
        additional_axes: prepared,
        canvas_width_css_px: size.css_width,
        canvas_height_css_px: size.css_height,
        text_pass: Some(first.text_pass),
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn prepare_axes_scene(
    snapshot: &FigureSnapshot,
    buffers: &HashMap<String, Vec<u8>>,
    size: CanvasSize,
    cache: &mut SceneCache,
    clear_color: Rgba,
    axes_fields: &openmat_plot_protocol::ObjectFields,
    axes_properties: &Axes2DProperties,
) -> Result<PreparedAxesScene, PlotWebError> {
    let object_by_id: HashMap<_, _> = snapshot
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect();
    let has_3d = axes_fields.children.iter().any(|child_id| {
        object_by_id.get(child_id.as_str()).is_some_and(|object| {
            matches!(object,
                GraphicsObject::SurfaceSeries { properties, .. } if properties.visible
            ) || matches!(object,
                GraphicsObject::PatchSeries { properties, .. }
                    if properties.visible && properties.vertices.shape[1] == 3
            ) || matches!(object,
                GraphicsObject::LineSeries { properties, .. }
                    if properties.visible && properties.z_data.as_ref().is_some()
            ) || matches!(object,
                GraphicsObject::ScatterSeries { properties, .. }
                    if properties.visible && properties.z_data.as_ref().is_some()
            )
        })
    });
    let render_position = axes_render_position(axes_properties, size);
    let render_position = if has_3d {
        render_position
    } else {
        fit_2d_plot_box(axes_properties, size, render_position)?
    };
    let mapping = axes_mapping(size, render_position)?;
    let viewport = mapping.viewport();
    let x_limits = SemanticLimits::new(
        Limits::new(axes_properties.x_limits[0], axes_properties.x_limits[1])
            .map_err(|_| invalid_scene("Axes X limits cannot be rendered."))?,
    );
    let y_limits = SemanticLimits::new(
        Limits::new(axes_properties.y_limits[0], axes_properties.y_limits[1])
            .map_err(|_| invalid_scene("Axes Y limits cannot be rendered."))?,
    );
    let transform = AxesTransform::from_scaled_semantic(
        x_limits,
        y_limits,
        layout_scale(axes_properties.x_scale),
        layout_scale(axes_properties.y_scale),
        layout_direction(axes_properties.x_direction),
        layout_direction(axes_properties.y_direction),
    )
    .map_err(|_| invalid_scene("Axes transform cannot be represented."))?;
    let x_domain = semantic_tick_domain(
        x_limits,
        axes_properties.x_scale,
        axes_properties.x_direction,
    );
    let y_domain = semantic_tick_domain(
        y_limits,
        axes_properties.y_scale,
        axes_properties.y_direction,
    );
    let mut x_ticks = axes_ticks(
        x_domain,
        &axes_properties.x_tick,
        axes_properties.x_tick_mode,
        &axes_properties.x_tick_label_code_units,
        axes_properties.x_tick_label_mode,
        responsive_tick_target(viewport.css.size.width.get(), 72.0, 9),
    )
    .map_err(|_| invalid_scene("Axes X ticks cannot be generated."))?;
    let mut y_ticks = axes_ticks(
        y_domain,
        &axes_properties.y_tick,
        axes_properties.y_tick_mode,
        &axes_properties.y_tick_label_code_units,
        axes_properties.y_tick_label_mode,
        responsive_tick_target(viewport.css.size.height.get(), 52.0, 8),
    )
    .map_err(|_| invalid_scene("Axes Y ticks cannot be generated."))?;
    if axes_properties.coordinate_system == AxesCoordinateSystem::Polar {
        apply_polar_tick_labels(&mut x_ticks, &axes_properties.x_tick_label_code_units);
        apply_polar_tick_labels(&mut y_ticks, &axes_properties.y_tick_label_code_units);
    }
    let z_limits = SemanticLimits::new(
        Limits::new(axes_properties.z_limits[0], axes_properties.z_limits[1])
            .map_err(|_| invalid_scene("Axes Z limits cannot be rendered."))?,
    );
    let z_domain = semantic_tick_domain(
        z_limits,
        axes_properties.z_scale,
        axes_properties.z_direction,
    );
    let mut z_ticks = axes_ticks(
        z_domain,
        &axes_properties.z_tick,
        axes_properties.z_tick_mode,
        &axes_properties.z_tick_label_code_units,
        axes_properties.z_tick_label_mode,
        responsive_tick_target(viewport.css.size.height.get(), 52.0, 8),
    )
    .map_err(|_| invalid_scene("Axes Z ticks cannot be generated."))?;
    if !axes_properties.visible {
        x_ticks.ticks.clear();
        y_ticks.ticks.clear();
        z_ticks.ticks.clear();
    }

    let colorbar = axes_fields.children.iter().find_map(|child_id| {
        match object_by_id.get(child_id.as_str()).copied() {
            Some(GraphicsObject::ColorBar { properties, .. }) => Some(properties),
            _ => None,
        }
    });
    let lighting_enabled = axes_fields.children.iter().any(|child_id| {
        object_by_id.get(child_id.as_str()).is_some_and(|object| {
            matches!(object,
                GraphicsObject::SurfaceSeries { properties, .. }
                    if properties.visible && properties.lighting_enabled
            ) || matches!(object,
                GraphicsObject::PatchSeries { properties, .. }
                    if properties.visible && properties.lighting_enabled
            )
        })
    });
    let mut additional_text = axes_text_intents(axes_properties, &object_by_id, mapping)?;
    additional_text.extend(colorbar_text_intents(axes_properties, colorbar, size)?);
    let legend = legend_intent(&axes_fields.children, &object_by_id, axes_properties)?;
    let axes_background = axes_background_color(axes_properties)?;
    let style = axes_style(axes_background.unwrap_or(clear_color), axes_properties)?;
    let (text_pass, mut builder, camera_3d) =
        if axes_properties.coordinate_system == AxesCoordinateSystem::Polar {
            if has_3d {
                return Err(invalid_scene(
                    "PolarAxes cannot contain 3D graphics objects.",
                ));
            }
            let layout = build_polar_axes_layout(PolarAxesLayoutInput {
                viewport: mapping,
                theta_domain: x_domain,
                radius_domain: y_domain,
                theta_ticks: &x_ticks,
                radius_ticks: &y_ticks,
                theta_period: match axes_properties.theta_axis_units {
                    ThetaAxisUnits::Degrees => 360.0,
                    ThetaAxisUnits::Radians => std::f64::consts::TAU,
                },
                theta_direction: match axes_properties.theta_direction {
                    ThetaDirection::Counterclockwise => PolarThetaDirection::Counterclockwise,
                    ThetaDirection::Clockwise => PolarThetaDirection::Clockwise,
                },
                theta_zero_location: match axes_properties.theta_zero_location {
                    ThetaZeroLocation::Right => PolarZeroLocation::Right,
                    ThetaZeroLocation::Top => PolarZeroLocation::Top,
                    ThetaZeroLocation::Left => PolarZeroLocation::Left,
                    ThetaZeroLocation::Bottom => PolarZeroLocation::Bottom,
                },
                r_axis_location: axes_properties.r_axis_location,
                theta_grid: axes_properties.visible && axes_properties.grid_x,
                radius_grid: axes_properties.visible && axes_properties.grid_y,
                theta_minor_grid: axes_properties.visible && axes_properties.minor_grid_x,
                radius_minor_grid: axes_properties.visible && axes_properties.minor_grid_y,
                font_revision: 1,
                style: style.clone(),
                additional_text,
            })
            .map_err(|_| invalid_scene("PolarAxes layout cannot be generated."))?;
            let text_pass = match legend {
                Some(legend) => layout.text.with_legend(legend),
                None => layout.text,
            };
            let overlay = complete_with_provisional_metrics(text_pass.clone())?;
            let mut builder = MirFrameBuilder::new(viewport, AxesRect::unit(), overlay);
            if axes_properties.visible {
                builder.add_grid(DrawOrder(0), None, layout.grid_lines);
                builder.add_grid(DrawOrder(i64::MAX - 1), None, layout.axis_lines);
            }
            (text_pass, builder, None)
        } else if has_3d {
            let aspect_ratio = f64::from(viewport.css.size.width.get())
                / f64::from(viewport.css.size.height.get());
            let projection = match axes_properties.projection {
                WireProjection::Orthographic => Projection3D::Orthographic {
                    vertical_span: 1.8 * axes_properties.camera_scale,
                    near: 0.1,
                    far: 10.0,
                },
                WireProjection::Perspective => Projection3D::Perspective {
                    vertical_fov_radians: (45.0 * axes_properties.camera_scale).to_radians(),
                    near: 0.1,
                    far: 10.0,
                },
            };
            let camera = OrbitCamera3D::new(
                Point3D {
                    x: 0.5,
                    y: 0.5,
                    z: 0.5,
                },
                3.0,
                -axes_properties.view[0].to_radians(),
                axes_properties.view[1].to_radians(),
                projection,
            )
            .map_err(|_| invalid_scene("Axes 3D camera cannot be represented."))?;
            let axes_scale = resolved_axes_scale(axes_properties)?;
            let plot_box_fit = plot_box_fit_target_3d(axes_properties, viewport.css)?;
            let view_projection =
                resolved_view_projection_3d(camera, aspect_ratio, axes_scale, plot_box_fit)?;
            let layout = build_projected_axes_3d(
                viewport.css,
                view_projection,
                camera,
                x_domain,
                y_domain,
                z_domain,
                &x_ticks,
                &y_ticks,
                &z_ticks,
                axes_properties,
                &style,
                additional_text,
            )?;
            let text_pass = match legend {
                Some(legend) => layout.text.with_legend(legend),
                None => layout.text,
            };
            let overlay = complete_with_provisional_metrics(text_pass.clone())?;
            let mut builder = MirFrameBuilder::new(viewport, AxesRect::unit(), overlay);
            builder.set_view_projection_3d(view_projection);
            if lighting_enabled {
                builder.set_lighting_3d(Lighting3D {
                    position: headlight_position_3d(camera)?,
                    enabled: true,
                });
            }
            builder.set_ruler_selection_3d(layout.ruler_selection);
            if axes_properties.visible && !layout.grid_lines.segments.is_empty() {
                builder.add_line_segments_3d(DrawOrder(0), None, layout.grid_lines);
            }
            if axes_properties.visible && !layout.axis_lines.segments.is_empty() {
                builder.add_line_segments_3d(DrawOrder(i64::MAX - 2), None, layout.axis_lines);
            }
            if axes_properties.visible && !layout.ruler_lines.lines.is_empty() {
                builder.add_ruler_lines_3d(DrawOrder(i64::MAX - 2), None, layout.ruler_lines);
            }
            if axes_properties.visible && !layout.tick_marks.marks.is_empty() {
                builder.add_tick_marks_3d(DrawOrder(i64::MAX - 1), None, layout.tick_marks);
            }
            (
                text_pass,
                builder,
                Some((camera, aspect_ratio, axes_scale, plot_box_fit)),
            )
        } else {
            let layout = build_linear_axes_layout(LinearAxesLayoutInput {
                viewport: mapping,
                x_domain,
                y_domain,
                x_ticks: &x_ticks,
                y_ticks: &y_ticks,
                grid_x: axes_properties.visible && axes_properties.grid_x,
                grid_y: axes_properties.visible && axes_properties.grid_y,
                minor_grid_x: axes_properties.visible && axes_properties.minor_grid_x,
                minor_grid_y: axes_properties.visible && axes_properties.minor_grid_y,
                box_enabled: axes_properties.visible && axes_properties.box_enabled,
                tick_direction: match axes_properties.tick_direction {
                    WireTickDirection::In => LayoutTickDirection::In,
                    WireTickDirection::Out => LayoutTickDirection::Out,
                    WireTickDirection::Both => LayoutTickDirection::Both,
                },
                font_revision: 1,
                style: style.clone(),
                additional_text,
            })
            .map_err(|_| invalid_scene("Axes overlay layout cannot be generated."))?;
            let axes_chrome = screen_lines_from_overlay(layout.text.base_overlay(), &style);
            let text_pass = match legend {
                Some(legend) => layout.text.with_legend(legend),
                None => layout.text,
            };
            let overlay = complete_with_provisional_metrics(text_pass.clone())?;
            let mut builder = MirFrameBuilder::new(viewport, AxesRect::unit(), overlay);
            builder.add_grid(DrawOrder(0), None, layout.grid_lines);
            if axes_properties.visible && !axes_chrome.lines.is_empty() {
                builder.add_screen_lines(DrawOrder(i64::MAX - 1), None, axes_chrome);
            }
            (text_pass, builder, None)
        };

    if axes_properties.visible
        && let Some(background) = axes_background
    {
        builder.add_triangle_mesh_2d(
            DrawOrder(i64::MIN + 5),
            None,
            axes_background_mesh(background),
        );
    }

    let mut drawable_objects = Vec::new();
    for child_id in &axes_fields.children {
        let Some(object) = object_by_id.get(child_id.as_str()).copied() else {
            return Err(invalid_scene(
                "Axes child does not resolve in the Figure snapshot.",
            ));
        };
        if let GraphicsObject::ChartGroup { fields, properties } = object {
            if !properties.visible {
                continue;
            }
            for primitive_id in &fields.children {
                let Some(primitive) = object_by_id.get(primitive_id.as_str()).copied() else {
                    return Err(invalid_scene(
                        "Chart primitive does not resolve in the Figure snapshot.",
                    ));
                };
                drawable_objects.push(primitive);
            }
        } else {
            drawable_objects.push(object);
        }
    }

    for (index, object) in drawable_objects.into_iter().enumerate() {
        let order = i64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| invalid_scene("Figure draw order is too large."))?;
        match object {
            GraphicsObject::LineSeries { properties, .. } if properties.visible => {
                if axes_properties.coordinate_system == AxesCoordinateSystem::Polar {
                    add_polar_line(
                        &mut builder,
                        properties,
                        buffers,
                        axes_properties,
                        DrawOrder(order),
                        size.device_pixel_ratio,
                    )?;
                } else if properties.z_data.as_ref().is_some() {
                    add_line_3d(
                        &mut builder,
                        properties,
                        buffers,
                        axes_properties,
                        DrawOrder(order),
                    )?;
                } else {
                    add_line(
                        &mut builder,
                        properties,
                        buffers,
                        axes_properties,
                        transform,
                        DrawOrder(order),
                        size.device_pixel_ratio,
                        viewport.css.size.width.get(),
                        axes_properties.x_limits,
                        cache,
                    )?;
                }
            }
            GraphicsObject::ScatterSeries { properties, .. } if properties.visible => {
                if properties.z_data.as_ref().is_some() {
                    add_scatter_3d(
                        &mut builder,
                        properties,
                        buffers,
                        axes_properties,
                        DrawOrder(order),
                    )?;
                } else {
                    add_scatter(
                        &mut builder,
                        properties,
                        buffers,
                        axes_properties,
                        transform,
                        DrawOrder(order),
                    )?;
                }
            }
            GraphicsObject::SurfaceSeries { properties, .. } if properties.visible => {
                add_surface(
                    &mut builder,
                    properties,
                    buffers,
                    axes_properties,
                    DrawOrder(order),
                )?;
            }
            GraphicsObject::PatchSeries { properties, .. } if properties.visible => {
                add_patch(
                    &mut builder,
                    properties,
                    buffers,
                    axes_properties,
                    DrawOrder(order),
                )?;
            }
            GraphicsObject::LineSeries { .. }
            | GraphicsObject::ScatterSeries { .. }
            | GraphicsObject::SurfaceSeries { .. }
            | GraphicsObject::PatchSeries { .. }
            | GraphicsObject::ChartGroup { .. }
            | GraphicsObject::Text { .. }
            | GraphicsObject::Legend { .. }
            | GraphicsObject::ColorBar { .. } => {}
            GraphicsObject::Figure { .. } | GraphicsObject::Axes2d { .. } => {
                return Err(invalid_scene("Axes contains an invalid drawable child."));
            }
        }
    }

    if axes_properties.colorbar_visible {
        let (gradient, border) = colorbar_screen_lines(axes_properties, colorbar, size)?;
        builder.add_screen_lines(DrawOrder(i64::MAX - 1), None, gradient);
        builder.add_screen_lines(DrawOrder(i64::MAX), None, border);
    }

    let frame = builder
        .finish()
        .map_err(|_| invalid_scene("Figure geometry could not be lowered to Plot MIR."))?;
    Ok(PreparedAxesScene {
        axes_id: axes_fields.id.clone(),
        frame,
        camera_3d,
        text_pass,
    })
}

fn figure_clear_color(snapshot: &FigureSnapshot) -> Result<Rgba, PlotWebError> {
    snapshot
        .objects
        .iter()
        .find_map(|object| match object {
            GraphicsObject::Figure { properties, .. } => {
                Some(convert_color(properties.background_rgba))
            }
            _ => None,
        })
        .transpose()
        .map(|color| {
            color.unwrap_or(Rgba {
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
            })
        })
}

fn axes_background_mesh(color: Rgba) -> TriangleMesh2D {
    TriangleMesh2D {
        vertices: vec![
            TriangleVertex2D {
                position: AxesPoint { x: 0.0, y: 0.0 },
                color,
            },
            TriangleVertex2D {
                position: AxesPoint { x: 1.0, y: 0.0 },
                color,
            },
            TriangleVertex2D {
                position: AxesPoint { x: 1.0, y: 1.0 },
                color,
            },
            TriangleVertex2D {
                position: AxesPoint { x: 0.0, y: 1.0 },
                color,
            },
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

fn axes_background_color(axes: &Axes2DProperties) -> Result<Option<Rgba>, PlotWebError> {
    axes.background_rgba
        .as_ref()
        .copied()
        .map(convert_color)
        .transpose()
}

fn empty_frame(size: CanvasSize) -> Result<PlotFrame, PlotWebError> {
    let css = CssRect::new(
        0.0,
        0.0,
        f64_to_f32(size.css_width)?,
        f64_to_f32(size.css_height)?,
    )
    .map_err(|_| invalid_scene("Figure viewport cannot be represented."))?;
    let mapping = ViewportMapping::new(
        css,
        DevicePixelRatio::new(size.device_pixel_ratio)
            .map_err(|_| invalid_scene("Figure DPR cannot be represented."))?,
    )
    .map_err(|_| invalid_scene("Figure viewport cannot be mapped."))?;
    MirFrameBuilder::new(
        mapping.viewport(),
        AxesRect::unit(),
        OverlayPlan {
            viewport_css: css,
            ..OverlayPlan::default()
        },
    )
    .finish()
    .map_err(|_| invalid_scene("Empty Figure could not be lowered to Plot MIR."))
}

fn axes_mapping(size: CanvasSize, position: [f64; 4]) -> Result<ViewportMapping, PlotWebError> {
    let [x, y, width, height] = position;
    let css_x = x * size.css_width;
    let css_y = (1.0 - y - height) * size.css_height;
    let css_width = width * size.css_width;
    let css_height = height * size.css_height;
    let css = CssRect::new(
        f64_to_f32(css_x)?,
        f64_to_f32(css_y)?,
        f64_to_f32(css_width)?,
        f64_to_f32(css_height)?,
    )
    .map_err(|_| invalid_scene("Axes CSS viewport cannot be represented."))?;
    let dpr = DevicePixelRatio::new(size.device_pixel_ratio)
        .map_err(|_| invalid_scene("Axes DPR cannot be represented."))?;
    ViewportMapping::new(css, dpr)
        .map_err(|_| invalid_scene("Axes device viewport cannot be represented."))
}

fn axes_render_position(axes: &Axes2DProperties, size: CanvasSize) -> [f64; 4] {
    let mut position = if axes.colorbar_visible {
        let [x, y, width, height] = axes.position_normalized;
        [
            x - width * 0.025_560_645_161_290_3,
            y,
            width * 0.847_619_047_619_047_6,
            height,
        ]
    } else {
        axes.position_normalized
    };
    if axes.coordinate_system == AxesCoordinateSystem::Polar {
        let width_css = position[2] * size.css_width;
        let height_css = position[3] * size.css_height;
        // Polar tick labels surround the complete circle and the title sits
        // above the north tick. Keep an internal margin rather than letting
        // that chrome compete for the rectangular Axes slot.
        let side_css = width_css.min(height_css) * 0.86;
        let square_width = side_css / size.css_width;
        let square_height = side_css / size.css_height;
        position[0] += (position[2] - square_width) * 0.5;
        position[1] += (position[3] - square_height) * 0.5;
        position[2] = square_width;
        position[3] = square_height;
    }
    position
}

fn fit_2d_plot_box(
    axes: &Axes2DProperties,
    size: CanvasSize,
    mut position: [f64; 4],
) -> Result<[f64; 4], PlotWebError> {
    if axes.coordinate_system == AxesCoordinateSystem::Polar
        || (axes.plot_box_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Auto
            && axes.data_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Auto)
    {
        return Ok(position);
    }

    let desired_width_to_height =
        if axes.plot_box_aspect_ratio_mode == openmat_plot_protocol::AxesLimitMode::Manual {
            axes.plot_box_aspect_ratio[0] / axes.plot_box_aspect_ratio[1]
        } else {
            let x_limits = transformed_axis_limits(axes.x_limits, axes.x_scale)?;
            let y_limits = transformed_axis_limits(axes.y_limits, axes.y_scale)?;
            let x_span = (x_limits[1] - x_limits[0]) / axes.data_aspect_ratio[0];
            let y_span = (y_limits[1] - y_limits[0]) / axes.data_aspect_ratio[1];
            x_span / y_span
        };
    let available_width = position[2] * size.css_width;
    let available_height = position[3] * size.css_height;
    if !desired_width_to_height.is_finite()
        || desired_width_to_height <= 0.0
        || !available_width.is_finite()
        || available_width <= 0.0
        || !available_height.is_finite()
        || available_height <= 0.0
    {
        return Err(invalid_scene("2D Axes aspect ratio cannot be represented."));
    }

    let available_ratio = available_width / available_height;
    if available_ratio > desired_width_to_height {
        let fitted_width = available_height * desired_width_to_height / size.css_width;
        position[0] += (position[2] - fitted_width) * 0.5;
        position[2] = fitted_width;
    } else {
        let fitted_height = available_width / desired_width_to_height / size.css_height;
        position[1] += (position[3] - fitted_height) * 0.5;
        position[3] = fitted_height;
    }
    Ok(position)
}

fn colorbar_rect(axes: &Axes2DProperties, size: CanvasSize) -> Result<CssRect, PlotWebError> {
    let [x, y, width, height] = axes.position_normalized;
    CssRect::new(
        f64_to_f32((x + width * 0.908_079_877_112_135_2) * size.css_width)?,
        f64_to_f32((1.0 - y - height) * size.css_height)?,
        f64_to_f32(width * 0.049_155_145_929_339_45 * size.css_width)?,
        f64_to_f32(height * size.css_height)?,
    )
    .map_err(|_| invalid_scene("Colorbar rectangle cannot be represented."))
}

fn colorbar_screen_lines(
    axes: &Axes2DProperties,
    colorbar: Option<&ColorBarProperties>,
    size: CanvasSize,
) -> Result<(ScreenLineBatch, ScreenLineBatch), PlotWebError> {
    let rect = colorbar_rect(axes, size)?;
    let colormap = axes_colormap(axes);
    let stripe_count = u32::try_from(colormap.len())
        .map_err(|_| invalid_scene("Axes colormap exceeds browser limits."))?;
    let stripe_count_f32 = f64_to_f32(f64::from(stripe_count))?;
    let stripe_height = rect.size.height.get() / stripe_count_f32;
    let mut lines = Vec::with_capacity(colormap.len());
    for index in 0..stripe_count {
        let y = rect.origin.y + (f64_to_f32(f64::from(index))? + 0.5) * stripe_height;
        let color_index = colormap.len()
            - 1
            - usize::try_from(index)
                .map_err(|_| invalid_scene("Axes colormap index exceeds browser limits."))?;
        lines.push(ScreenLine {
            segment: CssLineSegment {
                start: CssPoint::new(rect.origin.x, y)
                    .map_err(|_| invalid_scene("Colorbar stripe cannot be represented."))?,
                end: CssPoint::new(rect.right(), y)
                    .map_err(|_| invalid_scene("Colorbar stripe cannot be represented."))?,
            },
            color: colormap_color(colormap[color_index])?,
        });
    }
    let border_color = rgba(0.15, 0.15, 0.15, 1.0)?;
    let top_left = CssPoint::new(rect.origin.x, rect.origin.y)
        .map_err(|_| invalid_scene("Colorbar border cannot be represented."))?;
    let top_right = CssPoint::new(rect.right(), rect.origin.y)
        .map_err(|_| invalid_scene("Colorbar border cannot be represented."))?;
    let bottom_left = CssPoint::new(rect.origin.x, rect.bottom())
        .map_err(|_| invalid_scene("Colorbar border cannot be represented."))?;
    let bottom_right = CssPoint::new(rect.right(), rect.bottom())
        .map_err(|_| invalid_scene("Colorbar border cannot be represented."))?;
    let mut decoration = [
        (top_left, top_right),
        (top_right, bottom_right),
        (bottom_right, bottom_left),
        (bottom_left, top_left),
    ]
    .into_iter()
    .map(|(start, end)| ScreenLine {
        segment: CssLineSegment { start, end },
        color: border_color,
    })
    .collect::<Vec<_>>();
    let ticks = colorbar_ticks(axes, colorbar, size)?;
    decoration.reserve(ticks.ticks.len());
    for tick in ticks.ticks {
        let y = colorbar_tick_y(rect, axes.c_limits, tick.value)?;
        decoration.push(ScreenLine {
            segment: CssLineSegment {
                start: CssPoint::new(rect.right(), y)
                    .map_err(|_| invalid_scene("Colorbar tick cannot be represented."))?,
                end: CssPoint::new(rect.right() + COLORBAR_TICK_LENGTH_CSS_PX, y)
                    .map_err(|_| invalid_scene("Colorbar tick cannot be represented."))?,
            },
            color: border_color,
        });
    }
    Ok((
        ScreenLineBatch {
            lines,
            width_css_px: css_px(stripe_height + 0.75)?,
            pattern: LinePattern3D::Solid,
        },
        ScreenLineBatch {
            lines: decoration,
            width_css_px: css_px(1.0)?,
            pattern: LinePattern3D::Solid,
        },
    ))
}

fn colorbar_text_intents(
    axes: &Axes2DProperties,
    colorbar: Option<&ColorBarProperties>,
    size: CanvasSize,
) -> Result<Vec<TextIntent>, PlotWebError> {
    if !axes.colorbar_visible {
        return Ok(Vec::new());
    }
    let rect = colorbar_rect(axes, size)?;
    colorbar_ticks(axes, colorbar, size)?
        .ticks
        .into_iter()
        .enumerate()
        .map(|(index, tick)| {
            let y = colorbar_tick_y(rect, axes.c_limits, tick.value)?;
            Ok(TextIntent {
                key: format!("colorbar-tick-{index}"),
                role: OverlayTextRole::Annotation,
                text: Utf16Text::from_code_units(tick.label_code_units),
                anchor: CssPoint::new(rect.right() + COLORBAR_TICK_LENGTH_CSS_PX + 4.0, y)
                    .map_err(|_| invalid_scene("Colorbar label cannot be represented."))?,
                horizontal_alignment: HorizontalAlignment::Start,
                vertical_alignment: VerticalAlignment::Middle,
                rotation_radians: 0.0,
                color: rgba(0.15, 0.15, 0.15, 1.0)?,
                font: FontKey {
                    family_utf16: Vec::new(),
                    size_css_px: css_px(f64_to_f32(axes.font_size_css_px)?)?,
                    weight: FontWeight::new(400)
                        .map_err(|_| invalid_scene("Colorbar font cannot be represented."))?,
                    style: OverlayFontStyle::Normal,
                },
                interpreter: MirTextInterpreter::None,
            })
        })
        .collect()
}

fn colorbar_ticks(
    axes: &Axes2DProperties,
    colorbar: Option<&ColorBarProperties>,
    size: CanvasSize,
) -> Result<TickSet, PlotWebError> {
    let rect = colorbar_rect(axes, size)?;
    let limits = Limits::new(axes.c_limits[0], axes.c_limits[1])
        .map(SemanticLimits::new)
        .map(TickDomain::Semantic)
        .map_err(|_| invalid_scene("Colorbar limits cannot be rendered."))?;
    let intervals = responsive_tick_target(
        rect.size.height.get(),
        COLORBAR_IDEAL_TICK_SPACING_CSS_PX,
        COLORBAR_MAX_TICK_INTERVALS,
    );
    let Some(colorbar) = colorbar else {
        return linear_ticks_r2022b(limits, intervals.saturating_add(1))
            .map_err(|_| invalid_scene("Colorbar ticks cannot be generated."));
    };
    if colorbar.ticks_mode == AxesTickMode::Manual {
        let values = colorbar
            .ticks
            .iter()
            .map(|value| value.get())
            .collect::<Vec<_>>();
        let labels = (colorbar.tick_labels_mode == AxesTickMode::Manual)
            .then_some(colorbar.tick_label_code_units.as_slice());
        return explicit_ticks(limits, &values, labels)
            .map_err(|_| invalid_scene("Colorbar ticks cannot be generated."));
    }
    let mut ticks = linear_ticks_r2022b(limits, intervals.saturating_add(1))
        .map_err(|_| invalid_scene("Colorbar ticks cannot be generated."))?;
    if colorbar.tick_labels_mode == AxesTickMode::Manual {
        for (index, tick) in ticks.ticks.iter_mut().enumerate() {
            tick.label_code_units = if colorbar.tick_label_code_units.is_empty() {
                Vec::new()
            } else {
                colorbar.tick_label_code_units[index % colorbar.tick_label_code_units.len()].clone()
            };
        }
    }
    Ok(ticks)
}

fn colorbar_tick_y(rect: CssRect, limits: [f64; 2], value: f64) -> Result<f32, PlotWebError> {
    let fraction = (value - limits[0]) / (limits[1] - limits[0]);
    f64_to_f32(f64::from(rect.bottom()) - fraction * f64::from(rect.size.height.get()))
}

fn responsive_tick_target(extent_css_px: f32, ideal_spacing_css_px: f32, maximum: u32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let target = (extent_css_px / ideal_spacing_css_px).floor() as u32;
    target.clamp(2, maximum)
}

const fn layout_scale(scale: WireScale) -> LayoutAxisScale {
    match scale {
        WireScale::Linear => LayoutAxisScale::Linear,
        WireScale::Log => LayoutAxisScale::Log10,
    }
}

const fn layout_direction(direction: WireAxisDirection) -> LayoutAxisDirection {
    match direction {
        WireAxisDirection::Normal => LayoutAxisDirection::Normal,
        WireAxisDirection::Reverse => LayoutAxisDirection::Reverse,
    }
}

const fn semantic_tick_domain(
    limits: SemanticLimits,
    scale: WireScale,
    direction: WireAxisDirection,
) -> TickDomain {
    if matches!(scale, WireScale::Linear) && matches!(direction, WireAxisDirection::Normal) {
        TickDomain::Semantic(limits)
    } else {
        TickDomain::SemanticScaled {
            limits,
            scale: layout_scale(scale),
            direction: layout_direction(direction),
        }
    }
}

fn axes_ticks(
    domain: TickDomain,
    values: &[TickValue],
    value_mode: AxesTickMode,
    labels: &[Vec<u16>],
    label_mode: AxesTickMode,
    target: u32,
) -> Result<TickSet, openmat_plot_layout::LayoutError> {
    if values.is_empty() && value_mode == AxesTickMode::Auto {
        let mut ticks = if domain.scale() == LayoutAxisScale::Log10 {
            log_ticks(domain, target)?
        } else {
            linear_ticks(domain, target)?
        };
        if label_mode == AxesTickMode::Manual {
            for (index, tick) in ticks.ticks.iter_mut().enumerate() {
                tick.label_code_units = if labels.is_empty() {
                    Vec::new()
                } else {
                    labels[index % labels.len()].clone()
                };
            }
        }
        return Ok(ticks);
    }
    let values = values.iter().map(|value| value.get()).collect::<Vec<_>>();
    let manual_labels = (label_mode == AxesTickMode::Manual).then_some(labels);
    explicit_ticks(domain, &values, manual_labels)
}

fn apply_polar_tick_labels(ticks: &mut TickSet, labels: &[Vec<u16>]) {
    for (index, tick) in ticks.ticks.iter_mut().enumerate() {
        tick.label_code_units = labels.get(index).cloned().unwrap_or_default();
    }
}

fn axes_style(background: Rgba, axes: &Axes2DProperties) -> Result<LinearAxesStyle, PlotWebError> {
    let luminance = 0.2126 * background.red + 0.7152 * background.green + 0.0722 * background.blue;
    let dark = luminance < 0.45;
    let (axis_color, grid_color, text_color) = if dark {
        (
            rgba(0.82, 0.84, 0.86, 1.0)?,
            rgba(0.82, 0.84, 0.86, 0.22)?,
            rgba(0.92, 0.93, 0.94, 1.0)?,
        )
    } else {
        (
            rgba(0.15, 0.15, 0.15, 1.0)?,
            rgba(0.15, 0.15, 0.15, 0.16)?,
            rgba(0.12, 0.12, 0.12, 1.0)?,
        )
    };
    Ok(LinearAxesStyle {
        axis_color,
        grid_color,
        text_color,
        axis_width_css_px: css_px(f64_to_f32(axes.line_width_css_px)?)?,
        grid_width_css_px: css_px(0.75)?,
        tick_length_css_px: css_px(5.0)?,
        label_gap_css_px: css_px(4.0)?,
        tick_font: FontKey {
            family_utf16: if axes.font_family_code_units.is_empty() {
                "Arial, Helvetica, sans-serif".encode_utf16().collect()
            } else {
                axes.font_family_code_units.clone()
            },
            size_css_px: css_px(f64_to_f32(axes.font_size_css_px)?)?,
            weight: FontWeight::NORMAL,
            style: OverlayFontStyle::Normal,
        },
        tick_label_interpreter: convert_interpreter(axes.tick_label_interpreter),
    })
}

#[derive(Debug)]
struct Axes3DLayout {
    text: FirstLayoutPass,
    grid_lines: LineSegmentBatch3D,
    axis_lines: LineSegmentBatch3D,
    ruler_lines: RulerLineBatch3D,
    ruler_selection: RulerSelection3D,
    tick_marks: TickMarkBatch3D,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_projected_axes_3d(
    viewport: CssRect,
    view_projection: ViewProjection3D,
    camera: OrbitCamera3D,
    x_domain: TickDomain,
    y_domain: TickDomain,
    z_domain: TickDomain,
    x_ticks: &TickSet,
    y_ticks: &TickSet,
    z_ticks: &TickSet,
    axes: &Axes2DProperties,
    style: &LinearAxesStyle,
    mut additional_text: Vec<TextIntent>,
) -> Result<Axes3DLayout, PlotWebError> {
    let eye = camera.eye();
    let near_x = if eye.x >= 0.5 { 1.0 } else { 0.0 };
    let near_y = if eye.y >= 0.5 { 1.0 } else { 0.0 };
    let far_x = 1.0 - near_x;
    let far_y = 1.0 - near_y;
    let ruler_selection = select_projected_rulers_3d(view_projection)
        .map_err(|_| invalid_scene("Projected 3D rulers cannot be selected."))?;
    let center = project_axes_point_3d(view_projection, viewport, [0.5, 0.5, 0.5])?;
    let x_ruler = projected_ruler_layout(
        AxisDimension::X,
        ruler_selection.x,
        view_projection,
        viewport,
        center,
    )?;
    let y_ruler = projected_ruler_layout(
        AxisDimension::Y,
        ruler_selection.y,
        view_projection,
        viewport,
        center,
    )?;
    let z_ruler = projected_ruler_layout(
        AxisDimension::Z,
        ruler_selection.z,
        view_projection,
        viewport,
        center,
    )?;
    let mut overlay = OverlayPlan {
        viewport_css: viewport,
        ..OverlayPlan::default()
    };
    for ruler in [x_ruler, y_ruler, z_ruler].into_iter().flatten() {
        overlay.axis_lines.push(OverlayAxisLine {
            segment: CssLineSegment {
                start: ruler.start,
                end: ruler.end,
            },
            color: style.axis_color,
            width_css_px: style.axis_width_css_px,
        });
    }
    let mut axis_segments = Vec::new();
    if axes.box_enabled {
        for (start, end) in cube_edges() {
            push_line_3d(&mut axis_segments, start, end, style.axis_color)?;
        }
    }
    let mut ruler_lines = Vec::new();
    if !axes.box_enabled {
        for axis in [AxisDimension::X, AxisDimension::Y, AxisDimension::Z] {
            for candidate in 0..RulerSelection3D::CANDIDATE_COUNT {
                let edge = ruler_edge_3d(axis, candidate)
                    .map_err(|_| invalid_scene("3D ruler candidate cannot be represented."))?;
                ruler_lines.push(RulerLine3D {
                    segment: LineSegment3D {
                        start: edge.start,
                        end: edge.end,
                        color: style.axis_color,
                    },
                    axis,
                    candidate,
                });
            }
        }
    }

    let mut grid_segments = Vec::new();
    let mut tick_marks = Vec::new();
    for (index, tick) in x_ticks.ticks.iter().enumerate() {
        let Some(position) = normalized_visible_tick(tick.value, x_domain) else {
            continue;
        };
        append_tick_candidates(
            &mut tick_marks,
            AxisDimension::X,
            position,
            style.axis_color,
        )?;
        if let Some(ruler) = x_ruler {
            append_projected_tick(
                &mut overlay,
                &mut additional_text,
                AxisDimension::X,
                index,
                tick,
                ruler_point(AxisDimension::X, ruler_selection.x, position)?,
                ruler.outward,
                view_projection,
                viewport,
                style,
            )?;
        }
        if axes.grid_x {
            push_line_3d(
                &mut grid_segments,
                [position, 0.0, 0.0],
                [position, 1.0, 0.0],
                style.grid_color,
            )?;
            push_line_3d(
                &mut grid_segments,
                [position, far_y, 0.0],
                [position, far_y, 1.0],
                style.grid_color,
            )?;
        }
    }
    for (index, tick) in y_ticks.ticks.iter().enumerate() {
        let Some(position) = normalized_visible_tick(tick.value, y_domain) else {
            continue;
        };
        append_tick_candidates(
            &mut tick_marks,
            AxisDimension::Y,
            position,
            style.axis_color,
        )?;
        if let Some(ruler) = y_ruler {
            append_projected_tick(
                &mut overlay,
                &mut additional_text,
                AxisDimension::Y,
                index,
                tick,
                ruler_point(AxisDimension::Y, ruler_selection.y, position)?,
                ruler.outward,
                view_projection,
                viewport,
                style,
            )?;
        }
        if axes.grid_y {
            push_line_3d(
                &mut grid_segments,
                [0.0, position, 0.0],
                [1.0, position, 0.0],
                style.grid_color,
            )?;
            push_line_3d(
                &mut grid_segments,
                [far_x, position, 0.0],
                [far_x, position, 1.0],
                style.grid_color,
            )?;
        }
    }
    for (index, tick) in z_ticks.ticks.iter().enumerate() {
        let Some(position) = normalized_visible_tick(tick.value, z_domain) else {
            continue;
        };
        append_tick_candidates(
            &mut tick_marks,
            AxisDimension::Z,
            position,
            style.axis_color,
        )?;
        if let Some(ruler) = z_ruler {
            append_projected_tick(
                &mut overlay,
                &mut additional_text,
                AxisDimension::Z,
                index,
                tick,
                ruler_point(AxisDimension::Z, ruler_selection.z, position)?,
                ruler.outward,
                view_projection,
                viewport,
                style,
            )?;
        }
        if axes.grid_z {
            push_line_3d(
                &mut grid_segments,
                [far_x, 0.0, position],
                [far_x, 1.0, position],
                style.grid_color,
            )?;
            push_line_3d(
                &mut grid_segments,
                [0.0, far_y, position],
                [1.0, far_y, position],
                style.grid_color,
            )?;
        }
    }

    position_projected_axes_text(&mut additional_text, viewport, x_ruler, y_ruler, z_ruler)?;
    Ok(Axes3DLayout {
        text: FirstLayoutPass::from_overlay(overlay, 1, additional_text)
            .preserve_axes_text_anchors(),
        grid_lines: LineSegmentBatch3D {
            segments: grid_segments,
            width_css_px: style.grid_width_css_px,
            pattern: LinePattern3D::Solid,
        },
        axis_lines: LineSegmentBatch3D {
            segments: axis_segments,
            width_css_px: style.axis_width_css_px,
            pattern: LinePattern3D::Solid,
        },
        ruler_lines: RulerLineBatch3D {
            lines: ruler_lines,
            width_css_px: style.axis_width_css_px,
        },
        ruler_selection,
        tick_marks: TickMarkBatch3D {
            marks: tick_marks,
            length_css_px: style.tick_length_css_px,
            width_css_px: style.axis_width_css_px,
        },
    })
}

fn cube_edges() -> Vec<([f32; 3], [f32; 3])> {
    let mut edges = Vec::with_capacity(12);
    for &z in &[0.0, 1.0] {
        edges.extend([
            ([0.0, 0.0, z], [1.0, 0.0, z]),
            ([1.0, 0.0, z], [1.0, 1.0, z]),
            ([1.0, 1.0, z], [0.0, 1.0, z]),
            ([0.0, 1.0, z], [0.0, 0.0, z]),
        ]);
    }
    for &x in &[0.0, 1.0] {
        for &y in &[0.0, 1.0] {
            edges.push(([x, y, 0.0], [x, y, 1.0]));
        }
    }
    edges
}

fn normalized_visible_tick(value: f64, domain: TickDomain) -> Option<f32> {
    let limits = domain.limits();
    if !value.is_finite() || value < limits.lower() || value > limits.upper() {
        return None;
    }
    domain.normalize(value).ok()
}

#[derive(Clone, Copy, Debug)]
struct ProjectedRulerLayout {
    start: CssPoint,
    end: CssPoint,
    outward: [f32; 2],
}

fn projected_ruler_layout(
    axis: AxisDimension,
    candidate: u8,
    view_projection: ViewProjection3D,
    viewport: CssRect,
    center: CssPoint,
) -> Result<Option<ProjectedRulerLayout>, PlotWebError> {
    if candidate == RulerSelection3D::HIDDEN_CANDIDATE {
        return Ok(None);
    }
    let edge = ruler_edge_3d(axis, candidate)
        .map_err(|_| invalid_scene("3D ruler edge cannot be represented."))?;
    let start = project_axes_point_3d(
        view_projection,
        viewport,
        [edge.start.x, edge.start.y, edge.start.z],
    )?;
    let end = project_axes_point_3d(
        view_projection,
        viewport,
        [edge.end.x, edge.end.y, edge.end.z],
    )?;
    Ok(Some(ProjectedRulerLayout {
        start,
        end,
        outward: ruler_outward_vector(start, end, center),
    }))
}

fn ruler_point(
    axis: AxisDimension,
    candidate: u8,
    position: f32,
) -> Result<[f32; 3], PlotWebError> {
    let edge = ruler_edge_3d(axis, candidate)
        .map_err(|_| invalid_scene("3D ruler tick cannot be represented."))?;
    Ok(match axis {
        AxisDimension::X => [position, edge.start.y, edge.start.z],
        AxisDimension::Y => [edge.start.x, position, edge.start.z],
        AxisDimension::Z => [edge.start.x, edge.start.y, position],
    })
}

fn append_tick_candidates(
    marks: &mut Vec<TickMark3D>,
    axis: AxisDimension,
    position: f32,
    color: Rgba,
) -> Result<(), PlotWebError> {
    for candidate in 0..RulerSelection3D::CANDIDATE_COUNT {
        marks.push(TickMark3D {
            anchor: axes_point_3d(ruler_point(axis, candidate, position)?)?,
            color,
            axis,
            candidate,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_projected_tick(
    overlay: &mut OverlayPlan,
    text: &mut Vec<TextIntent>,
    axis: AxisDimension,
    index: usize,
    tick: &openmat_plot_layout::Tick,
    point: [f32; 3],
    outward: [f32; 2],
    view_projection: ViewProjection3D,
    viewport: CssRect,
    style: &LinearAxesStyle,
) -> Result<(), PlotWebError> {
    let anchor = project_axes_point_3d(view_projection, viewport, point)?;
    let mark_end = offset_css_point(anchor, outward, style.tick_length_css_px.get())?;
    let label_anchor = offset_css_point(mark_end, outward, style.label_gap_css_px.get() + 2.0)?;
    let prefix = match axis {
        AxisDimension::X => "x",
        AxisDimension::Y => "y",
        AxisDimension::Z => "z",
    };
    let key = format!("{prefix}-tick-{index}");
    let label_key = format!("{key}-label");
    overlay.ticks.push(OverlayTick {
        key,
        axis,
        value: tick.value,
        mark: CssLineSegment {
            start: anchor,
            end: mark_end,
        },
        label_key: label_key.clone(),
    });
    let (horizontal_alignment, vertical_alignment) = outward_text_alignment(outward);
    text.push(TextIntent {
        key: label_key,
        role: OverlayTextRole::TickLabel,
        text: Utf16Text::from_code_units(tick.label_code_units.clone()),
        anchor: label_anchor,
        horizontal_alignment,
        vertical_alignment,
        rotation_radians: 0.0,
        color: style.text_color,
        font: style.tick_font.clone(),
        interpreter: style.tick_label_interpreter,
    });
    Ok(())
}

fn position_projected_axes_text(
    intents: &mut Vec<TextIntent>,
    viewport: CssRect,
    x_ruler: Option<ProjectedRulerLayout>,
    y_ruler: Option<ProjectedRulerLayout>,
    z_ruler: Option<ProjectedRulerLayout>,
) -> Result<(), PlotWebError> {
    intents.retain(|intent| match intent.role {
        OverlayTextRole::XLabel => x_ruler.is_some(),
        OverlayTextRole::YLabel => y_ruler.is_some(),
        OverlayTextRole::ZLabel => z_ruler.is_some(),
        _ => true,
    });
    for intent in intents {
        let axis = match intent.role {
            OverlayTextRole::Title => {
                intent.anchor = CssPoint::new(
                    viewport.origin.x + viewport.size.width.get() * 0.5,
                    viewport.origin.y - 8.0,
                )
                .map_err(|_| invalid_scene("3D title anchor cannot be represented."))?;
                intent.horizontal_alignment = HorizontalAlignment::Center;
                intent.vertical_alignment = VerticalAlignment::Bottom;
                continue;
            }
            OverlayTextRole::XLabel => x_ruler.expect("hidden X labels were retained"),
            OverlayTextRole::YLabel => y_ruler.expect("hidden Y labels were retained"),
            OverlayTextRole::ZLabel => z_ruler.expect("hidden Z labels were retained"),
            OverlayTextRole::TickLabel
            | OverlayTextRole::LegendLabel
            | OverlayTextRole::Annotation => continue,
        };
        let midpoint = CssPoint::new(
            (axis.start.x + axis.end.x) * 0.5,
            (axis.start.y + axis.end.y) * 0.5,
        )
        .map_err(|_| invalid_scene("3D axis label midpoint cannot be represented."))?;
        intent.anchor = offset_css_point(midpoint, axis.outward, 32.0)?;
        (intent.horizontal_alignment, intent.vertical_alignment) =
            outward_text_alignment(axis.outward);
        intent.rotation_radians = readable_line_rotation(axis.start, axis.end);
    }
    Ok(())
}

fn readable_line_rotation(start: CssPoint, end: CssPoint) -> f32 {
    let mut rotation = (end.y - start.y).atan2(end.x - start.x);
    if rotation > std::f32::consts::FRAC_PI_2 {
        rotation -= std::f32::consts::PI;
    } else if rotation < -std::f32::consts::FRAC_PI_2 {
        rotation += std::f32::consts::PI;
    }
    rotation
}

fn ruler_outward_vector(start: CssPoint, end: CssPoint, center: CssPoint) -> [f32; 2] {
    let tangent = [end.x - start.x, end.y - start.y];
    let length = tangent[0].hypot(tangent[1]);
    if length > 1.0e-3 {
        let normal = [-tangent[1] / length, tangent[0] / length];
        let midpoint = [(start.x + end.x) * 0.5, (start.y + end.y) * 0.5];
        if normal[0].mul_add(midpoint[0] - center.x, normal[1] * (midpoint[1] - center.y)) >= 0.0 {
            normal
        } else {
            [-normal[0], -normal[1]]
        }
    } else {
        [0.0, 1.0]
    }
}

fn outward_text_alignment(outward: [f32; 2]) -> (HorizontalAlignment, VerticalAlignment) {
    let horizontal = if outward[0] > 0.35 {
        HorizontalAlignment::Start
    } else if outward[0] < -0.35 {
        HorizontalAlignment::End
    } else {
        HorizontalAlignment::Center
    };
    let vertical = if outward[1] > 0.35 {
        VerticalAlignment::Top
    } else if outward[1] < -0.35 {
        VerticalAlignment::Bottom
    } else {
        VerticalAlignment::Middle
    };
    (horizontal, vertical)
}

fn offset_css_point(
    point: CssPoint,
    direction: [f32; 2],
    distance: f32,
) -> Result<CssPoint, PlotWebError> {
    CssPoint::new(
        point.x + direction[0] * distance,
        point.y + direction[1] * distance,
    )
    .map_err(|_| invalid_scene("Projected 3D overlay cannot be represented."))
}

fn project_axes_point_3d(
    matrix: ViewProjection3D,
    viewport: CssRect,
    point: [f32; 3],
) -> Result<CssPoint, PlotWebError> {
    let columns = matrix.columns();
    let vector = [point[0], point[1], point[2], 1.0];
    let mut clip = [0.0_f32; 4];
    for (row, output) in clip.iter_mut().enumerate() {
        *output = (0..4)
            .map(|column| columns[column][row] * vector[column])
            .sum();
    }
    if !clip.iter().all(|value| value.is_finite()) || clip[3].abs() <= f32::EPSILON {
        return Err(invalid_scene(
            "Projected 3D point is outside the camera domain.",
        ));
    }
    let ndc_x = clip[0] / clip[3];
    let ndc_y = clip[1] / clip[3];
    CssPoint::new(
        viewport.origin.x + (ndc_x * 0.5 + 0.5) * viewport.size.width.get(),
        viewport.origin.y + (0.5 - ndc_y * 0.5) * viewport.size.height.get(),
    )
    .map_err(|_| invalid_scene("Projected 3D point cannot be represented."))
}

fn push_line_3d(
    output: &mut Vec<LineSegment3D>,
    start: [f32; 3],
    end: [f32; 3],
    color: Rgba,
) -> Result<(), PlotWebError> {
    output.push(LineSegment3D {
        start: axes_point_3d(start)?,
        end: axes_point_3d(end)?,
        color,
    });
    Ok(())
}

fn axes_point_3d(point: [f32; 3]) -> Result<AxesPoint3D, PlotWebError> {
    AxesPoint3D::new(point[0], point[1], point[2])
        .map_err(|_| invalid_scene("3D line point cannot be represented."))
}

fn screen_lines_from_overlay(overlay: &OverlayPlan, style: &LinearAxesStyle) -> ScreenLineBatch {
    let mut lines = Vec::with_capacity(overlay.axis_lines.len() + overlay.ticks.len());
    lines.extend(overlay.axis_lines.iter().map(|line| ScreenLine {
        segment: line.segment,
        color: line.color,
    }));
    lines.extend(overlay.ticks.iter().map(|tick| ScreenLine {
        segment: tick.mark,
        color: style.axis_color,
    }));
    ScreenLineBatch {
        lines,
        width_css_px: style.axis_width_css_px,
        pattern: LinePattern3D::Solid,
    }
}

const fn convert_interpreter(interpreter: WireTextInterpreter) -> MirTextInterpreter {
    match interpreter {
        WireTextInterpreter::Tex => MirTextInterpreter::Tex,
        WireTextInterpreter::Latex => MirTextInterpreter::Latex,
        WireTextInterpreter::None => MirTextInterpreter::None,
    }
}

fn complete_with_provisional_metrics(
    pass: openmat_plot_layout::FirstLayoutPass,
) -> Result<OverlayPlan, PlotWebError> {
    let measurements = pass
        .requests
        .iter()
        .map(|request| {
            let size = request.font.size_css_px.get();
            let width = provisional_text_width(&request.code_units, size);
            TextMetrics::new(width, size * 1.2, size * 0.8, size * 0.2, width)
                .map(|metrics| TextMeasurement {
                    key: request.key.clone(),
                    metrics,
                })
                .map_err(|_| invalid_scene("Overlay text metrics cannot be represented."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    pass.complete(measurements)
        .map_err(|_| invalid_scene("Axes overlay could not be completed."))
}

fn provisional_text_width(code_units: &[u16], font_size: f32) -> f32 {
    String::from_utf16_lossy(code_units)
        .chars()
        .map(|character| {
            if character.is_ascii_whitespace() {
                0.32
            } else if character.is_ascii() {
                0.58
            } else if matches!(character as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF) {
                1.0
            } else {
                0.68
            }
        })
        .sum::<f32>()
        * font_size
}

fn axes_text_intents(
    axes: &Axes2DProperties,
    objects: &HashMap<&str, &GraphicsObject>,
    mapping: ViewportMapping,
) -> Result<Vec<TextIntent>, PlotWebError> {
    let mut intents = Vec::new();
    for id in [
        axes.title_id.as_deref(),
        axes.x_label_id.as_deref(),
        axes.y_label_id.as_deref(),
        axes.z_label_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(GraphicsObject::Text { properties, .. }) = objects.get(id).copied() else {
            return Err(invalid_scene(
                "Axes text identifier does not resolve to Text.",
            ));
        };
        if properties.visible && (axes.visible || properties.role == TextRole::Title) {
            intents.push(text_intent(id, properties, mapping)?);
        }
    }
    Ok(intents)
}

fn legend_intent(
    axes_children: &[String],
    objects: &HashMap<&str, &GraphicsObject>,
    axes: &Axes2DProperties,
) -> Result<Option<LegendIntent>, PlotWebError> {
    let legends = axes_children
        .iter()
        .filter_map(|id| match objects.get(id.as_str()).copied() {
            Some(GraphicsObject::Legend { fields, properties }) if properties.visible => {
                Some((fields.id.as_str(), properties))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if legends.len() > 1 {
        return Err(invalid_scene("Axes contains more than one visible Legend."));
    }
    let Some((legend_id, properties)) = legends.first().copied() else {
        return Ok(None);
    };
    let entries = properties
        .series_ids
        .iter()
        .zip(&properties.label_code_units)
        .enumerate()
        .map(|(index, (series_id, label))| {
            let series = objects
                .get(series_id.as_str())
                .copied()
                .ok_or_else(|| invalid_scene("Legend series does not resolve."))?;
            Ok(LegendEntryIntent {
                key: format!("legend-{legend_id}-label-{index}"),
                label: Utf16Text::from_code_units(label.clone()),
                color: legend_series_color(series, objects, axes)?,
            })
        })
        .collect::<Result<Vec<_>, PlotWebError>>()?;
    Ok(Some(LegendIntent {
        key: format!("legend-{legend_id}"),
        placement: convert_legend_location(properties.location),
        entries,
        background: convert_color(properties.background_rgba)?,
        border: convert_color(properties.border_rgba)?,
        font: FontKey {
            family_utf16: properties.font_family_code_units.clone(),
            size_css_px: css_px(f64_to_f32(properties.font_size_css_px)?)?,
            weight: FontWeight::new(properties.font_weight)
                .map_err(|_| invalid_scene("Legend font weight cannot be represented."))?,
            style: OverlayFontStyle::Normal,
        },
        interpreter: convert_interpreter(properties.interpreter),
        columns: match properties.orientation {
            WireLegendOrientation::Horizontal => properties.series_ids.len().max(1),
            WireLegendOrientation::Vertical => usize::try_from(properties.num_columns)
                .unwrap_or(usize::MAX)
                .max(1),
        },
    }))
}

const fn convert_legend_location(location: LegendLocation) -> LegendPlacement {
    match location {
        LegendLocation::Best | LegendLocation::NorthEast => LegendPlacement::NorthEast,
        LegendLocation::North => LegendPlacement::North,
        LegendLocation::South => LegendPlacement::South,
        LegendLocation::East => LegendPlacement::East,
        LegendLocation::West => LegendPlacement::West,
        LegendLocation::NorthWest => LegendPlacement::NorthWest,
        LegendLocation::SouthEast => LegendPlacement::SouthEast,
        LegendLocation::SouthWest => LegendPlacement::SouthWest,
        LegendLocation::SouthOutside => LegendPlacement::SouthOutside,
    }
}

fn legend_series_color(
    series: &GraphicsObject,
    objects: &HashMap<&str, &GraphicsObject>,
    axes: &Axes2DProperties,
) -> Result<Rgba, PlotWebError> {
    match series {
        GraphicsObject::LineSeries { properties, .. } => convert_color(properties.color_rgba),
        GraphicsObject::ScatterSeries { properties, .. } => {
            let face = properties.marker_face_rgba;
            if face.0[3] > 0.0 {
                convert_color(face)
            } else {
                convert_color(properties.marker_edge_rgba)
            }
        }
        GraphicsObject::SurfaceSeries { properties, .. } => representative_area_color(
            properties.face_color,
            properties.face_rgba.as_ref().copied(),
            properties.edge_color,
            properties.edge_rgba.as_ref().copied(),
            axes,
        ),
        GraphicsObject::PatchSeries { properties, .. } => representative_area_color(
            properties.face_color,
            properties.face_rgba.as_ref().copied(),
            properties.edge_color,
            properties.edge_rgba.as_ref().copied(),
            axes,
        ),
        GraphicsObject::ChartGroup { fields, .. } => {
            let child = fields
                .children
                .iter()
                .find_map(|id| objects.get(id.as_str()).copied())
                .ok_or_else(|| invalid_scene("Legend chart has no retained primitive."))?;
            legend_series_color(child, objects, axes)
        }
        _ => Err(invalid_scene(
            "Legend membership is not a plottable series.",
        )),
    }
}

fn representative_area_color(
    face_mode: SurfaceColorMode,
    face: Option<WireRgba>,
    edge_mode: SurfaceColorMode,
    edge: Option<WireRgba>,
    axes: &Axes2DProperties,
) -> Result<Rgba, PlotWebError> {
    if face_mode == SurfaceColorMode::Uniform {
        return convert_color(face.ok_or_else(|| invalid_scene("Uniform face color is missing."))?);
    }
    if edge_mode == SurfaceColorMode::Uniform {
        return convert_color(edge.ok_or_else(|| invalid_scene("Uniform edge color is missing."))?);
    }
    let colormap = axes_colormap(axes);
    colormap_color(colormap[colormap.len() / 2])
}

fn text_intent(
    id: &str,
    properties: &TextProperties,
    mapping: ViewportMapping,
) -> Result<TextIntent, PlotWebError> {
    let anchor = mapping
        .axes_to_css(
            AxesPoint::new(
                f64_to_f32(properties.anchor_normalized[0])?,
                f64_to_f32(properties.anchor_normalized[1])?,
            )
            .map_err(|_| invalid_scene("Text anchor cannot be represented."))?,
        )
        .map_err(|_| invalid_scene("Text anchor cannot be mapped."))?;
    Ok(TextIntent {
        key: format!("text-{id}"),
        role: match properties.role {
            TextRole::Title => OverlayTextRole::Title,
            TextRole::XLabel => OverlayTextRole::XLabel,
            TextRole::YLabel => OverlayTextRole::YLabel,
            TextRole::ZLabel => OverlayTextRole::ZLabel,
            TextRole::Annotation => OverlayTextRole::Annotation,
        },
        text: Utf16Text::from_code_units(properties.code_units.clone()),
        anchor,
        horizontal_alignment: match properties.horizontal_alignment {
            WireHorizontalAlignment::Left => HorizontalAlignment::Start,
            WireHorizontalAlignment::Center => HorizontalAlignment::Center,
            WireHorizontalAlignment::Right => HorizontalAlignment::End,
        },
        vertical_alignment: match properties.vertical_alignment {
            WireVerticalAlignment::Top => VerticalAlignment::Top,
            WireVerticalAlignment::Middle => VerticalAlignment::Middle,
            WireVerticalAlignment::Bottom => VerticalAlignment::Bottom,
            WireVerticalAlignment::Baseline => VerticalAlignment::Baseline,
        },
        rotation_radians: -f64_to_f32(properties.rotation_degrees)?.to_radians(),
        color: convert_color(properties.color_rgba)?,
        font: FontKey {
            family_utf16: properties.font_family_code_units.clone(),
            size_css_px: css_px(f64_to_f32(properties.font_size_css_px)?)?,
            weight: FontWeight::new(properties.font_weight)
                .map_err(|_| invalid_scene("Text font weight cannot be represented."))?,
            style: match properties.font_style {
                FontStyle::Normal => OverlayFontStyle::Normal,
                FontStyle::Italic => OverlayFontStyle::Italic,
            },
        },
        interpreter: convert_interpreter(properties.interpreter),
    })
}

#[allow(clippy::too_many_arguments)]
fn add_line(
    builder: &mut MirFrameBuilder,
    properties: &LineSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    transform: AxesTransform,
    order: DrawOrder,
    device_pixel_ratio: f64,
    axes_width_css_px: f32,
    visible_x_limits: [f64; 2],
    cache: &mut SceneCache,
) -> Result<(), PlotWebError> {
    if !properties.clipping {
        return Err(invalid_scene(
            "The first WebGPU slice supports axes-clipped LineSeries.",
        ));
    }
    let x = decode_buffer(&properties.x_data, buffers)?;
    let y = decode_buffer(&properties.y_data, buffers)?;
    let masked = mask_logarithmic_series(&x, &y, axes)?;
    let input = masked.as_ref().map_or(
        SeriesGeometryInput {
            x: Some(x.view()),
            y: y.view(),
        },
        |(x, y)| SeriesGeometryInput {
            x: Some(NumericView::F64(x)),
            y: NumericView::F64(y),
        },
    );
    let color = convert_color(properties.color_rgba)?;
    let requested_width_css_px = f64_to_f32(properties.line_width_css_px)?;
    let hairline_width_css_px = f64_to_f32(1.0 / device_pixel_ratio)?;
    let stroke = (properties.line_style != LineStyle::None)
        .then(|| {
            line_stroke_style(
                properties.line_style,
                requested_width_css_px.max(hairline_width_css_px),
            )
        })
        .transpose()?;
    let device_columns = (f64::from(axes_width_css_px) * device_pixel_ratio)
        .ceil()
        .max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let direct_budget = (device_columns as usize)
        .saturating_mul(DIRECT_POINTS_PER_DEVICE_COLUMN)
        .max(MIN_DIRECT_POINT_BUDGET);
    let lod_runs = if input.y.len() > direct_budget {
        let view = LinearLodView::new(
            visible_x_limits[0],
            visible_x_limits[1],
            f64::from(axes_width_css_px),
            device_pixel_ratio,
        )
        .map_err(|_| invalid_scene("LineSeries LOD view could not be represented."))?;
        let key = LodCacheKey::new(
            LodSourceKey {
                x: Some(LodResourceRevision::new(
                    properties.x_data.buffer_id.clone(),
                    0,
                )),
                y: LodResourceRevision::new(properties.y_data.buffer_id.clone(), 0),
            },
            view,
        );
        Some(
            cache
                .get_or_build_lod(key, input, view)
                .map_err(|_| invalid_scene("LineSeries LOD could not be generated."))?,
        )
    } else {
        None
    };
    let mapper = |x, y| {
        transform
            .map(x, y)
            .map_err(|_| GeometryError::ArithmeticOverflow("axes transform"))
    };
    if let Some(stroke) = stroke {
        if let Some(runs) = &lod_runs {
            builder
                .add_line_runs(runs, order, None, stroke, color, 8, mapper)
                .map_err(|_| invalid_scene("LineSeries LOD geometry could not be generated."))?;
        } else {
            builder
                .add_line(input, order, None, stroke, color, 8, mapper)
                .map_err(|_| invalid_scene("LineSeries geometry could not be generated."))?;
        }
    }
    add_line_markers(
        builder,
        properties,
        input,
        lod_runs.as_deref().map(Vec::as_slice),
        order,
        color,
        transform,
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn add_polar_line(
    builder: &mut MirFrameBuilder,
    properties: &LineSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    order: DrawOrder,
    device_pixel_ratio: f64,
) -> Result<(), PlotWebError> {
    let theta = decode_buffer(&properties.x_data, buffers)?.values_f64();
    let radius = decode_buffer(&properties.y_data, buffers)?.values_f64();
    if theta.len() != radius.len() {
        return Err(invalid_scene(
            "Polar Line ThetaData and RData lengths differ.",
        ));
    }
    let period = match axes.theta_axis_units {
        ThetaAxisUnits::Degrees => 360.0,
        ThetaAxisUnits::Radians => std::f64::consts::TAU,
    };
    let unit_scale = period / std::f64::consts::TAU;
    let theta_span = axes.x_limits[1] - axes.x_limits[0];
    let radial_span = axes.y_limits[1] - axes.y_limits[0];
    let zero = match axes.theta_zero_location {
        ThetaZeroLocation::Right => 0.0,
        ThetaZeroLocation::Top => std::f64::consts::FRAC_PI_2,
        ThetaZeroLocation::Left => std::f64::consts::PI,
        ThetaZeroLocation::Bottom => -std::f64::consts::FRAC_PI_2,
    };
    let direction = match axes.theta_direction {
        ThetaDirection::Counterclockwise => 1.0,
        ThetaDirection::Clockwise => -1.0,
    };
    let mut mapped_x = Vec::with_capacity(theta.len());
    let mut mapped_y = Vec::with_capacity(theta.len());
    for (&theta_radians, &signed_radius) in theta.iter().zip(&radius) {
        if !theta_radians.is_finite() || !signed_radius.is_finite() {
            mapped_x.push(f64::NAN);
            mapped_y.push(f64::NAN);
            continue;
        }
        let active_theta = theta_radians * unit_scale;
        let displayed_theta = if theta_span <= period * (1.0 + 1.0e-12) {
            axes.x_limits[0] + (active_theta - axes.x_limits[0]).rem_euclid(period)
        } else {
            active_theta
        };
        let radius = signed_radius.abs();
        if displayed_theta < axes.x_limits[0] - 1.0e-12
            || displayed_theta > axes.x_limits[1] + 1.0e-12
            || radius < axes.y_limits[0]
            || radius > axes.y_limits[1]
        {
            mapped_x.push(f64::NAN);
            mapped_y.push(f64::NAN);
            continue;
        }
        let mut angle = zero + direction * displayed_theta / period * std::f64::consts::TAU;
        if signed_radius < 0.0 {
            angle += std::f64::consts::PI;
        }
        let normalized_radius = (radius - axes.y_limits[0]) / radial_span;
        mapped_x.push(0.5 + 0.5 * normalized_radius * angle.cos());
        mapped_y.push(0.5 + 0.5 * normalized_radius * angle.sin());
    }
    let input = SeriesGeometryInput {
        x: Some(NumericView::F64(&mapped_x)),
        y: NumericView::F64(&mapped_y),
    };
    let unit_limits = SemanticLimits::new(
        Limits::new(0.0, 1.0).map_err(|_| invalid_scene("Unit limits are invalid."))?,
    );
    let transform = AxesTransform::from_semantic(unit_limits, unit_limits)
        .map_err(|_| invalid_scene("Polar unit transform cannot be represented."))?;
    let color = convert_color(properties.color_rgba)?;
    let requested_width_css_px = f64_to_f32(properties.line_width_css_px)?;
    let hairline_width_css_px = f64_to_f32(1.0 / device_pixel_ratio)?;
    if properties.line_style != LineStyle::None {
        let stroke = line_stroke_style(
            properties.line_style,
            requested_width_css_px.max(hairline_width_css_px),
        )?;
        builder
            .add_line(input, order, None, stroke, color, 8, |x, y| {
                transform
                    .map(x, y)
                    .map_err(|_| GeometryError::ArithmeticOverflow("polar unit transform"))
            })
            .map_err(|_| invalid_scene("Polar Line geometry could not be generated."))?;
    }
    if properties.marker != Marker::None {
        let marker_indices = properties
            .marker_indices
            .as_ref()
            .map(|indices| indices.iter().map(|index| index.get()).collect::<Vec<_>>());
        let marker_face = properties
            .marker_face_rgba
            .map_or(Ok(Rgba::TRANSPARENT), convert_color)?;
        let marker_edge = properties
            .marker_edge_rgba
            .map_or(Ok(color), convert_color)?;
        add_marker_series(
            builder,
            input,
            marker_indices.as_deref(),
            order,
            properties.marker,
            properties.marker_size_css_px,
            marker_face,
            marker_edge,
            1.0,
            transform,
        )?;
    }
    Ok(())
}

type MaskedSeries = Option<(Vec<f64>, Vec<f64>)>;

fn mask_logarithmic_series(
    x: &NumericBuffer,
    y: &NumericBuffer,
    axes: &Axes2DProperties,
) -> Result<MaskedSeries, PlotWebError> {
    if axes.x_scale == WireScale::Linear && axes.y_scale == WireScale::Linear {
        return Ok(None);
    }
    let mut x = x.values_f64();
    let mut y = y.values_f64();
    if x.len() != y.len() {
        return Err(invalid_scene("Series XData and YData lengths differ."));
    }
    for index in 0..x.len() {
        if !axis_value_in_scale(x[index], axes.x_scale)
            || !axis_value_in_scale(y[index], axes.y_scale)
        {
            x[index] = f64::NAN;
            y[index] = f64::NAN;
        }
    }
    Ok(Some((x, y)))
}

fn axis_value_in_scale(value: f64, scale: WireScale) -> bool {
    value.is_finite() && (scale == WireScale::Linear || value > 0.0)
}

fn transformed_axis_limits(limits: [f64; 2], scale: WireScale) -> Result<[f64; 2], PlotWebError> {
    let mapped = match scale {
        WireScale::Linear => limits,
        WireScale::Log => [limits[0].log10(), limits[1].log10()],
    };
    if mapped[0].is_finite() && mapped[1].is_finite() && mapped[0] < mapped[1] {
        Ok(mapped)
    } else {
        Err(invalid_scene(
            "Logarithmic axes limits cannot be normalized.",
        ))
    }
}

fn transform_axis_value(
    value: f64,
    limits: [f64; 2],
    scale: WireScale,
    direction: WireAxisDirection,
) -> Option<f64> {
    if !axis_value_in_scale(value, scale) {
        return None;
    }
    let mapped = match scale {
        WireScale::Linear => value,
        WireScale::Log => value.log10(),
    };
    let mapped_limits = match scale {
        WireScale::Linear => limits,
        WireScale::Log => [limits[0].log10(), limits[1].log10()],
    };
    let directed = match direction {
        WireAxisDirection::Normal => mapped,
        WireAxisDirection::Reverse => mapped_limits[0] + (mapped_limits[1] - mapped),
    };
    directed.is_finite().then_some(directed)
}

fn transformed_bounds_3d(axes: &Axes2DProperties) -> Result<DataBounds3D, PlotWebError> {
    let x = transformed_axis_limits(axes.x_limits, axes.x_scale)?;
    let y = transformed_axis_limits(axes.y_limits, axes.y_scale)?;
    let z = transformed_axis_limits(axes.z_limits, axes.z_scale)?;
    DataBounds3D::new(x, y, z).map_err(|_| invalid_scene("3D axes limits cannot be normalized."))
}

fn transform_point_3d(
    x: f64,
    y: f64,
    z: f64,
    axes: &Axes2DProperties,
    bounds: DataBounds3D,
) -> Result<Option<AxesPoint3D>, PlotWebError> {
    let Some(x) = transform_axis_value(x, axes.x_limits, axes.x_scale, axes.x_direction) else {
        return Ok(None);
    };
    let Some(y) = transform_axis_value(y, axes.y_limits, axes.y_scale, axes.y_direction) else {
        return Ok(None);
    };
    let Some(z) = transform_axis_value(z, axes.z_limits, axes.z_scale, axes.z_direction) else {
        return Ok(None);
    };
    bounds
        .normalize_point(x, y, z)
        .map(Some)
        .map_err(|_| invalid_scene("3D point cannot be normalized."))
}

fn transform_axis_data(
    values: &[f64],
    limits: [f64; 2],
    scale: WireScale,
    direction: WireAxisDirection,
) -> Vec<f64> {
    values
        .iter()
        .map(|&value| transform_axis_value(value, limits, scale, direction).unwrap_or(f64::NAN))
        .collect()
}

fn transform_patch_vertices(
    vertices: &[f64],
    rows: usize,
    columns: usize,
    axes: &Axes2DProperties,
) -> Vec<f64> {
    let mut transformed = vertices.to_vec();
    let mappings = [
        (axes.x_limits, axes.x_scale, axes.x_direction),
        (axes.y_limits, axes.y_scale, axes.y_direction),
        (axes.z_limits, axes.z_scale, axes.z_direction),
    ];
    for (column, &(limits, scale, direction)) in mappings.iter().take(columns).enumerate() {
        let start = column * rows;
        let end = start + rows;
        let values = transform_axis_data(&vertices[start..end], limits, scale, direction);
        transformed[start..end].copy_from_slice(&values);
    }
    transformed
}

#[allow(clippy::too_many_arguments)]
fn add_line_markers(
    builder: &mut MirFrameBuilder,
    properties: &LineSeriesProperties,
    input: SeriesGeometryInput<'_>,
    lod_runs: Option<&[LineRun]>,
    order: DrawOrder,
    color: Rgba,
    transform: AxesTransform,
) -> Result<(), PlotWebError> {
    if properties.marker == Marker::None {
        return Ok(());
    }
    let marker_indices = properties
        .marker_indices
        .as_ref()
        .map(|indices| indices.iter().map(|index| index.get()).collect::<Vec<_>>());
    let lod_marker_data = if marker_indices.is_none() {
        lod_runs.map(|runs| {
            runs.iter()
                .flat_map(|run| run.points.iter().map(|point| (point.x, point.y)))
                .unzip::<_, _, Vec<_>, Vec<_>>()
        })
    } else {
        None
    };
    let marker_input = lod_marker_data
        .as_ref()
        .map_or(input, |(x, y)| SeriesGeometryInput {
            x: Some(NumericView::F64(x)),
            y: NumericView::F64(y),
        });
    let marker_face = properties
        .marker_face_rgba
        .map_or(Ok(Rgba::TRANSPARENT), convert_color)?;
    let marker_edge = properties
        .marker_edge_rgba
        .map_or(Ok(color), convert_color)?;
    add_marker_series(
        builder,
        marker_input,
        marker_indices.as_deref(),
        order,
        properties.marker,
        properties.marker_size_css_px,
        marker_face,
        marker_edge,
        1.0,
        transform,
    )
}

fn line_stroke_style(style: LineStyle, width_css_px: f32) -> Result<StrokeStyle, PlotWebError> {
    let width = css_px(width_css_px)?;
    let cap = if matches!(style, LineStyle::Dot | LineStyle::DashDot) {
        LineCap::Round
    } else {
        LineCap::Butt
    };
    let mut stroke = StrokeStyle::solid(width, cap, LineJoin::Miter)
        .map_err(|_| invalid_scene("LineSeries stroke cannot be represented."))?;
    let scale = width_css_px.max(1.0);
    let pattern = match style {
        LineStyle::None => unreachable!("none LineStyle is filtered before stroke creation"),
        LineStyle::Solid => &[][..],
        LineStyle::Dash => &[6.0, 3.0][..],
        LineStyle::Dot => &[1.0, 2.5][..],
        LineStyle::DashDot => &[6.0, 3.0, 1.0, 3.0][..],
    };
    stroke.dash_pattern_css_px = pattern
        .iter()
        .map(|length| css_px(length * scale))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stroke)
}

fn add_line_3d(
    builder: &mut MirFrameBuilder,
    properties: &LineSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    let z_ref = properties
        .z_data
        .as_ref()
        .ok_or_else(|| invalid_scene("A 3D Line requires ZData."))?;
    let x = decode_buffer(&properties.x_data, buffers)?.values_f64();
    let y = decode_buffer(&properties.y_data, buffers)?.values_f64();
    let z = decode_buffer(z_ref, buffers)?.values_f64();
    if x.len() != y.len() || x.len() != z.len() {
        return Err(invalid_scene("LineSeries X/Y/ZData lengths differ."));
    }
    let bounds = transformed_bounds_3d(axes)?;
    let color = convert_color(properties.color_rgba)?;
    let marker_face = properties
        .marker_face_rgba
        .map_or(Ok(Rgba::TRANSPARENT), convert_color)?;
    let marker_edge = properties
        .marker_edge_rgba
        .map_or(Ok(color), convert_color)?;
    let mut points = Vec::with_capacity(x.len());
    for index in 0..x.len() {
        points.push(transform_point_3d(
            x[index], y[index], z[index], axes, bounds,
        )?);
    }
    let segments = points
        .windows(2)
        .filter_map(|pair| {
            Some(LineSegment3D {
                start: pair[0]?,
                end: pair[1]?,
                color,
            })
        })
        .collect();
    if properties.line_style != LineStyle::None {
        let pattern = match properties.line_style {
            LineStyle::None => unreachable!("guarded above"),
            LineStyle::Solid => LinePattern3D::Solid,
            LineStyle::Dash => LinePattern3D::Dash,
            LineStyle::Dot => LinePattern3D::Dot,
            LineStyle::DashDot => LinePattern3D::DashDot,
        };
        builder.add_line_segments_3d(
            order,
            None,
            LineSegmentBatch3D {
                segments,
                width_css_px: css_px(f64_to_f32(properties.line_width_css_px)?)?,
                pattern,
            },
        );
    }
    if properties.marker != Marker::None {
        let indices = properties.marker_indices.as_ref().map_or_else(
            || (0..points.len()).collect::<Vec<_>>(),
            |indices| {
                indices
                    .iter()
                    .filter_map(|index| usize::try_from(index.get() - 1).ok())
                    .collect()
            },
        );
        let centers = indices
            .into_iter()
            .filter_map(|index| points.get(index).copied().flatten())
            .collect::<Vec<_>>();
        add_marker_points_3d(
            builder,
            properties.marker,
            centers,
            properties.marker_size_css_px,
            marker_face,
            marker_edge,
            1.0,
            order,
        )?;
    }
    Ok(())
}

fn add_scatter_3d(
    builder: &mut MirFrameBuilder,
    properties: &ScatterSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    if !properties.clipping {
        return Err(invalid_scene(
            "The 3D marker pipeline currently requires axes clipping.",
        ));
    }
    let z_ref = properties
        .z_data
        .as_ref()
        .ok_or_else(|| invalid_scene("A 3D Scatter requires ZData."))?;
    let x = decode_buffer(&properties.x_data, buffers)?.values_f64();
    let y = decode_buffer(&properties.y_data, buffers)?.values_f64();
    let z = decode_buffer(z_ref, buffers)?.values_f64();
    if x.len() != y.len() || x.len() != z.len() {
        return Err(invalid_scene("ScatterSeries X/Y/ZData lengths differ."));
    }
    let bounds = transformed_bounds_3d(axes)?;
    if properties.size_data.as_ref().is_none() && properties.color_data.as_ref().is_none() {
        let centers = (0..x.len())
            .map(|index| transform_point_3d(x[index], y[index], z[index], axes, bounds))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        return add_marker_points_3d(
            builder,
            properties.marker,
            centers,
            properties.marker_size_css_px,
            convert_color(properties.marker_face_rgba)?,
            convert_color(properties.marker_edge_rgba)?,
            properties.marker_edge_width_css_px,
            order,
        );
    }
    let sizes = properties
        .size_data
        .as_ref()
        .map(|data| decode_buffer(data, buffers).map(|buffer| buffer.values_f64()))
        .transpose()?;
    let colors = properties
        .color_data
        .as_ref()
        .map(|data| decode_buffer(data, buffers).map(|buffer| buffer.values_f64()))
        .transpose()?;
    validate_scatter_instance_data(x.len(), sizes.as_deref(), colors.as_deref())?;
    let rendering = supported_marker(properties.marker)?;
    let fixed_fill = convert_color(properties.marker_face_rgba)?;
    let fixed_stroke = convert_color(properties.marker_edge_rgba)?;
    let stroke_width = css_px(1.0)?;
    let mut instances = Vec::with_capacity(x.len());
    for index in 0..x.len() {
        let Some(center) = transform_point_3d(x[index], y[index], z[index], axes, bounds)? else {
            continue;
        };
        let mapped = colors
            .as_ref()
            .map(|colors| {
                surface_colormap(
                    colors[index],
                    axes.c_limits,
                    openmat_plot_protocol::CDataMapping::Scaled,
                    axes_colormap(axes),
                )
            })
            .transpose()?;
        instances.push(MarkerInstance3D {
            center,
            size_css_px: scatter_size_css_px(
                sizes
                    .as_ref()
                    .map_or(properties.marker_size_css_px, |values| {
                        values[if values.len() == 1 { 0 } else { index }].sqrt() * (96.0 / 72.0)
                    }),
                rendering.size_scale,
            )?,
            rotation_radians: rendering.rotation_radians,
            fill: if properties.color_data_target == ScatterColorTarget::Face {
                mapped.unwrap_or(fixed_fill)
            } else {
                fixed_fill
            },
            stroke: if properties.color_data_target == ScatterColorTarget::Edge {
                mapped.unwrap_or(fixed_stroke)
            } else {
                fixed_stroke
            },
            stroke_width_css_px: stroke_width,
        });
    }
    add_marker_instances_3d(builder, rendering, instances, order);
    Ok(())
}

fn add_marker_instances_3d(
    builder: &mut MirFrameBuilder,
    rendering: MarkerRendering,
    instances: Vec<MarkerInstance3D>,
    order: DrawOrder,
) {
    builder.add_markers_3d(
        order,
        None,
        MarkerBatch3D {
            shape: rendering.primary,
            instances: instances.clone(),
        },
    );
    if let Some(secondary) = rendering.secondary {
        builder.add_markers_3d(
            order,
            None,
            MarkerBatch3D {
                shape: secondary,
                instances,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn add_marker_points_3d(
    builder: &mut MirFrameBuilder,
    marker: Marker,
    centers: Vec<AxesPoint3D>,
    size_css_px: f64,
    fill: Rgba,
    stroke: Rgba,
    stroke_width_css_px: f64,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    let rendering = supported_marker(marker)?;
    let size = css_px(f64_to_f32(size_css_px)? * rendering.size_scale)?;
    let instances = centers
        .into_iter()
        .map(|center| {
            Ok(MarkerInstance3D {
                center,
                size_css_px: size,
                rotation_radians: rendering.rotation_radians,
                fill,
                stroke,
                stroke_width_css_px: css_px(f64_to_f32(stroke_width_css_px)?)?,
            })
        })
        .collect::<Result<Vec<_>, PlotWebError>>()?;
    let batch = |shape, instances| MarkerBatch3D { shape, instances };
    builder.add_markers_3d(order, None, batch(rendering.primary, instances.clone()));
    if let Some(shape) = rendering.secondary {
        builder.add_markers_3d(order, None, batch(shape, instances));
    }
    Ok(())
}

fn add_surface(
    builder: &mut MirFrameBuilder,
    properties: &SurfaceSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    let x = decode_buffer(&properties.x_data, buffers)?.values_f64();
    let y = decode_buffer(&properties.y_data, buffers)?.values_f64();
    let z = decode_buffer(&properties.z_data, buffers)?.values_f64();
    let rows = usize::try_from(properties.z_data.shape[0])
        .map_err(|_| invalid_scene("Surface row count exceeds browser limits."))?;
    let columns = usize::try_from(properties.z_data.shape[1])
        .map_err(|_| invalid_scene("Surface column count exceeds browser limits."))?;
    let resolved_colors = resolve_surface_colors(properties, buffers, axes)?;
    let x = transform_axis_data(&x, axes.x_limits, axes.x_scale, axes.x_direction);
    let y = transform_axis_data(&y, axes.y_limits, axes.y_scale, axes.y_direction);
    let z = transform_axis_data(&z, axes.z_limits, axes.z_scale, axes.z_direction);
    let bounds = transformed_bounds_3d(axes)?;
    let has_edges =
        properties.edge_color != SurfaceColorMode::None && properties.line_style != LineStyle::None;
    if properties.face_color != SurfaceColorMode::None {
        let colors = match properties.face_color {
            SurfaceColorMode::Uniform => SurfaceColors::Uniform(convert_color(
                properties
                    .face_rgba
                    .as_ref()
                    .copied()
                    .ok_or_else(|| invalid_scene("Uniform Surface face color is missing."))?,
            )?),
            SurfaceColorMode::Flat => SurfaceColors::Flat(&resolved_colors),
            SurfaceColorMode::Interp => SurfaceColors::Interp(&resolved_colors),
            SurfaceColorMode::None => unreachable!("guarded above"),
        };
        let mesh = tessellate_surface_grid(
            SurfaceGridView {
                rows,
                columns,
                x: &x,
                y: &y,
                z: &z,
                colors,
            },
            bounds,
        )
        .map_err(|_| invalid_scene("Surface geometry could not be generated."))?;
        if has_edges {
            builder.add_surface_with_edges_3d(order, None, mesh);
        } else {
            builder.add_surface_3d(order, None, mesh);
        }
    }
    if has_edges {
        let colors = match properties.edge_color {
            SurfaceColorMode::Uniform => SurfaceColors::Uniform(convert_color(
                properties
                    .edge_rgba
                    .as_ref()
                    .copied()
                    .ok_or_else(|| invalid_scene("Uniform Surface edge color is missing."))?,
            )?),
            SurfaceColorMode::Flat | SurfaceColorMode::Interp => {
                SurfaceColors::Flat(&resolved_colors)
            }
            SurfaceColorMode::None => unreachable!("guarded above"),
        };
        let pattern = match properties.line_style {
            LineStyle::None => unreachable!("has_edges excludes none"),
            LineStyle::Solid => LinePattern3D::Solid,
            LineStyle::Dash => LinePattern3D::Dash,
            LineStyle::Dot => LinePattern3D::Dot,
            LineStyle::DashDot => LinePattern3D::DashDot,
        };
        let edges = surface_grid_edges(
            SurfaceGridView {
                rows,
                columns,
                x: &x,
                y: &y,
                z: &z,
                colors,
            },
            bounds,
            css_px(f64_to_f32(properties.line_width_css_px)?)?,
            pattern,
        )
        .map_err(|_| invalid_scene("Surface edge geometry could not be generated."))?;
        builder.add_surface_edge_segments_3d(order, None, edges);
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Patch keeps its shared face/edge decode and validation in one path.
fn add_patch(
    builder: &mut MirFrameBuilder,
    properties: &PatchSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    let faces = decode_buffer(&properties.faces, buffers)?.values_f64();
    let vertices = decode_buffer(&properties.vertices, buffers)?.values_f64();
    let face_rows = usize::try_from(properties.faces.shape[0])
        .map_err(|_| invalid_scene("Patch face count exceeds browser limits."))?;
    let face_columns = usize::try_from(properties.faces.shape[1])
        .map_err(|_| invalid_scene("Patch face width exceeds browser limits."))?;
    let vertex_rows = usize::try_from(properties.vertices.shape[0])
        .map_err(|_| invalid_scene("Patch vertex count exceeds browser limits."))?;
    let vertex_columns = usize::try_from(properties.vertices.shape[1])
        .map_err(|_| invalid_scene("Patch coordinate count exceeds browser limits."))?;
    let vertices = transform_patch_vertices(&vertices, vertex_rows, vertex_columns, axes);
    let vertex_normals = properties
        .vertex_normals
        .as_ref()
        .map(|descriptor| decode_buffer(descriptor, buffers).map(|buffer| buffer.values_f64()))
        .transpose()?;
    let resolved_colors = resolve_patch_colors(properties, buffers, axes)?;
    let has_edges =
        properties.edge_color != SurfaceColorMode::None && properties.line_style != LineStyle::None;
    let is_3d = vertex_columns == 3;

    if properties.face_color != SurfaceColorMode::None {
        let colors = patch_colors(
            properties.face_color,
            properties.face_rgba.as_ref().copied(),
            &resolved_colors,
            face_rows,
            vertex_rows,
            "face",
        )?;
        let view = PatchMeshView {
            face_rows,
            face_columns,
            faces: &faces,
            vertex_rows,
            vertex_columns,
            vertices: &vertices,
            vertex_normals: vertex_normals.as_deref(),
            colors,
            smooth_normals: properties.smooth_normals,
        };
        if is_3d {
            let bounds = transformed_bounds_3d(axes)?;
            let mesh = tessellate_patch_mesh(view, bounds)
                .map_err(|error| patch_geometry_error("Patch face", &error))?;
            if has_edges {
                builder.add_surface_with_edges_3d(order, None, mesh);
            } else {
                builder.add_surface_3d(order, None, mesh);
            }
        } else {
            let x_limits = transformed_axis_limits(axes.x_limits, axes.x_scale)?;
            let y_limits = transformed_axis_limits(axes.y_limits, axes.y_scale)?;
            let mut mesh = tessellate_patch_mesh_2d(view, x_limits, y_limits)
                .map_err(|error| patch_geometry_error("2D Patch face", &error))?;
            let face_alpha = f64_to_f32(properties.face_alpha)?;
            for vertex in &mut mesh.vertices {
                vertex.color.alpha *= face_alpha;
            }
            builder.add_triangle_mesh_2d(order, None, mesh);
        }
    }

    if has_edges {
        let colors = patch_colors(
            properties.edge_color,
            properties.edge_rgba.as_ref().copied(),
            &resolved_colors,
            face_rows,
            vertex_rows,
            "edge",
        )?;
        let view = PatchMeshView {
            face_rows,
            face_columns,
            faces: &faces,
            vertex_rows,
            vertex_columns,
            vertices: &vertices,
            vertex_normals: vertex_normals.as_deref(),
            colors,
            smooth_normals: properties.smooth_normals,
        };
        if is_3d {
            let pattern = match properties.line_style {
                LineStyle::None => unreachable!("has_edges excludes none"),
                LineStyle::Solid => LinePattern3D::Solid,
                LineStyle::Dash => LinePattern3D::Dash,
                LineStyle::Dot => LinePattern3D::Dot,
                LineStyle::DashDot => LinePattern3D::DashDot,
            };
            let bounds = transformed_bounds_3d(axes)?;
            let edges = patch_mesh_edges(
                view,
                bounds,
                css_px(f64_to_f32(properties.line_width_css_px)?)?,
                pattern,
            )
            .map_err(|_| invalid_scene("Patch edge geometry could not be generated."))?;
            builder.add_surface_edge_segments_3d(order, None, edges);
        } else {
            add_patch_edges_2d(builder, view, properties, axes, order)?;
        }
    }
    Ok(())
}

fn add_patch_edges_2d(
    builder: &mut MirFrameBuilder,
    view: PatchMeshView<'_>,
    properties: &PatchSeriesProperties,
    axes: &Axes2DProperties,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    let x_limits = transformed_axis_limits(axes.x_limits, axes.x_scale)?;
    let y_limits = transformed_axis_limits(axes.y_limits, axes.y_scale)?;
    let edges = patch_mesh_edges_2d(view, x_limits, y_limits)
        .map_err(|_| invalid_scene("2D Patch edge geometry could not be generated."))?;
    let style = line_stroke_style(
        properties.line_style,
        f64_to_f32(properties.line_width_css_px)?,
    )?;
    let edge_alpha = f64_to_f32(properties.edge_alpha)?;
    for edge in edges {
        let mut color = edge.color;
        color.alpha *= edge_alpha;
        let run = LineRun {
            points: vec![
                GeometryDataPoint {
                    source_index: 0,
                    x: f64::from(edge.start.x),
                    y: f64::from(edge.start.y),
                },
                GeometryDataPoint {
                    source_index: 1,
                    x: f64::from(edge.end.x),
                    y: f64::from(edge.end.y),
                },
            ],
        };
        builder
            .add_line_runs(&[run], order, None, style.clone(), color, 8, |x, y| {
                #[allow(clippy::cast_possible_truncation)]
                AxesPoint::new(x as f32, y as f32).map_err(GeometryError::from)
            })
            .map_err(|_| invalid_scene("2D Patch edge stroke could not be generated."))?;
    }
    Ok(())
}

fn resolve_patch_colors(
    properties: &PatchSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
) -> Result<Vec<Rgba>, PlotWebError> {
    let needs_data_colors = matches!(
        properties.face_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    ) || matches!(
        properties.edge_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    );
    if !needs_data_colors {
        return Ok(Vec::new());
    }
    decode_buffer(&properties.face_vertex_cdata, buffers)?
        .values_f64()
        .into_iter()
        .map(|value| {
            surface_colormap(
                value,
                axes.c_limits,
                properties.c_data_mapping,
                axes_colormap(axes),
            )
        })
        .collect()
}

fn patch_colors<'a>(
    mode: SurfaceColorMode,
    uniform: Option<WireRgba>,
    mapped: &'a [Rgba],
    face_rows: usize,
    vertex_rows: usize,
    role: &'static str,
) -> Result<PatchColors<'a>, PlotWebError> {
    match mode {
        SurfaceColorMode::Uniform => Ok(PatchColors::Uniform(convert_color(
            uniform.ok_or_else(|| invalid_scene("Uniform Patch color is missing."))?,
        )?)),
        SurfaceColorMode::Flat if mapped.len() == face_rows => Ok(PatchColors::FlatFaces(mapped)),
        SurfaceColorMode::Flat if mapped.len() == vertex_rows => {
            Ok(PatchColors::FlatVertices(mapped))
        }
        SurfaceColorMode::Interp if mapped.len() == vertex_rows => {
            Ok(PatchColors::InterpVertices(mapped))
        }
        SurfaceColorMode::None => Err(invalid_scene("Patch color mode is disabled.")),
        SurfaceColorMode::Flat | SurfaceColorMode::Interp => Err(invalid_scene(match role {
            "face" => "Patch face CData cardinality is invalid.",
            _ => "Patch edge CData cardinality is invalid.",
        })),
    }
}

fn resolve_surface_colors(
    properties: &SurfaceSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
) -> Result<Vec<Rgba>, PlotWebError> {
    let needs_data_colors = matches!(
        properties.face_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    ) || matches!(
        properties.edge_color,
        SurfaceColorMode::Flat | SurfaceColorMode::Interp
    );
    if !needs_data_colors {
        return Ok(Vec::new());
    }
    decode_buffer(&properties.c_data, buffers)?
        .values_f64()
        .into_iter()
        .map(|value| {
            surface_colormap(
                value,
                axes.c_limits,
                properties.c_data_mapping,
                axes_colormap(axes),
            )
        })
        .collect()
}

fn surface_colormap(
    value: f64,
    limits: [f64; 2],
    mapping: openmat_plot_protocol::CDataMapping,
    colormap: &[[f64; 3]],
) -> Result<Rgba, PlotWebError> {
    let last = colormap
        .len()
        .checked_sub(1)
        .ok_or_else(|| invalid_scene("Axes colormap must contain at least one RGB row."))?;
    let index = match mapping {
        openmat_plot_protocol::CDataMapping::Scaled => {
            if value.is_nan() || value <= limits[0] {
                0
            } else if value >= limits[1] {
                last
            } else {
                let length = f64::from(
                    u32::try_from(colormap.len())
                        .map_err(|_| invalid_scene("Axes colormap exceeds browser limits."))?,
                );
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let index =
                    (((value - limits[0]) / (limits[1] - limits[0])) * length).floor() as usize;
                index.min(last)
            }
        }
        openmat_plot_protocol::CDataMapping::Direct => {
            if value.is_nan() || value <= 1.0 {
                0
            } else {
                let length = f64::from(
                    u32::try_from(colormap.len())
                        .map_err(|_| invalid_scene("Axes colormap exceeds browser limits."))?,
                );
                if value >= length {
                    last
                } else {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let one_based = value.floor() as usize;
                    one_based.saturating_sub(1).min(last)
                }
            }
        }
    };
    colormap_color(colormap[index])
}

fn axes_colormap(axes: &Axes2DProperties) -> &[[f64; 3]] {
    axes.colormap.as_deref().unwrap_or(&PARULA_R2022B)
}

fn colormap_color(color: [f64; 3]) -> Result<Rgba, PlotWebError> {
    rgba(
        f64_to_f32(color[0])?,
        f64_to_f32(color[1])?,
        f64_to_f32(color[2])?,
        1.0,
    )
}

#[allow(clippy::too_many_lines)]
fn add_scatter(
    builder: &mut MirFrameBuilder,
    properties: &ScatterSeriesProperties,
    buffers: &HashMap<String, Vec<u8>>,
    axes: &Axes2DProperties,
    transform: AxesTransform,
    order: DrawOrder,
) -> Result<(), PlotWebError> {
    if !properties.clipping {
        return Err(invalid_scene(
            "The WebGPU Scatter pipeline currently requires axes clipping.",
        ));
    }
    let x = decode_buffer(&properties.x_data, buffers)?;
    let y = decode_buffer(&properties.y_data, buffers)?;
    if properties.size_data.as_ref().is_none() && properties.color_data.as_ref().is_none() {
        let masked = mask_logarithmic_series(&x, &y, axes)?;
        let input = masked.as_ref().map_or(
            SeriesGeometryInput {
                x: Some(x.view()),
                y: y.view(),
            },
            |(x, y)| SeriesGeometryInput {
                x: Some(NumericView::F64(x)),
                y: NumericView::F64(y),
            },
        );
        return add_marker_series(
            builder,
            input,
            None,
            order,
            properties.marker,
            properties.marker_size_css_px,
            convert_color(properties.marker_face_rgba)?,
            convert_color(properties.marker_edge_rgba)?,
            properties.marker_edge_width_css_px,
            transform,
        );
    }
    let x = x.values_f64();
    let y = y.values_f64();
    if x.len() != y.len() {
        return Err(invalid_scene("ScatterSeries X/YData lengths differ."));
    }
    let sizes = properties
        .size_data
        .as_ref()
        .map(|data| decode_buffer(data, buffers).map(|buffer| buffer.values_f64()))
        .transpose()?;
    let colors = properties
        .color_data
        .as_ref()
        .map(|data| decode_buffer(data, buffers).map(|buffer| buffer.values_f64()))
        .transpose()?;
    validate_scatter_instance_data(x.len(), sizes.as_deref(), colors.as_deref())?;
    let rendering = supported_marker(properties.marker)?;
    let fixed_fill = convert_color(properties.marker_face_rgba)?;
    let fixed_stroke = convert_color(properties.marker_edge_rgba)?;
    let stroke_width = css_px(f64_to_f32(properties.marker_edge_width_css_px)?)?;
    let mut instances = Vec::with_capacity(x.len());
    for index in 0..x.len() {
        if !axis_value_in_scale(x[index], axes.x_scale)
            || !axis_value_in_scale(y[index], axes.y_scale)
        {
            continue;
        }
        let center = transform
            .map(x[index], y[index])
            .map_err(|_| invalid_scene("Scatter point cannot be normalized."))?;
        if !AxesRect::unit().contains(center) {
            continue;
        }
        let mapped = colors
            .as_ref()
            .map(|colors| {
                surface_colormap(
                    colors[index],
                    axes.c_limits,
                    openmat_plot_protocol::CDataMapping::Scaled,
                    axes_colormap(axes),
                )
            })
            .transpose()?;
        instances.push(MarkerInstance {
            center,
            size_css_px: scatter_size_css_px(
                sizes
                    .as_ref()
                    .map_or(properties.marker_size_css_px, |values| {
                        values[if values.len() == 1 { 0 } else { index }].sqrt() * (96.0 / 72.0)
                    }),
                rendering.size_scale,
            )?,
            rotation_radians: rendering.rotation_radians,
            fill: if properties.color_data_target == ScatterColorTarget::Face {
                mapped.unwrap_or(fixed_fill)
            } else {
                fixed_fill
            },
            stroke: if properties.color_data_target == ScatterColorTarget::Edge {
                mapped.unwrap_or(fixed_stroke)
            } else {
                fixed_stroke
            },
            stroke_width_css_px: stroke_width,
        });
    }
    builder.add_markers(
        order,
        None,
        MarkerBatch {
            shape: rendering.primary,
            instances: instances.clone(),
        },
    );
    if let Some(secondary) = rendering.secondary {
        builder.add_markers(
            order,
            None,
            MarkerBatch {
                shape: secondary,
                instances,
            },
        );
    }
    Ok(())
}

fn validate_scatter_instance_data(
    point_count: usize,
    sizes: Option<&[f64]>,
    colors: Option<&[f64]>,
) -> Result<(), PlotWebError> {
    if sizes.is_some_and(|values| {
        (values.len() != 1 && values.len() != point_count)
            || values
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
    }) {
        return Err(invalid_scene("Scatter SizeData is invalid."));
    }
    if colors.is_some_and(|values| values.len() != point_count) {
        return Err(invalid_scene(
            "Scatter CData length differs from its point count.",
        ));
    }
    Ok(())
}

fn scatter_size_css_px(size_css_px: f64, scale: f32) -> Result<CssPx, PlotWebError> {
    css_px(f64_to_f32(size_css_px)? * scale)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MarkerRendering {
    primary: MarkerShape,
    rotation_radians: f32,
    secondary: Option<MarkerShape>,
    size_scale: f32,
}

fn supported_marker(marker: Marker) -> Result<MarkerRendering, PlotWebError> {
    let rendering = match marker {
        Marker::Point => MarkerRendering {
            primary: MarkerShape::Circle,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 0.45,
        },
        Marker::Circle => MarkerRendering {
            primary: MarkerShape::Circle,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::Square => MarkerRendering {
            primary: MarkerShape::Square,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::Diamond => MarkerRendering {
            primary: MarkerShape::Diamond,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::TriangleUp => MarkerRendering {
            primary: MarkerShape::UpTriangle,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::TriangleDown => MarkerRendering {
            primary: MarkerShape::DownTriangle,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::TriangleRight => MarkerRendering {
            primary: MarkerShape::UpTriangle,
            rotation_radians: std::f32::consts::FRAC_PI_2,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::TriangleLeft => MarkerRendering {
            primary: MarkerShape::UpTriangle,
            rotation_radians: -std::f32::consts::FRAC_PI_2,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::Plus => MarkerRendering {
            primary: MarkerShape::Plus,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::Cross => MarkerRendering {
            primary: MarkerShape::Cross,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::HorizontalLine => MarkerRendering {
            primary: MarkerShape::HorizontalLine,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::VerticalLine => MarkerRendering {
            primary: MarkerShape::VerticalLine,
            rotation_radians: 0.0,
            secondary: None,
            size_scale: 1.0,
        },
        Marker::Star => MarkerRendering {
            primary: MarkerShape::Plus,
            rotation_radians: 0.0,
            secondary: Some(MarkerShape::Cross),
            size_scale: 1.0,
        },
        Marker::None | Marker::Pentagram | Marker::Hexagram => Err(invalid_scene(
            "This marker has no corresponding Plot MIR shape.",
        ))?,
    };
    Ok(rendering)
}

#[allow(clippy::too_many_arguments)]
fn add_marker_series(
    builder: &mut MirFrameBuilder,
    input: SeriesGeometryInput<'_>,
    source_indices_one_based: Option<&[u64]>,
    order: DrawOrder,
    marker: Marker,
    marker_size_css_px: f64,
    fill: Rgba,
    stroke: Rgba,
    stroke_width_css_px: f64,
    transform: AxesTransform,
) -> Result<(), PlotWebError> {
    let rendering = supported_marker(marker)?;
    let size = css_px(f64_to_f32(marker_size_css_px)? * rendering.size_scale)?;
    let stroke_width = css_px(f64_to_f32(stroke_width_css_px)?)?;
    let style = |shape| ScatterStyle {
        shape,
        size_css_px: size,
        fill,
        stroke,
        stroke_width_css_px: stroke_width,
    };
    let mapper = |x, y| {
        transform
            .map(x, y)
            .map_err(|_| GeometryError::ArithmeticOverflow("axes transform"))
    };
    let add = |builder: &mut MirFrameBuilder, shape, rotation_radians| {
        if let Some(indices) = source_indices_one_based {
            builder.add_scatter_rotated_at_indices(
                input,
                indices,
                order,
                None,
                style(shape),
                rotation_radians,
                mapper,
            )
        } else {
            builder.add_scatter_rotated(input, order, None, style(shape), rotation_radians, mapper)
        }
    };
    add(builder, rendering.primary, rendering.rotation_radians)
        .map_err(|_| invalid_scene("Marker geometry could not be generated."))?;
    if let Some(secondary) = rendering.secondary {
        add(builder, secondary, 0.0)
            .map_err(|_| invalid_scene("Composite marker geometry could not be generated."))?;
    }
    Ok(())
}

fn decode_buffer(
    descriptor: &DataRef,
    buffers: &HashMap<String, Vec<u8>>,
) -> Result<NumericBuffer, PlotWebError> {
    let bytes = buffers
        .get(&descriptor.buffer_id)
        .ok_or_else(|| invalid_scene("A Figure data buffer has not been ingested."))?;
    if u64::try_from(bytes.len()).ok() != Some(descriptor.byte_length) {
        return Err(invalid_scene(
            "A Figure data buffer has the wrong byte length.",
        ));
    }
    match descriptor.dtype {
        DataType::F32 => {
            let chunks = bytes.chunks_exact(4);
            if !chunks.remainder().is_empty() {
                return Err(invalid_scene(
                    "An f32 Figure buffer is not element-aligned.",
                ));
            }
            Ok(NumericBuffer::F32(
                chunks
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect(),
            ))
        }
        DataType::F64 => {
            let chunks = bytes.chunks_exact(8);
            if !chunks.remainder().is_empty() {
                return Err(invalid_scene(
                    "An f64 Figure buffer is not element-aligned.",
                ));
            }
            Ok(NumericBuffer::F64(
                chunks
                    .map(|chunk| {
                        f64::from_le_bytes([
                            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                            chunk[7],
                        ])
                    })
                    .collect(),
            ))
        }
    }
}

fn convert_color(color: WireRgba) -> Result<Rgba, PlotWebError> {
    let [red, green, blue, alpha] = color.0;
    rgba(
        f64_to_f32(red)?,
        f64_to_f32(green)?,
        f64_to_f32(blue)?,
        f64_to_f32(alpha)?,
    )
}

const fn rgba_array(color: Rgba) -> [f32; 4] {
    [color.red, color.green, color.blue, color.alpha]
}

fn rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Result<Rgba, PlotWebError> {
    Rgba::new(red, green, blue, alpha)
        .map_err(|_| invalid_scene("A Figure color cannot be represented."))
}

fn css_px(value: f32) -> Result<CssPx, PlotWebError> {
    CssPx::new(value).map_err(|_| invalid_scene("A CSS length cannot be represented."))
}

fn f64_to_f32(value: f64) -> Result<f32, PlotWebError> {
    if !value.is_finite() || value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
        return Err(invalid_scene(
            "A Figure value cannot be represented as f32.",
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as f32)
}

fn patch_geometry_error(context: &'static str, error: &GeometryError) -> PlotWebError {
    PlotWebError::new(
        PlotWebErrorKind::InvalidScene,
        format!("{context} geometry could not be generated: {error}"),
    )
}

fn invalid_scene(message: &'static str) -> PlotWebError {
    PlotWebError::new(PlotWebErrorKind::InvalidScene, message)
}

#[cfg(test)]
mod tests {
    use openmat_plot_mir::MirCommand;
    use openmat_plot_protocol::{
        AxesLimitMode, Endianness, FigureNextPlot, FigureProperties, GraphicsLimits, MarkerIndex,
        NextPlot, Nullable, ObjectFields, Scale, StorageOrder,
    };

    use super::*;

    fn fields(id: &str, parent_id: Option<&str>, children: &[&str]) -> ObjectFields {
        ObjectFields {
            id: id.to_owned(),
            generation: 1,
            object_revision: 1,
            parent_id: parent_id.map(str::to_owned),
            children: children.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn text_object(id: &str, role: TextRole, text: &str) -> GraphicsObject {
        GraphicsObject::Text {
            fields: fields(id, Some("axes"), &[]),
            properties: TextProperties {
                code_units: text.encode_utf16().collect(),
                role,
                anchor_normalized: [0.5, 1.0],
                horizontal_alignment: WireHorizontalAlignment::Center,
                vertical_alignment: WireVerticalAlignment::Bottom,
                color_rgba: WireRgba([1.0, 1.0, 1.0, 1.0]),
                font_family_code_units: "Helvetica".encode_utf16().collect(),
                font_size_css_px: 18.0,
                font_weight: 400,
                font_style: FontStyle::Normal,
                interpreter: WireTextInterpreter::Tex,
                rotation_degrees: 0.0,
                visible: true,
            },
        }
    }

    fn data_ref(id: &str, dtype: DataType, count: u64) -> DataRef {
        DataRef {
            buffer_id: id.to_owned(),
            dtype,
            shape: vec![1, count],
            order: StorageOrder::ColumnMajor,
            endianness: Endianness::Little,
            byte_offset: 0,
            byte_length: count * dtype.byte_width(),
        }
    }

    fn matrix_ref(id: &str, dtype: DataType, rows: u64, columns: u64) -> DataRef {
        DataRef {
            buffer_id: id.to_owned(),
            dtype,
            shape: vec![rows, columns],
            order: StorageOrder::ColumnMajor,
            endianness: Endianness::Little,
            byte_offset: 0,
            byte_length: rows * columns * dtype.byte_width(),
        }
    }

    fn projected_cube_bounds(matrix: ViewProjection3D) -> [f64; 4] {
        let columns = matrix.columns();
        let mut bounds = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for x in [0.0, 1.0] {
            for y in [0.0, 1.0] {
                for z in [0.0, 1.0] {
                    let input = [x, y, z, 1.0];
                    let mut clip = [0.0_f64; 4];
                    for (column, value) in input.into_iter().enumerate() {
                        for (row, output) in clip.iter_mut().enumerate() {
                            *output += f64::from(columns[column][row]) * value;
                        }
                    }
                    let projected = [clip[0] / clip[3], clip[1] / clip[3]];
                    bounds[0] = bounds[0].min(projected[0]);
                    bounds[1] = bounds[1].max(projected[0]);
                    bounds[2] = bounds[2].min(projected[1]);
                    bounds[3] = bounds[3].max(projected[1]);
                }
            }
        }
        bounds
    }

    #[allow(clippy::too_many_lines)]
    fn snapshot(line: bool) -> FigureSnapshot {
        let mut objects = vec![
            GraphicsObject::Figure {
                fields: fields("figure", None, &["axes"]),
                properties: FigureProperties {
                    number: 1,
                    name_code_units: Vec::new(),
                    number_title: true,
                    visible: true,
                    background_rgba: WireRgba([1.0, 1.0, 1.0, 1.0]),
                    initial_logical_size_css_pixels: [640.0, 480.0],
                    position_css_pixels: [100.0, 100.0, 640.0, 480.0],
                    next_plot: FigureNextPlot::Add,
                },
            },
            GraphicsObject::Axes2d {
                fields: fields("axes", Some("figure"), &["series"]),
                properties: Axes2DProperties {
                    coordinate_system: AxesCoordinateSystem::Cartesian,
                    position_normalized: [0.13, 0.11, 0.775, 0.815],
                    background_rgba: openmat_plot_protocol::Nullable(Some(WireRgba([
                        1.0, 1.0, 1.0, 1.0,
                    ]))),
                    theta_axis_units: ThetaAxisUnits::Degrees,
                    theta_direction: ThetaDirection::Counterclockwise,
                    theta_zero_location: ThetaZeroLocation::Right,
                    r_axis_location: 80.0,
                    x_scale: Scale::Linear,
                    y_scale: Scale::Linear,
                    z_scale: Scale::Linear,
                    x_direction: WireAxisDirection::Normal,
                    y_direction: WireAxisDirection::Normal,
                    z_direction: WireAxisDirection::Normal,
                    visible: true,
                    x_limits: [1.0e12, 1.0e12 + 4.0],
                    y_limits: [0.0, 4.0],
                    z_limits: [0.0, 1.0],
                    x_limits_mode: AxesLimitMode::Auto,
                    y_limits_mode: AxesLimitMode::Auto,
                    z_limits_mode: AxesLimitMode::Auto,
                    next_plot: NextPlot::Replace,
                    grid_x: true,
                    grid_y: true,
                    grid_z: false,
                    minor_grid_x: false,
                    minor_grid_y: false,
                    minor_grid_z: false,
                    color_order_index: 2,
                    x_tick: Vec::new(),
                    y_tick: Vec::new(),
                    z_tick: Vec::new(),
                    x_tick_label_code_units: Vec::new(),
                    y_tick_label_code_units: Vec::new(),
                    z_tick_label_code_units: Vec::new(),
                    x_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    y_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    z_tick_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    x_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    y_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    z_tick_label_mode: openmat_plot_protocol::AxesTickMode::Auto,
                    view: [0.0, 90.0],
                    camera_scale: 1.0,
                    projection: WireProjection::Orthographic,
                    data_aspect_ratio: [1.0, 1.0, 1.0],
                    data_aspect_ratio_mode: AxesLimitMode::Auto,
                    plot_box_aspect_ratio: [1.0, 1.0, 1.0],
                    plot_box_aspect_ratio_mode: AxesLimitMode::Auto,
                    c_limits: [0.0, 1.0],
                    c_limits_mode: AxesLimitMode::Auto,
                    colormap: None,
                    colorbar_visible: false,
                    box_enabled: false,
                    font_size_css_px: 10.0,
                    font_family_code_units: Vec::new(),
                    tick_direction: WireTickDirection::In,
                    line_width_css_px: 2.0 / 3.0,
                    tick_label_interpreter: openmat_plot_protocol::TextInterpreter::Tex,
                    title_id: Nullable(None),
                    x_label_id: Nullable(None),
                    y_label_id: Nullable(None),
                    z_label_id: Nullable(None),
                },
            },
        ];
        objects.push(if line {
            GraphicsObject::LineSeries {
                fields: fields("series", Some("axes"), &[]),
                properties: LineSeriesProperties {
                    x_data: data_ref("x", DataType::F64, 3),
                    y_data: data_ref("y", DataType::F64, 3),
                    z_data: Nullable(None),
                    color_rgba: WireRgba([0.0, 0.45, 0.74, 1.0]),
                    line_width_css_px: 0.5,
                    line_style: LineStyle::Solid,
                    marker: Marker::None,
                    marker_size_css_px: 6.0,
                    marker_face_rgba: None,
                    marker_edge_rgba: None,
                    marker_indices: None,
                    display_name_code_units: Vec::new(),
                    visible: true,
                    clipping: true,
                },
            }
        } else {
            GraphicsObject::ScatterSeries {
                fields: fields("series", Some("axes"), &[]),
                properties: ScatterSeriesProperties {
                    x_data: data_ref("x", DataType::F32, 3),
                    y_data: data_ref("y", DataType::F32, 3),
                    z_data: Nullable(None),
                    size_data: Nullable(None),
                    color_data: Nullable(None),
                    color_data_target: ScatterColorTarget::None,
                    marker: Marker::Circle,
                    marker_size_css_px: 6.0,
                    marker_edge_width_css_px: 1.0,
                    marker_face_rgba: WireRgba([0.85, 0.32, 0.10, 1.0]),
                    marker_edge_rgba: WireRgba([0.85, 0.32, 0.10, 1.0]),
                    display_name_code_units: Vec::new(),
                    visible: true,
                    clipping: true,
                },
            }
        });
        let dtype = if line { DataType::F64 } else { DataType::F32 };
        let snapshot = FigureSnapshot {
            snapshot_type: "figureSnapshot".to_owned(),
            figure_id: "figure".to_owned(),
            revision: 1,
            root_id: "figure".to_owned(),
            objects,
            referenced_buffers: vec![data_ref("x", dtype, 3), data_ref("y", dtype, 3)],
        };
        snapshot.validate(GraphicsLimits::default()).unwrap();
        snapshot
    }

    fn f64_bytes(values: &[f64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn polar_snapshot_lowers_to_square_webgpu_grid_and_tessellated_stroke() {
        let mut snapshot = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut snapshot.objects[1] else {
            panic!()
        };
        properties.coordinate_system = AxesCoordinateSystem::Polar;
        properties.x_limits = [0.0, 360.0];
        properties.y_limits = [0.0, 1.5];
        properties.x_tick = [0.0, 90.0, 180.0, 270.0, 360.0]
            .map(|value| TickValue::new(value).unwrap())
            .to_vec();
        properties.y_tick = [0.0, 0.5, 1.0, 1.5]
            .map(|value| TickValue::new(value).unwrap())
            .to_vec();
        properties.x_tick_label_code_units = ["0°", "90°", "180°", "270°"]
            .map(|value| value.encode_utf16().collect())
            .to_vec();
        properties.y_tick_label_code_units = ["0", "0.5", "1", "1.5"]
            .map(|value| value.encode_utf16().collect())
            .to_vec();
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[0.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 1.0, 1.0])),
        ]);
        snapshot.validate(GraphicsLimits::default()).unwrap();
        let prepared = prepare_scene(
            &snapshot,
            &buffers,
            CanvasSize::new(800.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        let viewport = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match operation.command {
                openmat_plot_mir::MirCommand::BeginViewport(viewport) => Some(viewport),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            viewport.css.size.width.get().to_bits(),
            viewport.css.size.height.get().to_bits()
        );
        assert_eq!(
            prepared
                .frame
                .operations
                .iter()
                .filter(|operation| matches!(
                    operation.command,
                    openmat_plot_mir::MirCommand::GridLineBatch(_)
                ))
                .count(),
            2
        );
        assert!(prepared.frame.operations.iter().any(|operation| {
            matches!(
                operation.command,
                openmat_plot_mir::MirCommand::StrokeMesh(_)
            )
        }));
        assert_eq!(prepared.text_pass.as_ref().unwrap().requests.len(), 9);
    }

    #[test]
    fn cartesian_equal_units_fit_a_centered_effective_2d_viewport() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            panic!()
        };
        properties.x_limits = [0.0, 8.0];
        properties.y_limits = [0.0, 2.0];
        properties.data_aspect_ratio = [1.0, 1.0, 1.0];
        properties.data_aspect_ratio_mode = AxesLimitMode::Manual;
        properties.plot_box_aspect_ratio_mode = AxesLimitMode::Auto;
        let size = CanvasSize::new(800.0, 500.0, 1.0).unwrap();
        let slot = axes_mapping(size, axes_render_position(properties, size))
            .unwrap()
            .viewport()
            .css;
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[0.0, 4.0, 8.0])),
            ("y".to_owned(), f64_bytes(&[0.0, 1.0, 2.0])),
        ]);
        let prepared = prepare_scene(&scene, &buffers, size).unwrap();
        let viewport = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match operation.command {
                MirCommand::BeginViewport(viewport) => Some(viewport.css),
                _ => None,
            })
            .unwrap();
        let width = f64::from(viewport.size.width.get());
        let height = f64::from(viewport.size.height.get());
        assert!((width / height - 4.0).abs() < 1.0e-6);
        assert!((width / 8.0 - height / 2.0).abs() < 1.0e-6);

        let viewport_center = [
            f64::from(viewport.origin.x) + width * 0.5,
            f64::from(viewport.origin.y) + height * 0.5,
        ];
        let slot_center = [
            f64::from(slot.origin.x) + f64::from(slot.size.width.get()) * 0.5,
            f64::from(slot.origin.y) + f64::from(slot.size.height.get()) * 0.5,
        ];
        assert!((viewport_center[0] - slot_center[0]).abs() < 1.0e-5);
        assert!((viewport_center[1] - slot_center[1]).abs() < 1.0e-5);
    }

    #[test]
    fn multiple_axes_lower_to_independent_viewports_and_unique_overlay_keys() {
        let mut snapshot = snapshot(true);
        let GraphicsObject::Axes2d {
            mut fields,
            mut properties,
        } = snapshot.objects[1].clone()
        else {
            panic!()
        };
        fields.id = "axes-2".to_owned();
        fields.children = vec!["series-2".to_owned()];
        properties.position_normalized = [
            0.570_340_909_090_909_1,
            0.11,
            0.334_659_090_909_090_96,
            0.815,
        ];
        let second_axes = GraphicsObject::Axes2d { fields, properties };
        if let GraphicsObject::Axes2d { properties, .. } = &mut snapshot.objects[1] {
            properties.position_normalized = [0.13, 0.11, 0.334_659_090_909_090_9, 0.815];
        }
        if let GraphicsObject::Figure { fields, .. } = &mut snapshot.objects[0] {
            fields.children.push("axes-2".to_owned());
        }
        let GraphicsObject::LineSeries {
            mut fields,
            mut properties,
        } = snapshot.objects[2].clone()
        else {
            panic!()
        };
        fields.id = "series-2".to_owned();
        fields.parent_id = Some("axes-2".to_owned());
        properties.x_data.buffer_id = "x-2".to_owned();
        properties.y_data.buffer_id = "y-2".to_owned();
        let second_series = GraphicsObject::LineSeries { fields, properties };
        snapshot.objects.push(second_axes);
        snapshot.objects.push(second_series);
        snapshot
            .referenced_buffers
            .push(data_ref("x-2", DataType::F64, 3));
        snapshot
            .referenced_buffers
            .push(data_ref("y-2", DataType::F64, 3));
        snapshot.validate(GraphicsLimits::default()).unwrap();

        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
            (
                "x-2".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y-2".to_owned(), f64_bytes(&[3.0, 1.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &snapshot,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        assert_eq!(prepared.axes_id.as_deref(), Some("axes"));
        assert_eq!(prepared.additional_axes.len(), 1);
        assert_eq!(prepared.additional_axes[0].axes_id, "axes-2");
        assert!(
            prepared.frame.overlay.viewport_css.right()
                < prepared.additional_axes[0]
                    .frame
                    .overlay
                    .viewport_css
                    .origin
                    .x
        );
        let overlay = prepared.overlay_dto();
        assert!(
            overlay
                .text
                .iter()
                .all(|text| { text.key.starts_with("axes::") || text.key.starts_with("axes-2::") })
        );
        let unique = overlay
            .text
            .iter()
            .map(|text| text.key.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), overlay.text.len());
    }

    #[test]
    fn lod_cache_reuses_exact_view_and_bounds_resident_entries() {
        let x = [0.0, 0.5, 1.0];
        let y = [0.0, 1.0, 0.0];
        let input = SeriesGeometryInput {
            x: Some(NumericView::F64(&x)),
            y: NumericView::F64(&y),
        };
        let source = LodSourceKey {
            x: Some(LodResourceRevision::new("x", 7)),
            y: LodResourceRevision::new("y", 9),
        };
        let first_view = LinearLodView::new(0.0, 1.0, 320.0, 2.0).unwrap();
        let first_key = LodCacheKey::new(source.clone(), first_view);
        let mut cache = SceneCache::default();
        let first = cache
            .get_or_build_lod(first_key.clone(), input, first_view)
            .unwrap();
        let reused = cache
            .get_or_build_lod(first_key.clone(), input, first_view)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &reused));

        for width in 1..=MAX_LOD_CACHE_ENTRIES {
            let width = f64::from(u32::try_from(width + 1).unwrap());
            let view = LinearLodView::new(0.0, 1.0, width, 1.0).unwrap();
            cache
                .get_or_build_lod(LodCacheKey::new(source.clone(), view), input, view)
                .unwrap();
        }
        assert_eq!(cache.lod_runs.len(), MAX_LOD_CACHE_ENTRIES);
        assert!(!cache.lod_runs.contains_key(&first_key));
    }

    #[test]
    fn lowers_f64_line_snapshot_to_non_empty_stroke_mir() {
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &snapshot(true),
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        let mesh =
            prepared
                .frame
                .operations
                .iter()
                .find_map(|operation| match &operation.command {
                    MirCommand::StrokeMesh(mesh) if !mesh.indices.is_empty() => Some(mesh),
                    _ => None,
                });
        let mesh = mesh.unwrap();
        assert_eq!(mesh.style.width_css_px.get().to_bits(), 0.5_f32.to_bits());
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.edge_distance_css_px.is_finite())
        );
        assert_eq!(prepared.clear_color.alpha.to_bits(), 1.0_f32.to_bits());
        assert!(prepared.frame.overlay.ticks.len() >= 4);
        assert_eq!(prepared.frame.operations[0].order, DrawOrder(i64::MIN));
        let overlay = prepared.overlay_dto();
        assert_eq!(overlay.canvas_width_css_px.to_bits(), 640.0_f64.to_bits());
        assert_eq!(overlay.canvas_height_css_px.to_bits(), 480.0_f64.to_bits());
        assert!(overlay.axis_lines.is_empty());
        assert!(overlay.ticks.is_empty());
        assert!(!overlay.text.is_empty());
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::ScreenLines(batch) if batch.lines.len() >= 6
        )));
    }

    #[test]
    fn visible_axes_draw_their_own_background_but_hidden_or_none_axes_do_not() {
        let mut scene = snapshot(true);
        let GraphicsObject::Figure { properties, .. } = &mut scene.objects[0] else {
            unreachable!();
        };
        properties.background_rgba = WireRgba([0.02, 0.02, 0.025, 1.0]);
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);

        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        let background = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::TriangleMesh2D(mesh) if operation.order == DrawOrder(i64::MIN + 5) => {
                    Some(mesh)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(background.indices, [0, 1, 2, 0, 2, 3]);
        assert!(background.vertices.iter().all(|vertex| vertex.color
            == Rgba {
                red: 1.0,
                green: 1.0,
                blue: 1.0,
                alpha: 1.0,
            }));

        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.visible = false;
        let hidden = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        assert!(!hidden.frame.operations.iter().any(|operation| {
            matches!(operation.command, MirCommand::TriangleMesh2D(_))
                && operation.order == DrawOrder(i64::MIN + 5)
        }));

        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.visible = true;
        properties.background_rgba = openmat_plot_protocol::Nullable(None);
        let transparent = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        assert!(!transparent.frame.operations.iter().any(|operation| {
            matches!(operation.command, MirCommand::TriangleMesh2D(_))
                && operation.order == DrawOrder(i64::MIN + 5)
        }));
    }

    #[test]
    fn hidden_axes_keep_the_title_but_hide_axis_labels() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { fields, properties } = &mut scene.objects[1] else {
            unreachable!();
        };
        fields
            .children
            .extend(["title".to_owned(), "xlabel".to_owned()]);
        properties.visible = false;
        properties.title_id = Nullable(Some("title".to_owned()));
        properties.x_label_id = Nullable(Some("xlabel".to_owned()));
        scene
            .objects
            .push(text_object("title", TextRole::Title, "Golden Phyllotaxis"));
        scene
            .objects
            .push(text_object("xlabel", TextRole::XLabel, "Hidden X Label"));
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);

        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        let text = &prepared.frame.overlay.text;

        assert_eq!(text.len(), 1);
        assert_eq!(text[0].role, OverlayTextRole::Title);
        assert_eq!(
            text[0].text.code_units,
            "Golden Phyllotaxis".encode_utf16().collect::<Vec<_>>()
        );
        assert!(prepared.frame.overlay.ticks.is_empty());
    }

    #[test]
    fn logarithmic_axes_skip_nonpositive_points_and_hide_chrome_when_requested() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_scale = Scale::Log;
        properties.x_limits = [1.0, 100.0];
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[-1.0, 1.0, 10.0])),
            ("y".to_owned(), f64_bytes(&[1.0, 2.0, 3.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepared
                .frame
                .overlay
                .ticks
                .iter()
                .filter(|tick| tick.axis == AxisDimension::X)
                .map(|tick| tick.value)
                .collect::<Vec<_>>(),
            [1.0, 10.0, 100.0]
        );
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::StrokeMesh(mesh) if !mesh.indices.is_empty()
        )));

        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.visible = false;
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        assert!(prepared.frame.overlay.ticks.is_empty());
        assert!(
            !prepared
                .frame
                .operations
                .iter()
                .any(|operation| matches!(&operation.command, MirCommand::ScreenLines(_)))
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn lowers_surface_snapshot_to_camera_and_triangle_mesh_mir() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [1.0, 3.0];
        properties.y_limits = [1.0, 2.0];
        properties.z_limits = [1.0, 5.0];
        properties.c_limits = [1.0, 5.0];
        properties.view = [-37.5, 30.0];
        properties.projection = WireProjection::Orthographic;
        properties.grid_x = true;
        properties.grid_y = true;
        properties.grid_z = true;
        scene.objects[2] = GraphicsObject::SurfaceSeries {
            fields: fields("series", Some("axes"), &[]),
            properties: SurfaceSeriesProperties {
                x_data: matrix_ref("x", DataType::F64, 2, 3),
                y_data: matrix_ref("y", DataType::F64, 2, 3),
                z_data: matrix_ref("z", DataType::F64, 2, 3),
                c_data: matrix_ref("c", DataType::F64, 2, 3),
                face_color: SurfaceColorMode::Flat,
                face_rgba: Nullable(None),
                edge_color: SurfaceColorMode::Uniform,
                edge_rgba: Nullable(Some(WireRgba([0.0, 0.0, 0.0, 1.0]))),
                line_width_css_px: 2.0 / 3.0,
                line_style: LineStyle::Solid,
                c_data_mapping: openmat_plot_protocol::CDataMapping::Scaled,
                face_alpha: 1.0,
                lighting_enabled: true,
                visible: true,
                clipping: true,
            },
        };
        scene.referenced_buffers = vec![
            matrix_ref("x", DataType::F64, 2, 3),
            matrix_ref("y", DataType::F64, 2, 3),
            matrix_ref("z", DataType::F64, 2, 3),
            matrix_ref("c", DataType::F64, 2, 3),
        ];
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0])),
            ("y".to_owned(), f64_bytes(&[1.0, 2.0, 1.0, 2.0, 1.0, 2.0])),
            ("z".to_owned(), f64_bytes(&[1.0, 2.0, 2.0, 5.0, 3.0, 4.0])),
            ("c".to_owned(), f64_bytes(&[1.0, 2.0, 2.0, 5.0, 3.0, 4.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();

        assert!(
            prepared.frame.operations.iter().any(|operation| {
                matches!(operation.command, MirCommand::SetViewProjection3D(_))
            })
        );
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            operation.command,
            MirCommand::SetLighting3D(lighting)
                if lighting.validate().is_ok()
                    && lighting.enabled
        )));
        assert!(prepared.camera_3d.is_some());
        let mesh = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::SurfaceMeshWithEdges3D(mesh) => Some(mesh),
                _ => None,
            })
            .unwrap();
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.indices, [0, 2, 3, 0, 3, 1, 2, 4, 5, 2, 5, 3]);
        assert!(mesh.vertices.iter().all(|vertex| {
            vertex.position.x.is_finite()
                && vertex.position.y.is_finite()
                && vertex.position.z.is_finite()
        }));
        for cell in mesh.indices.chunks_exact(6) {
            assert_eq!(cell[0], cell[3]);
        }
        assert_ne!(mesh.vertices[0].color, mesh.vertices[2].color);
        let edge_batch = prepared
            .frame
            .operations
            .iter()
            .filter_map(|operation| match &operation.command {
                MirCommand::SurfaceEdgeSegments3D(batch) => Some(batch),
                _ => None,
            })
            .find(|batch| batch.segments.len() == 7)
            .expect("2x3 Surface emits seven unique grid edges");
        assert_eq!(edge_batch.pattern, LinePattern3D::Solid);
        assert_eq!(
            edge_batch.width_css_px.get().to_bits(),
            (2.0_f32 / 3.0).to_bits()
        );
        assert_eq!(prepared.frame.overlay.axis_lines.len(), 3);
        assert!(
            prepared
                .frame
                .overlay
                .ticks
                .iter()
                .any(|tick| tick.axis == AxisDimension::Z)
        );
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::RulerLines3D(batch) if batch.lines.len() == 12
        )));
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::SetRulerSelection3D(selection) if selection.validate().is_ok()
        )));
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::TickMarks3D(batch) if batch.marks.iter().any(|mark| mark.anchor.z > 0.0)
        )));
        assert!(prepared.overlay_dto().axis_lines.is_empty());
        assert!(prepared.overlay_dto().ticks.is_empty());

        let size = CanvasSize::new(640.0, 480.0, 2.0).unwrap();
        let projection = Projection3D::Orthographic {
            vertical_span: 1.8,
            near: 0.1,
            far: 10.0,
        };
        let camera = |elevation_radians| {
            OrbitCamera3D::new(
                Point3D {
                    x: 0.5,
                    y: 0.5,
                    z: 0.5,
                },
                3.0,
                0.0,
                elevation_radians,
                projection,
            )
            .unwrap()
        };
        let top = projected_overlay_for_camera(
            &scene,
            size,
            "axes",
            camera(std::f64::consts::FRAC_PI_2),
            &prepared,
        )
        .unwrap();
        let oblique =
            projected_overlay_for_camera(&scene, size, "axes", camera(0.5), &prepared).unwrap();
        let overlay_keys = |projected: &ProjectedOverlay| {
            projected
                .overlay
                .text
                .iter()
                .map(|text| text.key.clone())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let layout_keys = |projected: &ProjectedOverlay| {
            projected
                .text_layout
                .pass
                .requests
                .iter()
                .map(|request| request.key.clone())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(overlay_keys(&top), layout_keys(&top));
        assert_eq!(overlay_keys(&oblique), layout_keys(&oblique));
        assert!(!overlay_keys(&top).iter().any(|key| key.starts_with("z-")));
        assert!(
            overlay_keys(&oblique)
                .iter()
                .any(|key| key.starts_with("z-"))
        );
        let measurements = oblique
            .text_layout
            .pass
            .requests
            .iter()
            .map(|request| TextMeasurement {
                key: request.key.clone(),
                metrics: TextMetrics::new(42.0, 12.0, 8.0, 2.0, 41.0).unwrap(),
            })
            .collect();
        assert!(oblique.text_layout.complete(1, measurements).is_ok());
    }

    #[test]
    fn lowers_concave_patch_to_webgpu_triangles_and_unique_aa_edges() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [0.0, 2.0];
        properties.y_limits = [0.0, 2.0];
        properties.z_limits = [-1.0, 1.0];
        properties.c_limits = [1.0, 5.0];
        scene.objects[2] = GraphicsObject::PatchSeries {
            fields: fields("series", Some("axes"), &[]),
            properties: PatchSeriesProperties {
                faces: matrix_ref("faces", DataType::F64, 1, 5),
                vertices: matrix_ref("vertices", DataType::F64, 5, 3),
                face_vertex_cdata: matrix_ref("cdata", DataType::F64, 5, 1),
                vertex_normals: Nullable(None),
                face_color: SurfaceColorMode::Interp,
                face_rgba: Nullable(None),
                edge_color: SurfaceColorMode::Uniform,
                edge_rgba: Nullable(Some(WireRgba([0.0, 0.0, 0.0, 1.0]))),
                line_width_css_px: 2.0 / 3.0,
                line_style: LineStyle::Solid,
                c_data_mapping: openmat_plot_protocol::CDataMapping::Scaled,
                face_alpha: 1.0,
                edge_alpha: 1.0,
                lighting_enabled: false,
                smooth_normals: false,
                visible: true,
                clipping: true,
            },
        };
        scene.referenced_buffers = vec![
            matrix_ref("faces", DataType::F64, 1, 5),
            matrix_ref("vertices", DataType::F64, 5, 3),
            matrix_ref("cdata", DataType::F64, 5, 1),
        ];
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("faces".to_owned(), f64_bytes(&[1.0, 2.0, 3.0, 4.0, 5.0])),
            (
                "vertices".to_owned(),
                f64_bytes(&[
                    0.0, 2.0, 2.0, 1.0, 0.0, 0.0, 0.0, 2.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ]),
            ),
            ("cdata".to_owned(), f64_bytes(&[1.0, 2.0, 3.0, 4.0, 5.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        let mesh = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::SurfaceMeshWithEdges3D(mesh) => Some(mesh),
                _ => None,
            })
            .expect("Patch face is sent to the indexed WebGPU triangle path");
        assert_eq!(mesh.indices.len(), 9);
        assert_eq!(mesh.vertices.len(), 9);
        assert_eq!(
            mesh.color_interpolation,
            openmat_plot_mir::SurfaceColorInterpolation::Smooth
        );
        let edges = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::SurfaceEdgeSegments3D(edges) => Some(edges),
                _ => None,
            })
            .expect("Patch boundary uses the screen-space depth-biased edge path");
        assert_eq!(edges.segments.len(), 5);
        assert!(prepared.camera_3d.is_some());
        assert_eq!(source_points_3d(&scene, &buffers, None).unwrap().len(), 5);
    }

    #[test]
    fn two_dimensional_patch_stays_in_linear_axes_with_translucent_webgpu_geometry() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [0.0, 1.0];
        properties.y_limits = [0.0, 1.0];
        scene.objects[2] = GraphicsObject::PatchSeries {
            fields: fields("series", Some("axes"), &[]),
            properties: PatchSeriesProperties {
                faces: matrix_ref("faces", DataType::F64, 1, 4),
                vertices: matrix_ref("vertices", DataType::F64, 4, 2),
                face_vertex_cdata: matrix_ref("cdata", DataType::F64, 0, 0),
                vertex_normals: Nullable(None),
                face_color: SurfaceColorMode::Uniform,
                face_rgba: Nullable(Some(WireRgba([1.0, 0.0, 0.0, 1.0]))),
                edge_color: SurfaceColorMode::Uniform,
                edge_rgba: Nullable(Some(WireRgba([0.0, 0.0, 0.0, 1.0]))),
                line_width_css_px: 2.0 / 3.0,
                line_style: LineStyle::Solid,
                c_data_mapping: openmat_plot_protocol::CDataMapping::Scaled,
                face_alpha: 0.4,
                edge_alpha: 0.6,
                lighting_enabled: false,
                smooth_normals: false,
                visible: true,
                clipping: true,
            },
        };
        scene.referenced_buffers = vec![
            matrix_ref("faces", DataType::F64, 1, 4),
            matrix_ref("vertices", DataType::F64, 4, 2),
            matrix_ref("cdata", DataType::F64, 0, 0),
        ];
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("faces".to_owned(), f64_bytes(&[1.0, 2.0, 3.0, 4.0])),
            (
                "vertices".to_owned(),
                f64_bytes(&[0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0]),
            ),
            ("cdata".to_owned(), Vec::new()),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        assert!(prepared.camera_3d.is_none());
        let mesh = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::TriangleMesh2D(mesh) if operation.order != DrawOrder(i64::MIN + 5) => {
                    Some(mesh)
                }
                _ => None,
            })
            .expect("2D Patch uses the dedicated 2D triangle path");
        assert_eq!(mesh.indices.len(), 6);
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.color == Rgba::new(1.0, 0.0, 0.0, 0.4).unwrap())
        );
        assert_eq!(
            prepared
                .frame
                .operations
                .iter()
                .filter(|operation| matches!(operation.command, MirCommand::StrokeMesh(_)))
                .count(),
            4
        );
        assert!(prepared.frame.operations.iter().all(|operation| {
            match &operation.command {
                MirCommand::StrokeMesh(mesh) => mesh.color.alpha.to_bits() == 0.6_f32.to_bits(),
                _ => true,
            }
        }));
        assert!(
            !prepared
                .frame
                .operations
                .iter()
                .any(|operation| matches!(operation.command, MirCommand::SetViewProjection3D(_)))
        );
    }

    #[test]
    fn plot_box_fit_tracks_camera_view_and_respects_manual_aspect() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            panic!()
        };
        let size = CanvasSize::new(800.0, 500.0, 1.0).unwrap();
        let viewport = axes_mapping(size, axes_render_position(properties, size))
            .unwrap()
            .viewport()
            .css;
        let aspect_ratio =
            f64::from(viewport.size.width.get()) / f64::from(viewport.size.height.get());
        let camera = |elevation_radians| {
            OrbitCamera3D::new(
                Point3D {
                    x: 0.5,
                    y: 0.5,
                    z: 0.5,
                },
                3.0,
                -37.5_f64.to_radians(),
                elevation_radians,
                Projection3D::Orthographic {
                    vertical_span: 1.8,
                    near: 0.1,
                    far: 10.0,
                },
            )
            .unwrap()
        };
        let fit = plot_box_fit_target_3d(properties, viewport).unwrap();
        assert_eq!(fit.mode(), PlotBoxFitMode3D::StretchToFill);
        let target_width = 2.0 - 4.0 * PLOT_BOX_INSET_CSS_PX / f64::from(viewport.size.width.get());
        let target_height =
            2.0 - 4.0 * PLOT_BOX_INSET_CSS_PX / f64::from(viewport.size.height.get());
        let first =
            resolved_view_projection_3d(camera(20.0_f64.to_radians()), aspect_ratio, [1.0; 3], fit)
                .unwrap();
        let second =
            resolved_view_projection_3d(camera(70.0_f64.to_radians()), aspect_ratio, [1.0; 3], fit)
                .unwrap();
        assert_ne!(first, second);
        for matrix in [first, second] {
            let [minimum_x, maximum_x, minimum_y, maximum_y] = projected_cube_bounds(matrix);
            assert!((maximum_x - minimum_x - target_width).abs() < 1.0e-5);
            assert!((maximum_y - minimum_y - target_height).abs() < 1.0e-5);
        }

        properties.plot_box_aspect_ratio_mode = AxesLimitMode::Manual;
        let manual_fit = plot_box_fit_target_3d(properties, viewport).unwrap();
        assert_eq!(manual_fit.mode(), PlotBoxFitMode3D::PreserveAspect);
        let manual = resolved_view_projection_3d(
            camera(20.0_f64.to_radians()),
            aspect_ratio,
            [1.0; 3],
            manual_fit,
        )
        .unwrap();
        let [minimum_x, maximum_x, minimum_y, maximum_y] = projected_cube_bounds(manual);
        let width = maximum_x - minimum_x;
        let height = maximum_y - minimum_y;
        assert!(width <= target_width + 1.0e-5);
        assert!(height <= target_height + 1.0e-5);
        assert!((target_width - width).min(target_height - height) < 1.0e-5);
    }

    #[test]
    fn interpolated_surface_and_colorbar_lower_to_smooth_gpu_and_overlay_commands() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [-1.0, 1.0];
        properties.y_limits = [-1.0, 1.0];
        properties.z_limits = [1.0, 4.0];
        properties.c_limits = [1.0, 4.0];
        properties.view = [-37.5, 30.0];
        properties.colorbar_visible = true;
        scene.objects[2] = GraphicsObject::SurfaceSeries {
            fields: fields("series", Some("axes"), &[]),
            properties: SurfaceSeriesProperties {
                x_data: matrix_ref("x", DataType::F64, 2, 2),
                y_data: matrix_ref("y", DataType::F64, 2, 2),
                z_data: matrix_ref("z", DataType::F64, 2, 2),
                c_data: matrix_ref("c", DataType::F64, 2, 2),
                face_color: SurfaceColorMode::Interp,
                face_rgba: Nullable(None),
                edge_color: SurfaceColorMode::None,
                edge_rgba: Nullable(None),
                line_width_css_px: 2.0 / 3.0,
                line_style: LineStyle::Solid,
                c_data_mapping: openmat_plot_protocol::CDataMapping::Scaled,
                face_alpha: 1.0,
                lighting_enabled: false,
                visible: true,
                clipping: true,
            },
        };
        scene.referenced_buffers = ["x", "y", "z", "c"]
            .into_iter()
            .map(|id| matrix_ref(id, DataType::F64, 2, 2))
            .collect();
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[-1.0, -1.0, 1.0, 1.0])),
            ("y".to_owned(), f64_bytes(&[-1.0, 1.0, -1.0, 1.0])),
            ("z".to_owned(), f64_bytes(&[1.0, 2.0, 3.0, 4.0])),
            ("c".to_owned(), f64_bytes(&[1.0, 2.0, 3.0, 4.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        let mesh = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::SurfaceMesh3D(mesh) => Some(mesh),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            mesh.color_interpolation,
            openmat_plot_mir::SurfaceColorInterpolation::Smooth
        );
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::ScreenLines(batch) if batch.lines.len() == PARULA_R2022B.len()
        )));
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::ScreenLines(batch) if batch.lines.len() == 11
        )));
        assert_eq!(
            prepared
                .frame
                .overlay
                .text
                .iter()
                .filter(|text| text.key.starts_with("colorbar-tick-"))
                .map(|text| String::from_utf16_lossy(&text.text.code_units))
                .collect::<Vec<_>>(),
            vec!["1", "1.5", "2", "2.5", "3", "3.5", "4"]
        );
    }

    #[test]
    fn colorbar_ticks_follow_r2022b_height_sensitive_steps() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.c_limits = [-0.999_999_251_732_804, 0.999_999_114_134_739_9];
        properties.colorbar_visible = true;

        let short = colorbar_ticks(
            properties,
            None,
            CanvasSize::new(560.0, 250.0, 1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            short
                .ticks
                .iter()
                .map(|tick| tick.value)
                .collect::<Vec<_>>(),
            vec![-0.5, 0.0, 0.5]
        );
        let tall = colorbar_ticks(
            properties,
            None,
            CanvasSize::new(560.0, 320.0, 1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            tall.ticks.iter().map(|tick| tick.value).collect::<Vec<_>>(),
            vec![
                -0.8,
                -0.600_000_000_000_000_1,
                -0.4,
                -0.199_999_999_999_999_96,
                0.0,
                0.199_999_999_999_999_96,
                0.400_000_000_000_000_13,
                0.600_000_000_000_000_1,
                0.8,
            ]
        );
        assert_eq!(
            tall.ticks
                .iter()
                .map(|tick| String::from_utf16_lossy(&tick.label_code_units))
                .collect::<Vec<_>>(),
            vec![
                "-0.8", "-0.6", "-0.4", "-0.2", "0", "0.2", "0.4", "0.6", "0.8"
            ]
        );

        let manual = ColorBarProperties {
            visible: true,
            ticks: vec![
                TickValue::new(-0.5).unwrap(),
                TickValue::new(0.0).unwrap(),
                TickValue::new(0.5).unwrap(),
            ],
            ticks_mode: AxesTickMode::Manual,
            tick_label_code_units: vec![
                "low".encode_utf16().collect(),
                "zero".encode_utf16().collect(),
            ],
            tick_labels_mode: AxesTickMode::Manual,
        };
        let manual = colorbar_ticks(
            properties,
            Some(&manual),
            CanvasSize::new(560.0, 320.0, 1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            manual
                .ticks
                .iter()
                .map(|tick| (tick.value, String::from_utf16_lossy(&tick.label_code_units)))
                .collect::<Vec<_>>(),
            vec![
                (-0.5, "low".to_owned()),
                (0.0, "zero".to_owned()),
                (0.5, "low".to_owned()),
            ]
        );
    }

    #[test]
    fn manual_marker_indices_bypass_lod_and_preserve_raw_order() {
        const COUNT: usize = 4_096;
        let count = u64::try_from(COUNT).unwrap();
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [0.0, 4_095.0];
        properties.y_limits = [0.0, 4_095.0];
        let GraphicsObject::LineSeries { properties, .. } = &mut scene.objects[2] else {
            unreachable!();
        };
        properties.x_data = data_ref("x", DataType::F64, count);
        properties.y_data = data_ref("y", DataType::F64, count);
        properties.marker = Marker::Circle;
        properties.marker_indices = Some(vec![
            MarkerIndex::new(3_001).unwrap(),
            MarkerIndex::new(2).unwrap(),
            MarkerIndex::new(3_001).unwrap(),
            MarkerIndex::new(7).unwrap(),
            MarkerIndex::new(u64::MAX).unwrap(),
        ]);
        scene.referenced_buffers = vec![
            data_ref("x", DataType::F64, count),
            data_ref("y", DataType::F64, count),
        ];
        scene.validate(GraphicsLimits::default()).unwrap();
        let values = (0..COUNT)
            .map(|index| f64::from(u32::try_from(index).unwrap()))
            .collect::<Vec<_>>();
        let mut y = values.clone();
        y[6] = f64::NAN;
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&values)),
            ("y".to_owned(), f64_bytes(&y)),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(100.0, 100.0, 1.0).unwrap(),
        )
        .unwrap();
        let markers = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::MarkerBatch(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(markers.instances.len(), 3);
        assert_eq!(
            markers.instances[0].center.x.to_bits(),
            markers.instances[2].center.x.to_bits()
        );
        assert!(markers.instances[0].center.x > markers.instances[1].center.x);
    }

    #[test]
    fn explicit_axes_ticks_box_and_style_reach_the_overlay() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_tick = vec![
            TickValue::new(f64::NEG_INFINITY).unwrap(),
            TickValue::new(1.0e12 + 1.0).unwrap(),
            TickValue::new(1.0e12 + 2.0).unwrap(),
            TickValue::new(f64::INFINITY).unwrap(),
        ];
        properties.x_tick_mode = AxesTickMode::Manual;
        properties.x_tick_label_code_units = vec!["$x$".encode_utf16().collect()];
        properties.x_tick_label_mode = AxesTickMode::Manual;
        properties.tick_label_interpreter = WireTextInterpreter::Latex;
        properties.box_enabled = true;
        properties.font_size_css_px = 14.0;
        properties.line_width_css_px = 2.0;
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        assert_eq!(prepared.frame.overlay.axis_lines.len(), 4);
        assert!(
            prepared
                .frame
                .overlay
                .axis_lines
                .iter()
                .all(|line| line.width_css_px.get().to_bits() == 2.0_f32.to_bits())
        );
        let x_labels = prepared
            .frame
            .overlay
            .text
            .iter()
            .filter(|text| text.key.starts_with("x-tick-"))
            .collect::<Vec<_>>();
        assert_eq!(x_labels.len(), 2);
        assert!(x_labels.iter().all(|text| {
            text.text.code_units == "$x$".encode_utf16().collect::<Vec<_>>()
                && text.font.size_css_px.get().to_bits() == 14.0_f32.to_bits()
                && text.interpreter == MirTextInterpreter::Latex
        }));
    }

    #[test]
    fn line_width_obeys_one_device_pixel_hairline_floor() {
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &snapshot(true),
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        let width = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::StrokeMesh(mesh) => Some(mesh.style.width_css_px.get()),
                _ => None,
            })
            .unwrap();
        assert_eq!(width.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn browser_metrics_complete_the_retained_second_layout_pass() {
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[1.0, 2.0, 3.0])),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &snapshot(true),
            &buffers,
            CanvasSize::new(640.0, 480.0, 1.0).unwrap(),
        )
        .unwrap();
        let text_layout = prepared.text_layout_state().unwrap();
        let measurements = text_layout
            .pass
            .requests
            .iter()
            .map(|request| TextMeasurement {
                key: request.key.clone(),
                metrics: TextMetrics::new(42.0, 12.0, 8.0, 2.0, 41.0).unwrap(),
            })
            .collect();
        let measured = text_layout.complete(1, measurements).unwrap();
        assert!(!measured.text.is_empty());
        assert_eq!(measured.font_cache_revision, 1);
        assert!(text_layout.complete(2, Vec::new()).is_err());
    }

    #[test]
    fn data_cursor_source_points_keep_original_indices_and_f64_values() {
        let scene = snapshot(true);
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[f64::NAN, 3.25, -2.5])),
        ]);
        let points = source_points(&scene, &buffers, "series").unwrap();
        assert_eq!(
            points,
            vec![
                SourcePoint {
                    source_index: 1,
                    x: 1.0e12 + 2.0,
                    y: 3.25,
                },
                SourcePoint {
                    source_index: 2,
                    x: 1.0e12 + 3.0,
                    y: -2.5,
                },
            ]
        );
    }

    #[test]
    fn line3_lowers_to_depth_geometry_and_preserves_pick_source_coordinates() {
        let mut scene = snapshot(true);
        let z_ref = data_ref("z", DataType::F64, 3);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            panic!()
        };
        properties.view = [-37.5, 30.0];
        properties.z_scale = Scale::Log;
        properties.z_direction = WireAxisDirection::Reverse;
        properties.z_limits = [1.0, 100.0];
        properties.data_aspect_ratio = [1.0, 1.0, 1.0];
        properties.data_aspect_ratio_mode = AxesLimitMode::Manual;
        let GraphicsObject::LineSeries { properties, .. } = &mut scene.objects[2] else {
            panic!()
        };
        properties.z_data = Nullable(Some(z_ref.clone()));
        properties.marker = Marker::Circle;
        scene.referenced_buffers.push(z_ref);
        let buffers = HashMap::from([
            (
                "x".to_owned(),
                f64_bytes(&[1.0e12 + 1.0, 1.0e12 + 2.0, 1.0e12 + 3.0]),
            ),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
            ("z".to_owned(), f64_bytes(&[1.0, 10.0, 100.0])),
        ]);
        scene
            .validate(openmat_plot_protocol::GraphicsLimits::default())
            .unwrap();
        let sources = source_points_3d(&scene, &buffers, None).unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(
            (sources[1].x, sources[1].y, sources[1].z),
            (1.0e12 + 2.0, 3.0, 10.0)
        );
        assert!((f64::from(sources[0].normalized.z) - 1.0).abs() < f64::EPSILON);
        assert!((f64::from(sources[1].normalized.z) - 0.5).abs() < f64::EPSILON);
        assert!(f64::from(sources[2].normalized.z).abs() < f64::EPSILON);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(640.0, 480.0, 2.0).unwrap(),
        )
        .unwrap();
        assert!(prepared.camera_3d.is_some());
        assert!(
            prepared
                .frame
                .operations
                .iter()
                .any(|operation| { matches!(operation.command, MirCommand::LineSegments3D(_)) })
        );
        assert!(
            prepared
                .frame
                .operations
                .iter()
                .any(|operation| { matches!(operation.command, MirCommand::MarkerBatch3D(_)) })
        );
    }

    #[test]
    fn maps_all_wire_line_styles_to_css_scaled_strokes() {
        let solid = line_stroke_style(LineStyle::Solid, 2.0).unwrap();
        assert!(solid.dash_pattern_css_px.is_empty());
        let dash = line_stroke_style(LineStyle::Dash, 2.0).unwrap();
        assert_eq!(
            dash.dash_pattern_css_px
                .iter()
                .map(|length| length.get())
                .collect::<Vec<_>>(),
            vec![12.0, 6.0]
        );
        let dot = line_stroke_style(LineStyle::Dot, 0.5).unwrap();
        assert_eq!(dot.cap, LineCap::Round);
        assert_eq!(
            dot.dash_pattern_css_px
                .iter()
                .map(|length| length.get())
                .collect::<Vec<_>>(),
            vec![1.0, 2.5]
        );
        let dash_dot = line_stroke_style(LineStyle::DashDot, 1.0).unwrap();
        assert_eq!(dash_dot.dash_pattern_css_px.len(), 4);
    }

    #[test]
    fn maps_every_marker_shape_currently_representable_in_plot_mir() {
        assert_eq!(
            supported_marker(Marker::Point).unwrap().primary,
            MarkerShape::Circle
        );
        assert_eq!(
            supported_marker(Marker::Circle).unwrap().primary,
            MarkerShape::Circle
        );
        assert_eq!(
            supported_marker(Marker::Square).unwrap().primary,
            MarkerShape::Square
        );
        assert_eq!(
            supported_marker(Marker::Diamond).unwrap().primary,
            MarkerShape::Diamond
        );
        assert_eq!(
            supported_marker(Marker::TriangleUp).unwrap().primary,
            MarkerShape::UpTriangle
        );
        assert_eq!(
            supported_marker(Marker::TriangleDown).unwrap().primary,
            MarkerShape::DownTriangle
        );
        assert_eq!(
            supported_marker(Marker::Plus).unwrap().primary,
            MarkerShape::Plus
        );
        assert_eq!(
            supported_marker(Marker::Cross).unwrap().primary,
            MarkerShape::Cross
        );
        assert_eq!(
            supported_marker(Marker::TriangleRight)
                .unwrap()
                .rotation_radians
                .to_bits(),
            std::f32::consts::FRAC_PI_2.to_bits()
        );
        assert_eq!(
            supported_marker(Marker::TriangleLeft)
                .unwrap()
                .rotation_radians
                .to_bits(),
            (-std::f32::consts::FRAC_PI_2).to_bits()
        );
        assert_eq!(
            supported_marker(Marker::Star).unwrap().secondary,
            Some(MarkerShape::Cross)
        );
    }

    #[test]
    fn lowers_visible_unicode_legend_to_mir_and_browser_dto() {
        let mut scene = snapshot(true);
        let GraphicsObject::Axes2d {
            fields: axes_fields,
            ..
        } = &mut scene.objects[1]
        else {
            unreachable!();
        };
        axes_fields.children.push("legend".to_owned());
        scene.objects.push(GraphicsObject::Legend {
            fields: fields("legend", Some("axes"), &[]),
            properties: LegendProperties {
                series_ids: vec!["series".to_owned()],
                label_code_units: vec!["温度 🌡️".encode_utf16().collect()],
                location: LegendLocation::SouthEast,
                visible: true,
                background_rgba: WireRgba([1.0, 1.0, 1.0, 0.9]),
                border_rgba: WireRgba([0.2, 0.2, 0.2, 1.0]),
                font_family_code_units: "Arial".encode_utf16().collect(),
                font_size_css_px: 10.0,
                font_weight: 400,
                interpreter: openmat_plot_protocol::TextInterpreter::Tex,
                orientation: WireLegendOrientation::Vertical,
                num_columns: 1,
            },
        });
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("x".to_owned(), f64_bytes(&[1.0, 2.0, 3.0])),
            ("y".to_owned(), f64_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(360.0, 240.0, 2.0).unwrap(),
        )
        .unwrap();
        let legend = prepared.frame.overlay.legend.as_ref().unwrap();
        assert_eq!(legend.entries.len(), 1);
        assert!(legend.bounds.right() <= prepared.frame.overlay.viewport_css.right());
        assert!(prepared.frame.overlay.text.iter().any(|text| {
            text.role == OverlayTextRole::LegendLabel
                && text.text.code_units == "温度 🌡️".encode_utf16().collect::<Vec<_>>()
        }));
        let dto = prepared.overlay_dto();
        assert_eq!(dto.legend.unwrap().entries.len(), 1);
        assert!(dto.text.iter().any(|text| text.role == "legendLabel"));
    }

    #[test]
    fn lowers_f32_scatter_snapshot_to_marker_batch() {
        let buffers = HashMap::from([
            ("x".to_owned(), f32_bytes(&[1.0, 2.0, 3.0])),
            ("y".to_owned(), f32_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let mut scene = snapshot(false);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [0.0, 4.0];
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(400.0, 300.0, 1.0).unwrap(),
        )
        .unwrap();
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::MarkerBatch(batch) if batch.instances.len() == 3
        )));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn scalar_cdata_scatter3_uses_per_instance_colormap_fill() {
        let mut scene = snapshot(false);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.x_limits = [0.0, 4.0];
        properties.y_limits = [0.0, 4.0];
        properties.z_limits = [0.0, 4.0];
        properties.c_limits = [10.0, 30.0];
        properties.view = [-37.5, 30.0];
        let GraphicsObject::ScatterSeries { properties, .. } = &mut scene.objects[2] else {
            unreachable!();
        };
        properties.z_data = Nullable(Some(data_ref("z", DataType::F32, 3)));
        properties.color_data = Nullable(Some(matrix_ref("c", DataType::F64, 3, 1)));
        properties.color_data_target = ScatterColorTarget::Face;
        properties.marker_face_rgba = WireRgba([0.0, 0.0, 0.0, 0.0]);
        properties.marker_edge_rgba = WireRgba([0.0, 0.0, 0.0, 0.0]);
        scene.referenced_buffers.extend([
            data_ref("z", DataType::F32, 3),
            matrix_ref("c", DataType::F64, 3, 1),
        ]);
        scene.validate(GraphicsLimits::default()).unwrap();
        let buffers = HashMap::from([
            ("x".to_owned(), f32_bytes(&[1.0, 2.0, 3.0])),
            ("y".to_owned(), f32_bytes(&[1.0, 3.0, 2.0])),
            ("z".to_owned(), f32_bytes(&[3.0, 2.0, 1.0])),
            ("c".to_owned(), f64_bytes(&[10.0, 20.0, 30.0])),
        ]);
        let prepared = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(400.0, 300.0, 1.0).unwrap(),
        )
        .unwrap();
        let batch = prepared
            .frame
            .operations
            .iter()
            .find_map(|operation| match &operation.command {
                MirCommand::MarkerBatch3D(batch) => Some(batch),
                _ => None,
            })
            .unwrap();
        assert_eq!(batch.instances.len(), 3);
        assert_ne!(batch.instances[0].fill, batch.instances[1].fill);
        assert_ne!(batch.instances[1].fill, batch.instances[2].fill);
        assert!(
            batch
                .instances
                .iter()
                .all(|instance| { instance.fill.alpha == 1.0 && instance.stroke.alpha == 0.0 })
        );
    }

    #[test]
    fn tiny_non_square_dark_scene_keeps_contrast_and_bounded_tick_density() {
        let mut scene = snapshot(false);
        let GraphicsObject::Figure { properties, .. } = &mut scene.objects[0] else {
            unreachable!();
        };
        properties.background_rgba = WireRgba([0.06, 0.07, 0.08, 1.0]);
        let GraphicsObject::Axes2d { properties, .. } = &mut scene.objects[1] else {
            unreachable!();
        };
        properties.background_rgba = openmat_plot_protocol::Nullable(None);
        properties.x_limits = [0.0, 4.0];
        let buffers = HashMap::from([
            ("x".to_owned(), f32_bytes(&[1.0, 2.0, 3.0])),
            ("y".to_owned(), f32_bytes(&[1.0, 3.0, 2.0])),
        ]);
        let prepared =
            prepare_scene(&scene, &buffers, CanvasSize::new(96.0, 44.0, 3.0).unwrap()).unwrap();
        assert!(prepared.frame.overlay.axis_lines[0].color.red > 0.8);
        assert!(
            prepared
                .frame
                .overlay
                .text
                .iter()
                .all(|text| text.anchor.x.is_finite() && text.anchor.y.is_finite())
        );
        assert!(prepared.frame.overlay.ticks.len() <= 6);
        assert!(prepared.frame.operations.iter().any(|operation| matches!(
            &operation.command,
            MirCommand::ScreenLines(batch)
                if batch.lines.first().is_some_and(|line| line.color.red > 0.8)
        )));
        assert!(prepared.overlay_dto().ticks.is_empty());
        assert_eq!(
            prepared.overlay_dto().canvas_width_css_px.to_bits(),
            96.0_f64.to_bits()
        );
    }

    #[test]
    fn colormap_mapping_uses_r2022b_discrete_scaled_and_direct_indices() {
        let colors = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        let scaled = openmat_plot_protocol::CDataMapping::Scaled;
        assert_eq!(
            surface_colormap(0.249_999, [0.0, 1.0], scaled, &colors).unwrap(),
            rgba(1.0, 0.0, 0.0, 1.0).unwrap()
        );
        assert_eq!(
            surface_colormap(0.25, [0.0, 1.0], scaled, &colors).unwrap(),
            rgba(0.0, 1.0, 0.0, 1.0).unwrap()
        );
        assert_eq!(
            surface_colormap(0.75, [0.0, 1.0], scaled, &colors).unwrap(),
            rgba(1.0, 1.0, 1.0, 1.0).unwrap()
        );

        let direct = openmat_plot_protocol::CDataMapping::Direct;
        assert_eq!(
            surface_colormap(1.9, [0.0, 1.0], direct, &colors).unwrap(),
            rgba(1.0, 0.0, 0.0, 1.0).unwrap()
        );
        assert_eq!(
            surface_colormap(2.0, [0.0, 1.0], direct, &colors).unwrap(),
            rgba(0.0, 1.0, 0.0, 1.0).unwrap()
        );
        assert_eq!(
            surface_colormap(f64::INFINITY, [0.0, 1.0], direct, &colors).unwrap(),
            rgba(1.0, 1.0, 1.0, 1.0).unwrap()
        );
    }

    #[test]
    fn missing_or_misaligned_buffers_are_rejected() {
        let scene = snapshot(true);
        let missing = prepare_scene(
            &scene,
            &HashMap::new(),
            CanvasSize::new(400.0, 300.0, 1.0).unwrap(),
        );
        assert_eq!(missing.unwrap_err().kind(), PlotWebErrorKind::InvalidScene);

        let buffers = HashMap::from([
            ("x".to_owned(), vec![0; 3]),
            ("y".to_owned(), f64_bytes(&[1.0, 2.0, 3.0])),
        ]);
        let invalid = prepare_scene(
            &scene,
            &buffers,
            CanvasSize::new(400.0, 300.0, 1.0).unwrap(),
        );
        assert_eq!(invalid.unwrap_err().kind(), PlotWebErrorKind::InvalidScene);
    }
}
