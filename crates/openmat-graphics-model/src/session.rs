use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    Axes2DProperties, AxesCoordinateSystem, Axis, AxisDirection, AxisScale, CDataMapping,
    ChartColor, ChartGroupInput, ChartGroupKind, ChartGroupProperties, ChartPrimitiveInput,
    ColorBarProperties, DataDType, DataRef, DataResource, DataResourceId, FigureCreationProperties,
    FigureProperties, FigureSnapshot, FontStyle, GraphicsClass, GraphicsDelta,
    GraphicsDeltaOperation, GraphicsError, GraphicsExecution, GraphicsHandle, GraphicsNotice,
    GraphicsNoticeKind, GraphicsObject, GraphicsProperty, GraphicsPropertyUpdate,
    GraphicsPropertyValue, GraphicsRequest, GraphicsResponse, GridMode, HirObject, HirProperties,
    HorizontalAlignment, Interpreter, IsoNormalsInput, LegendLocation, LegendOrientation,
    LegendProperties, LightingMode, LimitMethod, LimitMode, LineInput, LineSeriesProperties,
    LineStyle, Marker, MarkerIndicesClass, NextPlot, NextTileSelection, NumericData, PARULA_R2022B,
    PatchInput, PatchSeriesProperties, ProjectionMode, ScatterColorTarget, ScatterInput,
    ScatterSeriesProperties, ShadingMode, StyledLineInput, SurfaceColor, SurfaceInput,
    SurfaceSeriesProperties, SurfaceStyle, TextProperties, TextRole, ThetaAxisUnits,
    ThetaDirection, ThetaZeroLocation, TickDirection, TickMode, TilePadding, TileSpacing,
    TiledChartLayoutProperties, VerticalAlignment,
};

const MAX_OBJECTS: usize = 100_000;
const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
const DEFAULT_LIMITS: [f64; 2] = [0.0, 1.0];
const COLOR_ORDER: [[f32; 4]; 7] = [
    [0.0, 0.447, 0.741, 1.0],
    [0.85, 0.325, 0.098, 1.0],
    [0.929, 0.694, 0.125, 1.0],
    [0.494, 0.184, 0.556, 1.0],
    [0.466, 0.674, 0.188, 1.0],
    [0.301, 0.745, 0.933, 1.0],
    [0.635, 0.078, 0.184, 1.0],
];

static NEXT_SESSION_SERIAL: AtomicU64 = AtomicU64::new(1);

/// Narrow policy boundary for authoritative kernel-owned automatic limits.
pub trait AutoLimitPlanner: Send + Sync {
    /// Resolves finite increasing limits from all finite values on one axis.
    fn resolve(&self, finite_values: &[f64]) -> [f64; 2];
}

/// Deterministic first-tranche automatic-limit implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultAutoLimitPlanner;

impl AutoLimitPlanner for DefaultAutoLimitPlanner {
    fn resolve(&self, finite_values: &[f64]) -> [f64; 2] {
        let Some((&first, rest)) = finite_values.split_first() else {
            return DEFAULT_LIMITS;
        };
        let (minimum, maximum) = rest
            .iter()
            .fold((first, first), |(minimum, maximum), value| {
                (minimum.min(*value), maximum.max(*value))
            });
        if minimum < maximum {
            return [minimum, maximum];
        }
        let padding = (minimum.abs() * 0.05).max(1.0);
        let lower = minimum - padding;
        let upper = maximum + padding;
        if lower.is_finite() && upper.is_finite() && lower < upper {
            [lower, upper]
        } else {
            DEFAULT_LIMITS
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    public_id: String,
    object_revision: u64,
    parent: Option<GraphicsHandle>,
    children: Vec<GraphicsHandle>,
    object: GraphicsObject,
}

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    retired: bool,
    node: Option<Node>,
}

#[derive(Clone, Debug)]
struct FigureChange {
    handle: GraphicsHandle,
    closed: Option<ClosedFigure>,
}

#[derive(Clone, Debug)]
struct ClosedFigure {
    figure_id: String,
    previous_revision: u64,
    deleted: Vec<(String, u32)>,
}

#[derive(Clone, Debug)]
struct AppliedRequest {
    response: GraphicsResponse,
    change: Option<FigureChange>,
}

#[derive(Clone, Debug)]
struct ChartPrimitiveResourceUpdate {
    handle: GraphicsHandle,
    x_data: Option<DataRef>,
    y_data: Option<DataRef>,
    vertices: Option<DataRef>,
}

/// One session-owned retained graphics hierarchy and immutable resource table.
#[derive(Clone)]
pub struct GraphicsSession {
    slots: Vec<Slot>,
    free_slots: Vec<u32>,
    resources: BTreeMap<DataResourceId, DataResource>,
    current_figure: Option<GraphicsHandle>,
    current_axes: Option<GraphicsHandle>,
    current_layout: Option<GraphicsHandle>,
    next_resource_id: Option<u64>,
    next_public_id: Option<u64>,
    session_serial: u64,
    latest_deltas: BTreeMap<String, GraphicsDelta>,
    pending_deltas: Vec<GraphicsDelta>,
    figure_revisions: BTreeMap<String, u64>,
    auto_limits: Arc<dyn AutoLimitPlanner>,
}

impl GraphicsSession {
    /// Creates an empty graphics session with deterministic default auto limits.
    #[must_use]
    pub fn new() -> Self {
        Self::with_auto_limit_planner(Arc::new(DefaultAutoLimitPlanner))
    }

    /// Creates a session with an explicitly selected automatic-limit planner.
    #[must_use]
    pub fn with_auto_limit_planner(planner: Arc<dyn AutoLimitPlanner>) -> Self {
        let serial = NEXT_SESSION_SERIAL.fetch_add(1, Ordering::Relaxed);
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
            resources: BTreeMap::new(),
            current_figure: None,
            current_axes: None,
            current_layout: None,
            next_resource_id: Some(1),
            next_public_id: Some(1),
            session_serial: serial,
            latest_deltas: BTreeMap::new(),
            pending_deltas: Vec::new(),
            figure_revisions: BTreeMap::new(),
            auto_limits: planner,
        }
    }

    /// Executes one graphics builtin atomically.
    ///
    /// Validation and allocation occur in a private staged clone. A failure
    /// leaves the previously committed session untouched. A successful call is
    /// committed before the runtime attempts to publish its internal notice.
    ///
    /// # Errors
    ///
    /// Returns a structured model error without partial state.
    pub fn execute(
        &mut self,
        request: GraphicsRequest,
    ) -> Result<GraphicsExecution, GraphicsError> {
        if matches!(request, GraphicsRequest::CloseAllFigures) {
            return self.execute_close_all_figures();
        }
        let before = self.clone();
        let mut staged = self.clone();
        let applied = staged.apply_request(request)?;
        let notice = if let Some(change) = applied.change {
            Some(staged.finalize_change(&before, change)?)
        } else {
            None
        };
        staged.collect_unreferenced_resources();
        *self = staged;
        Ok(GraphicsExecution {
            response: applied.response,
            notice,
        })
    }

    /// Returns whether a handle still resolves to the exact generation/class.
    #[must_use]
    pub fn is_valid(&self, handle: GraphicsHandle) -> bool {
        self.node(handle).is_ok()
    }

    /// Returns the current Figure without creating one.
    #[must_use]
    pub const fn current_figure(&self) -> Option<GraphicsHandle> {
        self.current_figure
    }

    /// Returns the current Axes without creating one.
    #[must_use]
    pub const fn current_axes(&self) -> Option<GraphicsHandle> {
        self.current_axes
    }

    /// Returns the active Axes colormap length without creating graphics state.
    #[must_use]
    pub fn current_colormap_length(&self) -> Option<usize> {
        let axes = self.current_axes.filter(|handle| self.is_valid(*handle))?;
        let GraphicsObject::Axes2D(properties) = &self.node(axes).ok()?.object else {
            return None;
        };
        Some(properties.colormap.len())
    }

    /// Returns a borrowed strong object payload.
    ///
    /// # Errors
    ///
    /// Rejects stale, foreign-class, and unknown handles.
    pub fn object(&self, handle: GraphicsHandle) -> Result<&GraphicsObject, GraphicsError> {
        Ok(&self.node(handle)?.object)
    }

    /// Returns stable child handles.
    ///
    /// # Errors
    ///
    /// Rejects stale handles.
    pub fn children(&self, handle: GraphicsHandle) -> Result<&[GraphicsHandle], GraphicsError> {
        Ok(&self.node(handle)?.children)
    }

    /// Returns one immutable resource snapshot.
    #[must_use]
    pub fn data_resource(&self, id: DataResourceId) -> Option<DataResource> {
        self.resources.get(&id).cloned()
    }

    /// Returns the most recent delta for a Figure identifier.
    #[must_use]
    pub fn latest_delta(&self, figure_id: &str) -> Option<&GraphicsDelta> {
        self.latest_deltas.get(figure_id)
    }

    /// Drains every committed Figure delta in transaction order.
    ///
    /// This queue is intended for the owning host adapter. Reading snapshots
    /// does not consume it, and failed graphics transactions never append to it.
    pub fn take_pending_deltas(&mut self) -> Vec<GraphicsDelta> {
        std::mem::take(&mut self.pending_deltas)
    }

    /// Returns the public identifier for one exact live handle.
    ///
    /// # Errors
    ///
    /// Rejects stale, unknown, or class-mismatched handles.
    pub fn object_id(&self, handle: GraphicsHandle) -> Result<&str, GraphicsError> {
        Ok(self.node(handle)?.public_id.as_str())
    }

    /// Resolves one live object identifier back to its exact handle.
    #[must_use]
    pub fn handle_by_id(&self, object_id: &str) -> Option<GraphicsHandle> {
        self.live_handles().find(|handle| {
            self.node(*handle)
                .is_ok_and(|node| node.public_id == object_id)
        })
    }

    /// Returns complete snapshots for all live Figures in Figure-number order.
    ///
    /// # Errors
    ///
    /// Returns an error if a live Figure hierarchy is internally inconsistent.
    pub fn figure_snapshots(&self) -> Result<Vec<FigureSnapshot>, GraphicsError> {
        let mut figures = self
            .live_handles()
            .filter(|handle| handle.class() == GraphicsClass::Figure)
            .map(|handle| {
                let number = match self.object(handle)? {
                    GraphicsObject::Figure(properties) => properties.number,
                    _ => return Err(GraphicsError::invalid_state("Figure has wrong payload")),
                };
                Ok((number, handle))
            })
            .collect::<Result<Vec<_>, _>>()?;
        figures.sort_unstable_by_key(|(number, _)| *number);
        figures
            .into_iter()
            .map(|(_, handle)| self.snapshot(handle))
            .collect()
    }

    /// Builds a complete snapshot by public Figure identifier.
    ///
    /// # Errors
    ///
    /// Rejects an unknown identifier or a non-Figure object.
    pub fn snapshot_by_id(&self, figure_id: &str) -> Result<FigureSnapshot, GraphicsError> {
        let handle = self
            .handle_by_id(figure_id)
            .ok_or_else(|| GraphicsError::invalid_handle("unknown Figure identifier"))?;
        if handle.class() != GraphicsClass::Figure {
            return Err(GraphicsError::invalid_handle(
                "graphics identifier does not name a Figure",
            ));
        }
        self.snapshot(handle)
    }

    /// Returns one immutable resource by its public numeric identifier.
    #[must_use]
    pub fn data_resource_by_identifier(&self, identifier: u64) -> Option<DataResource> {
        self.resources
            .iter()
            .find_map(|(id, resource)| (id.identifier() == identifier).then(|| resource.clone()))
    }

    /// Builds a complete stable snapshot for a live Figure.
    ///
    /// # Errors
    ///
    /// Rejects stale or non-Figure handles and inconsistent descendants.
    pub fn snapshot(&self, figure: GraphicsHandle) -> Result<FigureSnapshot, GraphicsError> {
        let root = self.node_of_class(figure, GraphicsClass::Figure)?;
        let revision = self
            .figure_revisions
            .get(&root.public_id)
            .copied()
            .ok_or_else(|| GraphicsError::invalid_state("Figure has no committed revision"))?;
        let mut objects = Vec::new();
        let mut data_ids = BTreeSet::new();
        self.snapshot_subtree(figure, &mut objects, &mut data_ids)?;
        let referenced_data = data_ids
            .into_iter()
            .map(|id| {
                self.resources
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| GraphicsError::invalid_state("HIR references a missing buffer"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FigureSnapshot {
            figure_id: root.public_id.clone(),
            revision,
            root_id: root.public_id.clone(),
            objects,
            referenced_data,
        })
    }

    /// Returns the live arena object count.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.node.is_some()).count()
    }

    /// Returns the retained immutable resource count.
    #[must_use]
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    #[allow(clippy::too_many_lines)]
    fn apply_request(&mut self, request: GraphicsRequest) -> Result<AppliedRequest, GraphicsError> {
        match request {
            GraphicsRequest::Figure { number } => self.select_or_create_figure(number),
            GraphicsRequest::CreateFigure { properties } => {
                self.create_figure_with_properties(properties)
            }
            GraphicsRequest::CurrentFigure => self.current_or_create_figure(),
            GraphicsRequest::CurrentFigureOrEmpty => Ok(AppliedRequest {
                response: GraphicsResponse::Handles(
                    self.current_figure
                        .filter(|handle| self.is_valid(*handle))
                        .into_iter()
                        .collect(),
                ),
                change: None,
            }),
            GraphicsRequest::Axes { select } => self.create_or_select_axes(select),
            GraphicsRequest::CreateAxes { figure, properties } => {
                self.create_axes(figure, &properties)
            }
            GraphicsRequest::CreatePolarAxes { figure, properties } => {
                self.create_polar_axes(figure, &properties)
            }
            GraphicsRequest::Subplot { figure, position } => self.subplot(figure, position),
            GraphicsRequest::CreateTiledLayout {
                figure,
                rows,
                columns,
                tile_spacing,
                padding,
            } => self.create_tiled_layout(figure, rows, columns, tile_spacing, padding),
            GraphicsRequest::NextTile { layout, selection } => self.next_tile(layout, selection),
            GraphicsRequest::CurrentAxes => self.current_or_create_axes(),
            GraphicsRequest::Plot { lines } => self.plot(lines),
            GraphicsRequest::PlotStyled {
                axes,
                lines,
                properties,
            } => self.plot_styled(axes, lines, &properties),
            GraphicsRequest::PlotSeries {
                axes,
                series,
                properties,
            } => self.plot_series(axes, series, &properties),
            GraphicsRequest::PolarPlotSeries {
                axes,
                series,
                properties,
            } => self.polar_plot_series(axes, series, &properties),
            GraphicsRequest::PlotWithAxesProperties {
                axes,
                lines,
                properties,
                axes_properties,
            } => self.plot_styled_with_axes_properties(axes, lines, &properties, &axes_properties),
            GraphicsRequest::Scatter {
                axes,
                series,
                properties,
            } => self.scatter(axes, &series, &properties),
            GraphicsRequest::Surface {
                axes,
                surface,
                style,
                properties,
            } => self.surface(axes, &surface, style, &properties),
            GraphicsRequest::Patch {
                axes,
                patch,
                properties,
            } => self.patch(axes, &patch, &properties),
            GraphicsRequest::ChartGroup { axes, chart } => self.chart_group(axes, &chart),
            GraphicsRequest::Shading { axes, mode } => self.shading(axes, mode),
            GraphicsRequest::SetHeadlight { axes, enabled } => self.set_headlight(axes, enabled),
            GraphicsRequest::SetLighting { axes, mode } => self.set_lighting(axes, mode),
            GraphicsRequest::SetPatchSmoothNormals { patch } => {
                self.set_patch_smooth_normals(patch)
            }
            GraphicsRequest::SetPatchIsoNormals { patch, volume } => {
                self.set_patch_iso_normals(patch, &volume)
            }
            GraphicsRequest::Colorbar { axes, visible } => self.colorbar(axes, visible),
            GraphicsRequest::SetColormap { axes, colors } => self.set_colormap(axes, colors),
            GraphicsRequest::GetColormap { axes } => self.get_colormap(axes),
            GraphicsRequest::SetView {
                axes,
                azimuth_degrees,
                elevation_degrees,
            } => self.set_view(axes, azimuth_degrees, elevation_degrees),
            GraphicsRequest::SetAxesCamera {
                figure,
                axes,
                azimuth_degrees,
                elevation_degrees,
                camera_scale,
            } => self.set_axes_camera(
                figure,
                axes,
                azimuth_degrees,
                elevation_degrees,
                camera_scale,
            ),
            GraphicsRequest::SetHold { axes, enabled } => self.set_hold(axes, enabled),
            GraphicsRequest::IsHold { axes } => self.is_hold(axes),
            GraphicsRequest::GetChildren { parent } => self.get_children(parent),
            GraphicsRequest::IsGraphics { handles } => Ok(AppliedRequest {
                response: GraphicsResponse::Logicals(
                    handles
                        .into_iter()
                        .map(|handle| self.is_valid(handle))
                        .collect(),
                ),
                change: None,
            }),
            GraphicsRequest::SetProperties {
                handles,
                properties,
            } => self.set_properties(&handles, &properties),
            GraphicsRequest::GetProperties { handles, property } => {
                self.get_properties(&handles, property)
            }
            GraphicsRequest::SetText {
                axes,
                role,
                code_units,
                properties,
            } => self.set_text(axes, role, code_units, &properties),
            GraphicsRequest::Legend {
                axes,
                labels_utf16,
                properties,
            } => self.set_legend(axes, labels_utf16, &properties),
            GraphicsRequest::SetAxesProperties { axes, properties } => {
                self.set_axes_properties(axes, &properties)
            }
            GraphicsRequest::GetAxesProperty { axes, property } => {
                self.get_axes_property(axes, property)
            }
            GraphicsRequest::ToggleAxesBox { axes } => self.toggle_axes_box(axes),
            GraphicsRequest::SetLimits { axes, axis, limits } => {
                self.set_limits(axes, axis, limits)
            }
            GraphicsRequest::SetAxesLimits {
                figure,
                axes,
                x_limits,
                y_limits,
            } => self.set_axes_limits(figure, axes, x_limits, y_limits),
            GraphicsRequest::GetLimits { axes, axis } => self.get_limits(axes, axis),
            GraphicsRequest::SetLimitMode { axes, axis, mode } => {
                self.set_limit_mode(axes, axis, mode)
            }
            GraphicsRequest::SetGrid { axes, mode } => self.set_grid(axes, mode),
            GraphicsRequest::ClearAxes { axes } => self.clear_axes(axes),
            GraphicsRequest::ClearFigure { figure } => self.clear_figure(figure),
            GraphicsRequest::CloseFigure { figure } => self.close_figure(figure),
            GraphicsRequest::CloseAllFigures => {
                unreachable!("CloseAllFigures is handled by GraphicsSession::execute")
            }
        }
    }

    fn execute_close_all_figures(&mut self) -> Result<GraphicsExecution, GraphicsError> {
        let before = self.clone();
        let mut staged = self.clone();
        let figures = staged.figure_handles_in_number_order()?;
        let mut changes = Vec::with_capacity(figures.len());
        for figure in figures {
            let applied = staged.close_figure(Some(figure))?;
            if let Some(change) = applied.change {
                changes.push(change);
            }
        }
        for change in changes {
            staged.finalize_change(&before, change)?;
        }
        staged.current_figure = None;
        staged.current_axes = None;
        staged.current_layout = None;
        staged.collect_unreferenced_resources();
        *self = staged;
        Ok(GraphicsExecution {
            response: GraphicsResponse::None,
            // One notice cannot faithfully identify a multi-Figure close. The
            // authoritative per-Figure close deltas are still queued and are
            // what the host adapter publishes to existing windows.
            notice: None,
        })
    }

    fn figure_handles_in_number_order(&self) -> Result<Vec<GraphicsHandle>, GraphicsError> {
        let mut figures = self
            .live_handles()
            .filter(|handle| handle.class() == GraphicsClass::Figure)
            .map(|handle| {
                let GraphicsObject::Figure(properties) = self.object(handle)? else {
                    return Err(GraphicsError::invalid_state("Figure has wrong payload"));
                };
                Ok((properties.number, handle))
            })
            .collect::<Result<Vec<_>, _>>()?;
        figures.sort_unstable_by_key(|(number, _)| *number);
        Ok(figures.into_iter().map(|(_, handle)| handle).collect())
    }

    fn select_or_create_figure(
        &mut self,
        requested_number: Option<u32>,
    ) -> Result<AppliedRequest, GraphicsError> {
        if requested_number == Some(0) {
            return Err(GraphicsError::invalid_input(
                "Figure number must be positive",
            ));
        }
        let number = if let Some(number) = requested_number {
            if let Some(handle) = self.figure_by_number(number) {
                self.current_figure = Some(handle);
                self.current_axes = self.first_axes(handle);
                self.current_layout = self.layout_for_figure(handle);
                return Ok(AppliedRequest {
                    response: GraphicsResponse::Handle(handle),
                    change: None,
                });
            }
            number
        } else {
            self.smallest_available_figure_number()
        };
        let figure = self.allocate_figure(number, FigureCreationProperties::default())?;
        self.current_figure = Some(figure);
        self.current_axes = None;
        self.current_layout = None;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(figure),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn create_figure_with_properties(
        &mut self,
        properties: FigureCreationProperties,
    ) -> Result<AppliedRequest, GraphicsError> {
        if !valid_rgba(properties.background_rgba) {
            return Err(GraphicsError::invalid_input(
                "Figure Color must contain finite RGB values in [0,1]",
            ));
        }
        validate_figure_position(properties.position_css_pixels)?;
        let number = self.smallest_available_figure_number();
        let figure = self.allocate_figure(number, properties)?;
        self.current_figure = Some(figure);
        self.current_axes = None;
        self.current_layout = None;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(figure),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn allocate_figure(
        &mut self,
        number: u32,
        properties: FigureCreationProperties,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.allocate_node(
            None,
            GraphicsObject::Figure(FigureProperties {
                number,
                name_utf16: properties.name_utf16,
                number_title: properties.number_title,
                visible: properties.visible,
                background_rgba: properties.background_rgba,
                position_css_pixels: properties.position_css_pixels,
                next_plot: NextPlot::Add,
            }),
        )
    }

    fn current_or_create_figure(&mut self) -> Result<AppliedRequest, GraphicsError> {
        if let Some(figure) = self.current_figure.filter(|handle| self.is_valid(*handle)) {
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(figure),
                change: None,
            });
        }
        self.select_or_create_figure(None)
    }

    fn create_or_select_axes(
        &mut self,
        select: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        if let Some(axes) = select {
            let parent = self
                .node_of_class(axes, GraphicsClass::Axes2D)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            self.node_of_class(parent, GraphicsClass::Figure)?;
            self.current_figure = Some(parent);
            self.current_axes = Some(axes);
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(axes),
                change: None,
            });
        }
        let (figure, _) = self.ensure_current_figure()?;
        let axes = self.create_axes_node(figure)?;
        self.current_figure = Some(figure);
        self.current_axes = Some(axes);
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(axes),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn create_axes(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_updates(GraphicsClass::Axes2D, properties)?;
        let figure = if let Some(figure) = requested_figure {
            self.node_of_class(figure, GraphicsClass::Figure)?;
            figure
        } else {
            self.ensure_current_figure()?.0
        };
        let axes = self.create_axes_node(figure)?;
        apply_axes_updates(self.axes_properties_mut(axes)?, properties);
        validate_axes_state(self.axes_properties(axes)?)?;
        self.current_figure = Some(figure);
        self.current_axes = Some(axes);
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(axes),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn create_polar_axes(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_updates(GraphicsClass::PolarAxes, properties)?;
        let figure = if let Some(figure) = requested_figure {
            self.node_of_class(figure, GraphicsClass::Figure)?;
            figure
        } else {
            self.ensure_current_figure()?.0
        };
        let axes = self.create_polar_axes_node(figure)?;
        apply_axes_updates(self.axes_properties_mut(axes)?, properties);
        validate_axes_state(self.axes_properties(axes)?)?;
        self.resolve_auto_ticks(axes)?;
        self.current_figure = Some(figure);
        self.current_axes = Some(axes);
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(axes),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn subplot(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
        position: [f64; 4],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_axes_position(position)?;
        let figure = if let Some(figure) = requested_figure {
            self.node_of_class(figure, GraphicsClass::Figure)?;
            figure
        } else {
            self.ensure_current_figure()?.0
        };
        if self.layout_for_figure(figure).is_some() {
            self.delete_layouts_for_figure(figure)?;
            self.delete_children(figure)?;
        }
        let axes_children = self.node(figure)?.children.clone();
        if let Some(axes) = axes_children.iter().copied().find(|handle| {
            handle.class() == GraphicsClass::Axes2D
                && self.axes_properties(*handle).is_ok_and(|properties| {
                    same_axes_position(properties.position_normalized, position)
                })
        }) {
            self.current_figure = Some(figure);
            self.current_axes = Some(axes);
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(axes),
                change: None,
            });
        }

        let overlapping = axes_children
            .into_iter()
            .filter(|handle| handle.class() == GraphicsClass::Axes2D)
            .filter(|handle| {
                self.axes_properties(*handle).is_ok_and(|properties| {
                    axes_positions_overlap(properties.position_normalized, position)
                })
            })
            .collect::<Vec<_>>();
        for axes in overlapping {
            self.delete_subtree(axes)?;
        }
        let axes = self.create_axes_node(figure)?;
        self.axes_properties_mut(axes)?.position_normalized = position;
        self.current_figure = Some(figure);
        self.current_axes = Some(axes);
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(axes),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn create_tiled_layout(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
        rows: u32,
        columns: u32,
        tile_spacing: TileSpacing,
        padding: TilePadding,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, layout) =
            self.replace_with_tiled_layout(requested_figure, rows, columns, tile_spacing, padding)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(layout),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn replace_with_tiled_layout(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
        rows: u32,
        columns: u32,
        tile_spacing: TileSpacing,
        padding: TilePadding,
    ) -> Result<(GraphicsHandle, GraphicsHandle), GraphicsError> {
        let tile_count = rows
            .checked_mul(columns)
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count > 0 && *count <= MAX_OBJECTS)
            .ok_or_else(|| GraphicsError::limit("tiled layout grid exceeds model limits"))?;
        let figure = if let Some(figure) = requested_figure {
            self.node_of_class(figure, GraphicsClass::Figure)?;
            figure
        } else {
            self.ensure_current_figure()?.0
        };
        self.delete_layouts_for_figure(figure)?;
        self.delete_children(figure)?;
        let layout = self.allocate_node(
            None,
            GraphicsObject::TiledChartLayout(TiledChartLayoutProperties {
                parent: figure,
                rows,
                columns,
                tile_spacing,
                padding,
                tile_axes: vec![None; tile_count],
            }),
        )?;
        self.current_figure = Some(figure);
        self.current_axes = None;
        self.current_layout = Some(layout);
        Ok((figure, layout))
    }

    fn next_tile(
        &mut self,
        requested_layout: Option<GraphicsHandle>,
        selection: NextTileSelection,
    ) -> Result<AppliedRequest, GraphicsError> {
        let layout = if let Some(layout) = requested_layout {
            self.node_of_class(layout, GraphicsClass::TiledChartLayout)?;
            layout
        } else if let Some(layout) = self.current_layout.filter(|layout| self.is_valid(*layout)) {
            layout
        } else {
            self.replace_with_tiled_layout(None, 1, 1, TileSpacing::Loose, TilePadding::Loose)?
                .1
        };

        let properties = self.tiled_layout_properties(layout)?.clone();
        let figure = properties.parent;
        self.node_of_class(figure, GraphicsClass::Figure)?;
        let mut tile_axes = properties.tile_axes.clone();
        for axes in &mut tile_axes {
            if axes.is_some_and(|axes| !self.is_valid(axes)) {
                *axes = None;
            }
        }

        let (start_row, start_column, row_span, column_span, indices) =
            Self::resolve_tile_selection(&properties, &tile_axes, selection)?;
        if let Some(axes) = indices.iter().find_map(|index| tile_axes[*index]) {
            self.current_figure = Some(figure);
            self.current_axes = Some(axes);
            self.current_layout = Some(layout);
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(axes),
                change: None,
            });
        }

        let position = tiled_axes_position(
            properties.rows,
            properties.columns,
            start_row,
            start_column,
            row_span,
            column_span,
            properties.tile_spacing,
            properties.padding,
        )?;
        let axes = self.create_axes_node(figure)?;
        self.axes_properties_mut(axes)?.position_normalized = position;
        for index in indices {
            tile_axes[index] = Some(axes);
        }
        self.tiled_layout_properties_mut(layout)?.tile_axes = tile_axes;
        self.current_figure = Some(figure);
        self.current_axes = Some(axes);
        self.current_layout = Some(layout);
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(axes),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn resolve_tile_selection(
        properties: &TiledChartLayoutProperties,
        tile_axes: &[Option<GraphicsHandle>],
        selection: NextTileSelection,
    ) -> Result<(u32, u32, u32, u32, Vec<usize>), GraphicsError> {
        if let NextTileSelection::Tile(tile) = selection {
            let maximum = properties
                .rows
                .checked_mul(properties.columns)
                .ok_or_else(|| GraphicsError::limit("tiled layout grid exceeds model limits"))?;
            if tile == 0 || tile > maximum {
                return Err(GraphicsError::invalid_input(
                    "nexttile tile number is outside the layout grid",
                ));
            }
            let zero_based = tile - 1;
            let row = zero_based / properties.columns;
            let column = zero_based % properties.columns;
            let indices =
                tiled_block_indices(properties.rows, properties.columns, row, column, 1, 1)?;
            return Ok((row, column, 1, 1, indices));
        }

        let (row_span, column_span) = match selection {
            NextTileSelection::Automatic => (1, 1),
            NextTileSelection::Span { rows, columns } => (rows, columns),
            NextTileSelection::Tile(_) => unreachable!("handled above"),
        };
        if row_span == 0
            || column_span == 0
            || row_span > properties.rows
            || column_span > properties.columns
        {
            return Err(GraphicsError::invalid_input(
                "nexttile span is outside the layout grid",
            ));
        }
        for row in 0..=properties.rows - row_span {
            for column in 0..=properties.columns - column_span {
                let indices = tiled_block_indices(
                    properties.rows,
                    properties.columns,
                    row,
                    column,
                    row_span,
                    column_span,
                )?;
                if indices.iter().all(|index| tile_axes[*index].is_none()) {
                    return Ok((row, column, row_span, column_span, indices));
                }
            }
        }
        Err(GraphicsError::invalid_input(
            "nexttile could not find an unoccupied tile span",
        ))
    }

    fn current_or_create_axes(&mut self) -> Result<AppliedRequest, GraphicsError> {
        if let Some(axes) = self.current_axes.filter(|handle| self.is_valid(*handle)) {
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(axes),
                change: None,
            });
        }
        let (figure, _) = self.ensure_current_figure()?;
        if let Some(axes) = self.first_axes(figure) {
            self.current_axes = Some(axes);
            return Ok(AppliedRequest {
                response: GraphicsResponse::Handle(axes),
                change: None,
            });
        }
        self.create_or_select_axes(None)
    }

    fn plot(&mut self, lines: Vec<LineInput>) -> Result<AppliedRequest, GraphicsError> {
        self.plot_styled(None, lines, &[])
    }

    fn plot_styled(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        lines: Vec<LineInput>,
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        self.plot_styled_with_axes_properties(requested_axes, lines, properties, &[])
    }

    fn plot_series(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        series: Vec<StyledLineInput>,
        shared_properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        self.plot_series_with_axes_properties(requested_axes, series, shared_properties, &[])
    }

    fn polar_plot_series(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        series: Vec<StyledLineInput>,
        shared_properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        self.plot_series_for_coordinate_system(
            requested_axes,
            series,
            shared_properties,
            &[],
            AxesCoordinateSystem::Polar,
        )
    }

    fn plot_styled_with_axes_properties(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        lines: Vec<LineInput>,
        properties: &[GraphicsPropertyUpdate],
        axes_updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        let series = lines
            .into_iter()
            .map(|line| StyledLineInput {
                line,
                properties: Vec::new(),
            })
            .collect();
        self.plot_series_with_axes_properties(requested_axes, series, properties, axes_updates)
    }

    fn plot_series_with_axes_properties(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        series: Vec<StyledLineInput>,
        shared_properties: &[GraphicsPropertyUpdate],
        axes_updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        self.plot_series_for_coordinate_system(
            requested_axes,
            series,
            shared_properties,
            axes_updates,
            AxesCoordinateSystem::Cartesian,
        )
    }

    fn plot_series_for_coordinate_system(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        series: Vec<StyledLineInput>,
        shared_properties: &[GraphicsPropertyUpdate],
        axes_updates: &[GraphicsPropertyUpdate],
        coordinate_system: AxesCoordinateSystem,
    ) -> Result<AppliedRequest, GraphicsError> {
        for styled in &series {
            let line = &styled.line;
            validate_pair(&line.x, &line.y)?;
            if let Some(z) = &line.z {
                validate_pair(&line.x, z)?;
            }
            validate_property_updates(GraphicsClass::LineSeries, &styled.properties)?;
        }
        validate_property_updates(GraphicsClass::LineSeries, shared_properties)?;
        validate_property_updates(coordinate_system.graphics_class(), axes_updates)?;
        let (figure, axes, _) =
            self.ensure_axes_for_coordinate_system(requested_axes, coordinate_system)?;
        let replace = self.axes_properties(axes)?.next_plot == NextPlot::Replace;
        if replace {
            self.delete_children(axes)?;
            let properties = self.axes_properties_mut(axes)?;
            properties.title = None;
            properties.x_label = None;
            properties.y_label = None;
            properties.z_label = None;
            properties.color_order_index = 1;
            properties.x_limit_mode = LimitMode::Auto;
            properties.y_limit_mode = LimitMode::Auto;
            properties.z_limit_mode = LimitMode::Auto;
            properties.view_azimuth_degrees = 0.0;
            properties.view_elevation_degrees = 90.0;
        }
        apply_axes_updates(self.axes_properties_mut(axes)?, axes_updates);
        validate_axes_state(self.axes_properties(axes)?)?;
        let mut handles = Vec::with_capacity(series.len());
        for styled in series {
            let line = styled.line;
            let x_data = self.allocate_resource(&line.x)?;
            let y_data = self.allocate_resource(&line.y)?;
            let z_data = line
                .z
                .as_ref()
                .map(|z| self.allocate_resource(z))
                .transpose()?;
            let axes_properties = self.axes_properties(axes)?;
            let color_index = axes_properties.color_order_index;
            let color = axes_properties.color_order
                [(color_index.saturating_sub(1) as usize) % axes_properties.color_order.len()];
            let line_style = axes_properties.line_style_order
                [(color_index.saturating_sub(1) as usize) % axes_properties.line_style_order.len()];
            let mut line_properties = LineSeriesProperties {
                x_data,
                y_data,
                z_data,
                color_rgba: color,
                line_width_points: 0.5,
                line_style,
                marker: Marker::None,
                marker_size_points: 6.0,
                marker_face_color_rgba: [0.0, 0.0, 0.0, 0.0],
                marker_edge_color_rgba: color,
                marker_edge_color_automatic: true,
                marker_indices: default_marker_indices(line.x.len())?,
                marker_indices_class: MarkerIndicesClass::UInt64,
                marker_indices_automatic: true,
                display_name_utf16: Vec::new(),
                visible: true,
                clipping: true,
            };
            apply_line_updates(&mut line_properties, &styled.properties);
            apply_line_updates(&mut line_properties, shared_properties);
            let handle =
                self.allocate_node(Some(axes), GraphicsObject::LineSeries(line_properties))?;
            let color_count = u32::try_from(self.axes_properties(axes)?.color_order.len())
                .map_err(|_| GraphicsError::limit("ColorOrder exceeds model index limits"))?;
            self.axes_properties_mut(axes)?.color_order_index = color_index % color_count + 1;
            handles.push(handle);
        }
        if handles.iter().any(|handle| {
            matches!(
                self.object(*handle),
                Ok(GraphicsObject::LineSeries(properties)) if properties.z_data.is_some()
            )
        }) {
            let properties = self.axes_properties_mut(axes)?;
            properties.view_azimuth_degrees = -37.5;
            properties.view_elevation_degrees = 30.0;
            properties.projection = ProjectionMode::Orthographic;
            properties.camera_scale = 1.0;
        }
        self.resolve_auto_limits(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handles(handles),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn scatter(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        series: &ScatterInput,
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_pair(&series.x, &series.y)?;
        if let Some(z) = &series.z {
            validate_pair(&series.x, z)?;
        }
        if let Some(size_data) = &series.size_data {
            if size_data.len() != 1 && size_data.len() != series.x.len() {
                return Err(GraphicsError::invalid_input(
                    "Scatter SizeData must be scalar or match the point count",
                ));
            }
            let mut invalid = false;
            size_data.visit_f64(|value| invalid |= !value.is_finite() || value < 0.0);
            if invalid {
                return Err(GraphicsError::invalid_input(
                    "Scatter SizeData must contain nonnegative finite values",
                ));
            }
        }
        if let Some(color_data) = &series.color_data
            && color_data.len() != series.x.len()
        {
            return Err(GraphicsError::invalid_input(
                "Scatter scalar CData must match the point count",
            ));
        }
        validate_property_updates(GraphicsClass::ScatterSeries, properties)?;
        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        if self.axes_properties(axes)?.next_plot == NextPlot::Replace {
            self.delete_children(axes)?;
            let properties = self.axes_properties_mut(axes)?;
            properties.title = None;
            properties.x_label = None;
            properties.y_label = None;
            properties.z_label = None;
            properties.color_order_index = 1;
            properties.x_limit_mode = LimitMode::Auto;
            properties.y_limit_mode = LimitMode::Auto;
            properties.z_limit_mode = LimitMode::Auto;
            properties.view_azimuth_degrees = 0.0;
            properties.view_elevation_degrees = 90.0;
        }
        let x_data = self.allocate_resource(&series.x)?;
        let y_data = self.allocate_resource(&series.y)?;
        let z_data = series
            .z
            .as_ref()
            .map(|z| self.allocate_resource(z))
            .transpose()?;
        let size_data = series
            .size_data
            .as_ref()
            .map(|data| self.allocate_resource_shaped(data, [data.len() as u64, 1]))
            .transpose()?;
        let color_data = series
            .color_data
            .as_ref()
            .map(|data| self.allocate_resource_shaped(data, [data.len() as u64, 1]))
            .transpose()?;
        let color_index = self.axes_properties(axes)?.color_order_index;
        let color_order = &self.axes_properties(axes)?.color_order;
        let color = color_order[(color_index.saturating_sub(1) as usize) % color_order.len()];
        let mut scatter_properties = ScatterSeriesProperties {
            x_data,
            y_data,
            z_data,
            size_data,
            color_data,
            color_data_target: if series.color_data.is_none() {
                ScatterColorTarget::None
            } else if series.filled {
                ScatterColorTarget::Face
            } else {
                ScatterColorTarget::Edge
            },
            marker_size_points: 36.0_f32.sqrt(),
            line_width_points: 0.5,
            face_color_rgba: if series.color_data.is_some() && !series.filled {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                color
            },
            edge_color_rgba: if series.color_data.is_some() && series.filled {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                color
            },
            marker: Marker::Circle,
            display_name_utf16: Vec::new(),
            visible: true,
            clipping: true,
        };
        apply_scatter_updates(&mut scatter_properties, properties);
        let handle = self.allocate_node(
            Some(axes),
            GraphicsObject::ScatterSeries(scatter_properties),
        )?;
        let color_count = u32::try_from(self.axes_properties(axes)?.color_order.len())
            .map_err(|_| GraphicsError::limit("ColorOrder exceeds model index limits"))?;
        self.axes_properties_mut(axes)?.color_order_index = color_index % color_count + 1;
        if series.z.is_some() {
            let properties = self.axes_properties_mut(axes)?;
            properties.view_azimuth_degrees = -37.5;
            properties.view_elevation_degrees = 30.0;
            properties.projection = ProjectionMode::Orthographic;
            properties.camera_scale = 1.0;
        }
        self.resolve_auto_limits(axes)?;
        self.resolve_auto_color_limits(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(handle),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn surface(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        surface: &SurfaceInput,
        style: SurfaceStyle,
        updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_surface(surface)?;
        validate_property_updates(GraphicsClass::SurfaceSeries, updates)?;
        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        if self.axes_properties(axes)?.next_plot == NextPlot::Replace {
            self.delete_children(axes)?;
            let properties = self.axes_properties_mut(axes)?;
            properties.title = None;
            properties.x_label = None;
            properties.y_label = None;
            properties.z_label = None;
            properties.color_order_index = 1;
            properties.x_limit_mode = LimitMode::Auto;
            properties.y_limit_mode = LimitMode::Auto;
            properties.z_limit_mode = LimitMode::Auto;
            properties.c_limit_mode = LimitMode::Auto;
        }
        let matrix_shape = [surface.rows, surface.columns];
        let matrix_elements = usize::try_from(surface.rows)
            .ok()
            .and_then(|rows| {
                usize::try_from(surface.columns)
                    .ok()
                    .and_then(|columns| rows.checked_mul(columns))
            })
            .ok_or_else(|| GraphicsError::limit("surface element count overflowed"))?;
        let x_shape = if surface.x.len() == matrix_elements {
            matrix_shape
        } else {
            [1, surface.columns]
        };
        let y_shape = if surface.y.len() == matrix_elements {
            matrix_shape
        } else {
            [surface.rows, 1]
        };
        let x_data = self.allocate_resource_shaped(&surface.x, x_shape)?;
        let y_data = self.allocate_resource_shaped(&surface.y, y_shape)?;
        let z_data = self.allocate_resource_shaped(&surface.z, matrix_shape)?;
        let c_data = self.allocate_resource_shaped(&surface.c, matrix_shape)?;
        // New MATLAB surface objects start with FaceLighting='flat' regardless of
        // an earlier `lighting` command; an existing headlight therefore applies.
        let lighting_enabled = self.axes_properties(axes)?.headlight_enabled;
        let (face_color, edge_color) = match style {
            SurfaceStyle::Surf => (SurfaceColor::Flat, SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0])),
            SurfaceStyle::Mesh => (SurfaceColor::Rgba([1.0, 1.0, 1.0, 1.0]), SurfaceColor::Flat),
        };
        let mut properties = SurfaceSeriesProperties {
            x_data,
            y_data,
            z_data,
            c_data,
            face_color,
            edge_color,
            line_width_points: 0.5,
            line_style: LineStyle::Solid,
            c_data_mapping: CDataMapping::Scaled,
            face_alpha: 1.0,
            lighting_enabled,
            visible: true,
            clipping: true,
        };
        apply_surface_updates(&mut properties, updates);
        let handle = self.allocate_node(Some(axes), GraphicsObject::SurfaceSeries(properties))?;
        {
            let properties = self.axes_properties_mut(axes)?;
            properties.grid_x = true;
            properties.grid_y = true;
            properties.grid_z = true;
            properties.view_azimuth_degrees = -37.5;
            properties.view_elevation_degrees = 30.0;
            properties.projection = ProjectionMode::Orthographic;
            properties.camera_scale = 1.0;
        }
        self.resolve_auto_limits(axes)?;
        self.resolve_auto_color_limits(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(handle),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn patch(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        patch: &PatchInput,
        updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_patch(patch)?;
        validate_property_updates(GraphicsClass::PatchSeries, updates)?;
        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        if self.axes_properties(axes)?.next_plot == NextPlot::Replace {
            self.delete_children(axes)?;
            let properties = self.axes_properties_mut(axes)?;
            properties.title = None;
            properties.x_label = None;
            properties.y_label = None;
            properties.z_label = None;
            properties.color_order_index = 1;
            properties.x_limit_mode = LimitMode::Auto;
            properties.y_limit_mode = LimitMode::Auto;
            properties.z_limit_mode = LimitMode::Auto;
            properties.c_limit_mode = LimitMode::Auto;
        }
        let faces =
            self.allocate_resource_shaped(&patch.faces, [patch.face_rows, patch.face_columns])?;
        let vertices = self
            .allocate_resource_shaped(&patch.vertices, [patch.vertex_rows, patch.vertex_columns])?;
        let face_vertex_cdata = self.allocate_resource_shaped(
            &patch.face_vertex_cdata,
            [patch.cdata_rows, patch.cdata_columns],
        )?;
        let lighting_enabled = self.axes_properties(axes)?.headlight_enabled;
        let mut properties = PatchSeriesProperties {
            faces,
            vertices,
            face_vertex_cdata,
            vertex_normals: None,
            face_color: patch.face_color,
            edge_color: patch.edge_color,
            line_width_points: 0.5,
            line_style: LineStyle::Solid,
            c_data_mapping: CDataMapping::Scaled,
            face_alpha: 1.0,
            edge_alpha: 1.0,
            lighting_enabled,
            smooth_normals: false,
            visible: true,
            clipping: true,
        };
        apply_patch_updates(&mut properties, updates);
        let handle = self.allocate_node(Some(axes), GraphicsObject::PatchSeries(properties))?;
        if patch.vertex_columns == 3 {
            let properties = self.axes_properties_mut(axes)?;
            properties.view_azimuth_degrees = -37.5;
            properties.view_elevation_degrees = 30.0;
            properties.projection = ProjectionMode::Orthographic;
            properties.camera_scale = 1.0;
        }
        self.resolve_auto_limits(axes)?;
        self.resolve_auto_color_limits(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(handle),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn chart_group(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        chart: &ChartGroupInput,
    ) -> Result<AppliedRequest, GraphicsError> {
        if chart.primitives.is_empty() {
            return Err(GraphicsError::invalid_input(
                "a high-level chart must contain at least one retained primitive",
            ));
        }
        validate_property_updates(chart.kind.graphics_class(), &chart.properties)?;
        validate_property_updates(GraphicsClass::Axes2D, &chart.axes_properties)?;
        for primitive in &chart.primitives {
            match primitive {
                ChartPrimitiveInput::Line {
                    line, properties, ..
                } => {
                    validate_pair(&line.x, &line.y)?;
                    if let Some(z) = &line.z {
                        validate_pair(&line.x, z)?;
                    }
                    validate_property_updates(GraphicsClass::LineSeries, properties)?;
                }
                ChartPrimitiveInput::Patch {
                    patch, properties, ..
                } => {
                    validate_patch(patch)?;
                    validate_property_updates(GraphicsClass::PatchSeries, properties)?;
                }
            }
        }

        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        if self.axes_properties(axes)?.next_plot == NextPlot::Replace {
            self.delete_children(axes)?;
            let properties = self.axes_properties_mut(axes)?;
            properties.title = None;
            properties.x_label = None;
            properties.y_label = None;
            properties.z_label = None;
            properties.color_order_index = 1;
            properties.x_limit_mode = LimitMode::Auto;
            properties.y_limit_mode = LimitMode::Auto;
            properties.z_limit_mode = LimitMode::Auto;
            properties.c_limit_mode = LimitMode::Auto;
            properties.view_azimuth_degrees = 0.0;
            properties.view_elevation_degrees = 90.0;
        }
        apply_axes_updates(self.axes_properties_mut(axes)?, &chart.axes_properties);
        validate_axes_state(self.axes_properties(axes)?)?;

        let color_index = self.axes_properties(axes)?.color_order_index;
        let group = self.allocate_node(
            Some(axes),
            GraphicsObject::ChartGroup(default_chart_group_properties(chart.kind)),
        )?;
        let mut automatic_color_count = 0_u32;
        for primitive in &chart.primitives {
            match primitive {
                ChartPrimitiveInput::Line {
                    line,
                    properties,
                    automatic_color,
                } => {
                    let x_data = self.allocate_resource(&line.x)?;
                    let y_data = self.allocate_resource(&line.y)?;
                    let z_data = line
                        .z
                        .as_ref()
                        .map(|z| self.allocate_resource(z))
                        .transpose()?;
                    let primitive_color_index = color_index.saturating_add(automatic_color_count);
                    let (color, line_style) = {
                        let axes_properties = self.axes_properties(axes)?;
                        let offset = primitive_color_index.saturating_sub(1) as usize;
                        (
                            axes_properties.color_order[offset % axes_properties.color_order.len()],
                            axes_properties.line_style_order
                                [offset % axes_properties.line_style_order.len()],
                        )
                    };
                    let color_rgba = if *automatic_color {
                        automatic_color_count = automatic_color_count.saturating_add(1);
                        color
                    } else {
                        [0.0, 0.0, 0.0, 1.0]
                    };
                    let mut line_properties = LineSeriesProperties {
                        x_data,
                        y_data,
                        z_data,
                        color_rgba,
                        line_width_points: 0.5,
                        line_style,
                        marker: Marker::None,
                        marker_size_points: 6.0,
                        marker_face_color_rgba: [0.0, 0.0, 0.0, 0.0],
                        marker_edge_color_rgba: color_rgba,
                        marker_edge_color_automatic: true,
                        marker_indices: default_marker_indices(line.x.len())?,
                        marker_indices_class: MarkerIndicesClass::UInt64,
                        marker_indices_automatic: true,
                        display_name_utf16: Vec::new(),
                        visible: true,
                        clipping: true,
                    };
                    apply_line_updates(&mut line_properties, properties);
                    self.allocate_node(Some(group), GraphicsObject::LineSeries(line_properties))?;
                }
                ChartPrimitiveInput::Patch {
                    patch,
                    properties,
                    automatic_color,
                } => {
                    let faces = self.allocate_resource_shaped(
                        &patch.faces,
                        [patch.face_rows, patch.face_columns],
                    )?;
                    let vertices = self.allocate_resource_shaped(
                        &patch.vertices,
                        [patch.vertex_rows, patch.vertex_columns],
                    )?;
                    let face_vertex_cdata = self.allocate_resource_shaped(
                        &patch.face_vertex_cdata,
                        [patch.cdata_rows, patch.cdata_columns],
                    )?;
                    let lighting_enabled = self.axes_properties(axes)?.headlight_enabled;
                    let mut patch_properties = PatchSeriesProperties {
                        faces,
                        vertices,
                        face_vertex_cdata,
                        vertex_normals: None,
                        face_color: if *automatic_color {
                            let offset = color_index
                                .saturating_add(automatic_color_count)
                                .saturating_sub(1)
                                as usize;
                            let color = {
                                let axes_properties = self.axes_properties(axes)?;
                                axes_properties.color_order
                                    [offset % axes_properties.color_order.len()]
                            };
                            automatic_color_count = automatic_color_count.saturating_add(1);
                            SurfaceColor::Rgba(color)
                        } else {
                            patch.face_color
                        },
                        edge_color: patch.edge_color,
                        line_width_points: 0.5,
                        line_style: LineStyle::Solid,
                        c_data_mapping: CDataMapping::Scaled,
                        face_alpha: 1.0,
                        edge_alpha: 1.0,
                        lighting_enabled,
                        smooth_normals: false,
                        visible: true,
                        clipping: true,
                    };
                    apply_patch_updates(&mut patch_properties, properties);
                    self.allocate_node(Some(group), GraphicsObject::PatchSeries(patch_properties))?;
                }
            }
        }
        self.sync_chart_group_style(group)?;
        if let GraphicsObject::ChartGroup(properties) = &mut self.node_mut(group)?.object {
            apply_chart_group_updates(properties, &chart.properties);
        }
        if automatic_color_count > 0 {
            let color_count = u32::try_from(self.axes_properties(axes)?.color_order.len())
                .map_err(|_| GraphicsError::limit("ColorOrder exceeds model index limits"))?;
            self.axes_properties_mut(axes)?.color_order_index =
                (color_index - 1 + automatic_color_count) % color_count + 1;
        }
        self.resolve_auto_limits(axes)?;
        self.resolve_auto_color_limits(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(group),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn sync_chart_group_style(&mut self, group: GraphicsHandle) -> Result<(), GraphicsError> {
        let first_child = self
            .children(group)?
            .first()
            .copied()
            .ok_or_else(|| GraphicsError::invalid_state("chart has no retained primitive"))?;
        let first = self.object(first_child)?.clone();
        let GraphicsObject::ChartGroup(properties) = &mut self.node_mut(group)?.object else {
            return Err(GraphicsError::invalid_state(
                "chart handle lost its payload",
            ));
        };
        match first {
            GraphicsObject::LineSeries(line) => {
                properties.color = Some(ChartColor::Rgba(line.color_rgba));
                properties.line_width_points = line.line_width_points;
                properties.line_style = line.line_style;
                properties.marker = Some(line.marker);
                properties.marker_size_points = Some(line.marker_size_points);
                properties.marker_face_color = Some(if line.marker_face_color_rgba[3] == 0.0 {
                    ChartColor::None
                } else {
                    ChartColor::Rgba(line.marker_face_color_rgba)
                });
                properties.marker_edge_color = Some(if line.marker_edge_color_automatic {
                    ChartColor::Auto
                } else if line.marker_edge_color_rgba[3] == 0.0 {
                    ChartColor::None
                } else {
                    ChartColor::Rgba(line.marker_edge_color_rgba)
                });
            }
            GraphicsObject::PatchSeries(patch) => {
                properties.line_width_points = patch.line_width_points;
                properties.line_style = patch.line_style;
                properties.automatic_face_color_rgba = match patch.face_color {
                    SurfaceColor::Rgba(color) => Some(color),
                    _ => None,
                };
                properties.automatic_edge_color_rgba = match patch.edge_color {
                    SurfaceColor::Rgba(color) => Some(color),
                    _ => None,
                };
                if properties.kind != ChartGroupKind::Histogram {
                    properties.face_color = Some(chart_color_from_surface(patch.face_color));
                }
                properties.edge_color = Some(chart_color_from_surface(patch.edge_color));
                properties.face_alpha = Some(patch.face_alpha);
                properties.edge_alpha = Some(patch.edge_alpha);
            }
            _ => {
                return Err(GraphicsError::invalid_state(
                    "chart retained an unsupported primitive payload",
                ));
            }
        }
        Ok(())
    }

    fn set_view(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        azimuth_degrees: f64,
        elevation_degrees: f64,
    ) -> Result<AppliedRequest, GraphicsError> {
        if !azimuth_degrees.is_finite() || !elevation_degrees.is_finite() {
            return Err(GraphicsError::invalid_input(
                "view angles must be finite real scalars",
            ));
        }
        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        let properties = self.axes_properties_mut(axes)?;
        properties.view_azimuth_degrees = azimuth_degrees;
        properties.view_elevation_degrees = elevation_degrees.clamp(-90.0, 90.0);
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn shading(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        mode: ShadingMode,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        let surfaces = self
            .children(axes)?
            .iter()
            .copied()
            .filter(|handle| {
                matches!(
                    handle.class(),
                    GraphicsClass::SurfaceSeries | GraphicsClass::PatchSeries
                )
            })
            .collect::<Vec<_>>();
        let mut changed = created;
        for handle in surfaces {
            let (face_color, edge_color) = match mode {
                ShadingMode::Flat => (SurfaceColor::Flat, SurfaceColor::None),
                ShadingMode::Interp => (SurfaceColor::Interp, SurfaceColor::None),
                ShadingMode::Faceted => {
                    (SurfaceColor::Flat, SurfaceColor::Rgba([0.0, 0.0, 0.0, 1.0]))
                }
            };
            match &mut self.node_mut(handle)?.object {
                GraphicsObject::SurfaceSeries(surface) => {
                    let before = (surface.face_color, surface.edge_color);
                    (surface.face_color, surface.edge_color) = (face_color, edge_color);
                    changed |= before != (surface.face_color, surface.edge_color);
                }
                GraphicsObject::PatchSeries(patch) => {
                    if mode == ShadingMode::Interp
                        && patch.face_vertex_cdata.shape[0] != patch.vertices.shape[0]
                    {
                        return Err(GraphicsError::invalid_input(
                            "interpolated Patch shading requires one FaceVertexCData value per vertex",
                        ));
                    }
                    let before = (patch.face_color, patch.edge_color);
                    (patch.face_color, patch.edge_color) = (face_color, edge_color);
                    changed |= before != (patch.face_color, patch.edge_color);
                }
                _ => {
                    return Err(GraphicsError::invalid_state(
                        "Surface or Patch handle has wrong object class",
                    ));
                }
            }
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: changed.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_headlight(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        enabled: bool,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        let changed = self.axes_properties(axes)?.headlight_enabled != enabled;
        self.axes_properties_mut(axes)?.headlight_enabled = enabled;
        let descendants_changed = self.refresh_axes_lighting(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: (created || changed || descendants_changed).then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_lighting(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        mode: LightingMode,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        let changed = self.axes_properties(axes)?.lighting_mode != mode;
        self.axes_properties_mut(axes)?.lighting_mode = mode;
        let descendants_changed = self.refresh_axes_lighting(axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: (created || changed || descendants_changed).then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn refresh_axes_lighting(&mut self, axes: GraphicsHandle) -> Result<bool, GraphicsError> {
        let enabled = {
            let properties = self.axes_properties(axes)?;
            properties.headlight_enabled && properties.lighting_mode != LightingMode::None
        };
        let mut pending = self.children(axes)?.to_vec();
        let mut changed = false;
        while let Some(handle) = pending.pop() {
            pending.extend_from_slice(self.children(handle)?);
            match &mut self.node_mut(handle)?.object {
                GraphicsObject::SurfaceSeries(properties) => {
                    changed |= properties.lighting_enabled != enabled;
                    properties.lighting_enabled = enabled;
                }
                GraphicsObject::PatchSeries(properties) => {
                    changed |= properties.lighting_enabled != enabled;
                    properties.lighting_enabled = enabled;
                }
                _ => {}
            }
        }
        Ok(changed)
    }

    fn set_patch_smooth_normals(
        &mut self,
        patch: GraphicsHandle,
    ) -> Result<AppliedRequest, GraphicsError> {
        if patch.class() != GraphicsClass::PatchSeries {
            return Err(GraphicsError::invalid_handle(
                "isonormals target must be a Patch handle",
            ));
        }
        let figure = self.figure_for_handle(patch)?;
        let GraphicsObject::PatchSeries(properties) = &mut self.node_mut(patch)?.object else {
            return Err(GraphicsError::invalid_state(
                "Patch handle has the wrong object payload",
            ));
        };
        let changed = !properties.smooth_normals;
        properties.smooth_normals = true;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: changed.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_patch_iso_normals(
        &mut self,
        patch: GraphicsHandle,
        volume: &IsoNormalsInput,
    ) -> Result<AppliedRequest, GraphicsError> {
        if patch.class() != GraphicsClass::PatchSeries {
            return Err(GraphicsError::invalid_handle(
                "isonormals target must be a Patch handle",
            ));
        }
        let figure = self.figure_for_handle(patch)?;
        let vertices_ref = {
            let GraphicsObject::PatchSeries(properties) = &self.node(patch)?.object else {
                return Err(GraphicsError::invalid_state(
                    "Patch handle has the wrong object payload",
                ));
            };
            if properties.vertices.shape[1] != 3 {
                return Err(GraphicsError::invalid_input(
                    "isonormals requires a Patch with N-by-3 Vertices",
                ));
            }
            properties.vertices.clone()
        };
        let vertices_resource = self.resources.get(&vertices_ref.id).ok_or_else(|| {
            GraphicsError::invalid_state("Patch Vertices resource is unavailable")
        })?;
        let vertices = crate::isonormals::decode_resource_f64(vertices_resource)?;
        let normals =
            crate::isonormals::compute_vertex_normals(volume, &vertices, vertices_ref.shape[0])?;
        let normals_ref = self.allocate_resource_shaped(&normals, [vertices_ref.shape[0], 3])?;
        let GraphicsObject::PatchSeries(properties) = &mut self.node_mut(patch)?.object else {
            return Err(GraphicsError::invalid_state(
                "Patch handle has the wrong object payload",
            ));
        };
        properties.vertex_normals = Some(normals_ref);
        properties.smooth_normals = true;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn colorbar(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        visible: bool,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        let existing = self
            .children(axes)?
            .iter()
            .copied()
            .find(|handle| handle.class() == GraphicsClass::ColorBar);
        if let Some(handle) = existing {
            self.delete_subtree(handle)?;
        }
        self.axes_properties_mut(axes)?.colorbar_visible = visible;
        let response = if visible {
            let properties = default_colorbar_properties(self.axes_properties(axes)?.c_limits);
            GraphicsResponse::Handle(
                self.allocate_node(Some(axes), GraphicsObject::ColorBar(properties))?,
            )
        } else {
            GraphicsResponse::None
        };
        Ok(AppliedRequest {
            response,
            change: (created || visible || existing.is_some()).then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_colormap(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        colors: Vec<[f64; 3]>,
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_colormap(&colors)?;
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        let changed = self.axes_properties(axes)?.colormap != colors;
        if changed {
            self.axes_properties_mut(axes)?.colormap.clone_from(&colors);
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::Colormap(colors),
            change: (created || changed).then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn get_colormap(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, created) = self.ensure_any_axes(requested_axes)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Colormap(self.axes_properties(axes)?.colormap.clone()),
            change: created.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_axes_camera(
        &mut self,
        figure: GraphicsHandle,
        requested_axes: Option<GraphicsHandle>,
        azimuth_degrees: f64,
        elevation_degrees: f64,
        camera_scale: f64,
    ) -> Result<AppliedRequest, GraphicsError> {
        if !azimuth_degrees.is_finite()
            || !elevation_degrees.is_finite()
            || !(-90.0..=90.0).contains(&elevation_degrees)
            || !camera_scale.is_finite()
            || !(1.0 / 45.0..=170.0 / 45.0).contains(&camera_scale)
        {
            return Err(GraphicsError::invalid_input(
                "camera view and scale must be finite and representable",
            ));
        }
        self.node_of_class(figure, GraphicsClass::Figure)?;
        let axes = self.axes_for_figure(figure, requested_axes)?;
        let properties = self.axes_properties_mut(axes)?;
        properties.view_azimuth_degrees = azimuth_degrees;
        properties.view_elevation_degrees = elevation_degrees;
        properties.camera_scale = camera_scale;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_hold(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        enabled: bool,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes) = if let Some(axes) = requested_axes {
            let figure = self
                .node_of_axes(axes)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            (figure, axes)
        } else {
            let (figure, axes, _) = self.ensure_current_axes()?;
            (figure, axes)
        };
        self.axes_properties_mut(axes)?.next_plot = if enabled {
            NextPlot::Add
        } else {
            NextPlot::Replace
        };
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn is_hold(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        if let Some(axes) = requested_axes {
            let held = self.axes_properties(axes)?.next_plot == NextPlot::Add;
            return Ok(AppliedRequest {
                response: GraphicsResponse::Logical(held),
                change: None,
            });
        }
        let (figure, figure_created) = self.ensure_current_figure()?;
        let axes = self
            .current_axes
            .filter(|handle| self.is_valid(*handle))
            .or_else(|| self.first_axes(figure));
        let held = axes
            .map(|axes| self.axes_properties(axes))
            .transpose()?
            .is_some_and(|properties| properties.next_plot == NextPlot::Add);
        Ok(AppliedRequest {
            response: GraphicsResponse::Logical(held),
            change: figure_created.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn get_children(&self, parent: GraphicsHandle) -> Result<AppliedRequest, GraphicsError> {
        Ok(AppliedRequest {
            response: GraphicsResponse::Handles(self.children(parent)?.to_vec()),
            change: None,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn set_properties(
        &mut self,
        handles: &[GraphicsHandle],
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        let Some((&first, remaining)) = handles.split_first() else {
            return Ok(AppliedRequest {
                response: GraphicsResponse::None,
                change: None,
            });
        };
        let class = first.class();
        validate_property_updates(class, properties)?;
        let figure = self.figure_for_handle(first)?;
        for handle in remaining {
            if handle.class() != class {
                return Err(GraphicsError::invalid_handle(format!(
                    "graphics property update requires homogeneous handles, found {} and {}",
                    class.class_name(),
                    handle.class().class_name()
                )));
            }
            let target_figure = self.figure_for_handle(*handle)?;
            if target_figure != figure {
                return Err(GraphicsError::invalid_input(
                    "one graphics property transaction cannot span multiple Figures",
                ));
            }
        }
        if is_chart_group_class(class) {
            return self.set_chart_properties(handles, properties, figure);
        }

        let mut changed = false;
        let mut axes_to_resolve = BTreeSet::new();
        let mut colorbars_to_resolve = BTreeSet::new();
        for handle in handles {
            let replacement_x = properties.iter().rev().find_map(|update| match update {
                GraphicsPropertyUpdate::XData(data) => Some(data),
                _ => None,
            });
            let replacement_y = properties.iter().rev().find_map(|update| match update {
                GraphicsPropertyUpdate::YData(data) => Some(data),
                _ => None,
            });
            let replacement_z = properties.iter().rev().find_map(|update| match update {
                GraphicsPropertyUpdate::ZData(data) => Some(data),
                _ => None,
            });
            let existing_lengths = match &self.node(*handle)?.object {
                GraphicsObject::LineSeries(series) => (
                    series.x_data.shape[1],
                    series.y_data.shape[1],
                    series.z_data.as_ref().map(|data| data.shape[1]),
                    series.marker_indices_automatic,
                ),
                GraphicsObject::ScatterSeries(series) => (
                    series.x_data.shape[1],
                    series.y_data.shape[1],
                    series.z_data.as_ref().map(|data| data.shape[1]),
                    false,
                ),
                _ => (0, 0, None, false),
            };
            let x_len = replacement_x.map_or(existing_lengths.0, |data| data.len() as u64);
            let y_len = replacement_y.map_or(existing_lengths.1, |data| data.len() as u64);
            let z_len = replacement_z
                .map(|data| data.len() as u64)
                .or(existing_lengths.2);
            if x_len != y_len || z_len.is_some_and(|z_len| z_len != x_len) {
                return Err(GraphicsError::invalid_input(
                    "series XData, YData, and optional ZData must have equal lengths",
                ));
            }
            let x_data = replacement_x
                .map(|data| self.allocate_resource(data))
                .transpose()?;
            let y_data = replacement_y
                .map(|data| self.allocate_resource(data))
                .transpose()?;
            let z_data = replacement_z
                .map(|data| self.allocate_resource(data))
                .transpose()?;
            let refresh_default_indices = (replacement_x.is_some() || replacement_y.is_some())
                && existing_lengths.3
                && !properties
                    .iter()
                    .any(|update| matches!(update, GraphicsPropertyUpdate::MarkerIndices { .. }));
            let default_indices_update = refresh_default_indices
                .then(|| usize::try_from(x_len))
                .transpose()
                .map_err(|_| GraphicsError::limit("Line data length exceeds host limits"))?
                .map(default_marker_indices)
                .transpose()?;
            let node = self.node_mut(*handle)?;
            match &mut node.object {
                GraphicsObject::LineSeries(line) => {
                    let before = line.clone();
                    apply_line_updates(line, properties);
                    if let Some(data) = x_data {
                        line.x_data = data;
                    }
                    if let Some(data) = y_data {
                        line.y_data = data;
                    }
                    if let Some(data) = z_data {
                        line.z_data = Some(data);
                    }
                    if let Some(indices) = default_indices_update {
                        line.marker_indices = indices;
                    }
                    changed |= *line != before;
                    axes_to_resolve.insert(
                        node.parent.ok_or_else(|| {
                            GraphicsError::invalid_state("Line has no Axes parent")
                        })?,
                    );
                }
                GraphicsObject::ScatterSeries(scatter) => {
                    let before = scatter.clone();
                    apply_scatter_updates(scatter, properties);
                    if let Some(data) = x_data {
                        scatter.x_data = data;
                    }
                    if let Some(data) = y_data {
                        scatter.y_data = data;
                    }
                    if let Some(data) = z_data {
                        scatter.z_data = Some(data);
                    }
                    changed |= *scatter != before;
                    axes_to_resolve.insert(node.parent.ok_or_else(|| {
                        GraphicsError::invalid_state("Scatter has no Axes parent")
                    })?);
                }
                GraphicsObject::SurfaceSeries(surface) => {
                    let before = surface.clone();
                    apply_surface_updates(surface, properties);
                    changed |= *surface != before;
                }
                GraphicsObject::PatchSeries(patch) => {
                    let before = patch.clone();
                    apply_patch_updates(patch, properties);
                    changed |= *patch != before;
                    axes_to_resolve.insert(
                        node.parent.ok_or_else(|| {
                            GraphicsError::invalid_state("Patch has no Axes parent")
                        })?,
                    );
                }
                GraphicsObject::Axes2D(axes) => {
                    let before = axes.clone();
                    apply_axes_updates(axes, properties);
                    validate_axes_state(axes)?;
                    changed |= *axes != before;
                    axes_to_resolve.insert(*handle);
                }
                GraphicsObject::Text(text) => {
                    let before = text.clone();
                    apply_text_updates(text, properties);
                    changed |= *text != before;
                }
                GraphicsObject::Legend(legend) => {
                    let before = legend.clone();
                    apply_legend_updates(legend, properties);
                    changed |= *legend != before;
                }
                GraphicsObject::ColorBar(colorbar) => {
                    let before = colorbar.clone();
                    apply_colorbar_updates(colorbar, properties);
                    changed |= *colorbar != before;
                    colorbars_to_resolve.insert(node.parent.ok_or_else(|| {
                        GraphicsError::invalid_state("ColorBar has no Axes parent")
                    })?);
                }
                GraphicsObject::ChartGroup(chart) => {
                    let before = chart.clone();
                    for update in properties {
                        if let GraphicsPropertyUpdate::Visible(value) = update {
                            chart.visible = *value;
                        }
                    }
                    changed |= *chart != before;
                    axes_to_resolve.insert(
                        node.parent.ok_or_else(|| {
                            GraphicsError::invalid_state("chart has no Axes parent")
                        })?,
                    );
                }
                GraphicsObject::Figure(figure) => {
                    let before = figure.clone();
                    for update in properties {
                        match update {
                            GraphicsPropertyUpdate::Name(value) => {
                                figure.name_utf16.clone_from(value);
                            }
                            GraphicsPropertyUpdate::NumberTitle(value) => {
                                figure.number_title = *value;
                            }
                            GraphicsPropertyUpdate::Position(value) => {
                                figure.position_css_pixels = *value;
                            }
                            GraphicsPropertyUpdate::Color(value) => {
                                figure.background_rgba = *value;
                            }
                            GraphicsPropertyUpdate::Visible(value) => figure.visible = *value,
                            _ => {}
                        }
                    }
                    changed |= *figure != before;
                }
                GraphicsObject::TiledChartLayout(_) => {}
            }
        }
        for axes in axes_to_resolve {
            self.resolve_auto_limits(axes)?;
            self.resolve_auto_color_limits(axes)?;
        }
        for axes in colorbars_to_resolve {
            self.refresh_auto_colorbar_ticks(axes)?;
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: changed.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_chart_properties(
        &mut self,
        handles: &[GraphicsHandle],
        updates: &[GraphicsPropertyUpdate],
        figure: GraphicsHandle,
    ) -> Result<AppliedRequest, GraphicsError> {
        let mut changed = false;
        let mut axes_to_resolve = BTreeSet::new();
        for handle in handles {
            let before_group = match self.object(*handle)? {
                GraphicsObject::ChartGroup(properties) => properties.clone(),
                _ => {
                    return Err(GraphicsError::invalid_state(
                        "chart handle lost its payload",
                    ));
                }
            };
            let resource_updates =
                self.prepare_chart_geometry_updates(*handle, &before_group, updates)?;
            let children = self.children(*handle)?.to_vec();

            if let GraphicsObject::ChartGroup(properties) = &mut self.node_mut(*handle)?.object {
                apply_chart_group_updates(properties, updates);
                changed |= *properties != before_group;
            }
            for child in children {
                let resource_update = resource_updates.iter().find(|item| item.handle == child);
                let node = self.node_mut(child)?;
                match &mut node.object {
                    GraphicsObject::LineSeries(line) => {
                        let before = line.clone();
                        apply_line_updates(line, updates);
                        apply_chart_line_color_updates(line, updates);
                        let color_changed = updates
                            .iter()
                            .any(|update| matches!(update, GraphicsPropertyUpdate::Color(_)));
                        let marker_face_changed = updates.iter().any(|update| {
                            matches!(
                                update,
                                GraphicsPropertyUpdate::MarkerFaceColor(_)
                                    | GraphicsPropertyUpdate::ChartMarkerFaceColor(_)
                            )
                        });
                        if color_changed
                            && !marker_face_changed
                            && before_group.marker_face_color == Some(ChartColor::Auto)
                        {
                            line.marker_face_color_rgba = line.color_rgba;
                        }
                        if let Some(resource_update) = resource_update {
                            if let Some(data) = &resource_update.x_data {
                                line.x_data = data.clone();
                            }
                            if let Some(data) = &resource_update.y_data {
                                line.y_data = data.clone();
                            }
                        }
                        changed |= *line != before;
                    }
                    GraphicsObject::PatchSeries(patch) => {
                        let before = patch.clone();
                        apply_patch_updates(patch, updates);
                        apply_chart_patch_color_updates(patch, &before_group, updates);
                        if let Some(resource_update) = resource_update
                            && let Some(data) = &resource_update.vertices
                        {
                            patch.vertices = data.clone();
                        }
                        changed |= *patch != before;
                    }
                    _ => {
                        return Err(GraphicsError::invalid_state(
                            "chart retained an unsupported primitive payload",
                        ));
                    }
                }
            }
            axes_to_resolve.insert(
                self.node(*handle)?
                    .parent
                    .ok_or_else(|| GraphicsError::invalid_state("chart has no Axes parent"))?,
            );
        }
        for axes in axes_to_resolve {
            self.resolve_auto_limits(axes)?;
            self.resolve_auto_color_limits(axes)?;
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: changed.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    #[allow(clippy::too_many_lines)] // Geometry variants stay together for one transactional preflight.
    fn prepare_chart_geometry_updates(
        &mut self,
        handle: GraphicsHandle,
        current: &ChartGroupProperties,
        updates: &[GraphicsPropertyUpdate],
    ) -> Result<Vec<ChartPrimitiveResourceUpdate>, GraphicsError> {
        let next_base = updates.iter().rev().find_map(|update| match update {
            GraphicsPropertyUpdate::BaseValue(value) => Some(*value),
            _ => None,
        });
        let next_bar_width = updates.iter().rev().find_map(|update| match update {
            GraphicsPropertyUpdate::BarWidth(value) => Some(*value),
            _ => None,
        });
        let next_cap_size = updates.iter().rev().find_map(|update| match update {
            GraphicsPropertyUpdate::CapSize(value) => Some(*value),
            _ => None,
        });
        let base_changed = next_base.is_some_and(|value| Some(value) != current.base_value);
        let bar_width_changed =
            next_bar_width.is_some_and(|value| Some(value) != current.bar_width);
        let cap_size_changed =
            next_cap_size.is_some_and(|value| Some(value) != current.cap_size_points);
        if !base_changed && !bar_width_changed && !cap_size_changed {
            return Ok(Vec::new());
        }

        let mut prepared = Vec::new();
        for child in self.children(handle)?.to_vec() {
            let object = self.object(child)?.clone();
            let mut prepared_update = ChartPrimitiveResourceUpdate {
                handle: child,
                x_data: None,
                y_data: None,
                vertices: None,
            };
            match (current.kind, object) {
                (ChartGroupKind::Stem, GraphicsObject::LineSeries(line)) => {
                    if let Some(base) = next_base.filter(|_| base_changed) {
                        let mut values = self.data_values_f64(&line.y_data)?;
                        for stem in values.chunks_exact_mut(3) {
                            stem[0] = base;
                        }
                        prepared_update.y_data = Some(self.allocate_resource_shaped(
                            &numeric_like(line.y_data.dtype, values),
                            line.y_data.shape,
                        )?);
                    }
                }
                (ChartGroupKind::ErrorBar, GraphicsObject::LineSeries(line)) => {
                    if let Some(cap_size) = next_cap_size.filter(|_| cap_size_changed) {
                        let mut values = self.data_values_f64(&line.x_data)?;
                        if !values.is_empty() && (values.len() - 1).is_multiple_of(10) {
                            let source_count = (values.len() - 1) / 10;
                            let source = values[..source_count].to_vec();
                            let span = finite_data_span(&source).unwrap_or(1.0);
                            let half_width = span * cap_size / 500.0;
                            for (index, center) in source.into_iter().enumerate() {
                                let offset = source_count + 1 + index * 9;
                                values[offset + 3] = center - half_width;
                                values[offset + 4] = center + half_width;
                                values[offset + 6] = center - half_width;
                                values[offset + 7] = center + half_width;
                            }
                            prepared_update.x_data = Some(self.allocate_resource_shaped(
                                &numeric_like(line.x_data.dtype, values),
                                line.x_data.shape,
                            )?);
                        }
                    }
                }
                (
                    ChartGroupKind::Area | ChartGroupKind::Bar,
                    GraphicsObject::PatchSeries(patch),
                ) => {
                    let mut values = self.data_values_f64(&patch.vertices)?;
                    let rows = usize::try_from(patch.vertices.shape[0]).map_err(|_| {
                        GraphicsError::limit("chart vertex row count exceeds host limits")
                    })?;
                    if let Some(base) = next_base.filter(|_| base_changed) {
                        if current.kind == ChartGroupKind::Area {
                            if rows >= 2 && values.len() >= rows * 2 {
                                values[rows * 2 - 2] = base;
                                values[rows * 2 - 1] = base;
                            }
                        } else if rows.is_multiple_of(4) && values.len() >= rows * 2 {
                            for rectangle in 0..(rows / 4) {
                                let y_offset = rows + rectangle * 4;
                                values[y_offset] = base;
                                values[y_offset + 1] = base;
                            }
                        }
                    }
                    if current.kind == ChartGroupKind::Bar
                        && bar_width_changed
                        && let (Some(width), Some(old_width)) = (next_bar_width, current.bar_width)
                        && rows.is_multiple_of(4)
                    {
                        let factor = width / old_width;
                        for rectangle in 0..(rows / 4) {
                            let offset = rectangle * 4;
                            let center = f64::midpoint(values[offset], values[offset + 1]);
                            for x in &mut values[offset..offset + 4] {
                                *x = center + (*x - center) * factor;
                            }
                        }
                    }
                    if base_changed || bar_width_changed {
                        prepared_update.vertices = Some(self.allocate_resource_shaped(
                            &numeric_like(patch.vertices.dtype, values),
                            patch.vertices.shape,
                        )?);
                    }
                }
                _ => {}
            }
            if prepared_update.x_data.is_some()
                || prepared_update.y_data.is_some()
                || prepared_update.vertices.is_some()
            {
                prepared.push(prepared_update);
            }
        }
        Ok(prepared)
    }

    fn data_values_f64(&self, data: &DataRef) -> Result<Vec<f64>, GraphicsError> {
        let resource = self
            .resources
            .get(&data.id)
            .ok_or_else(|| GraphicsError::invalid_state("chart data resource is missing"))?;
        let bytes = resource.bytes();
        match data.dtype {
            DataDType::F32 => Ok(bytes
                .chunks_exact(4)
                .map(|chunk| f64::from(f32::from_le_bytes(chunk.try_into().expect("four bytes"))))
                .collect()),
            DataDType::F64 => Ok(bytes
                .chunks_exact(8)
                .map(|chunk| f64::from_le_bytes(chunk.try_into().expect("eight bytes")))
                .collect()),
        }
    }

    fn get_properties(
        &self,
        handles: &[GraphicsHandle],
        property: GraphicsProperty,
    ) -> Result<AppliedRequest, GraphicsError> {
        let Some((&first, remaining)) = handles.split_first() else {
            return Ok(AppliedRequest {
                response: GraphicsResponse::PropertyValues(Vec::new()),
                change: None,
            });
        };
        let class = first.class();
        validate_property_for_class(class, property)?;
        self.figure_for_handle(first)?;
        for handle in remaining {
            if handle.class() != class {
                return Err(GraphicsError::invalid_handle(format!(
                    "graphics property query requires homogeneous handles, found {} and {}",
                    class.class_name(),
                    handle.class().class_name()
                )));
            }
            self.figure_for_handle(*handle)?;
        }
        let values = handles
            .iter()
            .map(|handle| property_value(self, *handle, property))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AppliedRequest {
            response: GraphicsResponse::PropertyValues(values),
            change: None,
        })
    }

    fn set_axes_properties(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        properties: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_updates(GraphicsClass::Axes2D, properties)?;
        let (figure, axes, created) = self.ensure_axes(requested_axes)?;
        let before = self.axes_properties(axes)?.clone();
        apply_axes_updates(self.axes_properties_mut(axes)?, properties);
        validate_axes_state(self.axes_properties(axes)?)?;
        self.resolve_auto_limits(axes)?;
        self.resolve_auto_color_limits(axes)?;
        let changed = *self.axes_properties(axes)? != before;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: (created || changed).then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn get_axes_property(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        property: GraphicsProperty,
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_for_class(GraphicsClass::Axes2D, property)?;
        let (figure, axes, created) = self.ensure_axes(requested_axes)?;
        let value = property_value(self, axes, property)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::PropertyValues(vec![value]),
            change: created.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn toggle_axes_box(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, _) = self.ensure_any_axes(requested_axes)?;
        let properties = self.axes_properties_mut(axes)?;
        properties.box_enabled = !properties.box_enabled;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn figure_for_handle(&self, handle: GraphicsHandle) -> Result<GraphicsHandle, GraphicsError> {
        self.node(handle)?;
        match handle.class() {
            GraphicsClass::Figure => Ok(handle),
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes => self
                .node(handle)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent")),
            GraphicsClass::TiledChartLayout => {
                let GraphicsObject::TiledChartLayout(properties) = self.object(handle)? else {
                    return Err(GraphicsError::invalid_state(
                        "TiledChartLayout payload has wrong class",
                    ));
                };
                Ok(properties.parent)
            }
            GraphicsClass::LineSeries
            | GraphicsClass::ScatterSeries
            | GraphicsClass::SurfaceSeries
            | GraphicsClass::PatchSeries
            | GraphicsClass::Stair
            | GraphicsClass::Stem
            | GraphicsClass::ErrorBar
            | GraphicsClass::Area
            | GraphicsClass::Bar
            | GraphicsClass::Histogram
            | GraphicsClass::Contour
            | GraphicsClass::Image
            | GraphicsClass::Text
            | GraphicsClass::Legend
            | GraphicsClass::ColorBar => {
                let axes = self.node(handle)?.parent.ok_or_else(|| {
                    GraphicsError::invalid_state("graphics object has no Axes parent")
                })?;
                self.node_of_axes(axes)?;
                self.node(axes)?
                    .parent
                    .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))
            }
        }
    }

    fn set_text(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        role: TextRole,
        code_units: Vec<u16>,
        updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_updates(GraphicsClass::Text, updates)?;
        let (figure, axes, _) = self.ensure_any_axes(requested_axes)?;
        let existing = match role {
            TextRole::Title => self.axes_properties(axes)?.title,
            TextRole::XLabel => self.axes_properties(axes)?.x_label,
            TextRole::YLabel => self.axes_properties(axes)?.y_label,
            TextRole::ZLabel => self.axes_properties(axes)?.z_label,
        };
        let handle = if let Some(handle) = existing.filter(|handle| self.is_valid(*handle)) {
            let GraphicsObject::Text(properties) = &mut self.node_mut(handle)?.object else {
                return Err(GraphicsError::invalid_state(
                    "Axes text reference has wrong class",
                ));
            };
            properties.code_units = code_units;
            apply_text_updates(properties, updates);
            handle
        } else {
            let handle = self.allocate_node(
                Some(axes),
                GraphicsObject::Text({
                    let mut properties = TextProperties {
                        code_units,
                        role,
                        anchor_normalized: text_anchor(role),
                        horizontal_alignment: HorizontalAlignment::Center,
                        vertical_alignment: text_vertical_alignment(role),
                        color_rgba: [0.15, 0.15, 0.15, 1.0],
                        font_family_utf16: "Helvetica".encode_utf16().collect(),
                        font_size_css_px: if role == TextRole::Title { 13.0 } else { 11.0 },
                        font_weight: if role == TextRole::Title { 600 } else { 400 },
                        font_style: FontStyle::Normal,
                        rotation_degrees: if role == TextRole::YLabel { 90.0 } else { 0.0 },
                        interpreter: Interpreter::Tex,
                        visible: true,
                    };
                    apply_text_updates(&mut properties, updates);
                    properties
                }),
            )?;
            match role {
                TextRole::Title => self.axes_properties_mut(axes)?.title = Some(handle),
                TextRole::XLabel => self.axes_properties_mut(axes)?.x_label = Some(handle),
                TextRole::YLabel => self.axes_properties_mut(axes)?.y_label = Some(handle),
                TextRole::ZLabel => self.axes_properties_mut(axes)?.z_label = Some(handle),
            }
            handle
        };
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(handle),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_legend(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        mut labels_utf16: Vec<Vec<u16>>,
        updates: &[GraphicsPropertyUpdate],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_property_updates(GraphicsClass::Legend, updates)?;
        let (figure, axes, _) = self.ensure_any_axes(requested_axes)?;
        let series = self
            .children(axes)?
            .iter()
            .copied()
            .filter(|child| {
                matches!(
                    child.class(),
                    GraphicsClass::LineSeries
                        | GraphicsClass::ScatterSeries
                        | GraphicsClass::SurfaceSeries
                        | GraphicsClass::PatchSeries
                ) || is_chart_group_class(child.class())
            })
            .collect::<Vec<_>>();
        let existing = self
            .children(axes)?
            .iter()
            .copied()
            .find(|child| child.class() == GraphicsClass::Legend);
        if labels_utf16.is_empty() {
            if let Some(handle) = existing {
                let GraphicsObject::Legend(properties) = &self.node(handle)?.object else {
                    return Err(GraphicsError::invalid_state(
                        "Legend handle has wrong object class",
                    ));
                };
                labels_utf16.clone_from(&properties.labels_utf16);
            } else {
                labels_utf16 = series
                    .iter()
                    .enumerate()
                    .map(|(index, handle)| {
                        series_display_name(self, *handle)
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| {
                                format!("data{}", index + 1).encode_utf16().collect()
                            })
                    })
                    .collect();
            }
        }
        if labels_utf16.is_empty() || labels_utf16.len() > series.len() {
            return Err(GraphicsError::invalid_input(
                "legend labels must identify one or more existing series",
            ));
        }
        let membership = series
            .into_iter()
            .take(labels_utf16.len())
            .collect::<Vec<_>>();
        let handle = if let Some(handle) = existing {
            let GraphicsObject::Legend(properties) = &mut self.node_mut(handle)?.object else {
                return Err(GraphicsError::invalid_state(
                    "Legend handle has wrong object class",
                ));
            };
            properties.series = membership;
            properties.labels_utf16 = labels_utf16;
            properties.visible = true;
            apply_legend_updates(properties, updates);
            handle
        } else {
            self.allocate_node(
                Some(axes),
                GraphicsObject::Legend({
                    let mut properties = LegendProperties {
                        series: membership,
                        labels_utf16,
                        location: LegendLocation::Northeast,
                        visible: true,
                        background_rgba: [1.0, 1.0, 1.0, 0.9],
                        border_rgba: [0.2, 0.2, 0.2, 1.0],
                        font_family_utf16: "Helvetica".encode_utf16().collect(),
                        font_size_css_px: 10.0,
                        font_weight: 400,
                        interpreter: Interpreter::Tex,
                        orientation: LegendOrientation::Vertical,
                        num_columns: 1,
                        box_enabled: true,
                    };
                    apply_legend_updates(&mut properties, updates);
                    properties
                }),
            )?
        };
        Ok(AppliedRequest {
            response: GraphicsResponse::Handle(handle),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_limits(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        target_axis: Axis,
        limits: [f64; 2],
    ) -> Result<AppliedRequest, GraphicsError> {
        validate_limits(limits)?;
        let (figure, axes_handle, _) = self.ensure_axes(requested_axes)?;
        let properties = self.axes_properties_mut(axes_handle)?;
        match target_axis {
            Axis::X => {
                properties.x_limits = limits;
                properties.x_limit_mode = LimitMode::Manual;
            }
            Axis::Y => {
                properties.y_limits = limits;
                properties.y_limit_mode = LimitMode::Manual;
            }
            Axis::Z => {
                properties.z_limits = limits;
                properties.z_limit_mode = LimitMode::Manual;
            }
        }
        self.resolve_auto_ticks(axes_handle)?;
        Ok(AppliedRequest {
            response: GraphicsResponse::Limits(limits),
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_axes_limits(
        &mut self,
        figure: GraphicsHandle,
        requested_axes: Option<GraphicsHandle>,
        x_limits: [f64; 2],
        y_limits: [f64; 2],
    ) -> Result<AppliedRequest, GraphicsError> {
        let figure_node = self.node_of_class(figure, GraphicsClass::Figure)?;
        if figure_node.parent.is_some() {
            return Err(GraphicsError::invalid_state(
                "Figure unexpectedly belongs to another graphics object",
            ));
        }
        validate_limits(x_limits)?;
        validate_limits(y_limits)?;

        let axes = self.axes_for_figure(figure, requested_axes)?;
        let properties = self.axes_properties_mut(axes)?;
        properties.x_limits = x_limits;
        properties.y_limits = y_limits;
        properties.x_limit_mode = LimitMode::Manual;
        properties.y_limit_mode = LimitMode::Manual;
        self.resolve_auto_ticks(axes)?;

        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn axes_for_figure(
        &self,
        figure: GraphicsHandle,
        requested_axes: Option<GraphicsHandle>,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if let Some(axes) = requested_axes {
            let node = self.node_of_class(axes, GraphicsClass::Axes2D)?;
            if node.parent != Some(figure) {
                return Err(GraphicsError::invalid_handle(
                    "target Axes does not belong to the requested Figure",
                ));
            }
            return Ok(axes);
        }
        self.current_axes
            .filter(|candidate| {
                self.node(*candidate)
                    .is_ok_and(|node| node.parent == Some(figure))
            })
            .or_else(|| self.first_axes(figure))
            .ok_or_else(|| GraphicsError::invalid_handle("Figure has no target Axes"))
    }

    fn get_limits(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        target_axis: Axis,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes_handle, created) = self.ensure_axes(requested_axes)?;
        let properties = self.axes_properties(axes_handle)?;
        let limits = match target_axis {
            Axis::X => properties.x_limits,
            Axis::Y => properties.y_limits,
            Axis::Z => properties.z_limits,
        };
        Ok(AppliedRequest {
            response: GraphicsResponse::Limits(limits),
            change: created.then_some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_limit_mode(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        target_axis: Axis,
        mode: LimitMode,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, _) = self.ensure_axes(requested_axes)?;
        let properties = self.axes_properties_mut(axes)?;
        match target_axis {
            Axis::X => properties.x_limit_mode = mode,
            Axis::Y => properties.y_limit_mode = mode,
            Axis::Z => properties.z_limit_mode = mode,
        }
        if mode == LimitMode::Auto {
            self.resolve_auto_limits(axes)?;
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn set_grid(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
        mode: GridMode,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes, _) = self.ensure_any_axes(requested_axes)?;
        let enabled = mode == GridMode::On;
        let properties = self.axes_properties_mut(axes)?;
        properties.grid_x = enabled;
        properties.grid_y = enabled;
        properties.grid_z = enabled;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn clear_axes(
        &mut self,
        requested_axes: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        let (figure, axes) = if let Some(axes) = requested_axes {
            let figure = self
                .node_of_axes(axes)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            (figure, axes)
        } else {
            let (figure, axes, _) = self.ensure_current_axes()?;
            (figure, axes)
        };
        let retained_next_plot = self.axes_properties(axes)?.next_plot;
        let coordinate_system = self.axes_properties(axes)?.coordinate_system;
        self.delete_children(axes)?;
        let properties = self.axes_properties_mut(axes)?;
        *properties = default_axes_properties();
        if coordinate_system == AxesCoordinateSystem::Polar {
            apply_polar_factory_defaults(properties);
        }
        properties.next_plot = retained_next_plot;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn clear_figure(
        &mut self,
        requested_figure: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        let figure = if let Some(figure) = requested_figure {
            self.node_of_class(figure, GraphicsClass::Figure)?;
            figure
        } else {
            self.ensure_current_figure()?.0
        };
        self.delete_layouts_for_figure(figure)?;
        self.delete_children(figure)?;
        self.current_figure = Some(figure);
        self.current_axes = None;
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: None,
            }),
        })
    }

    fn close_figure(
        &mut self,
        requested: Option<GraphicsHandle>,
    ) -> Result<AppliedRequest, GraphicsError> {
        let figure = requested.or(self.current_figure).ok_or_else(|| {
            GraphicsError::invalid_handle("no current Figure is available to close")
        })?;
        let node = self.node_of_class(figure, GraphicsClass::Figure)?;
        let figure_id = node.public_id.clone();
        let previous_revision = self
            .figure_revisions
            .get(&figure_id)
            .copied()
            .ok_or_else(|| GraphicsError::invalid_state("Figure has no committed revision"))?;
        let mut deleted = Vec::new();
        self.collect_deleted(figure, &mut deleted)?;
        let was_current = self.current_figure == Some(figure);
        self.delete_layouts_for_figure(figure)?;
        self.delete_subtree(figure)?;
        if was_current {
            self.advance_current_after_figure_deletion();
        }
        Ok(AppliedRequest {
            response: GraphicsResponse::None,
            change: Some(FigureChange {
                handle: figure,
                closed: Some(ClosedFigure {
                    figure_id,
                    previous_revision,
                    deleted,
                }),
            }),
        })
    }

    fn ensure_current_figure(&mut self) -> Result<(GraphicsHandle, bool), GraphicsError> {
        if let Some(figure) = self.current_figure.filter(|handle| self.is_valid(*handle)) {
            return Ok((figure, false));
        }
        let AppliedRequest {
            response: GraphicsResponse::Handle(figure),
            ..
        } = self.select_or_create_figure(None)?
        else {
            return Err(GraphicsError::invalid_state(
                "Figure creation returned an unexpected response",
            ));
        };
        Ok((figure, true))
    }

    fn ensure_current_axes(
        &mut self,
    ) -> Result<(GraphicsHandle, GraphicsHandle, bool), GraphicsError> {
        if let Some(axes) = self.current_axes.filter(|handle| self.is_valid(*handle)) {
            let figure = self
                .node(axes)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            return Ok((figure, axes, false));
        }
        let (figure, figure_created) = self.ensure_current_figure()?;
        if let Some(axes) = self.first_axes(figure) {
            self.current_axes = Some(axes);
            return Ok((figure, axes, figure_created));
        }
        let axes = self.create_axes_node(figure)?;
        self.current_axes = Some(axes);
        Ok((figure, axes, true))
    }

    fn ensure_axes(
        &mut self,
        requested: Option<GraphicsHandle>,
    ) -> Result<(GraphicsHandle, GraphicsHandle, bool), GraphicsError> {
        self.ensure_axes_for_coordinate_system(requested, AxesCoordinateSystem::Cartesian)
    }

    fn ensure_any_axes(
        &mut self,
        requested: Option<GraphicsHandle>,
    ) -> Result<(GraphicsHandle, GraphicsHandle, bool), GraphicsError> {
        if let Some(axes) = requested {
            let figure = self
                .node_of_axes(axes)?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            self.node_of_class(figure, GraphicsClass::Figure)?;
            self.current_figure = Some(figure);
            self.current_axes = Some(axes);
            Ok((figure, axes, false))
        } else {
            self.ensure_current_axes()
        }
    }

    fn ensure_axes_for_coordinate_system(
        &mut self,
        requested: Option<GraphicsHandle>,
        coordinate_system: AxesCoordinateSystem,
    ) -> Result<(GraphicsHandle, GraphicsHandle, bool), GraphicsError> {
        if let Some(axes) = requested {
            let figure = self
                .node_of_class(axes, coordinate_system.graphics_class())?
                .parent
                .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?;
            self.node_of_class(figure, GraphicsClass::Figure)?;
            return Ok((figure, axes, false));
        }
        let (figure, figure_created) = self.ensure_current_figure()?;
        let candidate = self
            .current_axes
            .filter(|handle| self.is_valid(*handle))
            .or_else(|| self.first_axes(figure));
        if let Some(axes) = candidate {
            if axes.class() == coordinate_system.graphics_class() {
                self.current_axes = Some(axes);
                return Ok((figure, axes, figure_created));
            }
            self.delete_subtree(axes)?;
        }
        let axes = self.create_axes_node_for_coordinate_system(figure, coordinate_system)?;
        self.current_axes = Some(axes);
        Ok((figure, axes, true))
    }

    fn create_axes_node_for_coordinate_system(
        &mut self,
        figure: GraphicsHandle,
        coordinate_system: AxesCoordinateSystem,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.node_of_class(figure, GraphicsClass::Figure)?;
        let mut properties = default_axes_properties();
        properties.coordinate_system = coordinate_system;
        if coordinate_system == AxesCoordinateSystem::Polar {
            apply_polar_factory_defaults(&mut properties);
        }
        self.allocate_node(Some(figure), GraphicsObject::Axes2D(properties))
    }

    fn create_polar_axes_node(
        &mut self,
        figure: GraphicsHandle,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.create_axes_node_for_coordinate_system(figure, AxesCoordinateSystem::Polar)
    }

    fn create_axes_node(
        &mut self,
        figure: GraphicsHandle,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.create_axes_node_for_coordinate_system(figure, AxesCoordinateSystem::Cartesian)
    }

    fn node_of_axes(&self, handle: GraphicsHandle) -> Result<&Node, GraphicsError> {
        if matches!(
            handle.class(),
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes
        ) {
            self.node(handle)
        } else {
            Err(GraphicsError::invalid_handle(format!(
                "expected an Axes, found {}",
                handle.class().class_name()
            )))
        }
    }

    fn allocate_node(
        &mut self,
        parent: Option<GraphicsHandle>,
        object: GraphicsObject,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if self.object_count() >= MAX_OBJECTS {
            return Err(GraphicsError::limit("graphics object limit was exceeded"));
        }
        if let Some(parent_handle) = parent {
            let parent_class = self.node(parent_handle)?.object.class();
            if !allows_child(parent_class, object.class()) {
                return Err(GraphicsError::invalid_input(
                    "graphics parent/child classes are incompatible",
                ));
            }
        } else if !matches!(
            object.class(),
            GraphicsClass::Figure | GraphicsClass::TiledChartLayout
        ) {
            return Err(GraphicsError::invalid_input(
                "only a Figure or detached TiledChartLayout may be parented directly by the session",
            ));
        }
        let (slot_index, generation) = if let Some(slot_index) = self.free_slots.pop() {
            let index = usize::try_from(slot_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .ok_or_else(|| GraphicsError::invalid_state("free slot index is invalid"))?;
            let slot = self
                .slots
                .get(index)
                .ok_or_else(|| GraphicsError::invalid_state("free slot is missing"))?;
            if slot.retired || slot.node.is_some() {
                return Err(GraphicsError::invalid_state("free slot is not reusable"));
            }
            (slot_index, slot.generation)
        } else {
            let next = u32::try_from(self.slots.len())
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| GraphicsError::limit("graphics handle space was exhausted"))?;
            self.slots.push(Slot {
                generation: 1,
                retired: false,
                node: None,
            });
            (next, 1)
        };
        let handle = GraphicsHandle::new(slot_index, generation, object.class())
            .ok_or_else(|| GraphicsError::invalid_state("arena produced an invalid handle"))?;
        let public_id = self.allocate_public_id("object")?;
        let node = Node {
            public_id,
            object_revision: 0,
            parent,
            children: Vec::new(),
            object,
        };
        let index = usize::try_from(slot_index - 1)
            .map_err(|_| GraphicsError::invalid_state("slot index exceeds host limits"))?;
        self.slots[index].node = Some(node);
        if let Some(parent_handle) = parent {
            self.node_mut(parent_handle)?.children.push(handle);
        }
        Ok(handle)
    }

    fn allocate_resource(&mut self, data: &NumericData) -> Result<DataRef, GraphicsError> {
        let length = u64::try_from(data.len())
            .map_err(|_| GraphicsError::limit("graphics data length exceeds model limits"))?;
        self.allocate_resource_shaped(data, [1, length])
    }

    fn allocate_resource_shaped(
        &mut self,
        data: &NumericData,
        shape: [u64; 2],
    ) -> Result<DataRef, GraphicsError> {
        let identifier = self
            .next_resource_id
            .ok_or_else(|| GraphicsError::limit("graphics resource identifiers were exhausted"))?;
        let id = DataResourceId::new(identifier)
            .ok_or_else(|| GraphicsError::limit("graphics resource identifier is invalid"))?;
        let next = identifier
            .checked_add(1)
            .filter(|next| *next <= JSON_SAFE_INTEGER_MAX);
        let resource = DataResource::from_numeric_with_shape(id, data, shape)?;
        let descriptor = resource.descriptor().clone();
        self.resources.insert(id, resource);
        self.next_resource_id = next;
        Ok(descriptor)
    }

    fn allocate_public_id(&mut self, kind: &str) -> Result<String, GraphicsError> {
        let identifier = self
            .next_public_id
            .ok_or_else(|| GraphicsError::limit("graphics public identifiers were exhausted"))?;
        self.next_public_id = identifier
            .checked_add(1)
            .filter(|next| *next <= JSON_SAFE_INTEGER_MAX);
        Ok(format!(
            "openmat-{kind}-{}-{identifier}",
            self.session_serial
        ))
    }

    fn delete_children(&mut self, parent: GraphicsHandle) -> Result<(), GraphicsError> {
        let children = self.node(parent)?.children.clone();
        for child in children {
            self.delete_subtree(child)?;
        }
        self.node_mut(parent)?.children.clear();
        Ok(())
    }

    fn delete_layouts_for_figure(&mut self, figure: GraphicsHandle) -> Result<(), GraphicsError> {
        let layouts = self
            .live_handles()
            .filter(|handle| handle.class() == GraphicsClass::TiledChartLayout)
            .filter(|handle| {
                matches!(
                    self.object(*handle),
                    Ok(GraphicsObject::TiledChartLayout(properties))
                        if properties.parent == figure
                )
            })
            .collect::<Vec<_>>();
        for layout in layouts {
            self.delete_subtree(layout)?;
        }
        Ok(())
    }

    fn delete_subtree(&mut self, handle: GraphicsHandle) -> Result<(), GraphicsError> {
        let children = self.node(handle)?.children.clone();
        for child in children {
            self.delete_subtree(child)?;
        }
        let parent = self.node(handle)?.parent;
        if let Some(parent) = parent
            && let Ok(parent_node) = self.node_mut(parent)
        {
            parent_node.children.retain(|child| *child != handle);
        }
        let index = usize::try_from(handle.slot() - 1)
            .map_err(|_| GraphicsError::invalid_state("slot index exceeds host limits"))?;
        let slot = self
            .slots
            .get_mut(index)
            .ok_or_else(|| GraphicsError::invalid_handle("graphics handle slot is unknown"))?;
        slot.node = None;
        if slot.generation == u32::MAX {
            slot.retired = true;
        } else {
            slot.generation += 1;
            self.free_slots.push(handle.slot());
        }
        if self.current_axes == Some(handle) {
            self.current_axes = None;
        }
        if self.current_figure == Some(handle) {
            self.current_figure = None;
        }
        if self.current_layout == Some(handle) {
            self.current_layout = None;
        }
        Ok(())
    }

    fn collect_deleted(
        &self,
        handle: GraphicsHandle,
        deleted: &mut Vec<(String, u32)>,
    ) -> Result<(), GraphicsError> {
        let node = self.node(handle)?;
        deleted.push((node.public_id.clone(), handle.generation()));
        for child in &node.children {
            self.collect_deleted(*child, deleted)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn finalize_change(
        &mut self,
        before: &Self,
        change: FigureChange,
    ) -> Result<GraphicsNotice, GraphicsError> {
        if let Some(closed) = change.closed {
            let revision = closed
                .previous_revision
                .checked_add(1)
                .filter(|revision| *revision <= JSON_SAFE_INTEGER_MAX)
                .ok_or_else(|| GraphicsError::limit("Figure revision was exhausted"))?;
            let operations = closed
                .deleted
                .into_iter()
                .map(|(id, generation)| GraphicsDeltaOperation::DeleteObject { id, generation })
                .collect();
            let delta = GraphicsDelta {
                figure_id: closed.figure_id.clone(),
                base_revision: closed.previous_revision,
                revision,
                operations,
                added_data: Vec::new(),
                released_data_ids: before_data_ids(before, &closed.figure_id),
            };
            self.latest_deltas
                .insert(closed.figure_id.clone(), delta.clone());
            self.pending_deltas.push(delta);
            self.figure_revisions.remove(&closed.figure_id);
            return Ok(GraphicsNotice {
                figure_id: closed.figure_id,
                revision,
                kind: GraphicsNoticeKind::Closed,
                discovery: false,
            });
        }
        let figure_id = self
            .node_of_class(change.handle, GraphicsClass::Figure)?
            .public_id
            .clone();
        let base_revision = before
            .figure_revisions
            .get(&figure_id)
            .copied()
            .unwrap_or(0);
        let revision = base_revision
            .checked_add(1)
            .filter(|revision| *revision <= JSON_SAFE_INTEGER_MAX)
            .ok_or_else(|| GraphicsError::limit("Figure revision was exhausted"))?;
        let before_snapshot = before.snapshot(change.handle).ok();
        self.update_object_revisions(change.handle, before_snapshot.as_ref())?;
        self.figure_revisions.insert(figure_id, revision);
        let snapshot = self.snapshot(change.handle)?;
        let mut operations = Vec::new();
        if before_snapshot.is_none() {
            operations.push(GraphicsDeltaOperation::SetRoot {
                root_id: snapshot.root_id.clone(),
            });
        }
        let mut previous_data = BTreeMap::new();
        if let Some(previous) = &before_snapshot {
            previous_data.extend(
                previous
                    .referenced_data
                    .iter()
                    .map(|resource| (resource.descriptor().id, resource.descriptor().clone())),
            );
        }
        if let Some(previous) = before_snapshot {
            let current_ids = snapshot
                .objects
                .iter()
                .map(|object| object.id.as_str())
                .collect::<BTreeSet<_>>();
            for object in previous.objects {
                if !current_ids.contains(object.id.as_str()) {
                    operations.push(GraphicsDeltaOperation::DeleteObject {
                        id: object.id,
                        generation: object.generation,
                    });
                }
            }
        }
        let previous_objects = before
            .snapshot(change.handle)
            .ok()
            .map(|snapshot| {
                snapshot
                    .objects
                    .into_iter()
                    .map(|object| (object.id.clone(), object))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        for object in &snapshot.objects {
            let changed = previous_objects
                .get(&object.id)
                .is_none_or(|previous| previous != object);
            if changed {
                operations.push(GraphicsDeltaOperation::UpsertObject(Box::new(
                    object.clone(),
                )));
            }
            let reordered = previous_objects
                .get(&object.id)
                .is_some_and(|previous| previous.children != object.children);
            if reordered {
                operations.push(GraphicsDeltaOperation::ReorderChildren {
                    parent_id: object.id.clone(),
                    children: object.children.clone(),
                });
            }
        }
        let current_data = snapshot
            .referenced_data
            .iter()
            .map(|resource| (resource.descriptor().id, resource.descriptor().clone()))
            .collect::<BTreeMap<_, _>>();
        let added_data = current_data
            .iter()
            .filter(|(id, _)| !previous_data.contains_key(id))
            .map(|(_, descriptor)| descriptor.clone())
            .collect();
        let released_data_ids = previous_data
            .keys()
            .filter(|id| !current_data.contains_key(id))
            .copied()
            .collect();
        let delta = GraphicsDelta {
            figure_id: snapshot.figure_id.clone(),
            base_revision,
            revision,
            operations,
            added_data,
            released_data_ids,
        };
        self.latest_deltas
            .insert(snapshot.figure_id.clone(), delta.clone());
        self.pending_deltas.push(delta);
        Ok(GraphicsNotice {
            figure_id: snapshot.figure_id,
            revision,
            kind: GraphicsNoticeKind::Updated,
            discovery: base_revision == 0,
        })
    }

    fn update_object_revisions(
        &mut self,
        root: GraphicsHandle,
        previous: Option<&FigureSnapshot>,
    ) -> Result<(), GraphicsError> {
        let previous = previous
            .map(|snapshot| {
                snapshot
                    .objects
                    .iter()
                    .map(|object| (object.id.clone(), object.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let mut handles = Vec::new();
        self.collect_handles(root, &mut handles)?;
        for handle in handles {
            let current = self.hir_object(handle)?;
            let object_revision = if let Some(old) = previous.get(&current.id) {
                if same_object_semantics(old, &current) {
                    old.object_revision
                } else {
                    old.object_revision
                        .checked_add(1)
                        .filter(|revision| *revision <= JSON_SAFE_INTEGER_MAX)
                        .ok_or_else(|| GraphicsError::limit("object revision was exhausted"))?
                }
            } else {
                1
            };
            self.node_mut(handle)?.object_revision = object_revision;
        }
        Ok(())
    }

    fn snapshot_subtree(
        &self,
        handle: GraphicsHandle,
        objects: &mut Vec<HirObject>,
        data_ids: &mut BTreeSet<DataResourceId>,
    ) -> Result<(), GraphicsError> {
        let node = self.node(handle)?;
        let parent_id = node
            .parent
            .map(|parent| self.node(parent).map(|node| node.public_id.clone()))
            .transpose()?;
        let children = node
            .children
            .iter()
            .map(|child| self.node(*child).map(|node| node.public_id.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        collect_data_ids(&node.object, data_ids);
        objects.push(HirObject {
            id: node.public_id.clone(),
            generation: handle.generation(),
            object_revision: node.object_revision,
            class: handle.class(),
            parent_id,
            children,
            properties: HirProperties::from(&node.object),
        });
        for child in &node.children {
            self.snapshot_subtree(*child, objects, data_ids)?;
        }
        Ok(())
    }

    fn collect_handles(
        &self,
        handle: GraphicsHandle,
        handles: &mut Vec<GraphicsHandle>,
    ) -> Result<(), GraphicsError> {
        let node = self.node(handle)?;
        handles.push(handle);
        for child in &node.children {
            self.collect_handles(*child, handles)?;
        }
        Ok(())
    }

    fn hir_object(&self, handle: GraphicsHandle) -> Result<HirObject, GraphicsError> {
        let node = self.node(handle)?;
        let parent_id = node
            .parent
            .map(|parent| self.node(parent).map(|parent| parent.public_id.clone()))
            .transpose()?;
        let children = node
            .children
            .iter()
            .map(|child| self.node(*child).map(|child| child.public_id.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HirObject {
            id: node.public_id.clone(),
            generation: handle.generation(),
            object_revision: node.object_revision,
            class: handle.class(),
            parent_id,
            children,
            properties: HirProperties::from(&node.object),
        })
    }

    fn resolve_auto_limits(&mut self, axes: GraphicsHandle) -> Result<(), GraphicsError> {
        let properties = self.axes_properties(axes)?.clone();
        if properties.x_limit_mode == LimitMode::Manual
            && properties.y_limit_mode == LimitMode::Manual
            && properties.z_limit_mode == LimitMode::Manual
        {
            self.resolve_auto_aspect_ratio(axes)?;
            return self.resolve_auto_ticks(axes);
        }
        let children = self.drawable_descendants(axes)?;
        let mut x_values = Vec::new();
        let mut y_values = Vec::new();
        let mut z_values = Vec::new();
        for child in children {
            match &self.node(child)?.object {
                GraphicsObject::LineSeries(series) => {
                    self.append_finite_values(&series.x_data, &mut x_values)?;
                    self.append_finite_values(&series.y_data, &mut y_values)?;
                    if let Some(z_data) = &series.z_data {
                        self.append_finite_values(z_data, &mut z_values)?;
                    }
                }
                GraphicsObject::ScatterSeries(series) => {
                    self.append_finite_values(&series.x_data, &mut x_values)?;
                    self.append_finite_values(&series.y_data, &mut y_values)?;
                    if let Some(z_data) = &series.z_data {
                        self.append_finite_values(z_data, &mut z_values)?;
                    }
                }
                GraphicsObject::SurfaceSeries(series) => {
                    self.append_finite_values(&series.x_data, &mut x_values)?;
                    self.append_finite_values(&series.y_data, &mut y_values)?;
                    self.append_finite_values(&series.z_data, &mut z_values)?;
                }
                GraphicsObject::PatchSeries(series) => {
                    self.append_finite_matrix_column(&series.vertices, 0, &mut x_values)?;
                    self.append_finite_matrix_column(&series.vertices, 1, &mut y_values)?;
                    if series.vertices.shape[1] == 3 {
                        self.append_finite_matrix_column(&series.vertices, 2, &mut z_values)?;
                    }
                }
                _ => {}
            }
        }
        if properties.coordinate_system == AxesCoordinateSystem::Polar {
            let radial_values = y_values.iter().copied().map(f64::abs).collect::<Vec<_>>();
            let radial_resolved = self.auto_limits.resolve(&radial_values);
            let properties = self.axes_properties_mut(axes)?;
            if properties.x_limit_mode == LimitMode::Auto {
                properties.x_limits = polar_full_circle(properties.theta_axis_units);
            }
            if properties.y_limit_mode == LimitMode::Auto {
                properties.y_limits = [0.0, radial_resolved[1].max(0.0)];
                if properties.y_limits[1] <= properties.y_limits[0] {
                    properties.y_limits = [0.0, 1.0];
                }
            }
            properties.z_limits = DEFAULT_LIMITS;
            self.resolve_auto_aspect_ratio(axes)?;
            return self.resolve_auto_ticks(axes);
        }
        let x_limits = resolve_axis_limits(
            self.auto_limits.as_ref(),
            &x_values,
            properties.x_scale,
            properties.x_limit_method,
        );
        let y_limits = resolve_axis_limits(
            self.auto_limits.as_ref(),
            &y_values,
            properties.y_scale,
            properties.y_limit_method,
        );
        let z_limits = resolve_axis_limits(
            self.auto_limits.as_ref(),
            &z_values,
            properties.z_scale,
            properties.z_limit_method,
        );
        let properties = self.axes_properties_mut(axes)?;
        if properties.x_limit_mode == LimitMode::Auto {
            properties.x_limits = x_limits;
        }
        if properties.y_limit_mode == LimitMode::Auto {
            properties.y_limits = y_limits;
        }
        if properties.z_limit_mode == LimitMode::Auto {
            properties.z_limits = z_limits;
        }
        self.resolve_auto_aspect_ratio(axes)?;
        self.resolve_auto_ticks(axes)
    }

    fn resolve_auto_aspect_ratio(&mut self, axes: GraphicsHandle) -> Result<(), GraphicsError> {
        let properties = self.axes_properties_mut(axes)?;
        let spans = [
            properties.x_limits[1] - properties.x_limits[0],
            properties.y_limits[1] - properties.y_limits[0],
            properties.z_limits[1] - properties.z_limits[0],
        ];
        if properties.plot_box_aspect_ratio_mode == LimitMode::Auto
            && properties.data_aspect_ratio_mode == LimitMode::Manual
        {
            let raw = [
                spans[0] / properties.data_aspect_ratio[0],
                spans[1] / properties.data_aspect_ratio[1],
                spans[2] / properties.data_aspect_ratio[2],
            ];
            let minimum = raw.into_iter().fold(f64::INFINITY, f64::min);
            if minimum.is_finite() && minimum > 0.0 {
                properties.plot_box_aspect_ratio = raw.map(|value| value / minimum);
            }
        } else if properties.data_aspect_ratio_mode == LimitMode::Auto
            && properties.plot_box_aspect_ratio_mode == LimitMode::Manual
        {
            let raw = [
                spans[0] / properties.plot_box_aspect_ratio[0],
                spans[1] / properties.plot_box_aspect_ratio[1],
                spans[2] / properties.plot_box_aspect_ratio[2],
            ];
            let minimum = raw.into_iter().fold(f64::INFINITY, f64::min);
            if minimum.is_finite() && minimum > 0.0 {
                properties.data_aspect_ratio = raw.map(|value| value / minimum);
            }
        }
        Ok(())
    }

    fn resolve_auto_color_limits(&mut self, axes: GraphicsHandle) -> Result<(), GraphicsError> {
        if self.axes_properties(axes)?.c_limit_mode == LimitMode::Manual {
            return Ok(());
        }
        let children = self.drawable_descendants(axes)?;
        let mut values = Vec::new();
        for child in children {
            match &self.node(child)?.object {
                GraphicsObject::SurfaceSeries(series) => {
                    self.append_finite_values(&series.c_data, &mut values)?;
                }
                GraphicsObject::ScatterSeries(series) => {
                    if let Some(color_data) = &series.color_data {
                        self.append_finite_values(color_data, &mut values)?;
                    }
                }
                GraphicsObject::PatchSeries(series) => {
                    self.append_finite_values(&series.face_vertex_cdata, &mut values)?;
                }
                _ => {}
            }
        }
        self.axes_properties_mut(axes)?.c_limits = self.auto_limits.resolve(&values);
        self.refresh_auto_colorbar_ticks(axes)?;
        Ok(())
    }

    fn drawable_descendants(
        &self,
        parent: GraphicsHandle,
    ) -> Result<Vec<GraphicsHandle>, GraphicsError> {
        let mut descendants = Vec::new();
        let mut stack = self.node(parent)?.children.clone();
        while let Some(handle) = stack.pop() {
            if matches!(self.object(handle)?, GraphicsObject::ChartGroup(_)) {
                stack.extend(self.node(handle)?.children.iter().rev().copied());
            } else {
                descendants.push(handle);
            }
        }
        descendants.reverse();
        Ok(descendants)
    }

    fn refresh_auto_colorbar_ticks(&mut self, axes: GraphicsHandle) -> Result<(), GraphicsError> {
        let limits = self.axes_properties(axes)?.c_limits;
        let automatic = automatic_colorbar_ticks(limits);
        let children = self.children(axes)?.to_vec();
        for child in children {
            if let GraphicsObject::ColorBar(properties) = &mut self.node_mut(child)?.object
                && properties.tick_mode == TickMode::Auto
            {
                properties.ticks.clone_from(&automatic);
                if properties.tick_label_mode == TickMode::Auto {
                    properties.tick_labels_utf16 = automatic_tick_labels(&properties.ticks);
                }
            }
        }
        Ok(())
    }

    fn resolve_auto_ticks(&mut self, axes: GraphicsHandle) -> Result<(), GraphicsError> {
        let properties = self.axes_properties_mut(axes)?;
        if properties.coordinate_system == AxesCoordinateSystem::Polar {
            if properties.x_tick_mode == TickMode::Auto {
                properties.x_ticks =
                    automatic_polar_theta_ticks(properties.x_limits, properties.theta_axis_units);
            }
            if properties.y_tick_mode == TickMode::Auto {
                properties.y_ticks = automatic_ticks(properties.y_limits);
            }
            if properties.x_tick_label_mode == TickMode::Auto {
                properties.x_tick_labels_utf16 = automatic_polar_theta_labels(
                    &properties.x_ticks,
                    properties.x_limits,
                    properties.theta_axis_units,
                );
            }
            if properties.y_tick_label_mode == TickMode::Auto {
                properties.y_tick_labels_utf16 = automatic_tick_labels(&properties.y_ticks);
            }
            return Ok(());
        }
        if properties.x_tick_mode == TickMode::Auto {
            properties.x_ticks = automatic_axis_ticks(properties.x_limits, properties.x_scale);
        }
        if properties.y_tick_mode == TickMode::Auto {
            properties.y_ticks = automatic_axis_ticks(properties.y_limits, properties.y_scale);
        }
        if properties.z_tick_mode == TickMode::Auto {
            properties.z_ticks = automatic_axis_ticks(properties.z_limits, properties.z_scale);
        }
        if properties.x_tick_label_mode == TickMode::Auto {
            properties.x_tick_labels_utf16 =
                automatic_axis_tick_labels(&properties.x_ticks, properties.x_scale);
        }
        if properties.y_tick_label_mode == TickMode::Auto {
            properties.y_tick_labels_utf16 =
                automatic_axis_tick_labels(&properties.y_ticks, properties.y_scale);
        }
        if properties.z_tick_label_mode == TickMode::Auto {
            properties.z_tick_labels_utf16 =
                automatic_axis_tick_labels(&properties.z_ticks, properties.z_scale);
        }
        Ok(())
    }

    fn append_finite_values(
        &self,
        reference: &DataRef,
        values: &mut Vec<f64>,
    ) -> Result<(), GraphicsError> {
        let resource = self
            .resources
            .get(&reference.id)
            .ok_or_else(|| GraphicsError::invalid_state("series resource is missing"))?;
        match reference.dtype {
            DataDType::F32 => {
                for chunk in resource.bytes().chunks_exact(4) {
                    let value =
                        f64::from(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                    if value.is_finite() {
                        values.push(value);
                    }
                }
            }
            DataDType::F64 => {
                for chunk in resource.bytes().chunks_exact(8) {
                    let value = f64::from_le_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                        chunk[7],
                    ]);
                    if value.is_finite() {
                        values.push(value);
                    }
                }
            }
        }
        Ok(())
    }

    fn append_finite_matrix_column(
        &self,
        reference: &DataRef,
        column: u64,
        values: &mut Vec<f64>,
    ) -> Result<(), GraphicsError> {
        let rows = usize::try_from(reference.shape[0])
            .map_err(|_| GraphicsError::limit("series row count exceeds host limits"))?;
        let column = usize::try_from(column)
            .map_err(|_| GraphicsError::limit("series column exceeds host limits"))?;
        if column >= usize::try_from(reference.shape[1]).unwrap_or(0) {
            return Err(GraphicsError::invalid_state(
                "series column is outside its resource shape",
            ));
        }
        let resource = self
            .resources
            .get(&reference.id)
            .ok_or_else(|| GraphicsError::invalid_state("series resource is missing"))?;
        let byte_width = usize::try_from(reference.dtype.byte_width())
            .map_err(|_| GraphicsError::limit("series element width exceeds host limits"))?;
        let start = column
            .checked_mul(rows)
            .and_then(|value| value.checked_mul(byte_width))
            .ok_or_else(|| GraphicsError::limit("series column offset overflowed"))?;
        let end = start
            .checked_add(rows.saturating_mul(byte_width))
            .ok_or_else(|| GraphicsError::limit("series column extent overflowed"))?;
        let bytes = resource
            .bytes()
            .get(start..end)
            .ok_or_else(|| GraphicsError::invalid_state("series resource is truncated"))?;
        match reference.dtype {
            DataDType::F32 => bytes.chunks_exact(4).for_each(|chunk| {
                let value = f64::from(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                if value.is_finite() {
                    values.push(value);
                }
            }),
            DataDType::F64 => bytes.chunks_exact(8).for_each(|chunk| {
                let value = f64::from_le_bytes([
                    chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
                ]);
                if value.is_finite() {
                    values.push(value);
                }
            }),
        }
        Ok(())
    }

    fn collect_unreferenced_resources(&mut self) {
        let mut referenced = BTreeSet::new();
        for slot in &self.slots {
            if let Some(node) = &slot.node {
                collect_data_ids(&node.object, &mut referenced);
            }
        }
        self.resources.retain(|id, _| referenced.contains(id));
    }

    fn node(&self, handle: GraphicsHandle) -> Result<&Node, GraphicsError> {
        let index = usize::try_from(handle.slot() - 1)
            .map_err(|_| GraphicsError::invalid_handle("graphics handle slot is invalid"))?;
        let slot = self
            .slots
            .get(index)
            .ok_or_else(|| GraphicsError::invalid_handle("graphics handle slot is unknown"))?;
        if slot.generation != handle.generation() {
            return Err(GraphicsError::invalid_handle("graphics handle is stale"));
        }
        let node = slot
            .node
            .as_ref()
            .ok_or_else(|| GraphicsError::invalid_handle("graphics object was deleted"))?;
        if node.object.class() != handle.class() {
            return Err(GraphicsError::invalid_handle(
                "graphics handle carries the wrong public class",
            ));
        }
        Ok(node)
    }

    fn node_mut(&mut self, handle: GraphicsHandle) -> Result<&mut Node, GraphicsError> {
        let index = usize::try_from(handle.slot() - 1)
            .map_err(|_| GraphicsError::invalid_handle("graphics handle slot is invalid"))?;
        let slot = self
            .slots
            .get_mut(index)
            .ok_or_else(|| GraphicsError::invalid_handle("graphics handle slot is unknown"))?;
        if slot.generation != handle.generation() {
            return Err(GraphicsError::invalid_handle("graphics handle is stale"));
        }
        let node = slot
            .node
            .as_mut()
            .ok_or_else(|| GraphicsError::invalid_handle("graphics object was deleted"))?;
        if node.object.class() != handle.class() {
            return Err(GraphicsError::invalid_handle(
                "graphics handle carries the wrong public class",
            ));
        }
        Ok(node)
    }

    fn node_of_class(
        &self,
        handle: GraphicsHandle,
        expected: GraphicsClass,
    ) -> Result<&Node, GraphicsError> {
        let node = self.node(handle)?;
        if handle.class() == expected {
            Ok(node)
        } else {
            Err(GraphicsError::invalid_handle(format!(
                "expected {}, found {}",
                expected.class_name(),
                handle.class().class_name()
            )))
        }
    }

    fn axes_properties(&self, axes: GraphicsHandle) -> Result<&Axes2DProperties, GraphicsError> {
        let GraphicsObject::Axes2D(properties) = &self.node_of_axes(axes)?.object else {
            return Err(GraphicsError::invalid_state("Axes payload has wrong class"));
        };
        Ok(properties)
    }

    fn axes_properties_mut(
        &mut self,
        axes: GraphicsHandle,
    ) -> Result<&mut Axes2DProperties, GraphicsError> {
        let node = self.node_mut(axes)?;
        let GraphicsObject::Axes2D(properties) = &mut node.object else {
            return Err(GraphicsError::invalid_handle(
                "graphics handle is not an Axes",
            ));
        };
        Ok(properties)
    }

    fn tiled_layout_properties(
        &self,
        layout: GraphicsHandle,
    ) -> Result<&TiledChartLayoutProperties, GraphicsError> {
        let GraphicsObject::TiledChartLayout(properties) = &self
            .node_of_class(layout, GraphicsClass::TiledChartLayout)?
            .object
        else {
            return Err(GraphicsError::invalid_state(
                "TiledChartLayout payload has wrong class",
            ));
        };
        Ok(properties)
    }

    fn tiled_layout_properties_mut(
        &mut self,
        layout: GraphicsHandle,
    ) -> Result<&mut TiledChartLayoutProperties, GraphicsError> {
        let node = self.node_mut(layout)?;
        let GraphicsObject::TiledChartLayout(properties) = &mut node.object else {
            return Err(GraphicsError::invalid_handle(
                "graphics handle is not a TiledChartLayout",
            ));
        };
        Ok(properties)
    }

    fn figure_by_number(&self, number: u32) -> Option<GraphicsHandle> {
        self.live_handles().find(|handle| {
            if handle.class() != GraphicsClass::Figure {
                return false;
            }
            matches!(
                self.object(*handle),
                Ok(GraphicsObject::Figure(properties)) if properties.number == number
            )
        })
    }

    fn layout_for_figure(&self, figure: GraphicsHandle) -> Option<GraphicsHandle> {
        self.live_handles().find(|handle| {
            handle.class() == GraphicsClass::TiledChartLayout
                && matches!(
                    self.object(*handle),
                    Ok(GraphicsObject::TiledChartLayout(properties))
                        if properties.parent == figure
                )
        })
    }

    fn layout_for_axes(&self, axes: GraphicsHandle) -> Option<GraphicsHandle> {
        self.live_handles().find(|handle| {
            handle.class() == GraphicsClass::TiledChartLayout
                && matches!(
                    self.object(*handle),
                    Ok(GraphicsObject::TiledChartLayout(properties))
                        if properties.tile_axes.iter().flatten().any(|candidate| *candidate == axes)
                )
        })
    }

    fn smallest_available_figure_number(&self) -> u32 {
        let used = self
            .live_handles()
            .filter_map(|handle| match self.object(handle).ok()? {
                GraphicsObject::Figure(properties) => Some(properties.number),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        (1..=u32::MAX)
            .find(|number| !used.contains(number))
            .unwrap_or(u32::MAX)
    }

    fn live_handles(&self) -> impl Iterator<Item = GraphicsHandle> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let node = slot.node.as_ref()?;
            let slot_number = u32::try_from(index).ok()?.checked_add(1)?;
            GraphicsHandle::new(slot_number, slot.generation, node.object.class())
        })
    }

    fn first_axes(&self, figure: GraphicsHandle) -> Option<GraphicsHandle> {
        self.node(figure)
            .ok()?
            .children
            .iter()
            .copied()
            .find(|child| {
                matches!(
                    child.class(),
                    GraphicsClass::Axes2D | GraphicsClass::PolarAxes
                ) && self.is_valid(*child)
            })
    }

    fn advance_current_after_figure_deletion(&mut self) {
        let next = self
            .live_handles()
            .filter(|handle| handle.class() == GraphicsClass::Figure)
            .min_by_key(|handle| match self.object(*handle) {
                Ok(GraphicsObject::Figure(properties)) => properties.number,
                _ => u32::MAX,
            });
        self.current_figure = next;
        self.current_axes = next.and_then(|figure| self.first_axes(figure));
        self.current_layout = next.and_then(|figure| self.layout_for_figure(figure));
    }
}

impl Default for GraphicsSession {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::too_many_lines)] // Exhaustive property validation is intentionally centralized.
fn validate_property_updates(
    class: GraphicsClass,
    properties: &[GraphicsPropertyUpdate],
) -> Result<(), GraphicsError> {
    for update in properties {
        validate_property_for_class(class, update.property())?;
        if matches!(update, GraphicsPropertyUpdate::AxesColor(_))
            && !matches!(class, GraphicsClass::Axes2D | GraphicsClass::PolarAxes)
        {
            return Err(GraphicsError::invalid_input(
                "Axes Color updates require an Axes handle",
            ));
        }
        if matches!(update, GraphicsPropertyUpdate::Color(_))
            && matches!(class, GraphicsClass::Axes2D | GraphicsClass::PolarAxes)
        {
            return Err(GraphicsError::invalid_input(
                "Axes Color updates must preserve the optional 'none' state",
            ));
        }
        if matches!(
            update,
            GraphicsPropertyUpdate::ChartMarkerFaceColor(_)
                | GraphicsPropertyUpdate::ChartMarkerEdgeColor(_)
        ) && !matches!(
            class,
            GraphicsClass::Stair | GraphicsClass::Stem | GraphicsClass::ErrorBar
        ) {
            return Err(GraphicsError::invalid_input(
                "chart marker color intent requires a line-chart handle",
            ));
        }
        if matches!(
            update,
            GraphicsPropertyUpdate::ChartFaceColor(_) | GraphicsPropertyUpdate::ChartEdgeColor(_)
        ) && !matches!(
            class,
            GraphicsClass::Area | GraphicsClass::Bar | GraphicsClass::Histogram
        ) {
            return Err(GraphicsError::invalid_input(
                "chart patch color intent requires a patch-chart handle",
            ));
        }
        match update {
            GraphicsPropertyUpdate::Position(position) if class == GraphicsClass::Figure => {
                validate_figure_position(*position)?;
            }
            GraphicsPropertyUpdate::Position(position) => validate_axes_position(*position)?,
            GraphicsPropertyUpdate::LineWidth(value)
            | GraphicsPropertyUpdate::MarkerSize(value)
            | GraphicsPropertyUpdate::FontSize(value)
                if !value.is_finite() || *value <= 0.0 =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must be a positive finite scalar",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::SizeData(value) if !value.is_finite() || *value < 0.0 => {
                return Err(GraphicsError::invalid_input(
                    "SizeData must be a nonnegative finite scalar",
                ));
            }
            GraphicsPropertyUpdate::BaseValue(value) if !value.is_finite() => {
                return Err(GraphicsError::invalid_input(
                    "BaseValue must be a finite scalar",
                ));
            }
            GraphicsPropertyUpdate::BarWidth(value)
                if !value.is_finite() || *value <= 0.0 || *value > 1.0 =>
            {
                return Err(GraphicsError::invalid_input(
                    "BarWidth must be a finite scalar in (0,1]",
                ));
            }
            GraphicsPropertyUpdate::CapSize(value) if !value.is_finite() || *value < 0.0 => {
                return Err(GraphicsError::invalid_input(
                    "CapSize must be a nonnegative finite scalar",
                ));
            }
            GraphicsPropertyUpdate::FaceAlpha(value) | GraphicsPropertyUpdate::EdgeAlpha(value)
                if !value.is_finite() || !(0.0..=1.0).contains(value) =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must be a finite scalar in [0,1]",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::Color(color)
            | GraphicsPropertyUpdate::CData(color)
            | GraphicsPropertyUpdate::MarkerFaceColor(color)
            | GraphicsPropertyUpdate::MarkerEdgeColor(color)
                if !valid_rgba(*color) =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must be finite unpremultiplied sRGB in [0,1] with opaque alpha",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::ChartMarkerFaceColor(ChartColor::Rgba(color))
            | GraphicsPropertyUpdate::ChartMarkerEdgeColor(ChartColor::Rgba(color))
            | GraphicsPropertyUpdate::ChartFaceColor(ChartColor::Rgba(color))
            | GraphicsPropertyUpdate::ChartEdgeColor(ChartColor::Rgba(color))
                if !valid_rgba(*color) =>
            {
                return Err(GraphicsError::invalid_input(
                    "chart colors must be finite unpremultiplied sRGB in [0,1] with opaque alpha",
                ));
            }
            GraphicsPropertyUpdate::ChartFaceColor(ChartColor::Auto)
            | GraphicsPropertyUpdate::ChartEdgeColor(ChartColor::Auto)
                if class != GraphicsClass::Histogram =>
            {
                return Err(GraphicsError::invalid_input(
                    "automatic patch color is supported only by Histogram",
                ));
            }
            GraphicsPropertyUpdate::ColorOrder(colors)
                if colors.is_empty() || colors.iter().any(|color| !valid_rgba(*color)) =>
            {
                return Err(GraphicsError::invalid_input(
                    "ColorOrder must contain finite RGB rows in [0,1]",
                ));
            }
            GraphicsPropertyUpdate::AxesColor(Some(color)) if !valid_rgba(*color) => {
                return Err(GraphicsError::invalid_input(
                    "Color must be finite unpremultiplied sRGB in [0,1] with opaque alpha",
                ));
            }
            GraphicsPropertyUpdate::FaceColor(SurfaceColor::Rgba(color))
            | GraphicsPropertyUpdate::EdgeColor(SurfaceColor::Rgba(color))
                if !valid_rgba(*color) =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must be finite unpremultiplied sRGB in [0,1] with opaque alpha",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::LineStyleOrder(styles) if styles.is_empty() => {
                return Err(GraphicsError::invalid_input(
                    "LineStyleOrder must contain at least one style",
                ));
            }
            GraphicsPropertyUpdate::ColorOrderIndex(0) => {
                return Err(GraphicsError::invalid_input(
                    "ColorOrderIndex must be positive",
                ));
            }
            GraphicsPropertyUpdate::MarkerIndices { values, .. } if values.contains(&0) => {
                return Err(GraphicsError::invalid_input(
                    "MarkerIndices must contain positive integer indices",
                ));
            }
            GraphicsPropertyUpdate::XLim(limits)
            | GraphicsPropertyUpdate::YLim(limits)
            | GraphicsPropertyUpdate::ZLim(limits)
            | GraphicsPropertyUpdate::CLim(limits)
                if validate_limits(*limits).is_err() =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must contain two finite strictly increasing values",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::XTick(ticks)
            | GraphicsPropertyUpdate::YTick(ticks)
            | GraphicsPropertyUpdate::ZTick(ticks)
            | GraphicsPropertyUpdate::Ticks(ticks)
                if ticks.iter().any(|tick| tick.is_nan())
                    || ticks.windows(2).any(|pair| pair[0] >= pair[1]) =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must be empty or strictly increasing without NaN",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::DataAspectRatio(values)
            | GraphicsPropertyUpdate::PlotBoxAspectRatio(values)
                if values
                    .iter()
                    .any(|value| !value.is_finite() || *value <= 0.0) =>
            {
                return Err(GraphicsError::invalid_input(format!(
                    "{} must contain three positive finite values",
                    property_name(update.property())
                )));
            }
            GraphicsPropertyUpdate::RAxisLocation(value) if !value.is_finite() => {
                return Err(GraphicsError::invalid_input(
                    "RAxisLocation must be a finite scalar",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_property_for_class(
    class: GraphicsClass,
    property: GraphicsProperty,
) -> Result<(), GraphicsError> {
    let common = matches!(
        property,
        GraphicsProperty::Parent | GraphicsProperty::Children
    );
    let supported = common
        || match class {
            GraphicsClass::Figure => matches!(
                property,
                GraphicsProperty::Name
                    | GraphicsProperty::NumberTitle
                    | GraphicsProperty::Position
                    | GraphicsProperty::Color
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::TiledChartLayout => matches!(
                property,
                GraphicsProperty::GridSize
                    | GraphicsProperty::TileSpacing
                    | GraphicsProperty::Padding
            ),
            GraphicsClass::ColorBar => matches!(
                property,
                GraphicsProperty::Visible
                    | GraphicsProperty::Ticks
                    | GraphicsProperty::TicksMode
                    | GraphicsProperty::TickLabels
                    | GraphicsProperty::TickLabelsMode
            ),
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes => matches!(
                property,
                GraphicsProperty::Position
                    | GraphicsProperty::Color
                    | GraphicsProperty::ColorOrder
                    | GraphicsProperty::LineStyleOrder
                    | GraphicsProperty::ColorOrderIndex
                    | GraphicsProperty::XLim
                    | GraphicsProperty::YLim
                    | GraphicsProperty::ZLim
                    | GraphicsProperty::XLimMode
                    | GraphicsProperty::YLimMode
                    | GraphicsProperty::ZLimMode
                    | GraphicsProperty::XScale
                    | GraphicsProperty::YScale
                    | GraphicsProperty::ZScale
                    | GraphicsProperty::XLimitMethod
                    | GraphicsProperty::YLimitMethod
                    | GraphicsProperty::ZLimitMethod
                    | GraphicsProperty::XDir
                    | GraphicsProperty::YDir
                    | GraphicsProperty::ZDir
                    | GraphicsProperty::XGrid
                    | GraphicsProperty::YGrid
                    | GraphicsProperty::ZGrid
                    | GraphicsProperty::XMinorGrid
                    | GraphicsProperty::YMinorGrid
                    | GraphicsProperty::ZMinorGrid
                    | GraphicsProperty::TickDir
                    | GraphicsProperty::XTick
                    | GraphicsProperty::YTick
                    | GraphicsProperty::ZTick
                    | GraphicsProperty::XTickMode
                    | GraphicsProperty::YTickMode
                    | GraphicsProperty::ZTickMode
                    | GraphicsProperty::XTickLabel
                    | GraphicsProperty::YTickLabel
                    | GraphicsProperty::ZTickLabel
                    | GraphicsProperty::XTickLabelMode
                    | GraphicsProperty::YTickLabelMode
                    | GraphicsProperty::ZTickLabelMode
                    | GraphicsProperty::TickLabelInterpreter
                    | GraphicsProperty::FontSize
                    | GraphicsProperty::FontName
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::Visible
                    | GraphicsProperty::Box
                    | GraphicsProperty::View
                    | GraphicsProperty::Projection
                    | GraphicsProperty::DataAspectRatio
                    | GraphicsProperty::DataAspectRatioMode
                    | GraphicsProperty::PlotBoxAspectRatio
                    | GraphicsProperty::PlotBoxAspectRatioMode
                    | GraphicsProperty::CLim
                    | GraphicsProperty::CLimMode
                    | GraphicsProperty::ThetaLim
                    | GraphicsProperty::RLim
                    | GraphicsProperty::ThetaLimMode
                    | GraphicsProperty::RLimMode
                    | GraphicsProperty::ThetaTick
                    | GraphicsProperty::RTick
                    | GraphicsProperty::ThetaTickMode
                    | GraphicsProperty::RTickMode
                    | GraphicsProperty::ThetaTickLabel
                    | GraphicsProperty::RTickLabel
                    | GraphicsProperty::ThetaTickLabelMode
                    | GraphicsProperty::RTickLabelMode
                    | GraphicsProperty::ThetaAxisUnits
                    | GraphicsProperty::ThetaDir
                    | GraphicsProperty::ThetaZeroLocation
                    | GraphicsProperty::RAxisLocation
                    | GraphicsProperty::ThetaGrid
                    | GraphicsProperty::RGrid
                    | GraphicsProperty::ThetaMinorGrid
                    | GraphicsProperty::RMinorGrid
            ),
            GraphicsClass::LineSeries => matches!(
                property,
                GraphicsProperty::LineWidth
                    | GraphicsProperty::Color
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Marker
                    | GraphicsProperty::MarkerSize
                    | GraphicsProperty::MarkerIndices
                    | GraphicsProperty::MarkerFaceColor
                    | GraphicsProperty::MarkerEdgeColor
                    | GraphicsProperty::DisplayName
                    | GraphicsProperty::Clipping
                    | GraphicsProperty::Visible
                    | GraphicsProperty::XData
                    | GraphicsProperty::YData
                    | GraphicsProperty::ZData
                    | GraphicsProperty::ThetaData
                    | GraphicsProperty::RData
            ),
            GraphicsClass::ScatterSeries => matches!(
                property,
                GraphicsProperty::Marker
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::Visible
                    | GraphicsProperty::SizeData
                    | GraphicsProperty::CData
                    | GraphicsProperty::MarkerFaceColor
                    | GraphicsProperty::MarkerEdgeColor
                    | GraphicsProperty::DisplayName
                    | GraphicsProperty::Clipping
                    | GraphicsProperty::XData
                    | GraphicsProperty::YData
                    | GraphicsProperty::ZData
            ),
            GraphicsClass::SurfaceSeries => matches!(
                property,
                GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Visible
                    | GraphicsProperty::XData
                    | GraphicsProperty::YData
                    | GraphicsProperty::ZData
                    | GraphicsProperty::CData
                    | GraphicsProperty::FaceColor
                    | GraphicsProperty::EdgeColor
                    | GraphicsProperty::CDataMapping
                    | GraphicsProperty::FaceAlpha
            ),
            GraphicsClass::PatchSeries => matches!(
                property,
                GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Visible
                    | GraphicsProperty::XData
                    | GraphicsProperty::YData
                    | GraphicsProperty::ZData
                    | GraphicsProperty::CData
                    | GraphicsProperty::Faces
                    | GraphicsProperty::Vertices
                    | GraphicsProperty::FaceVertexCData
                    | GraphicsProperty::FaceColor
                    | GraphicsProperty::EdgeColor
                    | GraphicsProperty::CDataMapping
                    | GraphicsProperty::FaceAlpha
                    | GraphicsProperty::EdgeAlpha
            ),
            GraphicsClass::Stair => matches!(
                property,
                GraphicsProperty::Color
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Marker
                    | GraphicsProperty::MarkerSize
                    | GraphicsProperty::MarkerFaceColor
                    | GraphicsProperty::MarkerEdgeColor
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Stem => matches!(
                property,
                GraphicsProperty::Color
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Marker
                    | GraphicsProperty::MarkerSize
                    | GraphicsProperty::MarkerFaceColor
                    | GraphicsProperty::MarkerEdgeColor
                    | GraphicsProperty::BaseValue
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::ErrorBar => matches!(
                property,
                GraphicsProperty::Color
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::Marker
                    | GraphicsProperty::MarkerSize
                    | GraphicsProperty::MarkerFaceColor
                    | GraphicsProperty::MarkerEdgeColor
                    | GraphicsProperty::CapSize
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Area => matches!(
                property,
                GraphicsProperty::FaceColor
                    | GraphicsProperty::EdgeColor
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::FaceAlpha
                    | GraphicsProperty::EdgeAlpha
                    | GraphicsProperty::BaseValue
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Bar => matches!(
                property,
                GraphicsProperty::FaceColor
                    | GraphicsProperty::EdgeColor
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::FaceAlpha
                    | GraphicsProperty::EdgeAlpha
                    | GraphicsProperty::BaseValue
                    | GraphicsProperty::BarWidth
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Histogram => matches!(
                property,
                GraphicsProperty::FaceColor
                    | GraphicsProperty::EdgeColor
                    | GraphicsProperty::LineWidth
                    | GraphicsProperty::LineStyle
                    | GraphicsProperty::FaceAlpha
                    | GraphicsProperty::EdgeAlpha
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Contour | GraphicsClass::Image => {
                matches!(property, GraphicsProperty::Visible)
            }
            GraphicsClass::Text => matches!(
                property,
                GraphicsProperty::String
                    | GraphicsProperty::Color
                    | GraphicsProperty::FontSize
                    | GraphicsProperty::FontName
                    | GraphicsProperty::FontWeight
                    | GraphicsProperty::Rotation
                    | GraphicsProperty::Interpreter
                    | GraphicsProperty::Visible
            ),
            GraphicsClass::Legend => matches!(
                property,
                GraphicsProperty::String
                    | GraphicsProperty::Location
                    | GraphicsProperty::FontSize
                    | GraphicsProperty::FontName
                    | GraphicsProperty::FontWeight
                    | GraphicsProperty::Orientation
                    | GraphicsProperty::NumColumns
                    | GraphicsProperty::Box
                    | GraphicsProperty::Interpreter
                    | GraphicsProperty::Visible
            ),
        };
    if supported {
        Ok(())
    } else {
        Err(GraphicsError::invalid_input(format!(
            "{} does not define property {}",
            class.class_name(),
            property_name(property)
        )))
    }
}

#[allow(clippy::too_many_lines)]
fn property_name(property: GraphicsProperty) -> &'static str {
    match property {
        GraphicsProperty::Parent => "Parent",
        GraphicsProperty::Children => "Children",
        GraphicsProperty::Position => "Position",
        GraphicsProperty::Name => "Name",
        GraphicsProperty::NumberTitle => "NumberTitle",
        GraphicsProperty::GridSize => "GridSize",
        GraphicsProperty::TileSpacing => "TileSpacing",
        GraphicsProperty::Padding => "Padding",
        GraphicsProperty::LineWidth => "LineWidth",
        GraphicsProperty::Color => "Color",
        GraphicsProperty::LineStyle => "LineStyle",
        GraphicsProperty::Marker => "Marker",
        GraphicsProperty::MarkerSize => "MarkerSize",
        GraphicsProperty::MarkerIndices => "MarkerIndices",
        GraphicsProperty::DisplayName => "DisplayName",
        GraphicsProperty::Clipping => "Clipping",
        GraphicsProperty::Visible => "Visible",
        GraphicsProperty::SizeData => "SizeData",
        GraphicsProperty::CData => "CData",
        GraphicsProperty::MarkerFaceColor => "MarkerFaceColor",
        GraphicsProperty::MarkerEdgeColor => "MarkerEdgeColor",
        GraphicsProperty::BaseValue => "BaseValue",
        GraphicsProperty::BarWidth => "BarWidth",
        GraphicsProperty::CapSize => "CapSize",
        GraphicsProperty::ColorOrder => "ColorOrder",
        GraphicsProperty::LineStyleOrder => "LineStyleOrder",
        GraphicsProperty::ColorOrderIndex => "ColorOrderIndex",
        GraphicsProperty::XLim => "XLim",
        GraphicsProperty::YLim => "YLim",
        GraphicsProperty::ZLim => "ZLim",
        GraphicsProperty::XLimMode => "XLimMode",
        GraphicsProperty::YLimMode => "YLimMode",
        GraphicsProperty::ZLimMode => "ZLimMode",
        GraphicsProperty::ThetaLim => "ThetaLim",
        GraphicsProperty::RLim => "RLim",
        GraphicsProperty::ThetaLimMode => "ThetaLimMode",
        GraphicsProperty::RLimMode => "RLimMode",
        GraphicsProperty::XScale => "XScale",
        GraphicsProperty::YScale => "YScale",
        GraphicsProperty::ZScale => "ZScale",
        GraphicsProperty::XLimitMethod => "XLimitMethod",
        GraphicsProperty::YLimitMethod => "YLimitMethod",
        GraphicsProperty::ZLimitMethod => "ZLimitMethod",
        GraphicsProperty::XDir => "XDir",
        GraphicsProperty::YDir => "YDir",
        GraphicsProperty::ZDir => "ZDir",
        GraphicsProperty::XTick => "XTick",
        GraphicsProperty::YTick => "YTick",
        GraphicsProperty::ZTick => "ZTick",
        GraphicsProperty::XTickMode => "XTickMode",
        GraphicsProperty::YTickMode => "YTickMode",
        GraphicsProperty::ZTickMode => "ZTickMode",
        GraphicsProperty::XTickLabel => "XTickLabel",
        GraphicsProperty::YTickLabel => "YTickLabel",
        GraphicsProperty::ZTickLabel => "ZTickLabel",
        GraphicsProperty::XTickLabelMode => "XTickLabelMode",
        GraphicsProperty::YTickLabelMode => "YTickLabelMode",
        GraphicsProperty::ZTickLabelMode => "ZTickLabelMode",
        GraphicsProperty::ThetaTick => "ThetaTick",
        GraphicsProperty::RTick => "RTick",
        GraphicsProperty::ThetaTickMode => "ThetaTickMode",
        GraphicsProperty::RTickMode => "RTickMode",
        GraphicsProperty::ThetaTickLabel => "ThetaTickLabel",
        GraphicsProperty::RTickLabel => "RTickLabel",
        GraphicsProperty::ThetaTickLabelMode => "ThetaTickLabelMode",
        GraphicsProperty::RTickLabelMode => "RTickLabelMode",
        GraphicsProperty::ThetaAxisUnits => "ThetaAxisUnits",
        GraphicsProperty::ThetaDir => "ThetaDir",
        GraphicsProperty::ThetaZeroLocation => "ThetaZeroLocation",
        GraphicsProperty::RAxisLocation => "RAxisLocation",
        GraphicsProperty::ThetaGrid => "ThetaGrid",
        GraphicsProperty::RGrid => "RGrid",
        GraphicsProperty::ThetaMinorGrid => "ThetaMinorGrid",
        GraphicsProperty::RMinorGrid => "RMinorGrid",
        GraphicsProperty::Ticks => "Ticks",
        GraphicsProperty::TicksMode => "TicksMode",
        GraphicsProperty::TickLabels => "TickLabels",
        GraphicsProperty::TickLabelsMode => "TickLabelsMode",
        GraphicsProperty::TickLabelInterpreter => "TickLabelInterpreter",
        GraphicsProperty::Box => "Box",
        GraphicsProperty::XGrid => "XGrid",
        GraphicsProperty::YGrid => "YGrid",
        GraphicsProperty::ZGrid => "ZGrid",
        GraphicsProperty::XMinorGrid => "XMinorGrid",
        GraphicsProperty::YMinorGrid => "YMinorGrid",
        GraphicsProperty::ZMinorGrid => "ZMinorGrid",
        GraphicsProperty::TickDir => "TickDir",
        GraphicsProperty::XData => "XData",
        GraphicsProperty::YData => "YData",
        GraphicsProperty::ZData => "ZData",
        GraphicsProperty::ThetaData => "ThetaData",
        GraphicsProperty::RData => "RData",
        GraphicsProperty::View => "View",
        GraphicsProperty::Projection => "Projection",
        GraphicsProperty::DataAspectRatio => "DataAspectRatio",
        GraphicsProperty::DataAspectRatioMode => "DataAspectRatioMode",
        GraphicsProperty::PlotBoxAspectRatio => "PlotBoxAspectRatio",
        GraphicsProperty::PlotBoxAspectRatioMode => "PlotBoxAspectRatioMode",
        GraphicsProperty::CLim => "CLim",
        GraphicsProperty::CLimMode => "CLimMode",
        GraphicsProperty::FaceColor => "FaceColor",
        GraphicsProperty::EdgeColor => "EdgeColor",
        GraphicsProperty::CDataMapping => "CDataMapping",
        GraphicsProperty::FaceAlpha => "FaceAlpha",
        GraphicsProperty::EdgeAlpha => "EdgeAlpha",
        GraphicsProperty::Faces => "Faces",
        GraphicsProperty::Vertices => "Vertices",
        GraphicsProperty::FaceVertexCData => "FaceVertexCData",
        GraphicsProperty::String => "String",
        GraphicsProperty::Location => "Location",
        GraphicsProperty::FontSize => "FontSize",
        GraphicsProperty::FontName => "FontName",
        GraphicsProperty::FontWeight => "FontWeight",
        GraphicsProperty::Rotation => "Rotation",
        GraphicsProperty::Orientation => "Orientation",
        GraphicsProperty::NumColumns => "NumColumns",
        GraphicsProperty::Interpreter => "Interpreter",
    }
}

fn valid_rgba(color: [f32; 4]) -> bool {
    color
        .iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
        && color[3].to_bits() == 1.0_f32.to_bits()
}

#[allow(clippy::cast_possible_truncation)]
fn numeric_like(dtype: DataDType, values: Vec<f64>) -> NumericData {
    match dtype {
        DataDType::F32 => NumericData::from_f32(Arc::<[f32]>::from(
            values
                .into_iter()
                .map(|value| value as f32)
                .collect::<Vec<_>>(),
        )),
        DataDType::F64 => NumericData::from_f64(Arc::<[f64]>::from(values)),
    }
}

fn finite_data_span(values: &[f64]) -> Option<f64> {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for value in values.iter().copied().filter(|value| value.is_finite()) {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    (minimum.is_finite() && maximum.is_finite() && maximum > minimum).then_some(maximum - minimum)
}

fn validate_figure_position(position: [f64; 4]) -> Result<(), GraphicsError> {
    if position.iter().any(|value| !value.is_finite()) || position[2] <= 0.0 || position[3] <= 0.0 {
        return Err(GraphicsError::invalid_input(
            "Figure Position must be finite with positive width and height",
        ));
    }
    Ok(())
}

fn validate_axes_position(position: [f64; 4]) -> Result<(), GraphicsError> {
    let [left, bottom, width, height] = position;
    if position.iter().any(|value| !value.is_finite())
        || left < 0.0
        || bottom < 0.0
        || width <= 0.0
        || height <= 0.0
        || left + width > 1.0
        || bottom + height > 1.0
    {
        return Err(GraphicsError::invalid_input(
            "Position must be a finite normalized [left bottom width height] rectangle inside the Figure",
        ));
    }
    Ok(())
}

fn same_axes_position(left: [f64; 4], right: [f64; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| (left - right).abs() <= 1.0e-12)
}

fn axes_positions_overlap(left: [f64; 4], right: [f64; 4]) -> bool {
    let left_right = left[0] + left[2];
    let left_top = left[1] + left[3];
    let right_right = right[0] + right[2];
    let right_top = right[1] + right[3];
    left[0] < right_right && right[0] < left_right && left[1] < right_top && right[1] < left_top
}

fn apply_line_updates(properties: &mut LineSeriesProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::LineWidth(value) => properties.line_width_points = *value,
            GraphicsPropertyUpdate::Color(value) => {
                properties.color_rgba = *value;
                if properties.marker_edge_color_automatic {
                    properties.marker_edge_color_rgba = *value;
                }
            }
            GraphicsPropertyUpdate::LineStyle(value) => properties.line_style = *value,
            GraphicsPropertyUpdate::Marker(value) => properties.marker = *value,
            GraphicsPropertyUpdate::MarkerSize(value) => properties.marker_size_points = *value,
            GraphicsPropertyUpdate::MarkerIndices {
                values,
                value_class,
            } => {
                properties.marker_indices.clone_from(values);
                properties.marker_indices_class = *value_class;
                properties.marker_indices_automatic = false;
            }
            GraphicsPropertyUpdate::MarkerFaceColor(value) => {
                properties.marker_face_color_rgba = *value;
            }
            GraphicsPropertyUpdate::MarkerEdgeColor(value) => {
                properties.marker_edge_color_rgba = *value;
                properties.marker_edge_color_automatic = false;
            }
            GraphicsPropertyUpdate::DisplayName(value) => {
                properties.display_name_utf16.clone_from(value);
            }
            GraphicsPropertyUpdate::Clipping(value) => properties.clipping = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            _ => {}
        }
    }
}

fn apply_chart_line_color_updates(
    properties: &mut LineSeriesProperties,
    updates: &[GraphicsPropertyUpdate],
) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::ChartMarkerFaceColor(value) => {
                properties.marker_face_color_rgba = chart_color_rgba(*value, properties.color_rgba);
            }
            GraphicsPropertyUpdate::ChartMarkerEdgeColor(value) => {
                properties.marker_edge_color_rgba = chart_color_rgba(*value, properties.color_rgba);
                properties.marker_edge_color_automatic = *value == ChartColor::Auto;
            }
            _ => {}
        }
    }
}

fn apply_scatter_updates(
    properties: &mut ScatterSeriesProperties,
    updates: &[GraphicsPropertyUpdate],
) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::Marker(value) => properties.marker = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::SizeData(value) => properties.marker_size_points = value.sqrt(),
            GraphicsPropertyUpdate::CData(value) => {
                properties.face_color_rgba = *value;
                properties.edge_color_rgba = *value;
            }
            GraphicsPropertyUpdate::MarkerFaceColor(value) => properties.face_color_rgba = *value,
            GraphicsPropertyUpdate::MarkerEdgeColor(value) => properties.edge_color_rgba = *value,
            GraphicsPropertyUpdate::DisplayName(value) => {
                properties.display_name_utf16.clone_from(value);
            }
            GraphicsPropertyUpdate::Clipping(value) => properties.clipping = *value,
            _ => {}
        }
    }
}

fn apply_surface_updates(
    properties: &mut SurfaceSeriesProperties,
    updates: &[GraphicsPropertyUpdate],
) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::LineWidth(value) => properties.line_width_points = *value,
            GraphicsPropertyUpdate::LineStyle(value) => properties.line_style = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::FaceColor(value) => properties.face_color = *value,
            GraphicsPropertyUpdate::EdgeColor(value) => properties.edge_color = *value,
            GraphicsPropertyUpdate::CDataMapping(value) => properties.c_data_mapping = *value,
            GraphicsPropertyUpdate::FaceAlpha(value) => properties.face_alpha = *value,
            _ => {}
        }
    }
}

fn apply_patch_updates(properties: &mut PatchSeriesProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::LineWidth(value) => properties.line_width_points = *value,
            GraphicsPropertyUpdate::LineStyle(value) => properties.line_style = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::FaceColor(value) => properties.face_color = *value,
            GraphicsPropertyUpdate::EdgeColor(value) => properties.edge_color = *value,
            GraphicsPropertyUpdate::CDataMapping(value) => properties.c_data_mapping = *value,
            GraphicsPropertyUpdate::FaceAlpha(value) => properties.face_alpha = *value,
            GraphicsPropertyUpdate::EdgeAlpha(value) => properties.edge_alpha = *value,
            _ => {}
        }
    }
}

fn apply_chart_patch_color_updates(
    properties: &mut PatchSeriesProperties,
    chart: &ChartGroupProperties,
    updates: &[GraphicsPropertyUpdate],
) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::ChartFaceColor(value) => {
                properties.face_color = chart_surface_color(
                    *value,
                    chart.automatic_face_color_rgba,
                    properties.face_color,
                );
            }
            GraphicsPropertyUpdate::ChartEdgeColor(value) => {
                properties.edge_color = chart_surface_color(
                    *value,
                    chart.automatic_edge_color_rgba,
                    properties.edge_color,
                );
            }
            _ => {}
        }
    }
}

const fn chart_color_rgba(value: ChartColor, automatic: [f32; 4]) -> [f32; 4] {
    match value {
        ChartColor::Auto => automatic,
        ChartColor::None => [0.0, 0.0, 0.0, 0.0],
        ChartColor::Rgba(color) => color,
    }
}

const fn chart_surface_color(
    value: ChartColor,
    automatic_rgba: Option<[f32; 4]>,
    current: SurfaceColor,
) -> SurfaceColor {
    match value {
        ChartColor::Auto => match automatic_rgba {
            Some(color) => SurfaceColor::Rgba(color),
            None => current,
        },
        ChartColor::None => SurfaceColor::None,
        ChartColor::Rgba(color) => SurfaceColor::Rgba(color),
    }
}

#[allow(clippy::too_many_lines)] // Keep the property reducer exhaustive and mechanically auditable.
fn apply_axes_updates(properties: &mut Axes2DProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        if apply_axes_aspect_update(properties, update) {
            continue;
        }
        match update {
            GraphicsPropertyUpdate::Position(value) => properties.position_normalized = *value,
            GraphicsPropertyUpdate::AxesColor(value) => properties.background_rgba = *value,
            GraphicsPropertyUpdate::ColorOrder(value) => properties.color_order.clone_from(value),
            GraphicsPropertyUpdate::LineStyleOrder(value) => {
                properties.line_style_order.clone_from(value);
            }
            GraphicsPropertyUpdate::ColorOrderIndex(value) => properties.color_order_index = *value,
            GraphicsPropertyUpdate::XLim(value) => {
                properties.x_limits = *value;
                properties.x_limit_mode = LimitMode::Manual;
            }
            GraphicsPropertyUpdate::YLim(value) => {
                properties.y_limits = *value;
                properties.y_limit_mode = LimitMode::Manual;
            }
            GraphicsPropertyUpdate::ZLim(value) => {
                properties.z_limits = *value;
                properties.z_limit_mode = LimitMode::Manual;
            }
            GraphicsPropertyUpdate::XLimMode(value) => properties.x_limit_mode = *value,
            GraphicsPropertyUpdate::YLimMode(value) => properties.y_limit_mode = *value,
            GraphicsPropertyUpdate::ZLimMode(value) => properties.z_limit_mode = *value,
            GraphicsPropertyUpdate::XScale(value) => properties.x_scale = *value,
            GraphicsPropertyUpdate::YScale(value) => properties.y_scale = *value,
            GraphicsPropertyUpdate::ZScale(value) => properties.z_scale = *value,
            GraphicsPropertyUpdate::XLimitMethod(value) => properties.x_limit_method = *value,
            GraphicsPropertyUpdate::YLimitMethod(value) => properties.y_limit_method = *value,
            GraphicsPropertyUpdate::ZLimitMethod(value) => properties.z_limit_method = *value,
            GraphicsPropertyUpdate::XDir(value) => properties.x_direction = *value,
            GraphicsPropertyUpdate::YDir(value) => properties.y_direction = *value,
            GraphicsPropertyUpdate::ZDir(value) => properties.z_direction = *value,
            GraphicsPropertyUpdate::XTick(value) => {
                properties.x_ticks.clone_from(value);
                properties.x_tick_mode = TickMode::Manual;
                if properties.x_tick_label_mode == TickMode::Auto {
                    properties.x_tick_labels_utf16 =
                        automatic_axis_tick_labels(value, properties.x_scale);
                }
            }
            GraphicsPropertyUpdate::YTick(value) => {
                properties.y_ticks.clone_from(value);
                properties.y_tick_mode = TickMode::Manual;
                if properties.y_tick_label_mode == TickMode::Auto {
                    properties.y_tick_labels_utf16 =
                        automatic_axis_tick_labels(value, properties.y_scale);
                }
            }
            GraphicsPropertyUpdate::ZTick(value) => {
                properties.z_ticks.clone_from(value);
                properties.z_tick_mode = TickMode::Manual;
                if properties.z_tick_label_mode == TickMode::Auto {
                    properties.z_tick_labels_utf16 =
                        automatic_axis_tick_labels(value, properties.z_scale);
                }
            }
            GraphicsPropertyUpdate::XTickMode(value) => {
                properties.x_tick_mode = *value;
                if *value == TickMode::Auto {
                    properties.x_ticks =
                        automatic_axis_ticks(properties.x_limits, properties.x_scale);
                    if properties.x_tick_label_mode == TickMode::Auto {
                        properties.x_tick_labels_utf16 =
                            automatic_axis_tick_labels(&properties.x_ticks, properties.x_scale);
                    }
                }
            }
            GraphicsPropertyUpdate::YTickMode(value) => {
                properties.y_tick_mode = *value;
                if *value == TickMode::Auto {
                    properties.y_ticks =
                        automatic_axis_ticks(properties.y_limits, properties.y_scale);
                    if properties.y_tick_label_mode == TickMode::Auto {
                        properties.y_tick_labels_utf16 =
                            automatic_axis_tick_labels(&properties.y_ticks, properties.y_scale);
                    }
                }
            }
            GraphicsPropertyUpdate::ZTickMode(value) => {
                properties.z_tick_mode = *value;
                if *value == TickMode::Auto {
                    properties.z_ticks =
                        automatic_axis_ticks(properties.z_limits, properties.z_scale);
                    if properties.z_tick_label_mode == TickMode::Auto {
                        properties.z_tick_labels_utf16 =
                            automatic_axis_tick_labels(&properties.z_ticks, properties.z_scale);
                    }
                }
            }
            GraphicsPropertyUpdate::XTickLabel(value) => {
                if properties.coordinate_system == AxesCoordinateSystem::Polar {
                    properties.x_tick_labels_utf16 =
                        cycle_tick_labels(value, properties.x_ticks.len());
                } else {
                    properties.x_tick_labels_utf16.clone_from(value);
                }
                properties.x_tick_label_mode = TickMode::Manual;
            }
            GraphicsPropertyUpdate::YTickLabel(value) => {
                if properties.coordinate_system == AxesCoordinateSystem::Polar {
                    properties.y_tick_labels_utf16 =
                        cycle_tick_labels(value, properties.y_ticks.len());
                } else {
                    properties.y_tick_labels_utf16.clone_from(value);
                }
                properties.y_tick_label_mode = TickMode::Manual;
            }
            GraphicsPropertyUpdate::ZTickLabel(value) => {
                properties.z_tick_labels_utf16.clone_from(value);
                properties.z_tick_label_mode = TickMode::Manual;
            }
            GraphicsPropertyUpdate::XTickLabelMode(value) => {
                properties.x_tick_label_mode = *value;
                if *value == TickMode::Auto {
                    properties.x_tick_labels_utf16 =
                        automatic_axis_tick_labels(&properties.x_ticks, properties.x_scale);
                }
            }
            GraphicsPropertyUpdate::YTickLabelMode(value) => {
                properties.y_tick_label_mode = *value;
                if *value == TickMode::Auto {
                    properties.y_tick_labels_utf16 =
                        automatic_axis_tick_labels(&properties.y_ticks, properties.y_scale);
                }
            }
            GraphicsPropertyUpdate::ZTickLabelMode(value) => {
                properties.z_tick_label_mode = *value;
                if *value == TickMode::Auto {
                    properties.z_tick_labels_utf16 =
                        automatic_axis_tick_labels(&properties.z_ticks, properties.z_scale);
                }
            }
            GraphicsPropertyUpdate::TickLabelInterpreter(value) => {
                properties.tick_label_interpreter = *value;
            }
            GraphicsPropertyUpdate::FontSize(value) => properties.font_size_points = *value,
            GraphicsPropertyUpdate::FontName(value) => {
                properties.font_family_utf16.clone_from(value);
            }
            GraphicsPropertyUpdate::TickDir(value) => properties.tick_direction = *value,
            GraphicsPropertyUpdate::ThetaAxisUnits(value) => {
                if properties.theta_axis_units != *value {
                    let factor = match (properties.theta_axis_units, value) {
                        (ThetaAxisUnits::Degrees, ThetaAxisUnits::Radians) => {
                            std::f64::consts::PI / 180.0
                        }
                        (ThetaAxisUnits::Radians, ThetaAxisUnits::Degrees) => {
                            180.0 / std::f64::consts::PI
                        }
                        _ => 1.0,
                    };
                    properties.x_limits = properties.x_limits.map(|item| item * factor);
                    properties
                        .x_ticks
                        .iter_mut()
                        .for_each(|item| *item *= factor);
                    properties.r_axis_location *= factor;
                    properties.theta_axis_units = *value;
                    if properties.x_tick_label_mode == TickMode::Auto {
                        properties.x_tick_labels_utf16 = automatic_polar_theta_labels(
                            &properties.x_ticks,
                            properties.x_limits,
                            properties.theta_axis_units,
                        );
                    }
                }
            }
            GraphicsPropertyUpdate::ThetaDir(value) => properties.theta_direction = *value,
            GraphicsPropertyUpdate::ThetaZeroLocation(value) => {
                properties.theta_zero_location = *value;
            }
            GraphicsPropertyUpdate::RAxisLocation(value) => properties.r_axis_location = *value,
            GraphicsPropertyUpdate::XGrid(value) => properties.grid_x = *value,
            GraphicsPropertyUpdate::YGrid(value) => properties.grid_y = *value,
            GraphicsPropertyUpdate::ZGrid(value) => properties.grid_z = *value,
            GraphicsPropertyUpdate::XMinorGrid(value) => properties.minor_grid_x = *value,
            GraphicsPropertyUpdate::YMinorGrid(value) => properties.minor_grid_y = *value,
            GraphicsPropertyUpdate::ZMinorGrid(value) => properties.minor_grid_z = *value,
            GraphicsPropertyUpdate::LineWidth(value) => properties.line_width_points = *value,
            GraphicsPropertyUpdate::Box(value) => properties.box_enabled = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::Projection(value) => properties.projection = *value,
            GraphicsPropertyUpdate::CLim(value) => {
                properties.c_limits = *value;
                properties.c_limit_mode = LimitMode::Manual;
            }
            GraphicsPropertyUpdate::CLimMode(value) => properties.c_limit_mode = *value,
            _ => {}
        }
    }
}

fn apply_axes_aspect_update(
    properties: &mut Axes2DProperties,
    update: &GraphicsPropertyUpdate,
) -> bool {
    match update {
        GraphicsPropertyUpdate::DataAspectRatio(value) => {
            properties.data_aspect_ratio = *value;
            properties.data_aspect_ratio_mode = LimitMode::Manual;
        }
        GraphicsPropertyUpdate::DataAspectRatioMode(value) => {
            properties.data_aspect_ratio_mode = *value;
        }
        GraphicsPropertyUpdate::PlotBoxAspectRatio(value) => {
            properties.plot_box_aspect_ratio = *value;
            properties.plot_box_aspect_ratio_mode = LimitMode::Manual;
        }
        GraphicsPropertyUpdate::PlotBoxAspectRatioMode(value) => {
            properties.plot_box_aspect_ratio_mode = *value;
        }
        _ => return false,
    }
    true
}

fn apply_text_updates(properties: &mut TextProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::Color(value) => properties.color_rgba = *value,
            GraphicsPropertyUpdate::FontSize(value) => properties.font_size_css_px = *value,
            GraphicsPropertyUpdate::FontName(value) => {
                properties.font_family_utf16.clone_from(value);
            }
            GraphicsPropertyUpdate::FontWeight(value) => properties.font_weight = *value,
            GraphicsPropertyUpdate::Rotation(value) => properties.rotation_degrees = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::Interpreter(value) => properties.interpreter = *value,
            _ => {}
        }
    }
}

fn apply_legend_updates(properties: &mut LegendProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::FontSize(value) => properties.font_size_css_px = *value,
            GraphicsPropertyUpdate::FontName(value) => {
                properties.font_family_utf16.clone_from(value);
            }
            GraphicsPropertyUpdate::FontWeight(value) => properties.font_weight = *value,
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::Location(value) => properties.location = *value,
            GraphicsPropertyUpdate::Orientation(value) => properties.orientation = *value,
            GraphicsPropertyUpdate::NumColumns(value) => properties.num_columns = *value,
            GraphicsPropertyUpdate::Box(value) => {
                properties.box_enabled = *value;
                properties.border_rgba[3] = if *value { 1.0 } else { 0.0 };
            }
            GraphicsPropertyUpdate::Interpreter(value) => properties.interpreter = *value,
            _ => {}
        }
    }
}

fn apply_colorbar_updates(properties: &mut ColorBarProperties, updates: &[GraphicsPropertyUpdate]) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            GraphicsPropertyUpdate::Ticks(value) => {
                properties.ticks.clone_from(value);
                properties.tick_mode = TickMode::Manual;
                if properties.tick_label_mode == TickMode::Auto {
                    properties.tick_labels_utf16 = automatic_tick_labels(value);
                }
            }
            GraphicsPropertyUpdate::TicksMode(value) => properties.tick_mode = *value,
            GraphicsPropertyUpdate::TickLabels(value) => {
                properties.tick_labels_utf16.clone_from(value);
                properties.tick_label_mode = TickMode::Manual;
            }
            GraphicsPropertyUpdate::TickLabelsMode(value) => {
                properties.tick_label_mode = *value;
                if *value == TickMode::Auto {
                    properties.tick_labels_utf16 = automatic_tick_labels(&properties.ticks);
                }
            }
            _ => {}
        }
    }
}

fn default_chart_group_properties(kind: ChartGroupKind) -> ChartGroupProperties {
    let line_chart = matches!(
        kind,
        ChartGroupKind::Stair | ChartGroupKind::Stem | ChartGroupKind::ErrorBar
    );
    let patch_chart = matches!(
        kind,
        ChartGroupKind::Area | ChartGroupKind::Bar | ChartGroupKind::Histogram
    );
    ChartGroupProperties {
        kind,
        visible: true,
        color: line_chart.then_some(ChartColor::Auto),
        line_width_points: 0.5,
        line_style: LineStyle::Solid,
        marker: line_chart.then_some(if kind == ChartGroupKind::Stem {
            Marker::Circle
        } else {
            Marker::None
        }),
        marker_size_points: line_chart.then_some(6.0),
        marker_face_color: line_chart.then_some(ChartColor::None),
        marker_edge_color: line_chart.then_some(ChartColor::Auto),
        face_color: patch_chart.then_some(if kind == ChartGroupKind::Histogram {
            ChartColor::Auto
        } else {
            ChartColor::Rgba([0.0, 0.447, 0.741, 1.0])
        }),
        edge_color: patch_chart.then_some(ChartColor::Rgba([0.0, 0.0, 0.0, 1.0])),
        automatic_face_color_rgba: None,
        automatic_edge_color_rgba: None,
        face_alpha: patch_chart.then_some(if kind == ChartGroupKind::Histogram {
            0.6
        } else {
            1.0
        }),
        edge_alpha: patch_chart.then_some(1.0),
        base_value: matches!(
            kind,
            ChartGroupKind::Stem | ChartGroupKind::Area | ChartGroupKind::Bar
        )
        .then_some(0.0),
        bar_width: (kind == ChartGroupKind::Bar).then_some(0.8),
        cap_size_points: (kind == ChartGroupKind::ErrorBar).then_some(6.0),
    }
}

fn apply_chart_group_updates(
    properties: &mut ChartGroupProperties,
    updates: &[GraphicsPropertyUpdate],
) {
    for update in updates {
        match update {
            GraphicsPropertyUpdate::Color(value) => {
                properties.color = Some(ChartColor::Rgba(*value));
            }
            GraphicsPropertyUpdate::LineWidth(value) => properties.line_width_points = *value,
            GraphicsPropertyUpdate::LineStyle(value) => properties.line_style = *value,
            GraphicsPropertyUpdate::Marker(value) => properties.marker = Some(*value),
            GraphicsPropertyUpdate::MarkerSize(value) => {
                properties.marker_size_points = Some(*value);
            }
            GraphicsPropertyUpdate::MarkerFaceColor(value) => {
                properties.marker_face_color = Some(ChartColor::Rgba(*value));
            }
            GraphicsPropertyUpdate::MarkerEdgeColor(value) => {
                properties.marker_edge_color = Some(ChartColor::Rgba(*value));
            }
            GraphicsPropertyUpdate::ChartMarkerFaceColor(value) => {
                properties.marker_face_color = Some(*value);
            }
            GraphicsPropertyUpdate::ChartMarkerEdgeColor(value) => {
                properties.marker_edge_color = Some(*value);
            }
            GraphicsPropertyUpdate::FaceColor(value) => {
                properties.face_color = Some(chart_color_from_surface(*value));
            }
            GraphicsPropertyUpdate::EdgeColor(value) => {
                properties.edge_color = Some(chart_color_from_surface(*value));
            }
            GraphicsPropertyUpdate::ChartFaceColor(value) => properties.face_color = Some(*value),
            GraphicsPropertyUpdate::ChartEdgeColor(value) => properties.edge_color = Some(*value),
            GraphicsPropertyUpdate::FaceAlpha(value) => properties.face_alpha = Some(*value),
            GraphicsPropertyUpdate::EdgeAlpha(value) => properties.edge_alpha = Some(*value),
            GraphicsPropertyUpdate::BaseValue(value) => properties.base_value = Some(*value),
            GraphicsPropertyUpdate::BarWidth(value) => properties.bar_width = Some(*value),
            GraphicsPropertyUpdate::CapSize(value) => properties.cap_size_points = Some(*value),
            GraphicsPropertyUpdate::Visible(value) => properties.visible = *value,
            _ => {}
        }
    }
}

const fn chart_color_from_surface(value: SurfaceColor) -> ChartColor {
    match value {
        SurfaceColor::None => ChartColor::None,
        SurfaceColor::Rgba(color) => ChartColor::Rgba(color),
        SurfaceColor::Flat | SurfaceColor::Interp => ChartColor::Auto,
    }
}

fn chart_color_value(value: ChartColor) -> GraphicsPropertyValue {
    match value {
        ChartColor::Auto => GraphicsPropertyValue::Text("auto".encode_utf16().collect()),
        ChartColor::None => GraphicsPropertyValue::Text("none".encode_utf16().collect()),
        ChartColor::Rgba(color) => GraphicsPropertyValue::Color(color),
    }
}

#[allow(clippy::too_many_lines)]
fn property_value(
    session: &GraphicsSession,
    handle: GraphicsHandle,
    property: GraphicsProperty,
) -> Result<GraphicsPropertyValue, GraphicsError> {
    let node = session.node(handle)?;
    if property == GraphicsProperty::Parent {
        let parent = if let GraphicsObject::TiledChartLayout(properties) = &node.object {
            properties.parent
        } else if matches!(
            handle.class(),
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes
        ) {
            if let Some(layout) = session.layout_for_axes(handle) {
                layout
            } else {
                node.parent
                    .ok_or_else(|| GraphicsError::invalid_state("Axes has no Figure parent"))?
            }
        } else if matches!(
            handle.class(),
            GraphicsClass::Legend | GraphicsClass::ColorBar
        ) {
            session.figure_for_handle(handle)?
        } else {
            node.parent
                .ok_or_else(|| GraphicsError::invalid_input("Figure has no Parent"))?
        };
        return Ok(GraphicsPropertyValue::Handle(parent));
    }
    if property == GraphicsProperty::Children {
        let handles = match handle.class() {
            GraphicsClass::Figure => {
                if let Some(layout) = session.layout_for_figure(handle) {
                    return Ok(GraphicsPropertyValue::Handles(vec![layout]));
                }
                let mut handles = node.children.clone();
                for axes in &node.children {
                    handles.extend(session.children(*axes)?.iter().copied().filter(|child| {
                        matches!(
                            child.class(),
                            GraphicsClass::Legend | GraphicsClass::ColorBar
                        )
                    }));
                }
                handles
            }
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes => node
                .children
                .iter()
                .copied()
                .filter(|child| {
                    matches!(
                        child.class(),
                        GraphicsClass::LineSeries
                            | GraphicsClass::ScatterSeries
                            | GraphicsClass::SurfaceSeries
                            | GraphicsClass::PatchSeries
                    ) || is_chart_group_class(child.class())
                })
                .collect(),
            GraphicsClass::TiledChartLayout => {
                let GraphicsObject::TiledChartLayout(properties) = &node.object else {
                    return Err(GraphicsError::invalid_state(
                        "TiledChartLayout payload has wrong class",
                    ));
                };
                let mut seen = BTreeSet::new();
                properties
                    .tile_axes
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|handle| seen.insert(*handle))
                    .collect()
            }
            _ => Vec::new(),
        };
        return Ok(GraphicsPropertyValue::Handles(handles));
    }
    match (&node.object, property) {
        (GraphicsObject::TiledChartLayout(layout), GraphicsProperty::GridSize) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, 2],
                values: vec![f64::from(layout.rows), f64::from(layout.columns)],
            })
        }
        (GraphicsObject::TiledChartLayout(layout), GraphicsProperty::TileSpacing) => {
            Ok(GraphicsPropertyValue::Text(
                tile_spacing_name(layout.tile_spacing)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::TiledChartLayout(layout), GraphicsProperty::Padding) => Ok(
            GraphicsPropertyValue::Text(tile_padding_name(layout.padding).encode_utf16().collect()),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::Position) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, 4],
                values: axes.position_normalized.to_vec(),
            })
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::Color) => {
            if let Some(color) = axes.background_rgba {
                Ok(GraphicsPropertyValue::NumericMatrix {
                    shape: [1, 3],
                    values: color[..3]
                        .iter()
                        .map(|component| f64::from(*component))
                        .collect(),
                })
            } else {
                Ok(GraphicsPropertyValue::Text("none".encode_utf16().collect()))
            }
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ColorOrder) => {
            let mut values = Vec::with_capacity(axes.color_order.len() * 3);
            for component in 0..3 {
                values.extend(
                    axes.color_order
                        .iter()
                        .map(|color| f64::from(color[component])),
                );
            }
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [axes.color_order.len() as u64, 3],
                values,
            })
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::LineStyleOrder) => {
            Ok(GraphicsPropertyValue::TextList(
                axes.line_style_order
                    .iter()
                    .map(|style| line_style_name(*style).encode_utf16().collect())
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ColorOrderIndex) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(axes.color_order_index)),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XLim | GraphicsProperty::ThetaLim) => {
            Ok(numeric_row(axes.x_limits))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YLim | GraphicsProperty::RLim) => {
            Ok(numeric_row(axes.y_limits))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZLim) => Ok(numeric_row(axes.z_limits)),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::XLimMode | GraphicsProperty::ThetaLimMode,
        ) => Ok(GraphicsPropertyValue::LimitMode(axes.x_limit_mode)),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YLimMode | GraphicsProperty::RLimMode) => {
            Ok(GraphicsPropertyValue::LimitMode(axes.y_limit_mode))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZLimMode) => {
            Ok(GraphicsPropertyValue::LimitMode(axes.z_limit_mode))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XScale) => Ok(
            GraphicsPropertyValue::Text(axis_scale_name(axes.x_scale).encode_utf16().collect()),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YScale) => Ok(
            GraphicsPropertyValue::Text(axis_scale_name(axes.y_scale).encode_utf16().collect()),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZScale) => Ok(
            GraphicsPropertyValue::Text(axis_scale_name(axes.z_scale).encode_utf16().collect()),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XLimitMethod) => {
            Ok(GraphicsPropertyValue::Text(
                limit_method_name(axes.x_limit_method)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YLimitMethod) => {
            Ok(GraphicsPropertyValue::Text(
                limit_method_name(axes.y_limit_method)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZLimitMethod) => {
            Ok(GraphicsPropertyValue::Text(
                limit_method_name(axes.z_limit_method)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XDir) => Ok(GraphicsPropertyValue::Text(
            axis_direction_name(axes.x_direction)
                .encode_utf16()
                .collect(),
        )),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YDir) => Ok(GraphicsPropertyValue::Text(
            axis_direction_name(axes.y_direction)
                .encode_utf16()
                .collect(),
        )),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZDir) => Ok(GraphicsPropertyValue::Text(
            axis_direction_name(axes.z_direction)
                .encode_utf16()
                .collect(),
        )),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XTick | GraphicsProperty::ThetaTick) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, axes.x_ticks.len() as u64],
                values: axes.x_ticks.clone(),
            })
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YTick | GraphicsProperty::RTick) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, axes.y_ticks.len() as u64],
                values: axes.y_ticks.clone(),
            })
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZTick) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, axes.z_ticks.len() as u64],
                values: axes.z_ticks.clone(),
            })
        }
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::XTickMode | GraphicsProperty::ThetaTickMode,
        ) => Ok(GraphicsPropertyValue::TickMode(axes.x_tick_mode)),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::YTickMode | GraphicsProperty::RTickMode,
        ) => Ok(GraphicsPropertyValue::TickMode(axes.y_tick_mode)),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZTickMode) => {
            Ok(GraphicsPropertyValue::TickMode(axes.z_tick_mode))
        }
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::XTickLabel | GraphicsProperty::ThetaTickLabel,
        ) => Ok(GraphicsPropertyValue::TextColumn(
            axes.x_tick_labels_utf16.clone(),
        )),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::YTickLabel | GraphicsProperty::RTickLabel,
        ) => Ok(GraphicsPropertyValue::TextColumn(
            axes.y_tick_labels_utf16.clone(),
        )),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZTickLabel) => Ok(
            GraphicsPropertyValue::TextColumn(axes.z_tick_labels_utf16.clone()),
        ),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::XTickLabelMode | GraphicsProperty::ThetaTickLabelMode,
        ) => Ok(GraphicsPropertyValue::TickMode(axes.x_tick_label_mode)),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::YTickLabelMode | GraphicsProperty::RTickLabelMode,
        ) => Ok(GraphicsPropertyValue::TickMode(axes.y_tick_label_mode)),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZTickLabelMode) => {
            Ok(GraphicsPropertyValue::TickMode(axes.z_tick_label_mode))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::TickLabelInterpreter) => Ok(
            GraphicsPropertyValue::Interpreter(axes.tick_label_interpreter),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(axes.visible))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::FontSize) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(axes.font_size_points)),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::LineWidth) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(axes.line_width_points)),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::Box) => {
            Ok(GraphicsPropertyValue::Visible(axes.box_enabled))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::XGrid | GraphicsProperty::ThetaGrid) => {
            Ok(GraphicsPropertyValue::Visible(axes.grid_x))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::YGrid | GraphicsProperty::RGrid) => {
            Ok(GraphicsPropertyValue::Visible(axes.grid_y))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZGrid) => {
            Ok(GraphicsPropertyValue::Visible(axes.grid_z))
        }
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::XMinorGrid | GraphicsProperty::ThetaMinorGrid,
        ) => Ok(GraphicsPropertyValue::Visible(axes.minor_grid_x)),
        (
            GraphicsObject::Axes2D(axes),
            GraphicsProperty::YMinorGrid | GraphicsProperty::RMinorGrid,
        ) => Ok(GraphicsPropertyValue::Visible(axes.minor_grid_y)),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ZMinorGrid) => {
            Ok(GraphicsPropertyValue::Visible(axes.minor_grid_z))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ThetaAxisUnits) => {
            Ok(GraphicsPropertyValue::Text(
                theta_axis_units_name(axes.theta_axis_units)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ThetaDir) => {
            Ok(GraphicsPropertyValue::Text(
                theta_direction_name(axes.theta_direction)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::ThetaZeroLocation) => {
            Ok(GraphicsPropertyValue::Text(
                theta_zero_location_name(axes.theta_zero_location)
                    .encode_utf16()
                    .collect(),
            ))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::RAxisLocation) => {
            Ok(GraphicsPropertyValue::Scalar(axes.r_axis_location))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::View) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, 2],
                values: vec![axes.view_azimuth_degrees, axes.view_elevation_degrees],
            })
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::Projection) => {
            Ok(GraphicsPropertyValue::Projection(axes.projection))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::DataAspectRatio) => {
            Ok(numeric_row3(axes.data_aspect_ratio))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::DataAspectRatioMode) => Ok(
            GraphicsPropertyValue::LimitMode(axes.data_aspect_ratio_mode),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::PlotBoxAspectRatio) => {
            Ok(numeric_row3(axes.plot_box_aspect_ratio))
        }
        (GraphicsObject::Axes2D(axes), GraphicsProperty::PlotBoxAspectRatioMode) => Ok(
            GraphicsPropertyValue::LimitMode(axes.plot_box_aspect_ratio_mode),
        ),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::CLim) => Ok(numeric_row(axes.c_limits)),
        (GraphicsObject::Axes2D(axes), GraphicsProperty::CLimMode) => {
            Ok(GraphicsPropertyValue::LimitMode(axes.c_limit_mode))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::LineWidth) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(line.line_width_points)),
        ),
        (GraphicsObject::LineSeries(line), GraphicsProperty::Color) => {
            Ok(GraphicsPropertyValue::Color(line.color_rgba))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::LineStyle) => {
            Ok(GraphicsPropertyValue::LineStyle(line.line_style))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::Marker) => {
            Ok(GraphicsPropertyValue::Marker(line.marker))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::MarkerSize) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(line.marker_size_points)),
        ),
        (GraphicsObject::LineSeries(line), GraphicsProperty::MarkerIndices) => {
            Ok(GraphicsPropertyValue::MarkerIndices {
                values: line.marker_indices.clone(),
                value_class: line.marker_indices_class,
            })
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::MarkerFaceColor) => {
            Ok(GraphicsPropertyValue::Color(line.marker_face_color_rgba))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::MarkerEdgeColor) => {
            Ok(GraphicsPropertyValue::Color(line.marker_edge_color_rgba))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::DisplayName) => {
            Ok(GraphicsPropertyValue::Text(line.display_name_utf16.clone()))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::Clipping) => {
            Ok(GraphicsPropertyValue::Visible(line.clipping))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(line.visible))
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::XData) => {
            if node
                .parent
                .is_some_and(|parent| parent.class() == GraphicsClass::PolarAxes)
            {
                Ok(empty_numeric_matrix())
            } else {
                data_value(session, &line.x_data)
            }
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::YData) => {
            if node
                .parent
                .is_some_and(|parent| parent.class() == GraphicsClass::PolarAxes)
            {
                Ok(empty_numeric_matrix())
            } else {
                data_value(session, &line.y_data)
            }
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::ThetaData) => {
            if node
                .parent
                .is_some_and(|parent| parent.class() == GraphicsClass::PolarAxes)
            {
                data_value(session, &line.x_data)
            } else {
                Ok(empty_numeric_matrix())
            }
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::RData) => {
            if node
                .parent
                .is_some_and(|parent| parent.class() == GraphicsClass::PolarAxes)
            {
                data_value(session, &line.y_data)
            } else {
                Ok(empty_numeric_matrix())
            }
        }
        (GraphicsObject::LineSeries(line), GraphicsProperty::ZData) => {
            line.z_data.as_ref().map_or_else(
                || Ok(empty_numeric_matrix()),
                |data| data_value(session, data),
            )
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::Marker) => {
            Ok(GraphicsPropertyValue::Marker(scatter.marker))
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(scatter.visible))
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::SizeData) => {
            if let Some(data) = &scatter.size_data {
                data_value(session, data)
            } else {
                Ok(GraphicsPropertyValue::Scalar(f64::from(
                    scatter.marker_size_points.powi(2),
                )))
            }
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::CData) => {
            if let Some(data) = &scatter.color_data {
                data_value(session, data)
            } else {
                Ok(GraphicsPropertyValue::Color(scatter.face_color_rgba))
            }
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::MarkerFaceColor) => {
            Ok(GraphicsPropertyValue::Color(scatter.face_color_rgba))
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::MarkerEdgeColor) => {
            Ok(GraphicsPropertyValue::Color(scatter.edge_color_rgba))
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::DisplayName) => Ok(
            GraphicsPropertyValue::Text(scatter.display_name_utf16.clone()),
        ),
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::Clipping) => {
            Ok(GraphicsPropertyValue::Visible(scatter.clipping))
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::XData) => {
            data_value(session, &scatter.x_data)
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::YData) => {
            data_value(session, &scatter.y_data)
        }
        (GraphicsObject::ScatterSeries(scatter), GraphicsProperty::ZData) => {
            scatter.z_data.as_ref().map_or_else(
                || Ok(empty_numeric_matrix()),
                |data| data_value(session, data),
            )
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::LineWidth) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(surface.line_width_points)),
        ),
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::LineStyle) => {
            Ok(GraphicsPropertyValue::LineStyle(surface.line_style))
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(surface.visible))
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::XData) => {
            data_value(session, &surface.x_data)
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::YData) => {
            data_value(session, &surface.y_data)
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::ZData) => {
            data_value(session, &surface.z_data)
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::CData) => {
            data_value(session, &surface.c_data)
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::FaceColor) => {
            Ok(GraphicsPropertyValue::SurfaceColor(surface.face_color))
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::EdgeColor) => {
            Ok(GraphicsPropertyValue::SurfaceColor(surface.edge_color))
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::CDataMapping) => {
            Ok(GraphicsPropertyValue::CDataMapping(surface.c_data_mapping))
        }
        (GraphicsObject::SurfaceSeries(surface), GraphicsProperty::FaceAlpha) => {
            Ok(GraphicsPropertyValue::Scalar(f64::from(surface.face_alpha)))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::LineWidth) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(patch.line_width_points)),
        ),
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::LineStyle) => {
            Ok(GraphicsPropertyValue::LineStyle(patch.line_style))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(patch.visible))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::XData) => {
            patch_coordinate_data(session, patch, 0)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::YData) => {
            patch_coordinate_data(session, patch, 1)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::ZData) => {
            if patch.vertices.shape[1] == 3 {
                patch_coordinate_data(session, patch, 2)
            } else {
                Ok(empty_numeric_matrix())
            }
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::CData) => {
            patch_cdata(session, patch)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::Faces) => {
            data_value(session, &patch.faces)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::Vertices) => {
            data_value(session, &patch.vertices)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::FaceVertexCData) => {
            data_value(session, &patch.face_vertex_cdata)
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::FaceColor) => {
            Ok(GraphicsPropertyValue::SurfaceColor(patch.face_color))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::EdgeColor) => {
            Ok(GraphicsPropertyValue::SurfaceColor(patch.edge_color))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::CDataMapping) => {
            Ok(GraphicsPropertyValue::CDataMapping(patch.c_data_mapping))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::FaceAlpha) => {
            Ok(GraphicsPropertyValue::Scalar(f64::from(patch.face_alpha)))
        }
        (GraphicsObject::PatchSeries(patch), GraphicsProperty::EdgeAlpha) => {
            Ok(GraphicsPropertyValue::Scalar(f64::from(patch.edge_alpha)))
        }
        (GraphicsObject::Text(text), GraphicsProperty::String) => {
            Ok(GraphicsPropertyValue::Text(text.code_units.clone()))
        }
        (GraphicsObject::Text(text), GraphicsProperty::Color) => {
            Ok(GraphicsPropertyValue::Color(text.color_rgba))
        }
        (GraphicsObject::Text(text), GraphicsProperty::FontSize) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(text.font_size_css_px)),
        ),
        (GraphicsObject::Text(text), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(text.visible))
        }
        (GraphicsObject::Text(text), GraphicsProperty::Interpreter) => {
            Ok(GraphicsPropertyValue::Interpreter(text.interpreter))
        }
        (GraphicsObject::Legend(legend), GraphicsProperty::String) => {
            Ok(GraphicsPropertyValue::TextList(legend.labels_utf16.clone()))
        }
        (GraphicsObject::Legend(legend), GraphicsProperty::Location) => {
            Ok(GraphicsPropertyValue::LegendLocation(legend.location))
        }
        (GraphicsObject::Legend(legend), GraphicsProperty::FontSize) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(legend.font_size_css_px)),
        ),
        (GraphicsObject::Legend(legend), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(legend.visible))
        }
        (GraphicsObject::Legend(legend), GraphicsProperty::Interpreter) => {
            Ok(GraphicsPropertyValue::Interpreter(legend.interpreter))
        }
        (GraphicsObject::ColorBar(colorbar), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(colorbar.visible))
        }
        (GraphicsObject::ColorBar(colorbar), GraphicsProperty::Ticks) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, colorbar.ticks.len() as u64],
                values: colorbar.ticks.clone(),
            })
        }
        (GraphicsObject::ColorBar(colorbar), GraphicsProperty::TicksMode) => {
            Ok(GraphicsPropertyValue::TickMode(colorbar.tick_mode))
        }
        (GraphicsObject::ColorBar(colorbar), GraphicsProperty::TickLabels) => Ok(
            GraphicsPropertyValue::TextColumn(colorbar.tick_labels_utf16.clone()),
        ),
        (GraphicsObject::ColorBar(colorbar), GraphicsProperty::TickLabelsMode) => {
            Ok(GraphicsPropertyValue::TickMode(colorbar.tick_label_mode))
        }
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(chart.visible))
        }
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::Color) => chart
            .color
            .map(chart_color_value)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no Color property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::LineWidth) => Ok(
            GraphicsPropertyValue::Scalar(f64::from(chart.line_width_points)),
        ),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::LineStyle) => {
            Ok(GraphicsPropertyValue::LineStyle(chart.line_style))
        }
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::Marker) => chart
            .marker
            .map(GraphicsPropertyValue::Marker)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no Marker property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::MarkerSize) => chart
            .marker_size_points
            .map(|value| GraphicsPropertyValue::Scalar(f64::from(value)))
            .ok_or_else(|| GraphicsError::invalid_state("chart has no MarkerSize property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::MarkerFaceColor) => chart
            .marker_face_color
            .map(chart_color_value)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no MarkerFaceColor property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::MarkerEdgeColor) => chart
            .marker_edge_color
            .map(chart_color_value)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no MarkerEdgeColor property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::FaceColor) => chart
            .face_color
            .map(chart_color_value)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no FaceColor property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::EdgeColor) => chart
            .edge_color
            .map(chart_color_value)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no EdgeColor property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::FaceAlpha) => chart
            .face_alpha
            .map(|value| GraphicsPropertyValue::Scalar(f64::from(value)))
            .ok_or_else(|| GraphicsError::invalid_state("chart has no FaceAlpha property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::EdgeAlpha) => chart
            .edge_alpha
            .map(|value| GraphicsPropertyValue::Scalar(f64::from(value)))
            .ok_or_else(|| GraphicsError::invalid_state("chart has no EdgeAlpha property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::BaseValue) => chart
            .base_value
            .map(GraphicsPropertyValue::Scalar)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no BaseValue property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::BarWidth) => chart
            .bar_width
            .map(GraphicsPropertyValue::Scalar)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no BarWidth property")),
        (GraphicsObject::ChartGroup(chart), GraphicsProperty::CapSize) => chart
            .cap_size_points
            .map(GraphicsPropertyValue::Scalar)
            .ok_or_else(|| GraphicsError::invalid_state("chart has no CapSize property")),
        (GraphicsObject::Figure(figure), GraphicsProperty::Name) => {
            Ok(GraphicsPropertyValue::Text(figure.name_utf16.clone()))
        }
        (GraphicsObject::Figure(figure), GraphicsProperty::NumberTitle) => {
            Ok(GraphicsPropertyValue::Visible(figure.number_title))
        }
        (GraphicsObject::Figure(figure), GraphicsProperty::Position) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, 4],
                values: figure.position_css_pixels.to_vec(),
            })
        }
        (GraphicsObject::Figure(figure), GraphicsProperty::Visible) => {
            Ok(GraphicsPropertyValue::Visible(figure.visible))
        }
        (GraphicsObject::Figure(figure), GraphicsProperty::Color) => {
            Ok(GraphicsPropertyValue::NumericMatrix {
                shape: [1, 3],
                values: figure.background_rgba[..3]
                    .iter()
                    .map(|component| f64::from(*component))
                    .collect(),
            })
        }
        _ => Err(GraphicsError::invalid_state(
            "validated graphics property does not match its object payload",
        )),
    }
}

fn numeric_row(values: [f64; 2]) -> GraphicsPropertyValue {
    GraphicsPropertyValue::NumericMatrix {
        shape: [1, 2],
        values: values.to_vec(),
    }
}

fn numeric_row3(values: [f64; 3]) -> GraphicsPropertyValue {
    GraphicsPropertyValue::NumericMatrix {
        shape: [1, 3],
        values: values.to_vec(),
    }
}

fn empty_numeric_matrix() -> GraphicsPropertyValue {
    GraphicsPropertyValue::NumericMatrix {
        shape: [0, 0],
        values: Vec::new(),
    }
}

fn data_value(
    session: &GraphicsSession,
    reference: &DataRef,
) -> Result<GraphicsPropertyValue, GraphicsError> {
    Ok(GraphicsPropertyValue::NumericMatrix {
        shape: reference.shape,
        values: data_values(session, reference)?,
    })
}

fn data_values(session: &GraphicsSession, reference: &DataRef) -> Result<Vec<f64>, GraphicsError> {
    let resource = session
        .resources
        .get(&reference.id)
        .ok_or_else(|| GraphicsError::invalid_state("series resource is missing"))?;
    let element_count = reference.shape[0]
        .checked_mul(reference.shape[1])
        .ok_or_else(|| GraphicsError::limit("series shape exceeds model limits"))?;
    let capacity = usize::try_from(element_count)
        .map_err(|_| GraphicsError::limit("series length exceeds host limits"))?;
    let mut values = Vec::with_capacity(capacity);
    match reference.dtype {
        DataDType::F32 => resource.bytes().chunks_exact(4).for_each(|chunk| {
            values.push(f64::from(f32::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3],
            ])));
        }),
        DataDType::F64 => resource.bytes().chunks_exact(8).for_each(|chunk| {
            values.push(f64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]));
        }),
    }
    Ok(values)
}

fn patch_coordinate_data(
    session: &GraphicsSession,
    patch: &PatchSeriesProperties,
    component: usize,
) -> Result<GraphicsPropertyValue, GraphicsError> {
    let faces = data_values(session, &patch.faces)?;
    let vertices = data_values(session, &patch.vertices)?;
    let face_rows = usize::try_from(patch.faces.shape[0])
        .map_err(|_| GraphicsError::limit("Patch face count exceeds host limits"))?;
    let face_columns = usize::try_from(patch.faces.shape[1])
        .map_err(|_| GraphicsError::limit("Patch face width exceeds host limits"))?;
    let vertex_rows = usize::try_from(patch.vertices.shape[0])
        .map_err(|_| GraphicsError::limit("Patch vertex count exceeds host limits"))?;
    let component_offset = component
        .checked_mul(vertex_rows)
        .ok_or_else(|| GraphicsError::limit("Patch coordinate offset overflowed"))?;
    let capacity = face_rows
        .checked_mul(face_columns)
        .ok_or_else(|| GraphicsError::limit("Patch coordinate matrix size overflowed"))?;
    let mut values = Vec::with_capacity(capacity);
    for face_row in 0..face_rows {
        for face_column in 0..face_columns {
            let source = faces[face_row + face_column * face_rows];
            if source.is_nan() {
                values.push(f64::NAN);
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let vertex = source as usize - 1;
            values.push(vertices[component_offset + vertex]);
        }
    }
    Ok(GraphicsPropertyValue::NumericMatrix {
        shape: [patch.faces.shape[1], patch.faces.shape[0]],
        values,
    })
}

fn patch_cdata(
    session: &GraphicsSession,
    patch: &PatchSeriesProperties,
) -> Result<GraphicsPropertyValue, GraphicsError> {
    if patch.face_vertex_cdata.shape[0] == 0 {
        return Ok(empty_numeric_matrix());
    }
    if patch.face_vertex_cdata.shape[0] == patch.faces.shape[0] {
        return data_value(session, &patch.face_vertex_cdata);
    }
    let faces = data_values(session, &patch.faces)?;
    let colors = data_values(session, &patch.face_vertex_cdata)?;
    let face_rows = usize::try_from(patch.faces.shape[0])
        .map_err(|_| GraphicsError::limit("Patch face count exceeds host limits"))?;
    let face_columns = usize::try_from(patch.faces.shape[1])
        .map_err(|_| GraphicsError::limit("Patch face width exceeds host limits"))?;
    let capacity = face_rows
        .checked_mul(face_columns)
        .ok_or_else(|| GraphicsError::limit("Patch CData matrix size overflowed"))?;
    let mut values = Vec::with_capacity(capacity);
    for face_row in 0..face_rows {
        for face_column in 0..face_columns {
            let source = faces[face_row + face_column * face_rows];
            if source.is_nan() {
                values.push(f64::NAN);
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let vertex = source as usize - 1;
            values.push(colors[vertex]);
        }
    }
    Ok(GraphicsPropertyValue::NumericMatrix {
        shape: [patch.faces.shape[1], patch.faces.shape[0]],
        values,
    })
}

const fn line_style_name(style: LineStyle) -> &'static str {
    match style {
        LineStyle::None => "none",
        LineStyle::Solid => "-",
        LineStyle::Dash => "--",
        LineStyle::Dot => ":",
        LineStyle::DashDot => "-.",
    }
}

const fn axis_scale_name(scale: AxisScale) -> &'static str {
    match scale {
        AxisScale::Linear => "linear",
        AxisScale::Log => "log",
    }
}

const fn axis_direction_name(direction: AxisDirection) -> &'static str {
    match direction {
        AxisDirection::Normal => "normal",
        AxisDirection::Reverse => "reverse",
    }
}

const fn limit_method_name(method: LimitMethod) -> &'static str {
    match method {
        LimitMethod::TickAligned => "tickaligned",
        LimitMethod::Tight => "tight",
        LimitMethod::Padded => "padded",
    }
}

const fn tile_spacing_name(spacing: TileSpacing) -> &'static str {
    match spacing {
        TileSpacing::Loose => "loose",
        TileSpacing::Compact => "compact",
        TileSpacing::None => "none",
    }
}

const fn tile_padding_name(padding: TilePadding) -> &'static str {
    match padding {
        TilePadding::Loose => "loose",
        TilePadding::Compact => "compact",
        TilePadding::Tight => "tight",
    }
}

fn validate_pair(x: &NumericData, y: &NumericData) -> Result<(), GraphicsError> {
    if x.len() != y.len() {
        return Err(GraphicsError::invalid_input(
            "series X and Y vectors must have equal lengths",
        ));
    }
    Ok(())
}

fn validate_surface(surface: &SurfaceInput) -> Result<(), GraphicsError> {
    if surface.rows < 2 || surface.columns < 2 {
        return Err(GraphicsError::invalid_input(
            "surface ZData must contain at least two rows and two columns",
        ));
    }
    let rows = usize::try_from(surface.rows)
        .map_err(|_| GraphicsError::limit("surface row count exceeds host limits"))?;
    let columns = usize::try_from(surface.columns)
        .map_err(|_| GraphicsError::limit("surface column count exceeds host limits"))?;
    let elements = rows
        .checked_mul(columns)
        .ok_or_else(|| GraphicsError::limit("surface element count overflowed"))?;
    let vector_coordinates = surface.x.len() == columns && surface.y.len() == rows;
    let matrix_coordinates = surface.x.len() == elements && surface.y.len() == elements;
    if (!vector_coordinates && !matrix_coordinates)
        || surface.z.len() != elements
        || surface.c.len() != elements
    {
        return Err(GraphicsError::invalid_input(
            "surface X, Y, Z, and CData dimensions are inconsistent",
        ));
    }
    Ok(())
}

fn validate_patch(patch: &PatchInput) -> Result<(), GraphicsError> {
    if patch.face_rows == 0 || patch.face_columns < 3 {
        return Err(GraphicsError::invalid_input(
            "Patch Faces must contain at least one row and three columns",
        ));
    }
    if patch.vertex_rows < 3 || !matches!(patch.vertex_columns, 2 | 3) {
        return Err(GraphicsError::invalid_input(
            "Patch Vertices must be an N-by-2 or N-by-3 matrix with at least three rows",
        ));
    }
    let expected_faces = patch
        .face_rows
        .checked_mul(patch.face_columns)
        .ok_or_else(|| GraphicsError::limit("Patch Faces shape overflowed"))?;
    let expected_vertices = patch
        .vertex_rows
        .checked_mul(patch.vertex_columns)
        .ok_or_else(|| GraphicsError::limit("Patch Vertices shape overflowed"))?;
    let expected_cdata = patch
        .cdata_rows
        .checked_mul(patch.cdata_columns)
        .ok_or_else(|| GraphicsError::limit("Patch FaceVertexCData shape overflowed"))?;
    if u64::try_from(patch.faces.len()).ok() != Some(expected_faces)
        || u64::try_from(patch.vertices.len()).ok() != Some(expected_vertices)
        || u64::try_from(patch.face_vertex_cdata.len()).ok() != Some(expected_cdata)
    {
        return Err(GraphicsError::invalid_input(
            "Patch data shapes do not match their element counts",
        ));
    }
    if expected_cdata != 0
        && (patch.cdata_columns != 1
            || !matches!(patch.cdata_rows, rows if rows == patch.face_rows || rows == patch.vertex_rows))
    {
        return Err(GraphicsError::invalid_input(
            "Patch FaceVertexCData must contain one scalar per face or vertex",
        ));
    }
    validate_patch_faces(patch)?;
    let mut invalid_vertex = false;
    patch
        .vertices
        .visit_f64(|value| invalid_vertex |= !value.is_finite());
    if invalid_vertex {
        return Err(GraphicsError::invalid_input(
            "Patch Vertices must contain finite real coordinates",
        ));
    }
    let has_cdata = !patch.face_vertex_cdata.is_empty();
    if (matches!(patch.face_color, SurfaceColor::Flat | SurfaceColor::Interp)
        || matches!(patch.edge_color, SurfaceColor::Flat | SurfaceColor::Interp))
        && !has_cdata
    {
        return Err(GraphicsError::invalid_input(
            "mapped Patch colors require FaceVertexCData",
        ));
    }
    if (patch.face_color == SurfaceColor::Interp || patch.edge_color == SurfaceColor::Interp)
        && patch.cdata_rows != patch.vertex_rows
    {
        return Err(GraphicsError::invalid_input(
            "interpolated Patch colors require one FaceVertexCData value per vertex",
        ));
    }
    Ok(())
}

fn validate_patch_faces(patch: &PatchInput) -> Result<(), GraphicsError> {
    let mut faces = Vec::with_capacity(patch.faces.len());
    patch.faces.visit_f64(|value| faces.push(value));
    let face_rows = usize::try_from(patch.face_rows)
        .map_err(|_| GraphicsError::limit("Patch face count exceeds host limits"))?;
    let face_columns = usize::try_from(patch.face_columns)
        .map_err(|_| GraphicsError::limit("Patch face width exceeds host limits"))?;
    for row in 0..face_rows {
        let mut count = 0;
        let mut padded = false;
        for column in 0..face_columns {
            let value = faces[row + column * face_rows];
            if value.is_nan() {
                padded = true;
                continue;
            }
            if padded
                || !value.is_finite()
                || value < 1.0
                || value.fract() != 0.0
                || value >= 18_446_744_073_709_551_616.0
            {
                return Err(GraphicsError::invalid_input(
                    "Patch Faces must contain one-based integer indices followed only by NaN padding",
                ));
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let vertex_index = value as u64;
            if vertex_index > patch.vertex_rows {
                return Err(GraphicsError::invalid_input(
                    "Patch Faces must contain one-based integer indices followed only by NaN padding",
                ));
            }
            count += 1;
        }
        if count < 3 {
            return Err(GraphicsError::invalid_input(
                "every Patch face must contain at least three vertices",
            ));
        }
    }
    Ok(())
}

fn validate_limits(limits: [f64; 2]) -> Result<(), GraphicsError> {
    if limits.iter().all(|limit| limit.is_finite()) && limits[0] < limits[1] {
        Ok(())
    } else {
        Err(GraphicsError::invalid_input(
            "axis limits must be finite and strictly increasing",
        ))
    }
}

fn validate_axes_state(properties: &Axes2DProperties) -> Result<(), GraphicsError> {
    if !properties.r_axis_location.is_finite() {
        return Err(GraphicsError::invalid_input(
            "RAxisLocation must be a finite scalar",
        ));
    }
    if properties.coordinate_system == AxesCoordinateSystem::Polar
        && (properties.x_scale != AxisScale::Linear || properties.y_scale != AxisScale::Linear)
    {
        return Err(GraphicsError::invalid_input(
            "PolarAxes supports only linear theta and radial scales",
        ));
    }
    for (scale, mode, limits, name) in [
        (
            properties.x_scale,
            properties.x_limit_mode,
            properties.x_limits,
            "XLim",
        ),
        (
            properties.y_scale,
            properties.y_limit_mode,
            properties.y_limits,
            "YLim",
        ),
        (
            properties.z_scale,
            properties.z_limit_mode,
            properties.z_limits,
            "ZLim",
        ),
    ] {
        if scale == AxisScale::Log && mode == LimitMode::Manual && limits[0] <= 0.0 {
            return Err(GraphicsError::invalid_input(format!(
                "{name} must be positive when its axis Scale is log"
            )));
        }
    }
    Ok(())
}

fn resolve_axis_limits(
    planner: &dyn AutoLimitPlanner,
    finite_values: &[f64],
    scale: AxisScale,
    method: LimitMethod,
) -> [f64; 2] {
    let positive_values;
    let values = if scale == AxisScale::Log {
        positive_values = finite_values
            .iter()
            .copied()
            .filter(|value| *value > 0.0)
            .collect::<Vec<_>>();
        positive_values.as_slice()
    } else {
        finite_values
    };
    if values.is_empty() && scale == AxisScale::Log {
        return [0.1, 10.0];
    }
    let limits = planner.resolve(values);
    if method != LimitMethod::Padded {
        return limits;
    }
    let transformed = if scale == AxisScale::Log {
        limits.map(f64::log10)
    } else {
        limits
    };
    let padding = (transformed[1] - transformed[0]) * 0.07;
    let padded = [transformed[0] - padding, transformed[1] + padding];
    if scale == AxisScale::Log {
        padded.map(|value| 10.0_f64.powf(value))
    } else {
        padded
    }
}

fn default_marker_indices(length: usize) -> Result<Vec<u64>, GraphicsError> {
    let length_u64 = u64::try_from(length)
        .map_err(|_| GraphicsError::limit("Line data length exceeds MarkerIndices limits"))?;
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(length)
        .map_err(|_| GraphicsError::limit("MarkerIndices allocation failed"))?;
    indices.extend(1..=length_u64);
    Ok(indices)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn automatic_ticks(limits: [f64; 2]) -> Vec<f64> {
    automatic_ticks_with_intervals(limits, 7.0)
}

fn automatic_axis_ticks(limits: [f64; 2], scale: AxisScale) -> Vec<f64> {
    if scale == AxisScale::Linear {
        return automatic_ticks(limits);
    }
    let first = limits[0].log10().ceil();
    let last = limits[1].log10().floor();
    if !first.is_finite() || !last.is_finite() || first > last || last - first > 100.0 {
        return limits.to_vec();
    }
    let mut exponent = first;
    let mut ticks = Vec::new();
    while exponent <= last {
        ticks.push(10.0_f64.powf(exponent));
        exponent += 1.0;
    }
    ticks
}

const fn polar_full_circle(units: ThetaAxisUnits) -> [f64; 2] {
    match units {
        ThetaAxisUnits::Degrees => [0.0, 360.0],
        ThetaAxisUnits::Radians => [0.0, std::f64::consts::TAU],
    }
}

fn automatic_polar_theta_ticks(limits: [f64; 2], units: ThetaAxisUnits) -> Vec<f64> {
    let full = polar_full_circle(units);
    let span = limits[1] - limits[0];
    let full_span = full[1] - full[0];
    if (span - full_span).abs() <= full_span * 1.0e-12 {
        let step = full_span / 12.0;
        return (0_u32..=12)
            .map(|index| limits[0] + f64::from(index) * step)
            .collect();
    }
    automatic_ticks_with_intervals(limits, 12.0)
}

fn automatic_polar_theta_labels(
    ticks: &[f64],
    limits: [f64; 2],
    units: ThetaAxisUnits,
) -> Vec<Vec<u16>> {
    let full_span = polar_full_circle(units)[1];
    let duplicate_endpoint = (limits[1] - limits[0] - full_span).abs() <= full_span * 1.0e-12;
    let label_ticks = if duplicate_endpoint && ticks.len() > 1 {
        &ticks[..ticks.len() - 1]
    } else {
        ticks
    };
    label_ticks
        .iter()
        .map(|value| {
            let text = match units {
                ThetaAxisUnits::Degrees => format!("{}°", format_tick_label(*value)),
                ThetaAxisUnits::Radians => format_radian_tick_label(*value),
            };
            text.encode_utf16().collect()
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
fn format_radian_tick_label(value: f64) -> String {
    if value.abs() <= f64::EPSILON {
        return String::from("0");
    }
    let ratio = value / std::f64::consts::PI;
    for denominator in 1_i32..=12 {
        let numerator = (ratio * f64::from(denominator)).round() as i32;
        if (ratio - f64::from(numerator) / f64::from(denominator)).abs() <= 1.0e-10 {
            let divisor = integer_gcd(numerator.unsigned_abs(), denominator.unsigned_abs());
            let reduced_numerator = numerator / i32::try_from(divisor).unwrap_or(1);
            let reduced_denominator = denominator / i32::try_from(divisor).unwrap_or(1);
            return match (reduced_numerator, reduced_denominator) {
                (1, 1) => String::from("\\pi"),
                (-1, 1) => String::from("-\\pi"),
                (number, 1) => format!("{number}\\pi"),
                (1, divisor) => format!("\\pi/{divisor}"),
                (-1, divisor) => format!("-\\pi/{divisor}"),
                (number, divisor) => format!("{number}\\pi/{divisor}"),
            };
        }
    }
    format_tick_label(value)
}

const fn integer_gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left }
}

const fn theta_axis_units_name(units: ThetaAxisUnits) -> &'static str {
    match units {
        ThetaAxisUnits::Degrees => "degrees",
        ThetaAxisUnits::Radians => "radians",
    }
}

const fn theta_direction_name(direction: ThetaDirection) -> &'static str {
    match direction {
        ThetaDirection::Counterclockwise => "counterclockwise",
        ThetaDirection::Clockwise => "clockwise",
    }
}

const fn theta_zero_location_name(location: ThetaZeroLocation) -> &'static str {
    match location {
        ThetaZeroLocation::Right => "right",
        ThetaZeroLocation::Top => "top",
        ThetaZeroLocation::Left => "left",
        ThetaZeroLocation::Bottom => "bottom",
    }
}

fn automatic_axis_tick_labels(ticks: &[f64], scale: AxisScale) -> Vec<Vec<u16>> {
    if scale == AxisScale::Linear {
        return automatic_tick_labels(ticks);
    }
    ticks
        .iter()
        .map(|value| {
            let exponent = value.log10().round();
            let label = if exponent == 0.0 {
                "10^{0}".to_owned()
            } else {
                format!("10^{{{exponent:.0}}}")
            };
            label.encode_utf16().collect()
        })
        .collect()
}

fn automatic_colorbar_ticks(limits: [f64; 2]) -> Vec<f64> {
    automatic_ticks_with_intervals(limits, 8.0)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn automatic_ticks_with_intervals(limits: [f64; 2], intervals: f64) -> Vec<f64> {
    let span = limits[1] - limits[0];
    if !span.is_finite() || span <= 0.0 {
        return limits.to_vec();
    }
    let raw_step = span / intervals;
    let magnitude = 10.0_f64.powf(raw_step.log10().floor());
    let normalized = raw_step / magnitude;
    let nice = if normalized <= 1.25 {
        1.0
    } else if normalized <= 2.5 {
        2.0
    } else if normalized <= 7.5 {
        5.0
    } else {
        10.0
    };
    let step = nice * magnitude;
    let first = (limits[0] / step).ceil() * step;
    let last = (limits[1] / step).floor() * step;
    let count = (((last - first) / step).round() + 1.0).max(0.0) as usize;
    (0..count)
        .map(|index| {
            let value = first + index as f64 * step;
            if value.abs() <= step.abs() * 1.0e-12 {
                0.0
            } else {
                value
            }
        })
        .collect()
}

fn default_colorbar_properties(limits: [f64; 2]) -> ColorBarProperties {
    let ticks = automatic_colorbar_ticks(limits);
    ColorBarProperties {
        visible: true,
        tick_labels_utf16: automatic_tick_labels(&ticks),
        ticks,
        tick_mode: TickMode::Auto,
        tick_label_mode: TickMode::Auto,
    }
}

fn automatic_tick_labels(ticks: &[f64]) -> Vec<Vec<u16>> {
    ticks
        .iter()
        .map(|tick| format_tick_label(*tick).encode_utf16().collect())
        .collect()
}

fn cycle_tick_labels(labels: &[Vec<u16>], tick_count: usize) -> Vec<Vec<u16>> {
    if labels.is_empty() {
        return Vec::new();
    }
    (0..tick_count)
        .map(|index| labels[index % labels.len()].clone())
        .collect()
}

fn format_tick_label(value: f64) -> String {
    if value == f64::INFINITY {
        return String::from("Inf");
    }
    if value == f64::NEG_INFINITY {
        return String::from("-Inf");
    }
    if value == 0.0 {
        return String::from("0");
    }
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        return format!("{value:.0}");
    }
    let mut text = format!("{value:.12}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn series_display_name(session: &GraphicsSession, handle: GraphicsHandle) -> Option<Vec<u16>> {
    match session.object(handle).ok()? {
        GraphicsObject::LineSeries(properties) => Some(properties.display_name_utf16.clone()),
        GraphicsObject::ScatterSeries(properties) => Some(properties.display_name_utf16.clone()),
        GraphicsObject::SurfaceSeries(_)
        | GraphicsObject::PatchSeries(_)
        | GraphicsObject::ChartGroup(_) => Some(Vec::new()),
        _ => None,
    }
}

const fn is_chart_group_class(class: GraphicsClass) -> bool {
    matches!(
        class,
        GraphicsClass::Stair
            | GraphicsClass::Stem
            | GraphicsClass::ErrorBar
            | GraphicsClass::Area
            | GraphicsClass::Bar
            | GraphicsClass::Histogram
            | GraphicsClass::Contour
            | GraphicsClass::Image
    )
}

fn tiled_block_indices(
    rows: u32,
    columns: u32,
    start_row: u32,
    start_column: u32,
    row_span: u32,
    column_span: u32,
) -> Result<Vec<usize>, GraphicsError> {
    let end_row = start_row
        .checked_add(row_span)
        .filter(|end| *end <= rows)
        .ok_or_else(|| GraphicsError::invalid_input("tile span exceeds layout rows"))?;
    let end_column = start_column
        .checked_add(column_span)
        .filter(|end| *end <= columns)
        .ok_or_else(|| GraphicsError::invalid_input("tile span exceeds layout columns"))?;
    let mut indices = Vec::new();
    for row in start_row..end_row {
        for column in start_column..end_column {
            let index = row
                .checked_mul(columns)
                .and_then(|index| index.checked_add(column))
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| GraphicsError::limit("tile index exceeds host limits"))?;
            indices.push(index);
        }
    }
    Ok(indices)
}

#[allow(clippy::too_many_arguments)]
fn tiled_axes_position(
    rows: u32,
    columns: u32,
    start_row: u32,
    start_column: u32,
    row_span: u32,
    column_span: u32,
    tile_spacing: TileSpacing,
    padding: TilePadding,
) -> Result<[f64; 4], GraphicsError> {
    let [left, bottom, width, height] = match padding {
        TilePadding::Loose => [0.13, 0.11, 0.775, 0.815],
        TilePadding::Compact => [
            0.065,
            0.058_095_240_214_514_36,
            0.8875,
            0.904_404_759_785_485_7,
        ],
        TilePadding::Tight => [
            0.037_619_049_208_504_81,
            0.045_396_827_516_101_66,
            0.946_904_759_747_641_4,
            0.936_349_204_788_372_3,
        ],
    };
    let horizontal_gap: f64 = match (tile_spacing, columns) {
        (_, 1) | (TileSpacing::None, _) => 0.0,
        (TileSpacing::Compact, _) | (TileSpacing::Loose, 3) => 0.07,
        (TileSpacing::Loose, 2) => 0.14,
        (TileSpacing::Loose, _) => 0.055_595_238_285_750_95,
    };
    let vertical_gap: f64 = match (tile_spacing, rows) {
        (_, 1) | (TileSpacing::None, _) => 0.0,
        (TileSpacing::Compact, _) => 0.076_349_208_090_485_65,
        (TileSpacing::Loose, 2) => 0.14,
        (TileSpacing::Loose, 3) => 0.084_761_906_048_608_1,
        (TileSpacing::Loose, _) => 0.080_476_190_261_950_5,
    };
    let x_gap = horizontal_gap.min(width / (2.0 * f64::from(columns)));
    let y_gap = vertical_gap.min(height / (2.0 * f64::from(rows)));
    let cell_width = (width - x_gap * f64::from(columns.saturating_sub(1))) / f64::from(columns);
    let cell_height = (height - y_gap * f64::from(rows.saturating_sub(1))) / f64::from(rows);
    let rectangle = [
        left + f64::from(start_column) * (cell_width + x_gap),
        bottom + f64::from(rows - start_row - row_span) * (cell_height + y_gap),
        f64::from(column_span) * cell_width + f64::from(column_span.saturating_sub(1)) * x_gap,
        f64::from(row_span) * cell_height + f64::from(row_span.saturating_sub(1)) * y_gap,
    ];
    validate_axes_position(rectangle)?;
    Ok(rectangle)
}

fn allows_child(parent: GraphicsClass, child: GraphicsClass) -> bool {
    matches!(
        (parent, child),
        (
            GraphicsClass::Figure,
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes
        ) | (
            GraphicsClass::Axes2D | GraphicsClass::PolarAxes,
            GraphicsClass::LineSeries
                | GraphicsClass::ScatterSeries
                | GraphicsClass::SurfaceSeries
                | GraphicsClass::PatchSeries
                | GraphicsClass::Text
                | GraphicsClass::Legend
                | GraphicsClass::ColorBar
                | GraphicsClass::Stair
                | GraphicsClass::Stem
                | GraphicsClass::ErrorBar
                | GraphicsClass::Area
                | GraphicsClass::Bar
                | GraphicsClass::Histogram
                | GraphicsClass::Contour
                | GraphicsClass::Image
        ) | (
            GraphicsClass::Stair
                | GraphicsClass::Stem
                | GraphicsClass::ErrorBar
                | GraphicsClass::Area
                | GraphicsClass::Bar
                | GraphicsClass::Histogram
                | GraphicsClass::Contour
                | GraphicsClass::Image,
            GraphicsClass::LineSeries | GraphicsClass::PatchSeries
        )
    )
}

fn default_axes_properties() -> Axes2DProperties {
    let default_ticks = automatic_ticks(DEFAULT_LIMITS);
    let default_tick_labels = automatic_tick_labels(&default_ticks);
    Axes2DProperties {
        coordinate_system: AxesCoordinateSystem::Cartesian,
        position_normalized: [0.13, 0.11, 0.775, 0.815],
        background_rgba: Some([1.0, 1.0, 1.0, 1.0]),
        theta_axis_units: ThetaAxisUnits::Degrees,
        theta_direction: ThetaDirection::Counterclockwise,
        theta_zero_location: ThetaZeroLocation::Right,
        r_axis_location: 80.0,
        x_limits: DEFAULT_LIMITS,
        y_limits: DEFAULT_LIMITS,
        z_limits: DEFAULT_LIMITS,
        x_limit_mode: LimitMode::Auto,
        y_limit_mode: LimitMode::Auto,
        z_limit_mode: LimitMode::Auto,
        x_scale: AxisScale::Linear,
        y_scale: AxisScale::Linear,
        z_scale: AxisScale::Linear,
        x_limit_method: LimitMethod::TickAligned,
        y_limit_method: LimitMethod::TickAligned,
        z_limit_method: LimitMethod::TickAligned,
        x_direction: AxisDirection::Normal,
        y_direction: AxisDirection::Normal,
        z_direction: AxisDirection::Normal,
        x_ticks: default_ticks.clone(),
        y_ticks: default_ticks,
        z_ticks: automatic_ticks(DEFAULT_LIMITS),
        x_tick_mode: TickMode::Auto,
        y_tick_mode: TickMode::Auto,
        z_tick_mode: TickMode::Auto,
        x_tick_labels_utf16: default_tick_labels.clone(),
        y_tick_labels_utf16: default_tick_labels,
        z_tick_labels_utf16: automatic_tick_labels(&automatic_ticks(DEFAULT_LIMITS)),
        x_tick_label_mode: TickMode::Auto,
        y_tick_label_mode: TickMode::Auto,
        z_tick_label_mode: TickMode::Auto,
        tick_label_interpreter: Interpreter::Tex,
        font_size_points: 10.0,
        font_family_utf16: "Helvetica".encode_utf16().collect(),
        tick_direction: TickDirection::In,
        line_width_points: 0.5,
        box_enabled: false,
        visible: true,
        next_plot: NextPlot::Replace,
        grid_x: false,
        grid_y: false,
        grid_z: false,
        minor_grid_x: false,
        minor_grid_y: false,
        minor_grid_z: false,
        view_azimuth_degrees: 0.0,
        view_elevation_degrees: 90.0,
        projection: ProjectionMode::Orthographic,
        camera_scale: 1.0,
        headlight_enabled: false,
        lighting_mode: LightingMode::Flat,
        data_aspect_ratio: [1.0, 1.0, 1.0],
        data_aspect_ratio_mode: LimitMode::Auto,
        plot_box_aspect_ratio: [1.0, 1.0, 1.0],
        plot_box_aspect_ratio_mode: LimitMode::Auto,
        c_limits: DEFAULT_LIMITS,
        c_limit_mode: LimitMode::Auto,
        colormap: PARULA_R2022B.to_vec(),
        colorbar_visible: false,
        color_order_index: 1,
        color_order: COLOR_ORDER.to_vec(),
        line_style_order: vec![LineStyle::Solid],
        title: None,
        x_label: None,
        y_label: None,
        z_label: None,
    }
}

fn apply_polar_factory_defaults(properties: &mut Axes2DProperties) {
    properties.coordinate_system = AxesCoordinateSystem::Polar;
    properties.x_limits = [0.0, 360.0];
    properties.y_limits = [0.0, 1.0];
    properties.z_limits = DEFAULT_LIMITS;
    properties.x_ticks = (0_u32..=12).map(|index| f64::from(index) * 30.0).collect();
    properties.x_tick_labels_utf16 = (0_u32..12)
        .map(|index| format!("{}°", index * 30).encode_utf16().collect())
        .collect();
    properties.y_ticks = (0_u32..=5).map(|index| f64::from(index) * 0.2).collect();
    properties.y_tick_labels_utf16 = automatic_tick_labels(&properties.y_ticks);
    properties.grid_x = true;
    properties.grid_y = true;
}

fn validate_colormap(colors: &[[f64; 3]]) -> Result<(), GraphicsError> {
    if colors.is_empty() {
        return Err(GraphicsError::invalid_input(
            "colormap must contain at least one RGB row",
        ));
    }
    if colors
        .iter()
        .flatten()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err(GraphicsError::invalid_input(
            "colormap RGB components must be finite values in [0, 1]",
        ));
    }
    Ok(())
}

fn text_anchor(role: TextRole) -> [f64; 2] {
    match role {
        TextRole::Title => [0.5, 1.0],
        TextRole::XLabel => [0.5, 0.0],
        TextRole::YLabel => [0.0, 0.5],
        TextRole::ZLabel => [1.0, 0.5],
    }
}

fn text_vertical_alignment(role: TextRole) -> VerticalAlignment {
    match role {
        TextRole::Title => VerticalAlignment::Bottom,
        TextRole::XLabel => VerticalAlignment::Top,
        TextRole::YLabel | TextRole::ZLabel => VerticalAlignment::Middle,
    }
}

fn same_object_semantics(left: &HirObject, right: &HirObject) -> bool {
    left.id == right.id
        && left.generation == right.generation
        && left.class == right.class
        && left.parent_id == right.parent_id
        && left.children == right.children
        && left.properties == right.properties
}

fn before_data_ids(session: &GraphicsSession, figure_id: &str) -> Vec<DataResourceId> {
    let figure = session.live_handles().find(|handle| {
        handle.class() == GraphicsClass::Figure
            && session
                .node(*handle)
                .is_ok_and(|node| node.public_id == figure_id)
    });
    figure
        .and_then(|figure| session.snapshot(figure).ok())
        .map(|snapshot| {
            snapshot
                .referenced_data
                .into_iter()
                .map(|resource| resource.descriptor().id)
                .collect()
        })
        .unwrap_or_default()
}

fn collect_data_ids(object: &GraphicsObject, ids: &mut BTreeSet<DataResourceId>) {
    match object {
        GraphicsObject::LineSeries(series) => {
            ids.insert(series.x_data.id);
            ids.insert(series.y_data.id);
            if let Some(z_data) = &series.z_data {
                ids.insert(z_data.id);
            }
        }
        GraphicsObject::ScatterSeries(series) => {
            ids.insert(series.x_data.id);
            ids.insert(series.y_data.id);
            if let Some(z_data) = &series.z_data {
                ids.insert(z_data.id);
            }
            if let Some(size_data) = &series.size_data {
                ids.insert(size_data.id);
            }
            if let Some(color_data) = &series.color_data {
                ids.insert(color_data.id);
            }
        }
        GraphicsObject::SurfaceSeries(series) => {
            ids.insert(series.x_data.id);
            ids.insert(series.y_data.id);
            ids.insert(series.z_data.id);
            ids.insert(series.c_data.id);
        }
        GraphicsObject::PatchSeries(series) => {
            ids.insert(series.faces.id);
            ids.insert(series.vertices.id);
            ids.insert(series.face_vertex_cdata.id);
            if let Some(vertex_normals) = &series.vertex_normals {
                ids.insert(vertex_normals.id);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: &[f64], y: &[f64]) -> LineInput {
        LineInput {
            x: NumericData::from_f64(Arc::<[f64]>::from(x)),
            y: NumericData::from_f64(Arc::<[f64]>::from(y)),
            z: None,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn response_handle(execution: GraphicsExecution) -> GraphicsHandle {
        let GraphicsResponse::Handle(handle) = execution.response else {
            panic!("expected one graphics handle")
        };
        handle
    }

    fn response_property(
        session: &mut GraphicsSession,
        handle: GraphicsHandle,
        property: GraphicsProperty,
    ) -> GraphicsPropertyValue {
        let GraphicsResponse::PropertyValues(mut values) = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![handle],
                property,
            })
            .unwrap()
            .response
        else {
            panic!("expected one graphics property value")
        };
        assert_eq!(values.len(), 1);
        values.remove(0)
    }

    fn same_f64_values<const N: usize>(left: [f64; N], right: [f64; N]) -> bool {
        left.iter()
            .zip(right)
            .all(|(left, right)| (left - right).abs() <= f64::EPSILON)
    }

    #[test]
    fn axes_creation_position_and_subplot_reuse_are_transactional() {
        let mut session = GraphicsSession::new();
        let figure = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let custom_position = [0.2, 0.25, 0.5, 0.5];
        let custom = response_handle(
            session
                .execute(GraphicsRequest::CreateAxes {
                    figure: Some(figure),
                    properties: vec![GraphicsPropertyUpdate::Position(custom_position)],
                })
                .unwrap(),
        );
        assert!(matches!(
            session.object(custom),
            Ok(GraphicsObject::Axes2D(properties))
                if same_f64_values(properties.position_normalized, custom_position)
        ));

        let left_position = [0.13, 0.11, 0.334_659_090_909_090_9, 0.815];
        let left = response_handle(
            session
                .execute(GraphicsRequest::Subplot {
                    figure: Some(figure),
                    position: left_position,
                })
                .unwrap(),
        );
        let selected = response_handle(
            session
                .execute(GraphicsRequest::Subplot {
                    figure: Some(figure),
                    position: left_position,
                })
                .unwrap(),
        );
        assert_eq!(left, selected);
        assert!(session.is_valid(left));

        let span = response_handle(
            session
                .execute(GraphicsRequest::Subplot {
                    figure: Some(figure),
                    position: [0.13, 0.11, 0.775, 0.815],
                })
                .unwrap(),
        );
        assert!(!session.is_valid(custom));
        assert!(!session.is_valid(left));
        assert!(session.is_valid(span));
        assert_eq!(session.children(figure).unwrap(), &[span]);
    }

    #[test]
    fn invalid_axes_position_rolls_back_without_allocating_an_axes() {
        let mut session = GraphicsSession::new();
        let figure = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let count = session.object_count();
        let result = session.execute(GraphicsRequest::CreateAxes {
            figure: Some(figure),
            properties: vec![GraphicsPropertyUpdate::Position([0.8, 0.1, 0.3, 0.5])],
        });
        assert!(matches!(
            result,
            Err(GraphicsError {
                category: crate::GraphicsErrorCategory::InvalidInput,
                ..
            })
        ));
        assert_eq!(session.object_count(), count);
        assert!(session.children(figure).unwrap().is_empty());
    }

    #[test]
    fn explicit_multi_axes_interaction_target_changes_only_the_named_axes() {
        let mut session = GraphicsSession::new();
        let figure = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let first = response_handle(
            session
                .execute(GraphicsRequest::CreateAxes {
                    figure: Some(figure),
                    properties: vec![GraphicsPropertyUpdate::Position([0.13, 0.11, 0.334, 0.815])],
                })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::CreateAxes {
                    figure: Some(figure),
                    properties: vec![GraphicsPropertyUpdate::Position([0.57, 0.11, 0.334, 0.815])],
                })
                .unwrap(),
        );
        session
            .execute(GraphicsRequest::SetAxesLimits {
                figure,
                axes: Some(second),
                x_limits: [-5.0, 5.0],
                y_limits: [10.0, 20.0],
            })
            .unwrap();

        assert!(matches!(
            session.object(first),
            Ok(GraphicsObject::Axes2D(properties))
                if !same_f64_values(properties.x_limits, [-5.0, 5.0])
                    && !same_f64_values(properties.y_limits, [10.0, 20.0])
        ));
        assert!(matches!(
            session.object(second),
            Ok(GraphicsObject::Axes2D(properties))
                if same_f64_values(properties.x_limits, [-5.0, 5.0])
                    && same_f64_values(properties.y_limits, [10.0, 20.0])
                    && properties.x_limit_mode == LimitMode::Manual
                    && properties.y_limit_mode == LimitMode::Manual
        ));
    }

    #[test]
    fn tiled_layout_replaces_old_axes_reuses_tiles_and_tracks_matlab_positions() {
        let mut session = GraphicsSession::new();
        let old_axes = response_handle(
            session
                .execute(GraphicsRequest::Axes { select: None })
                .unwrap(),
        );
        let layout = response_handle(
            session
                .execute(GraphicsRequest::CreateTiledLayout {
                    figure: None,
                    rows: 2,
                    columns: 2,
                    tile_spacing: TileSpacing::Loose,
                    padding: TilePadding::Loose,
                })
                .unwrap(),
        );
        assert_eq!(layout.class(), GraphicsClass::TiledChartLayout);
        assert!(!session.is_valid(old_axes));
        assert!(matches!(
            session.object(layout),
            Ok(GraphicsObject::TiledChartLayout(properties))
                if properties.rows == 2
                    && properties.columns == 2
                    && properties.tile_spacing == TileSpacing::Loose
                    && properties.padding == TilePadding::Loose
        ));

        let first = response_handle(
            session
                .execute(GraphicsRequest::NextTile {
                    layout: Some(layout),
                    selection: NextTileSelection::Automatic,
                })
                .unwrap(),
        );
        let selected = response_handle(
            session
                .execute(GraphicsRequest::NextTile {
                    layout: Some(layout),
                    selection: NextTileSelection::Tile(1),
                })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::NextTile {
                    layout: Some(layout),
                    selection: NextTileSelection::Automatic,
                })
                .unwrap(),
        );
        assert_eq!(first, selected);
        assert_ne!(first, second);
        let Ok(GraphicsObject::Axes2D(first_properties)) = session.object(first) else {
            panic!("first tile must be an Axes");
        };
        assert!(
            first_properties
                .position_normalized
                .iter()
                .zip([0.13, 0.5875, 0.3175, 0.3375])
                .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-14)
        );
        let Ok(GraphicsObject::Axes2D(second_properties)) = session.object(second) else {
            panic!("second tile must be an Axes");
        };
        assert!(
            second_properties
                .position_normalized
                .iter()
                .zip([0.5875, 0.5875, 0.3175, 0.3375])
                .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-14)
        );

        let GraphicsResponse::PropertyValues(values) = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![layout],
                property: GraphicsProperty::GridSize,
            })
            .unwrap()
            .response
        else {
            panic!("GridSize must be queryable");
        };
        assert_eq!(
            values,
            vec![GraphicsPropertyValue::NumericMatrix {
                shape: [1, 2],
                values: vec![2.0, 2.0],
            }]
        );
    }

    #[test]
    fn tiled_layout_parentage_and_subplot_switch_match_r2022b() {
        let mut session = GraphicsSession::new();
        let layout = response_handle(
            session
                .execute(GraphicsRequest::CreateTiledLayout {
                    figure: None,
                    rows: 2,
                    columns: 2,
                    tile_spacing: TileSpacing::Loose,
                    padding: TilePadding::Loose,
                })
                .unwrap(),
        );
        let first = response_handle(
            session
                .execute(GraphicsRequest::NextTile {
                    layout: Some(layout),
                    selection: NextTileSelection::Automatic,
                })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::NextTile {
                    layout: Some(layout),
                    selection: NextTileSelection::Automatic,
                })
                .unwrap(),
        );
        let first_parent = response_property(&mut session, first, GraphicsProperty::Parent);
        let layout_parent = response_property(&mut session, layout, GraphicsProperty::Parent);
        assert_eq!(first_parent, GraphicsPropertyValue::Handle(layout));

        let figure = session.tiled_layout_properties(layout).unwrap().parent;
        assert_eq!(layout_parent, GraphicsPropertyValue::Handle(figure));
        let figure_children = response_property(&mut session, figure, GraphicsProperty::Children);
        let layout_children = response_property(&mut session, layout, GraphicsProperty::Children);
        assert_eq!(
            figure_children,
            GraphicsPropertyValue::Handles(vec![layout]),
        );
        assert_eq!(
            layout_children,
            GraphicsPropertyValue::Handles(vec![first, second]),
        );

        let subplot = response_handle(
            session
                .execute(GraphicsRequest::Subplot {
                    figure: Some(figure),
                    position: [0.13, 0.11, 0.775, 0.815],
                })
                .unwrap(),
        );
        assert!(!session.is_valid(layout));
        assert!(!session.is_valid(first));
        assert!(!session.is_valid(second));
        assert!(session.is_valid(subplot));
    }

    #[test]
    fn stale_generation_never_resolves_after_slot_reuse() {
        let mut session = GraphicsSession::new();
        let first = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        session
            .execute(GraphicsRequest::CloseFigure {
                figure: Some(first),
            })
            .unwrap();
        let replacement = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        assert_eq!(first.slot(), replacement.slot());
        assert_ne!(first.generation(), replacement.generation());
        assert!(!session.is_valid(first));
        assert!(session.is_valid(replacement));
    }

    #[test]
    fn figure_numbering_reuses_smallest_free_positive_number() {
        let mut session = GraphicsSession::new();
        let first = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        assert!(
            matches!(session.object(first), Ok(GraphicsObject::Figure(value)) if value.number == 1)
        );
        assert!(
            matches!(session.object(second), Ok(GraphicsObject::Figure(value)) if value.number == 2)
        );
        session
            .execute(GraphicsRequest::CloseFigure {
                figure: Some(first),
            })
            .unwrap();
        let reused = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        assert!(
            matches!(session.object(reused), Ok(GraphicsObject::Figure(value)) if value.number == 1)
        );
    }

    #[test]
    fn explicit_figure_selection_preserves_identity_and_auto_number_uses_one() {
        let mut session = GraphicsSession::new();
        let seven = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: Some(7) })
                .unwrap(),
        );
        let selected = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: Some(7) })
                .unwrap(),
        );
        assert_eq!(seven, selected);
        let automatic = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        assert!(
            matches!(session.object(automatic), Ok(GraphicsObject::Figure(value)) if value.number == 1)
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn figure_constructor_properties_are_committed_with_first_discovery() {
        let mut session = GraphicsSession::new();
        let execution = session
            .execute(GraphicsRequest::CreateFigure {
                properties: FigureCreationProperties {
                    name_utf16: "Möbius".encode_utf16().collect(),
                    number_title: false,
                    visible: false,
                    background_rgba: [1.0, 1.0, 1.0, 1.0],
                    position_css_pixels: [101.0, 202.0, 640.0, 480.0],
                },
            })
            .unwrap();
        let notice = execution.notice.as_ref().unwrap();
        assert!(notice.discovery);
        let figure = response_handle(execution);
        assert!(matches!(
            session.object(figure),
            Ok(GraphicsObject::Figure(properties))
                if properties.number == 1
                    && properties.name_utf16 == "Möbius".encode_utf16().collect::<Vec<_>>()
                    && !properties.number_title
                    && !properties.visible
                    && properties.background_rgba == [1.0, 1.0, 1.0, 1.0]
                    && properties.position_css_pixels == [101.0, 202.0, 640.0, 480.0]
        ));
    }

    #[test]
    fn current_figure_query_is_noncreating_and_figure_color_is_mutable() {
        let mut session = GraphicsSession::new();
        let query = session
            .execute(GraphicsRequest::CurrentFigureOrEmpty)
            .unwrap();
        assert_eq!(query.response, GraphicsResponse::Handles(Vec::new()));
        assert_eq!(session.object_count(), 0);

        let figure = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![figure],
                properties: vec![GraphicsPropertyUpdate::Color([0.25, 0.5, 0.75, 1.0])],
            })
            .unwrap();
        let color = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![figure],
                property: GraphicsProperty::Color,
            })
            .unwrap();
        assert_eq!(
            color.response,
            GraphicsResponse::PropertyValues(vec![GraphicsPropertyValue::NumericMatrix {
                shape: [1, 3],
                values: vec![0.25, 0.5, 0.75],
            }])
        );

        session
            .execute(GraphicsRequest::CloseFigure {
                figure: Some(figure),
            })
            .unwrap();
        let query = session
            .execute(GraphicsRequest::CurrentFigureOrEmpty)
            .unwrap();
        assert_eq!(query.response, GraphicsResponse::Handles(Vec::new()));
        assert_eq!(session.object_count(), 0);
    }

    #[test]
    fn axes_color_defaults_to_white_and_round_trips_none() {
        let mut session = GraphicsSession::new();
        let (_, axes, _) = session.ensure_current_axes().unwrap();
        assert_eq!(
            session.axes_properties(axes).unwrap().background_rgba,
            Some([1.0, 1.0, 1.0, 1.0])
        );

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![axes],
                properties: vec![GraphicsPropertyUpdate::AxesColor(Some([
                    0.25, 0.5, 0.75, 1.0,
                ]))],
            })
            .unwrap();
        let color = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![axes],
                property: GraphicsProperty::Color,
            })
            .unwrap();
        assert_eq!(
            color.response,
            GraphicsResponse::PropertyValues(vec![GraphicsPropertyValue::NumericMatrix {
                shape: [1, 3],
                values: vec![0.25, 0.5, 0.75],
            }])
        );

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![axes],
                properties: vec![GraphicsPropertyUpdate::AxesColor(None)],
            })
            .unwrap();
        let transparent = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![axes],
                property: GraphicsProperty::Color,
            })
            .unwrap();
        assert_eq!(
            transparent.response,
            GraphicsResponse::PropertyValues(vec![GraphicsPropertyValue::Text(
                "none".encode_utf16().collect(),
            )])
        );
    }

    #[test]
    fn invalid_figure_constructor_color_rolls_back_creation() {
        let mut session = GraphicsSession::new();
        let result = session.execute(GraphicsRequest::CreateFigure {
            properties: FigureCreationProperties {
                background_rgba: [1.1, 0.0, 0.0, 1.0],
                ..FigureCreationProperties::default()
            },
        });
        assert!(matches!(
            result,
            Err(GraphicsError {
                category: crate::GraphicsErrorCategory::InvalidInput,
                ..
            })
        ));
        assert_eq!(session.object_count(), 0);
        assert!(session.current_figure().is_none());
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    fn close_all_figures_is_one_atomic_request_with_per_figure_deltas() {
        let mut session = GraphicsSession::new();
        let first = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        session.take_pending_deltas();

        let execution = session.execute(GraphicsRequest::CloseAllFigures).unwrap();

        assert_eq!(execution.response, GraphicsResponse::None);
        assert!(execution.notice.is_none());
        assert!(!session.is_valid(first));
        assert!(!session.is_valid(second));
        assert!(session.current_figure().is_none());
        assert!(session.current_axes().is_none());
        assert_eq!(session.object_count(), 0);
        let deltas = session.take_pending_deltas();
        assert_eq!(deltas.len(), 2);
        assert!(deltas.iter().all(|delta| {
            delta
                .operations
                .iter()
                .any(|operation| matches!(operation, GraphicsDeltaOperation::DeleteObject { .. }))
        }));
    }

    #[test]
    fn close_all_rolls_back_every_figure_when_one_revision_is_exhausted() {
        let mut session = GraphicsSession::new();
        let first = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        session.take_pending_deltas();
        let second_id = session.object_id(second).unwrap().to_owned();
        session
            .figure_revisions
            .insert(second_id, JSON_SAFE_INTEGER_MAX);

        let result = session.execute(GraphicsRequest::CloseAllFigures);

        assert!(matches!(
            result,
            Err(GraphicsError {
                category: crate::GraphicsErrorCategory::Limit,
                ..
            })
        ));
        assert!(session.is_valid(first));
        assert!(session.is_valid(second));
        assert_eq!(session.current_figure(), Some(second));
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    fn deleting_figure_recursively_invalidates_axes_and_series() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0], &[3.0, 4.0])],
            })
            .unwrap();
        let figure = session.current_figure().unwrap();
        let axes = session.current_axes().unwrap();
        let series = session.children(axes).unwrap()[0];
        session
            .execute(GraphicsRequest::CloseFigure {
                figure: Some(figure),
            })
            .unwrap();
        assert!(!session.is_valid(figure));
        assert!(!session.is_valid(axes));
        assert!(!session.is_valid(series));
        assert_eq!(session.object_count(), 0);
        assert_eq!(session.resource_count(), 0);
    }

    #[test]
    fn closing_a_non_current_figure_preserves_the_current_figure() {
        let mut session = GraphicsSession::new();
        let first = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );
        let second = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: None })
                .unwrap(),
        );

        session
            .execute(GraphicsRequest::CloseFigure {
                figure: Some(first),
            })
            .unwrap();

        assert_eq!(session.current_figure(), Some(second));
        assert!(session.is_valid(second));
    }

    #[test]
    fn invalid_transaction_rolls_back_all_allocations_and_revision() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0], &[2.0])],
            })
            .unwrap();
        let figure = session.current_figure().unwrap();
        let before = session.snapshot(figure).unwrap();
        let error = session.execute(GraphicsRequest::Plot {
            lines: vec![line(&[1.0, 2.0], &[3.0])],
        });
        assert!(error.is_err());
        assert_eq!(session.snapshot(figure).unwrap(), before);
    }

    #[test]
    fn hold_add_and_replace_follow_measured_child_semantics() {
        let mut session = GraphicsSession::new();
        let first = session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0], &[1.0])],
            })
            .unwrap();
        let GraphicsResponse::Handles(first_handles) = first.response else {
            panic!()
        };
        let first_line = first_handles[0];
        assert_eq!(
            session
                .execute(GraphicsRequest::IsHold { axes: None })
                .unwrap()
                .response,
            GraphicsResponse::Logical(false)
        );
        session
            .execute(GraphicsRequest::SetHold {
                axes: None,
                enabled: true,
            })
            .unwrap();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[2.0], &[2.0])],
            })
            .unwrap();
        let axes = session.current_axes().unwrap();
        assert_eq!(session.children(axes).unwrap().len(), 2);
        assert!(session.is_valid(first_line));
        session
            .execute(GraphicsRequest::SetHold {
                axes: None,
                enabled: false,
            })
            .unwrap();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[3.0], &[3.0])],
            })
            .unwrap();
        assert_eq!(session.children(axes).unwrap().len(), 1);
        assert!(!session.is_valid(first_line));
        assert_eq!(
            session.axes_properties(axes).unwrap().next_plot,
            NextPlot::Replace
        );
    }

    #[test]
    fn resources_are_independent_rows_and_preserve_f32_f64_and_nan_bits() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![
                    LineInput {
                        x: NumericData::from_f32(Arc::from([1.0_f32, f32::NAN])),
                        y: NumericData::from_f32(Arc::from([2.0_f32, 3.0])),
                        z: None,
                    },
                    line(&[10.0, 20.0], &[30.0, 40.0]),
                ],
            })
            .unwrap();
        let axes = session.current_axes().unwrap();
        let first = session.object(session.children(axes).unwrap()[0]).unwrap();
        let second = session.object(session.children(axes).unwrap()[1]).unwrap();
        let GraphicsObject::LineSeries(first) = first else {
            panic!()
        };
        let GraphicsObject::LineSeries(second) = second else {
            panic!()
        };
        assert_eq!(first.x_data.shape, [1, 2]);
        assert_eq!(first.x_data.dtype, DataDType::F32);
        assert_eq!(second.x_data.shape, [1, 2]);
        assert_eq!(second.x_data.dtype, DataDType::F64);
        assert_ne!(first.x_data.id, second.x_data.id);
        let resource = session.data_resource(first.x_data.id).unwrap();
        assert_eq!(&resource.bytes()[4..8], &f32::NAN.to_bits().to_le_bytes());
    }

    #[test]
    fn immutable_snapshot_keeps_cow_bytes_after_later_replace() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0], &[3.0, 4.0])],
            })
            .unwrap();
        let figure = session.current_figure().unwrap();
        let snapshot = session.snapshot(figure).unwrap();
        let saved = snapshot.referenced_data[0].clone();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[9.0], &[10.0])],
            })
            .unwrap();
        assert_eq!(saved.bytes(), snapshot.referenced_data[0].bytes());
        assert!(saved.shares_bytes_with(&snapshot.referenced_data[0]));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn limits_are_resolved_in_kernel_model_and_manual_assignment_sets_mode() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[10.0, 20.0], &[-2.0, 4.0])],
            })
            .unwrap();
        let axes = session.current_axes().unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [10.0, 20.0]);
        assert_eq!(properties.y_limits, [-2.0, 4.0]);
        session
            .execute(GraphicsRequest::SetLimits {
                axes: None,
                axis: Axis::X,
                limits: [0.0, 100.0],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [0.0, 100.0]);
        assert_eq!(properties.x_limit_mode, LimitMode::Manual);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn both_axes_limits_commit_once_to_the_target_figures_axes() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0], &[3.0, 4.0])],
            })
            .unwrap();
        let first_figure = session.current_figure().unwrap();
        let first_axes = session.current_axes().unwrap();
        session
            .execute(GraphicsRequest::Figure { number: Some(2) })
            .unwrap();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[10.0, 20.0], &[30.0, 40.0])],
            })
            .unwrap();
        let second_figure = session.current_figure().unwrap();
        let second_axes = session.current_axes().unwrap();
        let before_first = session.snapshot(first_figure).unwrap();
        let before_second = session.snapshot(second_figure).unwrap();
        session.take_pending_deltas();

        let execution = session
            .execute(GraphicsRequest::SetAxesLimits {
                figure: first_figure,
                axes: None,
                x_limits: [-5.0, 5.0],
                y_limits: [100.0, 200.0],
            })
            .unwrap();

        assert_eq!(execution.response, GraphicsResponse::None);
        assert_eq!(session.current_axes(), Some(second_axes));
        let properties = session.axes_properties(first_axes).unwrap();
        assert_eq!(properties.x_limits, [-5.0, 5.0]);
        assert_eq!(properties.y_limits, [100.0, 200.0]);
        assert_eq!(properties.x_limit_mode, LimitMode::Manual);
        assert_eq!(properties.y_limit_mode, LimitMode::Manual);
        assert_eq!(session.snapshot(second_figure).unwrap(), before_second);

        let after_first = session.snapshot(first_figure).unwrap();
        assert_eq!(after_first.revision, before_first.revision + 1);
        let deltas = session.take_pending_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].base_revision, before_first.revision);
        assert_eq!(deltas[0].revision, after_first.revision);
        assert!(deltas[0].added_data.is_empty());
        assert!(deltas[0].released_data_ids.is_empty());
        assert!(matches!(
            deltas[0].operations.as_slice(),
            [GraphicsDeltaOperation::UpsertObject(object)]
                if object.class == GraphicsClass::Axes2D
                    && matches!(&object.properties, HirProperties::Axes2D(value)
                        if value.x_limits == [-5.0, 5.0]
                            && value.y_limits == [100.0, 200.0]
                            && value.x_limit_mode == LimitMode::Manual
                            && value.y_limit_mode == LimitMode::Manual)
        ));
    }

    #[test]
    fn both_axes_limits_reject_all_invalid_inputs_without_mutation() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0], &[2.0])],
            })
            .unwrap();
        let figure = session.current_figure().unwrap();
        let axes = session.current_axes().unwrap();
        let before = session.snapshot(figure).unwrap();
        session.take_pending_deltas();

        for request in [
            GraphicsRequest::SetAxesLimits {
                figure,
                axes: None,
                x_limits: [0.0, 1.0],
                y_limits: [2.0, 2.0],
            },
            GraphicsRequest::SetAxesLimits {
                figure,
                axes: None,
                x_limits: [0.0, f64::NAN],
                y_limits: [0.0, 1.0],
            },
            GraphicsRequest::SetAxesLimits {
                figure: axes,
                axes: None,
                x_limits: [0.0, 1.0],
                y_limits: [0.0, 1.0],
            },
        ] {
            assert!(session.execute(request).is_err());
            assert_eq!(session.snapshot(figure).unwrap(), before);
            assert!(session.take_pending_deltas().is_empty());
        }

        let empty_figure = response_handle(
            session
                .execute(GraphicsRequest::Figure { number: Some(3) })
                .unwrap(),
        );
        let empty_before = session.snapshot(empty_figure).unwrap();
        session.take_pending_deltas();
        assert!(
            session
                .execute(GraphicsRequest::SetAxesLimits {
                    figure: empty_figure,
                    axes: None,
                    x_limits: [0.0, 1.0],
                    y_limits: [0.0, 1.0],
                })
                .is_err()
        );
        assert_eq!(session.snapshot(empty_figure).unwrap(), empty_before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn measured_auto_limit_basics_match_r2022b_samples() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0, 3.0], &[5.0, 5.0, 5.0])],
            })
            .unwrap();
        let axes = session.current_axes().unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [1.0, 3.0]);
        assert_eq!(properties.y_limits, [4.0, 6.0]);

        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0], &[1.0])],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [0.0, 2.0]);
        assert_eq!(properties.y_limits, [0.0, 2.0]);

        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(
                    &[1.0e12, 1.0e12 + 1.0, 1.0e12 + 2.0],
                    &[0.0, 1.0, 2.0],
                )],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [1.0e12, 1.0e12 + 2.0]);
        assert_eq!(properties.y_limits, [0.0, 2.0]);

        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0, 3.0], &[f64::NAN, 2.0, f64::INFINITY])],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [1.0, 3.0]);
        assert_eq!(properties.y_limits, [1.0, 3.0]);

        session
            .execute(GraphicsRequest::Plot { lines: Vec::new() })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [0.0, 1.0]);
        assert_eq!(properties.y_limits, [0.0, 1.0]);

        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[0.0, 0.0], &[0.0, 0.0])],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [-1.0, 1.0]);
        assert_eq!(properties.y_limits, [-1.0, 1.0]);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn log_scales_and_padded_limit_methods_resolve_in_the_model() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 10.0, 100.0], &[0.1, 1.0, 10.0])],
            })
            .unwrap();
        let axes = session.current_axes().unwrap();
        session
            .execute(GraphicsRequest::SetAxesProperties {
                axes: Some(axes),
                properties: vec![
                    GraphicsPropertyUpdate::XScale(AxisScale::Log),
                    GraphicsPropertyUpdate::YScale(AxisScale::Log),
                ],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.x_limits, [1.0, 100.0]);
        assert_eq!(properties.y_limits, [0.1, 10.0]);
        assert_eq!(properties.x_ticks, [1.0, 10.0, 100.0]);
        assert_eq!(
            properties.x_tick_labels_utf16,
            ["10^{0}", "10^{1}", "10^{2}"].map(|label| label.encode_utf16().collect::<Vec<_>>())
        );

        session
            .execute(GraphicsRequest::SetAxesProperties {
                axes: Some(axes),
                properties: vec![
                    GraphicsPropertyUpdate::XScale(AxisScale::Linear),
                    GraphicsPropertyUpdate::YScale(AxisScale::Linear),
                    GraphicsPropertyUpdate::XLimitMethod(LimitMethod::Padded),
                    GraphicsPropertyUpdate::YLimitMethod(LimitMethod::Padded),
                ],
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert!((properties.x_limits[0] + 5.93).abs() <= 1.0e-12);
        assert!((properties.x_limits[1] - 106.93).abs() <= 1.0e-12);
        assert!((properties.y_limits[0] + 0.593).abs() <= 1.0e-12);
        assert!((properties.y_limits[1] - 10.693).abs() <= 1.0e-12);
    }

    #[test]
    fn ishold_without_axes_creates_only_figure_and_returns_false() {
        let mut session = GraphicsSession::new();
        let execution = session
            .execute(GraphicsRequest::IsHold { axes: None })
            .unwrap();
        assert_eq!(execution.response, GraphicsResponse::Logical(false));
        assert_eq!(session.object_count(), 1);
        assert!(session.current_figure().is_some());
        assert!(session.current_axes().is_none());
    }

    #[test]
    fn text_is_exact_utf16_and_cla_clf_have_distinct_lifetimes() {
        let mut session = GraphicsSession::new();
        let title = response_handle(
            session
                .execute(GraphicsRequest::SetText {
                    axes: None,
                    role: TextRole::Title,
                    code_units: vec![0x0041, 0xD800],
                    properties: Vec::new(),
                })
                .unwrap(),
        );
        let axes = session.current_axes().unwrap();
        assert!(
            matches!(session.object(title), Ok(GraphicsObject::Text(value)) if value.code_units == [0x0041, 0xD800])
        );
        session
            .execute(GraphicsRequest::ClearAxes { axes: None })
            .unwrap();
        assert!(session.is_valid(axes));
        assert!(!session.is_valid(title));
        session
            .execute(GraphicsRequest::ClearFigure { figure: None })
            .unwrap();
        assert!(!session.is_valid(axes));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn styled_plot_and_property_update_emit_complete_object_upserts() {
        let mut session = GraphicsSession::new();
        let execution = session
            .execute(GraphicsRequest::PlotStyled {
                axes: None,
                lines: vec![line(&[1.0, 2.0], &[3.0, 4.0])],
                properties: vec![
                    GraphicsPropertyUpdate::LineStyle(LineStyle::Dash),
                    GraphicsPropertyUpdate::Marker(Marker::Square),
                    GraphicsPropertyUpdate::LineWidth(2.0),
                    GraphicsPropertyUpdate::Color([0.1, 0.2, 0.3, 1.0]),
                ],
            })
            .unwrap();
        let GraphicsResponse::Handles(handles) = execution.response else {
            panic!()
        };
        let line_handle = handles[0];
        let GraphicsObject::LineSeries(properties) = session.object(line_handle).unwrap() else {
            panic!()
        };
        assert_eq!(properties.line_style, LineStyle::Dash);
        assert_eq!(properties.marker, Marker::Square);
        assert_eq!(properties.line_width_points, 2.0);
        assert_eq!(properties.color_rgba, [0.1, 0.2, 0.3, 1.0]);

        session.take_pending_deltas();
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![line_handle],
                properties: vec![
                    GraphicsPropertyUpdate::Visible(false),
                    GraphicsPropertyUpdate::Marker(Marker::Diamond),
                ],
            })
            .unwrap();
        let deltas = session.take_pending_deltas();
        assert_eq!(deltas.len(), 1);
        assert!(deltas[0].added_data.is_empty());
        assert!(deltas[0].released_data_ids.is_empty());
        assert!(matches!(
            deltas[0].operations.as_slice(),
            [GraphicsDeltaOperation::UpsertObject(object)]
                if object.class == GraphicsClass::LineSeries
                    && matches!(&object.properties, HirProperties::LineSeries(value)
                        if !value.visible && value.marker == Marker::Diamond)
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn surface_defaults_and_camera_commit_match_r2022b_visual_contract() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::Surface {
                axes: None,
                surface: SurfaceInput {
                    x: NumericData::from_f64(vec![1.0, 2.0]),
                    y: NumericData::from_f64(vec![1.0, 2.0]),
                    z: NumericData::from_f64(vec![1.0, 2.0, 3.0, 4.0]),
                    c: NumericData::from_f64(vec![1.0, 2.0, 3.0, 4.0]),
                    rows: 2,
                    columns: 2,
                },
                style: SurfaceStyle::Surf,
                properties: Vec::new(),
            })
            .unwrap();
        let figure = session.current_figure().unwrap();
        let axes = session.current_axes().unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.view_azimuth_degrees, -37.5);
        assert_eq!(properties.view_elevation_degrees, 30.0);
        assert!(properties.grid_x && properties.grid_y && properties.grid_z);
        assert!(!properties.box_enabled);
        assert_eq!(properties.camera_scale, 1.0);

        session.take_pending_deltas();
        session
            .execute(GraphicsRequest::SetAxesCamera {
                figure,
                axes: None,
                azimuth_degrees: 55.0,
                elevation_degrees: 24.0,
                camera_scale: 0.8,
            })
            .unwrap();
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.view_azimuth_degrees, 55.0);
        assert_eq!(properties.view_elevation_degrees, 24.0);
        assert_eq!(properties.camera_scale, 0.8);
        let deltas = session.take_pending_deltas();
        assert_eq!(deltas.len(), 1);
        assert!(matches!(
            deltas[0].operations.as_slice(),
            [GraphicsDeltaOperation::UpsertObject(object)]
                if object.class == GraphicsClass::Axes2D
        ));

        let invalid = session
            .execute(GraphicsRequest::SetAxesCamera {
                figure,
                axes: None,
                azimuth_degrees: 0.0,
                elevation_degrees: 91.0,
                camera_scale: 1.0,
            })
            .unwrap_err();
        assert_eq!(invalid.category, crate::GraphicsErrorCategory::InvalidInput);
        assert!(session.take_pending_deltas().is_empty());
        let properties = session.axes_properties(axes).unwrap();
        assert_eq!(properties.view_azimuth_degrees, 55.0);
        assert_eq!(properties.view_elevation_degrees, 24.0);
    }

    #[test]
    fn property_transactions_validate_all_targets_and_values_before_commit() {
        let mut session = GraphicsSession::new();
        let line_execution = session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0], &[2.0])],
            })
            .unwrap();
        let GraphicsResponse::Handles(line_handles) = line_execution.response else {
            panic!()
        };
        session
            .execute(GraphicsRequest::SetHold {
                axes: None,
                enabled: true,
            })
            .unwrap();
        let scatter_execution = session
            .execute(GraphicsRequest::Scatter {
                axes: None,
                series: ScatterInput {
                    x: NumericData::from_f64(Arc::from([1.0])),
                    y: NumericData::from_f64(Arc::from([3.0])),
                    z: None,
                    size_data: None,
                    color_data: None,
                    filled: false,
                },
                properties: Vec::new(),
            })
            .unwrap();
        let GraphicsResponse::Handle(scatter) = scatter_execution.response else {
            panic!()
        };
        let figure = session.current_figure().unwrap();
        let before = session.snapshot(figure).unwrap();
        session.take_pending_deltas();

        let heterogeneous = session.execute(GraphicsRequest::SetProperties {
            handles: vec![line_handles[0], scatter],
            properties: vec![GraphicsPropertyUpdate::Color([1.0, 0.0, 0.0, 1.0])],
        });
        assert_eq!(
            heterogeneous.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidHandle
        );
        let invalid_width = session.execute(GraphicsRequest::SetProperties {
            handles: vec![line_handles[0]],
            properties: vec![GraphicsPropertyUpdate::LineWidth(f32::NAN)],
        });
        assert_eq!(
            invalid_width.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidInput
        );
        assert_eq!(session.snapshot(figure).unwrap(), before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn chart_properties_round_trip_propagate_and_reject_atomically() {
        let mut session = GraphicsSession::new();
        let chart = response_handle(
            session
                .execute(GraphicsRequest::ChartGroup {
                    axes: None,
                    chart: ChartGroupInput {
                        kind: ChartGroupKind::Stem,
                        primitives: vec![ChartPrimitiveInput::Line {
                            line: line(&[1.0, 1.0, f64::NAN], &[0.0, 2.0, f64::NAN]),
                            properties: vec![GraphicsPropertyUpdate::Marker(Marker::Circle)],
                            automatic_color: true,
                        }],
                        properties: Vec::new(),
                        axes_properties: Vec::new(),
                    },
                })
                .unwrap(),
        );
        session.take_pending_deltas();
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![chart],
                properties: vec![
                    GraphicsPropertyUpdate::Color([0.1, 0.2, 0.3, 1.0]),
                    GraphicsPropertyUpdate::LineWidth(2.5),
                    GraphicsPropertyUpdate::LineStyle(LineStyle::Dash),
                    GraphicsPropertyUpdate::Marker(Marker::Square),
                    GraphicsPropertyUpdate::MarkerSize(8.0),
                    GraphicsPropertyUpdate::ChartMarkerFaceColor(ChartColor::Rgba([
                        0.4, 0.5, 0.6, 1.0,
                    ])),
                    GraphicsPropertyUpdate::BaseValue(-2.0),
                ],
            })
            .unwrap();

        assert_eq!(
            response_property(&mut session, chart, GraphicsProperty::LineWidth),
            GraphicsPropertyValue::Scalar(2.5)
        );
        assert_eq!(
            response_property(&mut session, chart, GraphicsProperty::Marker),
            GraphicsPropertyValue::Marker(Marker::Square)
        );
        assert_eq!(
            response_property(&mut session, chart, GraphicsProperty::BaseValue),
            GraphicsPropertyValue::Scalar(-2.0)
        );
        let child = session.children(chart).unwrap()[0];
        assert!(matches!(
            session.object(child),
            Ok(GraphicsObject::LineSeries(properties))
                if properties.color_rgba == [0.1, 0.2, 0.3, 1.0]
                    && properties.line_width_points == 2.5
                    && properties.line_style == LineStyle::Dash
                    && properties.marker == Marker::Square
                    && properties.marker_size_points == 8.0
                    && properties.marker_face_color_rgba == [0.4, 0.5, 0.6, 1.0]
        ));
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![chart],
                properties: vec![GraphicsPropertyUpdate::ChartMarkerFaceColor(
                    ChartColor::Auto,
                )],
            })
            .unwrap();
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![chart],
                properties: vec![GraphicsPropertyUpdate::Color([0.2, 0.3, 0.4, 1.0])],
            })
            .unwrap();
        assert!(matches!(
            session.object(child),
            Ok(GraphicsObject::LineSeries(properties))
                if properties.color_rgba == [0.2, 0.3, 0.4, 1.0]
                    && properties.marker_face_color_rgba == [0.2, 0.3, 0.4, 1.0]
        ));

        let figure = session.current_figure().unwrap();
        let before = session.snapshot(figure).unwrap();
        session.take_pending_deltas();
        let invalid = session.execute(GraphicsRequest::SetProperties {
            handles: vec![chart],
            properties: vec![
                GraphicsPropertyUpdate::LineWidth(3.0),
                GraphicsPropertyUpdate::BarWidth(2.0),
            ],
        });
        assert_eq!(
            invalid.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidInput
        );
        assert_eq!(session.snapshot(figure).unwrap(), before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn scatter_scalar_properties_round_trip_without_allocating_data_resources() {
        let mut session = GraphicsSession::new();
        let execution = session
            .execute(GraphicsRequest::Scatter {
                axes: None,
                series: ScatterInput {
                    x: NumericData::from_f64(Arc::from([1.0, 2.0])),
                    y: NumericData::from_f64(Arc::from([3.0, 4.0])),
                    z: None,
                    size_data: None,
                    color_data: None,
                    filled: false,
                },
                properties: Vec::new(),
            })
            .unwrap();
        let GraphicsResponse::Handle(scatter) = execution.response else {
            panic!()
        };
        let resources = session.resource_count();
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![scatter],
                properties: vec![
                    GraphicsPropertyUpdate::SizeData(49.0),
                    GraphicsPropertyUpdate::CData([0.2, 0.4, 0.6, 1.0]),
                    GraphicsPropertyUpdate::Marker(Marker::TriangleUp),
                    GraphicsPropertyUpdate::Visible(false),
                ],
            })
            .unwrap();
        assert_eq!(session.resource_count(), resources);
        let query = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![scatter],
                property: GraphicsProperty::SizeData,
            })
            .unwrap();
        assert_eq!(
            query.response,
            GraphicsResponse::PropertyValues(vec![GraphicsPropertyValue::Scalar(49.0)])
        );
        assert!(matches!(
            session.object(scatter),
            Ok(GraphicsObject::ScatterSeries(value))
                if value.marker_size_points == 7.0
                    && value.face_color_rgba == [0.2, 0.4, 0.6, 1.0]
                    && value.edge_color_rgba == [0.2, 0.4, 0.6, 1.0]
                    && value.marker == Marker::TriangleUp
                    && !value.visible
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn scatter_vector_cdata_preserves_column_shape_and_drives_auto_clim() {
        let mut session = GraphicsSession::new();
        let execution = session
            .execute(GraphicsRequest::Scatter {
                axes: None,
                series: ScatterInput {
                    x: NumericData::from_f64(Arc::from([1.0, 2.0, 3.0])),
                    y: NumericData::from_f64(Arc::from([3.0, 2.0, 1.0])),
                    z: Some(NumericData::from_f64(Arc::from([2.0, 1.0, 3.0]))),
                    size_data: None,
                    color_data: Some(NumericData::from_f64(Arc::from([10.0, 20.0, 30.0]))),
                    filled: true,
                },
                properties: vec![GraphicsPropertyUpdate::SizeData(40.0)],
            })
            .unwrap();
        let GraphicsResponse::Handle(scatter) = execution.response else {
            panic!()
        };
        let axes = session.current_axes().unwrap();
        assert_eq!(
            session.axes_properties(axes).unwrap().c_limits,
            [10.0, 30.0]
        );
        let GraphicsObject::ScatterSeries(properties) = session.object(scatter).unwrap() else {
            panic!()
        };
        assert_eq!(properties.color_data_target, ScatterColorTarget::Face);
        assert_eq!(properties.color_data.as_ref().unwrap().shape, [3, 1]);
        let query = session
            .execute(GraphicsRequest::GetProperties {
                handles: vec![scatter],
                property: GraphicsProperty::CData,
            })
            .unwrap();
        assert_eq!(
            query.response,
            GraphicsResponse::PropertyValues(vec![GraphicsPropertyValue::NumericMatrix {
                shape: [3, 1],
                values: vec![10.0, 20.0, 30.0],
            }])
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn styled_colors_preserve_hold_color_order_and_replacement_rules() {
        let mut session = GraphicsSession::new();
        session
            .execute(GraphicsRequest::PlotStyled {
                axes: None,
                lines: vec![line(&[1.0], &[1.0])],
                properties: vec![GraphicsPropertyUpdate::Color([1.0, 0.0, 0.0, 1.0])],
            })
            .unwrap();
        session
            .execute(GraphicsRequest::SetHold {
                axes: None,
                enabled: true,
            })
            .unwrap();
        let second = session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[2.0], &[2.0])],
            })
            .unwrap();
        let GraphicsResponse::Handles(second) = second.response else {
            panic!()
        };
        assert!(matches!(
            session.object(second[0]),
            Ok(GraphicsObject::LineSeries(value)) if value.color_rgba == COLOR_ORDER[1]
        ));

        session
            .execute(GraphicsRequest::SetHold {
                axes: None,
                enabled: false,
            })
            .unwrap();
        let replacement = session
            .execute(GraphicsRequest::PlotStyled {
                axes: None,
                lines: vec![line(&[3.0], &[3.0])],
                properties: vec![GraphicsPropertyUpdate::LineStyle(LineStyle::Dot)],
            })
            .unwrap();
        let GraphicsResponse::Handles(replacement) = replacement.response else {
            panic!()
        };
        let axes = session.current_axes().unwrap();
        assert_eq!(session.children(axes).unwrap(), replacement);
        assert!(matches!(
            session.object(replacement[0]),
            Ok(GraphicsObject::LineSeries(value))
                if value.color_rgba == COLOR_ORDER[0] && value.line_style == LineStyle::Dot
        ));
        assert_eq!(session.axes_properties(axes).unwrap().color_order_index, 2);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn data_updates_recompute_only_automatic_limits_in_one_revision() {
        let mut session = GraphicsSession::new();
        let plotted = session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0], &[10.0, 20.0])],
            })
            .unwrap();
        let GraphicsResponse::Handles(lines) = plotted.response else {
            panic!()
        };
        let figure = session.current_figure().unwrap();
        let axes = session.current_axes().unwrap();
        let initial_revision = session.snapshot(figure).unwrap().revision;
        session.take_pending_deltas();

        session
            .execute(GraphicsRequest::SetProperties {
                handles: lines.clone(),
                properties: vec![
                    GraphicsPropertyUpdate::XData(NumericData::from_f64(Arc::from([
                        100.0, 200.0, 300.0,
                    ]))),
                    GraphicsPropertyUpdate::YData(NumericData::from_f64(Arc::from([
                        -3.0, 0.0, 9.0,
                    ]))),
                ],
            })
            .unwrap();
        let automatic = session.axes_properties(axes).unwrap();
        assert_eq!(automatic.x_limits, [100.0, 300.0]);
        assert_eq!(automatic.y_limits, [-3.0, 9.0]);
        assert_eq!(automatic.x_limit_mode, LimitMode::Auto);
        assert_eq!(automatic.y_limit_mode, LimitMode::Auto);
        let automatic_delta = session.take_pending_deltas();
        assert_eq!(automatic_delta.len(), 1);
        assert_eq!(automatic_delta[0].revision, initial_revision + 1);

        session
            .execute(GraphicsRequest::SetAxesLimits {
                figure,
                axes: None,
                x_limits: [0.0, 1.0],
                y_limits: [-1.0, 1.0],
            })
            .unwrap();
        session.take_pending_deltas();
        let manual_revision = session.snapshot(figure).unwrap().revision;
        session
            .execute(GraphicsRequest::SetProperties {
                handles: lines,
                properties: vec![
                    GraphicsPropertyUpdate::XData(NumericData::from_f64(Arc::from([
                        1_000.0, 2_000.0,
                    ]))),
                    GraphicsPropertyUpdate::YData(NumericData::from_f64(Arc::from([100.0, 200.0]))),
                ],
            })
            .unwrap();
        let manual = session.axes_properties(axes).unwrap();
        assert_eq!(manual.x_limits, [0.0, 1.0]);
        assert_eq!(manual.y_limits, [-1.0, 1.0]);
        assert_eq!(manual.x_limit_mode, LimitMode::Manual);
        assert_eq!(manual.y_limit_mode, LimitMode::Manual);
        let manual_delta = session.take_pending_deltas();
        assert_eq!(manual_delta.len(), 1);
        assert_eq!(manual_delta[0].revision, manual_revision + 1);
    }

    #[test]
    fn marker_indices_track_default_data_length_then_freeze_explicit_source_indices() {
        let mut session = GraphicsSession::new();
        let plotted = session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0])],
            })
            .unwrap();
        let GraphicsResponse::Handles(lines) = plotted.response else {
            panic!()
        };
        let handle = lines[0];
        assert!(matches!(
            session.object(handle),
            Ok(GraphicsObject::LineSeries(properties))
                if properties.marker_indices == [1, 2, 3]
                    && properties.marker_indices_automatic
        ));

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![handle],
                properties: vec![
                    GraphicsPropertyUpdate::XData(NumericData::from_f64(Arc::from([
                        1.0, 2.0, 3.0, 4.0, 5.0,
                    ]))),
                    GraphicsPropertyUpdate::YData(NumericData::from_f64(Arc::from([
                        1.0, 2.0, 3.0, 4.0, 5.0,
                    ]))),
                ],
            })
            .unwrap();
        assert!(matches!(
            session.object(handle),
            Ok(GraphicsObject::LineSeries(properties))
                if properties.marker_indices == [1, 2, 3, 4, 5]
                    && properties.marker_indices_automatic
        ));

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![handle],
                properties: vec![GraphicsPropertyUpdate::MarkerIndices {
                    values: vec![5, 2, 5, 9],
                    value_class: MarkerIndicesClass::Double,
                }],
            })
            .unwrap();
        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![handle],
                properties: vec![
                    GraphicsPropertyUpdate::XData(NumericData::from_f64(Arc::from([1.0, 2.0]))),
                    GraphicsPropertyUpdate::YData(NumericData::from_f64(Arc::from([3.0, 4.0]))),
                ],
            })
            .unwrap();
        assert!(matches!(
            session.object(handle),
            Ok(GraphicsObject::LineSeries(properties))
                if properties.marker_indices == [5, 2, 5, 9]
                    && !properties.marker_indices_automatic
        ));

        let figure = session.current_figure().unwrap();
        let before = session.snapshot(figure).unwrap();
        session.take_pending_deltas();
        let invalid = session.execute(GraphicsRequest::SetProperties {
            handles: vec![handle],
            properties: vec![GraphicsPropertyUpdate::MarkerIndices {
                values: vec![1, 0, 2],
                value_class: MarkerIndicesClass::Double,
            }],
        });
        assert_eq!(
            invalid.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidInput
        );
        assert_eq!(session.snapshot(figure).unwrap(), before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn axes_ticks_labels_style_and_modes_follow_measured_r2022b_transitions() {
        let mut session = GraphicsSession::new();
        let axes = response_handle(session.execute(GraphicsRequest::CurrentAxes).unwrap());
        let figure = session.current_figure().unwrap();
        session
            .execute(GraphicsRequest::SetAxesProperties {
                axes: Some(axes),
                properties: vec![
                    GraphicsPropertyUpdate::XTick(vec![0.0, 5.0, 10.0]),
                    GraphicsPropertyUpdate::XTickLabel(vec![
                        "A".encode_utf16().collect(),
                        "B".encode_utf16().collect(),
                    ]),
                    GraphicsPropertyUpdate::YTick(vec![f64::NEG_INFINITY, 0.0, f64::INFINITY]),
                    GraphicsPropertyUpdate::TickLabelInterpreter(Interpreter::Latex),
                    GraphicsPropertyUpdate::FontSize(12.0),
                    GraphicsPropertyUpdate::LineWidth(1.0),
                    GraphicsPropertyUpdate::Box(true),
                ],
            })
            .unwrap();
        session
            .execute(GraphicsRequest::SetLimits {
                axes: Some(axes),
                axis: Axis::X,
                limits: [0.0, 20.0],
            })
            .unwrap();
        let manual = session.axes_properties(axes).unwrap();
        assert_eq!(manual.x_ticks, [0.0, 5.0, 10.0]);
        assert_eq!(manual.x_tick_mode, TickMode::Manual);
        assert_eq!(manual.x_tick_label_mode, TickMode::Manual);
        assert_eq!(manual.x_tick_labels_utf16.len(), 2);
        assert_eq!(manual.y_ticks, [f64::NEG_INFINITY, 0.0, f64::INFINITY]);
        assert_eq!(manual.tick_label_interpreter, Interpreter::Latex);
        assert_eq!(manual.font_size_points, 12.0);
        assert_eq!(manual.line_width_points, 1.0);
        assert!(manual.box_enabled);

        session
            .execute(GraphicsRequest::SetAxesProperties {
                axes: Some(axes),
                properties: vec![GraphicsPropertyUpdate::XTickMode(TickMode::Auto)],
            })
            .unwrap();
        let automatic = session.axes_properties(axes).unwrap();
        assert_eq!(automatic.x_ticks, [0.0, 5.0, 10.0, 15.0, 20.0]);
        assert_eq!(automatic.x_tick_mode, TickMode::Auto);
        assert_eq!(automatic.x_tick_label_mode, TickMode::Manual);
        assert_eq!(automatic.x_tick_labels_utf16.len(), 2);

        let before = session.snapshot(figure).unwrap();
        session.take_pending_deltas();
        let invalid = session.execute(GraphicsRequest::SetAxesProperties {
            axes: Some(axes),
            properties: vec![GraphicsPropertyUpdate::XTick(vec![0.0, f64::NAN])],
        });
        assert_eq!(
            invalid.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidInput
        );
        assert_eq!(session.snapshot(figure).unwrap(), before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    fn text_and_legend_interpreters_are_semantic_model_fields() {
        let mut session = GraphicsSession::new();
        let title = response_handle(
            session
                .execute(GraphicsRequest::SetText {
                    axes: None,
                    role: TextRole::Title,
                    code_units: "$x$".encode_utf16().collect(),
                    properties: vec![GraphicsPropertyUpdate::Interpreter(Interpreter::Latex)],
                })
                .unwrap(),
        );
        assert!(matches!(
            session.object(title),
            Ok(GraphicsObject::Text(properties)) if properties.interpreter == Interpreter::Latex
        ));
        session
            .execute(GraphicsRequest::Plot {
                lines: vec![line(&[1.0, 2.0], &[3.0, 4.0])],
            })
            .unwrap();
        let legend = response_handle(
            session
                .execute(GraphicsRequest::Legend {
                    axes: None,
                    labels_utf16: vec!["$f$".encode_utf16().collect()],
                    properties: vec![
                        GraphicsPropertyUpdate::Interpreter(Interpreter::None),
                        GraphicsPropertyUpdate::Location(LegendLocation::Best),
                    ],
                })
                .unwrap(),
        );
        assert!(matches!(
            session.object(legend),
            Ok(GraphicsObject::Legend(properties))
                if properties.interpreter == Interpreter::None
                    && properties.location == LegendLocation::Best
        ));
    }

    #[test]
    fn colormap_updates_are_atomic_and_visible_in_snapshots() {
        let mut session = GraphicsSession::new();
        let axes = response_handle(session.execute(GraphicsRequest::CurrentAxes).unwrap());
        let figure = session.current_figure().unwrap();
        session.take_pending_deltas();
        let colors = vec![[0.0, 0.25, 0.5], [0.5, 0.75, 1.0]];
        let execution = session
            .execute(GraphicsRequest::SetColormap {
                axes: Some(axes),
                colors: colors.clone(),
            })
            .unwrap();
        assert_eq!(
            execution.response,
            GraphicsResponse::Colormap(colors.clone())
        );
        assert_eq!(session.axes_properties(axes).unwrap().colormap, colors);
        assert_eq!(session.take_pending_deltas().len(), 1);

        let before = session.snapshot(figure).unwrap();
        let invalid = session.execute(GraphicsRequest::SetColormap {
            axes: Some(axes),
            colors: vec![[0.0, f64::NAN, 1.0]],
        });
        assert_eq!(
            invalid.unwrap_err().category,
            crate::GraphicsErrorCategory::InvalidInput
        );
        assert_eq!(session.snapshot(figure).unwrap(), before);
        assert!(session.take_pending_deltas().is_empty());
    }

    #[test]
    fn colorbar_ticks_support_automatic_and_manual_property_state() {
        let mut session = GraphicsSession::new();
        let colorbar = response_handle(
            session
                .execute(GraphicsRequest::Colorbar {
                    axes: None,
                    visible: true,
                })
                .unwrap(),
        );
        assert!(matches!(
            session.object(colorbar),
            Ok(GraphicsObject::ColorBar(properties))
                if properties.tick_mode == TickMode::Auto
                    && properties.tick_label_mode == TickMode::Auto
                    && properties.ticks.len() == 11
                    && properties.ticks.first() == Some(&0.0)
                    && properties.ticks.last() == Some(&1.0)
        ));

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![colorbar],
                properties: vec![
                    GraphicsPropertyUpdate::Ticks(vec![0.25, 0.75]),
                    GraphicsPropertyUpdate::TickLabels(vec![
                        "low".encode_utf16().collect(),
                        "high".encode_utf16().collect(),
                    ]),
                ],
            })
            .unwrap();
        assert!(matches!(
            session.object(colorbar),
            Ok(GraphicsObject::ColorBar(properties))
                if properties.ticks == [0.25, 0.75]
                    && properties.tick_mode == TickMode::Manual
                    && properties.tick_label_mode == TickMode::Manual
        ));

        session
            .execute(GraphicsRequest::SetProperties {
                handles: vec![colorbar],
                properties: vec![
                    GraphicsPropertyUpdate::TicksMode(TickMode::Auto),
                    GraphicsPropertyUpdate::TickLabelsMode(TickMode::Auto),
                ],
            })
            .unwrap();
        assert!(matches!(
            session.object(colorbar),
            Ok(GraphicsObject::ColorBar(properties))
                if properties.ticks.len() == 11
                    && properties.tick_mode == TickMode::Auto
                    && properties.tick_label_mode == TickMode::Auto
                    && properties.tick_labels_utf16.len() == 11
        ));
    }
}
