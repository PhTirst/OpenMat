//! Adapter from the kernel-owned Graphics Object Model to shared graphics DTOs.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use openmat_graphics_model as model;
use openmat_plot_protocol as wire;

use crate::graphics::{
    BufferLeaseRelease, CloseFigureOutcome, GraphicsBuffer, GraphicsHubError,
    GraphicsHubInitialization, GraphicsSessionHub, SequencedGraphicsEvent, SetAxesCameraOutcome,
    SetAxesLimitsOutcome,
};

const CSS_PX_PER_POINT: f64 = 96.0 / 72.0;

#[derive(Clone)]
struct RetainedBuffer {
    bytes: Arc<[u8]>,
    figure_references: u64,
    client_leases: u64,
}

#[derive(Default)]
struct HubState {
    next_sequence: u64,
    events: Vec<SequencedGraphicsEvent>,
    buffers: HashMap<String, RetainedBuffer>,
}

/// One protocol adapter over the exact graphics hierarchy used by Runtime.
pub(crate) struct RuntimeGraphicsHub {
    session: Arc<Mutex<model::GraphicsSession>>,
    state: Mutex<HubState>,
}

impl RuntimeGraphicsHub {
    #[must_use]
    pub(crate) fn new(session: Arc<Mutex<model::GraphicsSession>>) -> Self {
        Self {
            session,
            state: Mutex::new(HubState {
                next_sequence: 1,
                ..HubState::default()
            }),
        }
    }

    /// Publishes committed model deltas after one kernel execution. The caller
    /// supplies the exact transaction queue drained from the same session.
    pub(crate) fn publish(
        &self,
        deltas: Vec<model::GraphicsDelta>,
    ) -> Result<(), GraphicsHubError> {
        let session = self.lock_session()?;
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        // Kernel discovery and graphics events travel over separate sockets.
        // If one execution creates and then mutates a Figure, publishing the
        // later positive-base delta can beat discovery and reach a client that
        // has never seen the Figure. The discovery path requests one complete
        // snapshot, so suppress every delta for Figures created in this batch.
        let created_figures: HashSet<String> = deltas
            .iter()
            .filter(|delta| delta.base_revision == 0)
            .map(|delta| delta.figure_id.clone())
            .collect();
        for delta in deltas {
            if created_figures.contains(delta.figure_id.as_str()) {
                // A newly discovered Figure is acquired by getSnapshot after
                // its MIME discovery. This includes later mutations committed
                // in the same execution batch.
                continue;
            }
            let event = if session.snapshot_by_id(&delta.figure_id).is_ok() {
                wire::Event::FigureDelta(convert_delta(&delta, &session)?)
            } else {
                wire::Event::FigureClosed(wire::FigureClosedEvent {
                    figure_id: delta.figure_id,
                    closed_revision: delta.revision,
                })
            };
            push_event(&mut state, event)?;
        }
        Ok(())
    }

    fn lock_session(&self) -> Result<MutexGuard<'_, model::GraphicsSession>, GraphicsHubError> {
        self.session.lock().map_err(|_| session_closed())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, HubState>, GraphicsHubError> {
        self.state.lock().map_err(|_| session_closed())
    }
}

impl GraphicsSessionHub for RuntimeGraphicsHub {
    fn initialize(&self) -> Result<GraphicsHubInitialization, GraphicsHubError> {
        let session = self.lock_session()?;
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let figures = snapshots
            .iter()
            .map(|snapshot| wire::FigureSummary {
                figure_id: snapshot.figure_id.clone(),
                revision: snapshot.revision,
            })
            .collect();
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        Ok(GraphicsHubInitialization {
            implementation: wire::ImplementationInfo {
                name: "openmat-runtime".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            limits: wire::GraphicsLimits::default(),
            figures,
            event_cursor: state.next_sequence.saturating_sub(1),
        })
    }

    fn get_snapshot(&self, figure_id: &str) -> Result<wire::FigureSnapshot, GraphicsHubError> {
        let session = self.lock_session()?;
        let snapshot = session.snapshot_by_id(figure_id).map_err(model_failure)?;
        let converted = convert_snapshot(&snapshot, &session)?;
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        Ok(converted)
    }

    fn get_buffer(
        &self,
        buffer_id: &str,
        acquire_lease: bool,
    ) -> Result<GraphicsBuffer, GraphicsHubError> {
        let mut state = self.lock_state()?;
        let retained = state.buffers.get_mut(buffer_id).ok_or_else(|| {
            GraphicsHubError::new(
                wire::ErrorCategory::UnknownBuffer,
                "unknown graphics buffer",
            )
        })?;
        if acquire_lease {
            retained.client_leases = retained.client_leases.checked_add(1).ok_or_else(|| {
                GraphicsHubError::new(
                    wire::ErrorCategory::ResidentLimit,
                    "graphics buffer lease count exhausted",
                )
            })?;
        }
        Ok(GraphicsBuffer {
            buffer_id: buffer_id.to_owned(),
            bytes: Arc::clone(&retained.bytes),
        })
    }

    fn release_buffer_lease(
        &self,
        buffer_id: &str,
    ) -> Result<BufferLeaseRelease, GraphicsHubError> {
        let mut state = self.lock_state()?;
        let retained = state.buffers.get_mut(buffer_id).ok_or_else(|| {
            GraphicsHubError::new(
                wire::ErrorCategory::UnknownBuffer,
                "unknown graphics buffer",
            )
        })?;
        if retained.client_leases == 0 {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::UnknownBuffer,
                "graphics buffer has no active lease",
            ));
        }
        retained.client_leases -= 1;
        let reclaimed = retained.client_leases == 0 && retained.figure_references == 0;
        if reclaimed {
            state.buffers.remove(buffer_id);
        }
        Ok(BufferLeaseRelease { reclaimed })
    }

    fn close_figure(
        &self,
        figure_id: &str,
        expected_revision: u64,
    ) -> Result<CloseFigureOutcome, GraphicsHubError> {
        let mut session = self.lock_session()?;
        let snapshot = session.snapshot_by_id(figure_id).map_err(model_failure)?;
        if snapshot.revision != expected_revision {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::RevisionConflict,
                "Figure revision does not match expectedRevision",
            ));
        }
        let figure = session.handle_by_id(figure_id).ok_or_else(|| {
            GraphicsHubError::new(wire::ErrorCategory::UnknownFigure, "unknown Figure")
        })?;
        let execution = session
            .execute(model::GraphicsRequest::CloseFigure {
                figure: Some(figure),
            })
            .map_err(model_failure)?;
        let notice = execution.notice.ok_or_else(|| {
            GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "close transaction did not produce a graphics notice",
            )
        })?;
        let deltas = session.take_pending_deltas();
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        for delta in deltas {
            if delta.figure_id == figure_id && delta.revision == notice.revision {
                push_event(
                    &mut state,
                    wire::Event::FigureClosed(wire::FigureClosedEvent {
                        figure_id: figure_id.to_owned(),
                        closed_revision: notice.revision,
                    }),
                )?;
            } else if delta.base_revision > 0 {
                push_event(
                    &mut state,
                    wire::Event::FigureDelta(convert_delta(&delta, &session)?),
                )?;
            }
        }
        Ok(CloseFigureOutcome {
            closed_revision: notice.revision,
            event_cursor: state.next_sequence.saturating_sub(1),
        })
    }

    fn set_axes_limits(
        &self,
        figure_id: &str,
        axes_id: Option<&str>,
        expected_revision: u64,
        x_limits: [f64; 2],
        y_limits: [f64; 2],
    ) -> Result<SetAxesLimitsOutcome, GraphicsHubError> {
        let mut session = self.lock_session()?;
        let before = session.snapshot_by_id(figure_id).map_err(model_failure)?;
        if before.revision != expected_revision {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::RevisionConflict,
                "Figure revision does not match expectedRevision",
            ));
        }
        if !before
            .objects
            .iter()
            .any(|object| object.class == model::GraphicsClass::Axes2D)
        {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidRequest,
                "Figure has no applicable 2D Axes",
            ));
        }
        let figure = session.handle_by_id(figure_id).ok_or_else(|| {
            GraphicsHubError::new(wire::ErrorCategory::UnknownFigure, "unknown Figure")
        })?;
        let axes = axes_id
            .map(|axes_id| {
                session.handle_by_id(axes_id).ok_or_else(|| {
                    GraphicsHubError::new(
                        wire::ErrorCategory::InvalidRequest,
                        "unknown target Axes",
                    )
                })
            })
            .transpose()?;
        let execution = session
            .execute(model::GraphicsRequest::SetAxesLimits {
                figure,
                axes,
                x_limits,
                y_limits,
            })
            .map_err(model_failure)?;
        let notice = execution.notice.ok_or_else(|| {
            GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "limits transaction did not produce a graphics notice",
            )
        })?;
        if notice.figure_id != figure_id
            || expected_revision.checked_add(1) != Some(notice.revision)
        {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "limits transaction produced an invalid Figure revision",
            ));
        }
        let deltas = session.take_pending_deltas();
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        let mut committed_delta_seen = false;
        for delta in deltas {
            if delta.figure_id == figure_id && delta.revision == notice.revision {
                if committed_delta_seen || delta.base_revision != expected_revision {
                    return Err(GraphicsHubError::new(
                        wire::ErrorCategory::InvalidScene,
                        "limits transaction produced an invalid delta set",
                    ));
                }
                committed_delta_seen = true;
            }
            if delta.base_revision > 0 {
                push_event(
                    &mut state,
                    wire::Event::FigureDelta(convert_delta(&delta, &session)?),
                )?;
            }
        }
        if !committed_delta_seen {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "limits transaction did not produce its Figure delta",
            ));
        }
        Ok(SetAxesLimitsOutcome {
            committed_revision: notice.revision,
            event_cursor: state.next_sequence.saturating_sub(1),
        })
    }

    fn set_axes_camera(
        &self,
        figure_id: &str,
        axes_id: Option<&str>,
        expected_revision: u64,
        view: [f64; 2],
        camera_scale: f64,
    ) -> Result<SetAxesCameraOutcome, GraphicsHubError> {
        let mut session = self.lock_session()?;
        let before = session.snapshot_by_id(figure_id).map_err(model_failure)?;
        if before.revision != expected_revision {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::RevisionConflict,
                "Figure revision does not match expectedRevision",
            ));
        }
        if !before
            .objects
            .iter()
            .any(|object| object.class == model::GraphicsClass::Axes2D)
        {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidRequest,
                "Figure has no applicable 3D Axes",
            ));
        }
        let figure = session.handle_by_id(figure_id).ok_or_else(|| {
            GraphicsHubError::new(wire::ErrorCategory::UnknownFigure, "unknown Figure")
        })?;
        let axes = axes_id
            .map(|axes_id| {
                session.handle_by_id(axes_id).ok_or_else(|| {
                    GraphicsHubError::new(
                        wire::ErrorCategory::InvalidRequest,
                        "unknown target Axes",
                    )
                })
            })
            .transpose()?;
        let execution = session
            .execute(model::GraphicsRequest::SetAxesCamera {
                figure,
                axes,
                azimuth_degrees: view[0],
                elevation_degrees: view[1],
                camera_scale,
            })
            .map_err(model_failure)?;
        let notice = execution.notice.ok_or_else(|| {
            GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "camera transaction did not produce a graphics notice",
            )
        })?;
        if notice.figure_id != figure_id
            || expected_revision.checked_add(1) != Some(notice.revision)
        {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "camera transaction produced an invalid Figure revision",
            ));
        }
        let deltas = session.take_pending_deltas();
        let snapshots = session.figure_snapshots().map_err(model_failure)?;
        let mut state = self.lock_state()?;
        synchronize_buffers(&mut state, &snapshots);
        let mut committed_delta_seen = false;
        for delta in deltas {
            if delta.figure_id == figure_id && delta.revision == notice.revision {
                if committed_delta_seen || delta.base_revision != expected_revision {
                    return Err(GraphicsHubError::new(
                        wire::ErrorCategory::InvalidScene,
                        "camera transaction produced an invalid delta set",
                    ));
                }
                committed_delta_seen = true;
            }
            if delta.base_revision > 0 {
                push_event(
                    &mut state,
                    wire::Event::FigureDelta(convert_delta(&delta, &session)?),
                )?;
            }
        }
        if !committed_delta_seen {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "camera transaction did not produce its Figure delta",
            ));
        }
        Ok(SetAxesCameraOutcome {
            committed_revision: notice.revision,
            event_cursor: state.next_sequence.saturating_sub(1),
        })
    }

    fn resync_figure(
        &self,
        figure_id: &str,
        _known_revision: u64,
    ) -> Result<wire::FigureSnapshot, GraphicsHubError> {
        self.get_snapshot(figure_id)
    }

    fn events_after(&self, cursor: u64) -> Result<Vec<SequencedGraphicsEvent>, GraphicsHubError> {
        let state = self.lock_state()?;
        Ok(state
            .events
            .iter()
            .filter(|event| event.sequence > cursor)
            .cloned()
            .collect())
    }
}

fn push_event(state: &mut HubState, event: wire::Event) -> Result<(), GraphicsHubError> {
    let sequence = state.next_sequence;
    state.next_sequence = sequence.checked_add(1).ok_or_else(|| {
        GraphicsHubError::new(
            wire::ErrorCategory::SessionClosed,
            "graphics event sequence exhausted",
        )
    })?;
    state
        .events
        .push(SequencedGraphicsEvent { sequence, event });
    Ok(())
}

fn synchronize_buffers(state: &mut HubState, snapshots: &[model::FigureSnapshot]) {
    for retained in state.buffers.values_mut() {
        retained.figure_references = 0;
    }
    for snapshot in snapshots {
        for resource in &snapshot.referenced_data {
            let buffer_id = buffer_id(resource.descriptor().id);
            let retained = state
                .buffers
                .entry(buffer_id)
                .or_insert_with(|| RetainedBuffer {
                    bytes: resource.shared_bytes(),
                    figure_references: 0,
                    client_leases: 0,
                });
            retained.figure_references = retained.figure_references.saturating_add(1);
        }
    }
    state
        .buffers
        .retain(|_, retained| retained.figure_references > 0 || retained.client_leases > 0);
}

fn convert_snapshot(
    snapshot: &model::FigureSnapshot,
    session: &model::GraphicsSession,
) -> Result<wire::FigureSnapshot, GraphicsHubError> {
    let converted = wire::FigureSnapshot {
        snapshot_type: "figureSnapshot".to_owned(),
        figure_id: snapshot.figure_id.clone(),
        revision: snapshot.revision,
        root_id: snapshot.root_id.clone(),
        objects: snapshot
            .objects
            .iter()
            .map(|object| convert_object(object, session))
            .collect::<Result<Vec<_>, _>>()?,
        referenced_buffers: snapshot
            .referenced_data
            .iter()
            .map(|resource| convert_data_ref(resource.descriptor()))
            .collect(),
    };
    converted
        .validate(wire::GraphicsLimits::default())
        .map_err(|error| {
            GraphicsHubError::new(wire::ErrorCategory::InvalidScene, error.to_string())
        })?;
    Ok(converted)
}

fn convert_delta(
    delta: &model::GraphicsDelta,
    session: &model::GraphicsSession,
) -> Result<wire::FigureDelta, GraphicsHubError> {
    let converted = wire::FigureDelta {
        figure_id: delta.figure_id.clone(),
        base_revision: delta.base_revision,
        revision: delta.revision,
        operations: delta
            .operations
            .iter()
            .map(|operation| match operation {
                model::GraphicsDeltaOperation::UpsertObject(object) => {
                    Ok(wire::DeltaOperation::UpsertObject {
                        object: convert_object(object, session)?,
                    })
                }
                model::GraphicsDeltaOperation::DeleteObject { id, generation } => {
                    Ok(wire::DeltaOperation::DeleteObject {
                        id: id.clone(),
                        generation: u64::from(*generation),
                    })
                }
                model::GraphicsDeltaOperation::ReorderChildren {
                    parent_id,
                    children,
                } => Ok(wire::DeltaOperation::ReorderChildren {
                    parent_id: parent_id.clone(),
                    children: children.clone(),
                }),
                model::GraphicsDeltaOperation::SetRoot { root_id } => {
                    Ok(wire::DeltaOperation::SetRoot {
                        root_id: root_id.clone(),
                    })
                }
            })
            .collect::<Result<Vec<_>, GraphicsHubError>>()?,
        added_buffers: delta.added_data.iter().map(convert_data_ref).collect(),
        released_buffer_ids: delta
            .released_data_ids
            .iter()
            .copied()
            .map(buffer_id)
            .collect(),
    };
    converted
        .validate_structure(wire::GraphicsLimits::default())
        .map_err(|error| {
            GraphicsHubError::new(wire::ErrorCategory::InvalidScene, error.to_string())
        })?;
    Ok(converted)
}

#[allow(clippy::too_many_lines)]
fn convert_object(
    object: &model::HirObject,
    session: &model::GraphicsSession,
) -> Result<wire::GraphicsObject, GraphicsHubError> {
    let fields = wire::ObjectFields {
        id: object.id.clone(),
        generation: u64::from(object.generation),
        object_revision: object.object_revision,
        parent_id: object.parent_id.clone(),
        children: object.children.clone(),
    };
    Ok(match &object.properties {
        model::HirProperties::TiledChartLayout(_) => {
            return Err(GraphicsHubError::new(
                wire::ErrorCategory::InvalidScene,
                "detached TiledChartLayout leaked into the render HIR",
            ));
        }
        model::HirProperties::Figure(properties) => wire::GraphicsObject::Figure {
            fields,
            properties: wire::FigureProperties {
                number: u64::from(properties.number),
                name_code_units: properties.name_utf16.clone(),
                number_title: properties.number_title,
                visible: properties.visible,
                background_rgba: rgba(properties.background_rgba),
                initial_logical_size_css_pixels: [
                    properties.position_css_pixels[2],
                    properties.position_css_pixels[3],
                ],
                position_css_pixels: properties.position_css_pixels,
                next_plot: match properties.next_plot {
                    model::NextPlot::Add => wire::FigureNextPlot::Add,
                    model::NextPlot::Replace => wire::FigureNextPlot::New,
                },
            },
        },
        model::HirProperties::Axes2D(properties) => wire::GraphicsObject::Axes2d {
            fields,
            properties: wire::Axes2DProperties {
                coordinate_system: match properties.coordinate_system {
                    model::AxesCoordinateSystem::Cartesian => wire::AxesCoordinateSystem::Cartesian,
                    model::AxesCoordinateSystem::Polar => wire::AxesCoordinateSystem::Polar,
                },
                position_normalized: properties.position_normalized,
                background_rgba: wire::Nullable(properties.background_rgba.map(rgba)),
                theta_axis_units: match properties.theta_axis_units {
                    model::ThetaAxisUnits::Degrees => wire::ThetaAxisUnits::Degrees,
                    model::ThetaAxisUnits::Radians => wire::ThetaAxisUnits::Radians,
                },
                theta_direction: match properties.theta_direction {
                    model::ThetaDirection::Counterclockwise => {
                        wire::ThetaDirection::Counterclockwise
                    }
                    model::ThetaDirection::Clockwise => wire::ThetaDirection::Clockwise,
                },
                theta_zero_location: match properties.theta_zero_location {
                    model::ThetaZeroLocation::Right => wire::ThetaZeroLocation::Right,
                    model::ThetaZeroLocation::Top => wire::ThetaZeroLocation::Top,
                    model::ThetaZeroLocation::Left => wire::ThetaZeroLocation::Left,
                    model::ThetaZeroLocation::Bottom => wire::ThetaZeroLocation::Bottom,
                },
                r_axis_location: properties.r_axis_location,
                x_scale: axis_scale(properties.x_scale),
                y_scale: axis_scale(properties.y_scale),
                z_scale: axis_scale(properties.z_scale),
                x_direction: axis_direction(properties.x_direction),
                y_direction: axis_direction(properties.y_direction),
                z_direction: axis_direction(properties.z_direction),
                visible: properties.visible,
                x_limits: properties.x_limits,
                y_limits: properties.y_limits,
                z_limits: properties.z_limits,
                x_limits_mode: limit_mode(properties.x_limit_mode),
                y_limits_mode: limit_mode(properties.y_limit_mode),
                z_limits_mode: limit_mode(properties.z_limit_mode),
                next_plot: next_plot(properties.next_plot),
                grid_x: properties.grid_x,
                grid_y: properties.grid_y,
                grid_z: properties.grid_z,
                minor_grid_x: properties.minor_grid_x,
                minor_grid_y: properties.minor_grid_y,
                minor_grid_z: properties.minor_grid_z,
                color_order_index: u64::from(properties.color_order_index),
                x_tick: tick_values(&properties.x_ticks)?,
                y_tick: tick_values(&properties.y_ticks)?,
                z_tick: tick_values(&properties.z_ticks)?,
                x_tick_label_code_units: properties.x_tick_labels_utf16.clone(),
                y_tick_label_code_units: properties.y_tick_labels_utf16.clone(),
                z_tick_label_code_units: properties.z_tick_labels_utf16.clone(),
                x_tick_mode: tick_mode(properties.x_tick_mode),
                y_tick_mode: tick_mode(properties.y_tick_mode),
                z_tick_mode: tick_mode(properties.z_tick_mode),
                x_tick_label_mode: tick_mode(properties.x_tick_label_mode),
                y_tick_label_mode: tick_mode(properties.y_tick_label_mode),
                z_tick_label_mode: tick_mode(properties.z_tick_label_mode),
                view: [
                    properties.view_azimuth_degrees,
                    properties.view_elevation_degrees,
                ],
                projection: match properties.projection {
                    model::ProjectionMode::Orthographic => wire::Projection::Orthographic,
                    model::ProjectionMode::Perspective => wire::Projection::Perspective,
                },
                camera_scale: properties.camera_scale,
                data_aspect_ratio: properties.data_aspect_ratio,
                data_aspect_ratio_mode: limit_mode(properties.data_aspect_ratio_mode),
                plot_box_aspect_ratio: properties.plot_box_aspect_ratio,
                plot_box_aspect_ratio_mode: limit_mode(properties.plot_box_aspect_ratio_mode),
                c_limits: properties.c_limits,
                c_limits_mode: limit_mode(properties.c_limit_mode),
                colormap: Some(properties.colormap.clone()),
                colorbar_visible: properties.colorbar_visible,
                box_enabled: properties.box_enabled,
                font_size_css_px: points_to_css_px(properties.font_size_points),
                font_family_code_units: properties.font_family_utf16.clone(),
                tick_direction: match properties.tick_direction {
                    model::TickDirection::In => wire::TickDirection::In,
                    model::TickDirection::Out => wire::TickDirection::Out,
                    model::TickDirection::Both => wire::TickDirection::Both,
                },
                line_width_css_px: points_to_css_px(properties.line_width_points),
                tick_label_interpreter: interpreter(properties.tick_label_interpreter),
                title_id: wire::Nullable(resolve_id(session, properties.title)?),
                x_label_id: wire::Nullable(resolve_id(session, properties.x_label)?),
                y_label_id: wire::Nullable(resolve_id(session, properties.y_label)?),
                z_label_id: wire::Nullable(resolve_id(session, properties.z_label)?),
            },
        },
        model::HirProperties::LineSeries(properties) => wire::GraphicsObject::LineSeries {
            fields,
            properties: wire::LineSeriesProperties {
                x_data: convert_data_ref(&properties.x_data),
                y_data: convert_data_ref(&properties.y_data),
                z_data: wire::Nullable(properties.z_data.as_ref().map(convert_data_ref)),
                color_rgba: rgba(properties.color_rgba),
                line_width_css_px: points_to_css_px(properties.line_width_points),
                line_style: line_style(properties.line_style),
                marker: marker(properties.marker),
                marker_size_css_px: points_to_css_px(properties.marker_size_points),
                marker_face_rgba: Some(rgba(properties.marker_face_color_rgba)),
                marker_edge_rgba: Some(rgba(properties.marker_edge_color_rgba)),
                marker_indices: if properties.marker_indices_automatic {
                    None
                } else {
                    Some(marker_indices(&properties.marker_indices)?)
                },
                display_name_code_units: properties.display_name_utf16.clone(),
                visible: properties.visible,
                clipping: properties.clipping,
            },
        },
        model::HirProperties::ScatterSeries(properties) => wire::GraphicsObject::ScatterSeries {
            fields,
            properties: wire::ScatterSeriesProperties {
                x_data: convert_data_ref(&properties.x_data),
                y_data: convert_data_ref(&properties.y_data),
                z_data: wire::Nullable(properties.z_data.as_ref().map(convert_data_ref)),
                size_data: wire::Nullable(properties.size_data.as_ref().map(convert_data_ref)),
                color_data: wire::Nullable(properties.color_data.as_ref().map(convert_data_ref)),
                color_data_target: match properties.color_data_target {
                    model::ScatterColorTarget::None => wire::ScatterColorTarget::None,
                    model::ScatterColorTarget::Face => wire::ScatterColorTarget::Face,
                    model::ScatterColorTarget::Edge => wire::ScatterColorTarget::Edge,
                },
                marker: marker(properties.marker),
                marker_size_css_px: points_to_css_px(properties.marker_size_points),
                marker_edge_width_css_px: points_to_css_px(properties.line_width_points),
                marker_face_rgba: rgba(properties.face_color_rgba),
                marker_edge_rgba: rgba(properties.edge_color_rgba),
                display_name_code_units: properties.display_name_utf16.clone(),
                visible: properties.visible,
                clipping: properties.clipping,
            },
        },
        model::HirProperties::SurfaceSeries(properties) => {
            let (face_color, face_rgba) = surface_color(properties.face_color);
            let (edge_color, edge_rgba) = surface_color(properties.edge_color);
            wire::GraphicsObject::SurfaceSeries {
                fields,
                properties: wire::SurfaceSeriesProperties {
                    x_data: convert_data_ref(&properties.x_data),
                    y_data: convert_data_ref(&properties.y_data),
                    z_data: convert_data_ref(&properties.z_data),
                    c_data: convert_data_ref(&properties.c_data),
                    face_color,
                    face_rgba: wire::Nullable(face_rgba),
                    edge_color,
                    edge_rgba: wire::Nullable(edge_rgba),
                    line_width_css_px: points_to_css_px(properties.line_width_points),
                    line_style: line_style(properties.line_style),
                    c_data_mapping: match properties.c_data_mapping {
                        model::CDataMapping::Scaled => wire::CDataMapping::Scaled,
                        model::CDataMapping::Direct => wire::CDataMapping::Direct,
                    },
                    face_alpha: f64::from(properties.face_alpha),
                    lighting_enabled: properties.lighting_enabled,
                    visible: properties.visible,
                    clipping: properties.clipping,
                },
            }
        }
        model::HirProperties::PatchSeries(properties) => {
            let (face_color, face_rgba) = surface_color(properties.face_color);
            let (edge_color, edge_rgba) = surface_color(properties.edge_color);
            wire::GraphicsObject::PatchSeries {
                fields,
                properties: wire::PatchSeriesProperties {
                    faces: convert_data_ref(&properties.faces),
                    vertices: convert_data_ref(&properties.vertices),
                    face_vertex_cdata: convert_data_ref(&properties.face_vertex_cdata),
                    vertex_normals: wire::Nullable(
                        properties.vertex_normals.as_ref().map(convert_data_ref),
                    ),
                    face_color,
                    face_rgba: wire::Nullable(face_rgba),
                    edge_color,
                    edge_rgba: wire::Nullable(edge_rgba),
                    line_width_css_px: points_to_css_px(properties.line_width_points),
                    line_style: line_style(properties.line_style),
                    c_data_mapping: match properties.c_data_mapping {
                        model::CDataMapping::Scaled => wire::CDataMapping::Scaled,
                        model::CDataMapping::Direct => wire::CDataMapping::Direct,
                    },
                    face_alpha: f64::from(properties.face_alpha),
                    edge_alpha: f64::from(properties.edge_alpha),
                    lighting_enabled: properties.lighting_enabled,
                    smooth_normals: properties.smooth_normals,
                    visible: properties.visible,
                    clipping: properties.clipping,
                },
            }
        }
        model::HirProperties::ChartGroup(properties) => wire::GraphicsObject::ChartGroup {
            fields,
            properties: wire::ChartGroupProperties {
                chart_type: match properties.kind {
                    model::ChartGroupKind::Stair => wire::ChartType::Stair,
                    model::ChartGroupKind::Stem => wire::ChartType::Stem,
                    model::ChartGroupKind::ErrorBar => wire::ChartType::ErrorBar,
                    model::ChartGroupKind::Area => wire::ChartType::Area,
                    model::ChartGroupKind::Bar => wire::ChartType::Bar,
                    model::ChartGroupKind::Histogram => wire::ChartType::Histogram,
                    model::ChartGroupKind::Contour => wire::ChartType::Contour,
                    model::ChartGroupKind::Image => wire::ChartType::Image,
                },
                visible: properties.visible,
                color: wire::Nullable(properties.color.map(chart_color)),
                line_width_css_px: points_to_css_px(properties.line_width_points),
                line_style: line_style(properties.line_style),
                marker: wire::Nullable(properties.marker.map(marker)),
                marker_size_css_px: wire::Nullable(
                    properties.marker_size_points.map(points_to_css_px),
                ),
                marker_face_color: wire::Nullable(properties.marker_face_color.map(chart_color)),
                marker_edge_color: wire::Nullable(properties.marker_edge_color.map(chart_color)),
                face_color: wire::Nullable(properties.face_color.map(chart_color)),
                edge_color: wire::Nullable(properties.edge_color.map(chart_color)),
                face_alpha: wire::Nullable(properties.face_alpha.map(f64::from)),
                edge_alpha: wire::Nullable(properties.edge_alpha.map(f64::from)),
                base_value: wire::Nullable(properties.base_value),
                bar_width: wire::Nullable(properties.bar_width),
                cap_size_css_px: wire::Nullable(
                    properties
                        .cap_size_points
                        .map(|value| value * CSS_PX_PER_POINT),
                ),
            },
        },
        model::HirProperties::Text(properties) => wire::GraphicsObject::Text {
            fields,
            properties: wire::TextProperties {
                code_units: properties.code_units.clone(),
                role: match properties.role {
                    model::TextRole::Title => wire::TextRole::Title,
                    model::TextRole::XLabel => wire::TextRole::XLabel,
                    model::TextRole::YLabel => wire::TextRole::YLabel,
                    model::TextRole::ZLabel => wire::TextRole::ZLabel,
                },
                anchor_normalized: properties.anchor_normalized,
                horizontal_alignment: match properties.horizontal_alignment {
                    model::HorizontalAlignment::Left => wire::HorizontalAlignment::Left,
                    model::HorizontalAlignment::Center => wire::HorizontalAlignment::Center,
                    model::HorizontalAlignment::Right => wire::HorizontalAlignment::Right,
                },
                vertical_alignment: match properties.vertical_alignment {
                    model::VerticalAlignment::Bottom => wire::VerticalAlignment::Bottom,
                    model::VerticalAlignment::Middle => wire::VerticalAlignment::Middle,
                    model::VerticalAlignment::Top => wire::VerticalAlignment::Top,
                    model::VerticalAlignment::Baseline => wire::VerticalAlignment::Baseline,
                },
                color_rgba: rgba(properties.color_rgba),
                font_family_code_units: properties.font_family_utf16.clone(),
                font_size_css_px: f64::from(properties.font_size_css_px),
                font_weight: properties.font_weight,
                font_style: match properties.font_style {
                    model::FontStyle::Normal => wire::FontStyle::Normal,
                    model::FontStyle::Italic => wire::FontStyle::Italic,
                },
                interpreter: interpreter(properties.interpreter),
                rotation_degrees: f64::from(properties.rotation_degrees),
                visible: properties.visible,
            },
        },
        model::HirProperties::Legend(properties) => wire::GraphicsObject::Legend {
            fields,
            properties: wire::LegendProperties {
                series_ids: properties
                    .series
                    .iter()
                    .copied()
                    .map(|handle| {
                        session
                            .object_id(handle)
                            .map(str::to_owned)
                            .map_err(model_failure)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                label_code_units: properties.labels_utf16.clone(),
                location: match properties.location {
                    model::LegendLocation::Best => wire::LegendLocation::Best,
                    model::LegendLocation::Northeast => wire::LegendLocation::NorthEast,
                    model::LegendLocation::Northwest => wire::LegendLocation::NorthWest,
                    model::LegendLocation::SouthOutside => wire::LegendLocation::SouthOutside,
                },
                visible: properties.visible,
                background_rgba: rgba(properties.background_rgba),
                border_rgba: rgba(properties.border_rgba),
                font_family_code_units: properties.font_family_utf16.clone(),
                font_size_css_px: f64::from(properties.font_size_css_px),
                font_weight: properties.font_weight,
                interpreter: interpreter(properties.interpreter),
                orientation: match properties.orientation {
                    model::LegendOrientation::Vertical => wire::LegendOrientation::Vertical,
                    model::LegendOrientation::Horizontal => wire::LegendOrientation::Horizontal,
                },
                num_columns: properties.num_columns,
            },
        },
        model::HirProperties::ColorBar(properties) => wire::GraphicsObject::ColorBar {
            fields,
            properties: wire::ColorBarProperties {
                visible: properties.visible,
                ticks: if properties.tick_mode == model::TickMode::Auto {
                    Vec::new()
                } else {
                    tick_values(&properties.ticks)?
                },
                ticks_mode: tick_mode(properties.tick_mode),
                tick_label_code_units: properties.tick_labels_utf16.clone(),
                tick_labels_mode: tick_mode(properties.tick_label_mode),
            },
        },
    })
}

fn resolve_id(
    session: &model::GraphicsSession,
    handle: Option<model::GraphicsHandle>,
) -> Result<Option<String>, GraphicsHubError> {
    handle
        .map(|handle| {
            session
                .object_id(handle)
                .map(str::to_owned)
                .map_err(model_failure)
        })
        .transpose()
}

fn convert_data_ref(reference: &model::DataRef) -> wire::DataRef {
    wire::DataRef {
        buffer_id: buffer_id(reference.id),
        dtype: match reference.dtype {
            model::DataDType::F32 => wire::DataType::F32,
            model::DataDType::F64 => wire::DataType::F64,
        },
        shape: reference.shape.to_vec(),
        order: wire::StorageOrder::ColumnMajor,
        endianness: wire::Endianness::Little,
        byte_offset: 0,
        byte_length: reference.byte_length,
    }
}

fn buffer_id(id: model::DataResourceId) -> String {
    format!("buffer-{}", id.identifier())
}

const fn limit_mode(mode: model::LimitMode) -> wire::AxesLimitMode {
    match mode {
        model::LimitMode::Auto => wire::AxesLimitMode::Auto,
        model::LimitMode::Manual => wire::AxesLimitMode::Manual,
    }
}

const fn axis_scale(scale: model::AxisScale) -> wire::Scale {
    match scale {
        model::AxisScale::Linear => wire::Scale::Linear,
        model::AxisScale::Log => wire::Scale::Log,
    }
}

const fn axis_direction(direction: model::AxisDirection) -> wire::AxisDirection {
    match direction {
        model::AxisDirection::Normal => wire::AxisDirection::Normal,
        model::AxisDirection::Reverse => wire::AxisDirection::Reverse,
    }
}

const fn tick_mode(mode: model::TickMode) -> wire::AxesTickMode {
    match mode {
        model::TickMode::Auto => wire::AxesTickMode::Auto,
        model::TickMode::Manual => wire::AxesTickMode::Manual,
    }
}

const fn interpreter(value: model::Interpreter) -> wire::TextInterpreter {
    match value {
        model::Interpreter::Tex => wire::TextInterpreter::Tex,
        model::Interpreter::Latex => wire::TextInterpreter::Latex,
        model::Interpreter::None => wire::TextInterpreter::None,
    }
}

fn tick_values(values: &[f64]) -> Result<Vec<wire::TickValue>, GraphicsHubError> {
    values
        .iter()
        .copied()
        .map(|value| {
            wire::TickValue::new(value).ok_or_else(|| {
                GraphicsHubError::new(
                    wire::ErrorCategory::InvalidScene,
                    "graphics model produced a NaN tick value",
                )
            })
        })
        .collect()
}

fn marker_indices(values: &[u64]) -> Result<Vec<wire::MarkerIndex>, GraphicsHubError> {
    values
        .iter()
        .copied()
        .map(|value| {
            wire::MarkerIndex::new(value).ok_or_else(|| {
                GraphicsHubError::new(
                    wire::ErrorCategory::InvalidScene,
                    "graphics model produced a zero marker index",
                )
            })
        })
        .collect()
}

const fn next_plot(value: model::NextPlot) -> wire::NextPlot {
    match value {
        model::NextPlot::Replace => wire::NextPlot::Replace,
        model::NextPlot::Add => wire::NextPlot::Add,
    }
}

const fn line_style(value: model::LineStyle) -> wire::LineStyle {
    match value {
        model::LineStyle::None => wire::LineStyle::None,
        model::LineStyle::Solid => wire::LineStyle::Solid,
        model::LineStyle::Dash => wire::LineStyle::Dash,
        model::LineStyle::Dot => wire::LineStyle::Dot,
        model::LineStyle::DashDot => wire::LineStyle::DashDot,
    }
}

const fn marker(value: model::Marker) -> wire::Marker {
    match value {
        model::Marker::None => wire::Marker::None,
        model::Marker::Point => wire::Marker::Point,
        model::Marker::Circle => wire::Marker::Circle,
        model::Marker::Plus => wire::Marker::Plus,
        model::Marker::Star => wire::Marker::Star,
        model::Marker::Cross => wire::Marker::Cross,
        model::Marker::Square => wire::Marker::Square,
        model::Marker::Diamond => wire::Marker::Diamond,
        model::Marker::TriangleUp => wire::Marker::TriangleUp,
        model::Marker::TriangleDown => wire::Marker::TriangleDown,
        model::Marker::TriangleRight => wire::Marker::TriangleRight,
        model::Marker::TriangleLeft => wire::Marker::TriangleLeft,
        model::Marker::HorizontalLine => wire::Marker::HorizontalLine,
        model::Marker::VerticalLine => wire::Marker::VerticalLine,
    }
}

fn rgba(value: [f32; 4]) -> wire::Rgba {
    wire::Rgba(value.map(f64::from))
}

fn surface_color(value: model::SurfaceColor) -> (wire::SurfaceColorMode, Option<wire::Rgba>) {
    match value {
        model::SurfaceColor::Flat => (wire::SurfaceColorMode::Flat, None),
        model::SurfaceColor::Interp => (wire::SurfaceColorMode::Interp, None),
        model::SurfaceColor::None => (wire::SurfaceColorMode::None, None),
        model::SurfaceColor::Rgba(color) => (wire::SurfaceColorMode::Uniform, Some(rgba(color))),
    }
}

fn chart_color(value: model::ChartColor) -> wire::ChartColorProperties {
    match value {
        model::ChartColor::Auto => wire::ChartColorProperties {
            mode: wire::ChartColorMode::Auto,
            rgba: wire::Nullable(None),
        },
        model::ChartColor::None => wire::ChartColorProperties {
            mode: wire::ChartColorMode::None,
            rgba: wire::Nullable(None),
        },
        model::ChartColor::Rgba(color) => wire::ChartColorProperties {
            mode: wire::ChartColorMode::Uniform,
            rgba: wire::Nullable(Some(rgba(color))),
        },
    }
}

fn points_to_css_px(value: f32) -> f64 {
    f64::from(value) * CSS_PX_PER_POINT
}

fn model_failure(error: model::GraphicsError) -> GraphicsHubError {
    let category = match error.category {
        model::GraphicsErrorCategory::Unsupported => wire::ErrorCategory::UnsupportedRequest,
        model::GraphicsErrorCategory::InvalidHandle => wire::ErrorCategory::UnknownFigure,
        model::GraphicsErrorCategory::InvalidInput => wire::ErrorCategory::InvalidRequest,
        model::GraphicsErrorCategory::Limit => wire::ErrorCategory::ResidentLimit,
        model::GraphicsErrorCategory::InvalidState => wire::ErrorCategory::InvalidScene,
    };
    GraphicsHubError::new(category, error.message)
}

fn session_closed() -> GraphicsHubError {
    GraphicsHubError::new(
        wire::ErrorCategory::SessionClosed,
        "graphics session synchronization failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_model_snapshot_buffer_delta_and_close_share_one_session() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        let execution = session
            .lock()
            .unwrap()
            .execute(model::GraphicsRequest::Plot {
                lines: vec![model::LineInput {
                    x: model::NumericData::from_f64(vec![1.0, 2.0, 3.0]),
                    y: model::NumericData::from_f64(vec![4.0, 5.0, 6.0]),
                    z: None,
                }],
            })
            .unwrap();
        assert!(execution.notice.unwrap().discovery);
        let deltas = session.lock().unwrap().take_pending_deltas();
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();

        let initialized = hub.initialize().unwrap();
        assert_eq!(initialized.figures.len(), 1);
        assert_eq!(initialized.event_cursor, 0);
        let figure = &initialized.figures[0];
        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        snapshot.validate(wire::GraphicsLimits::default()).unwrap();
        let line_properties = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::LineSeries { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert!((line_properties.line_width_css_px - (2.0 / 3.0)).abs() < f64::EPSILON);
        assert_eq!(
            line_properties.marker_size_css_px.to_bits(),
            8.0_f64.to_bits()
        );
        assert!(line_properties.marker_indices.is_none());
        assert_eq!(snapshot.referenced_buffers.len(), 2);
        let buffer_id = snapshot.referenced_buffers[0].buffer_id.clone();
        let buffer = hub.get_buffer(&buffer_id, true).unwrap();
        assert_eq!(buffer.bytes.len(), 24);

        let closed = hub
            .close_figure(&figure.figure_id, figure.revision)
            .unwrap();
        assert_eq!(closed.closed_revision, figure.revision + 1);
        assert!(hub.get_snapshot(&figure.figure_id).is_err());
        assert!(matches!(
            hub.events_after(0).unwrap().as_slice(),
            [SequencedGraphicsEvent {
                event: wire::Event::FigureClosed(_),
                ..
            }]
        ));
        assert!(hub.release_buffer_lease(&buffer_id).unwrap().reclaimed);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn converts_structured_surface_resources_and_camera_without_json_data() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        session
            .lock()
            .unwrap()
            .execute(model::GraphicsRequest::Surface {
                axes: None,
                surface: model::SurfaceInput {
                    x: model::NumericData::from_f64(vec![1.0, 2.0, 3.0]),
                    y: model::NumericData::from_f64(vec![1.0, 2.0]),
                    z: model::NumericData::from_f32(vec![1.0, 2.0, 2.0, 5.0, 3.0, 4.0]),
                    c: model::NumericData::from_f32(vec![1.0, 2.0, 2.0, 5.0, 3.0, 4.0]),
                    rows: 2,
                    columns: 3,
                },
                style: model::SurfaceStyle::Surf,
                properties: Vec::new(),
            })
            .unwrap();
        let deltas = session.lock().unwrap().take_pending_deltas();
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        let figure = hub.initialize().unwrap().figures.remove(0);
        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        snapshot.validate(wire::GraphicsLimits::default()).unwrap();

        let axes = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::Axes2d { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(axes.view, [-37.5, 30.0]);
        assert_eq!(axes.z_limits, [1.0, 5.0]);
        assert_eq!(axes.c_limits, [1.0, 5.0]);
        assert_eq!(axes.projection, wire::Projection::Orthographic);

        let surface = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::SurfaceSeries { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(surface.x_data.shape, [1, 3]);
        assert_eq!(surface.y_data.shape, [2, 1]);
        assert_eq!(surface.z_data.shape, [2, 3]);
        assert_eq!(surface.c_data.shape, [2, 3]);
        assert_eq!(surface.z_data.dtype, wire::DataType::F32);
        assert_eq!(surface.face_color, wire::SurfaceColorMode::Flat);
        assert_eq!(surface.edge_color, wire::SurfaceColorMode::Uniform);
        assert_eq!(snapshot.referenced_buffers.len(), 4);
        assert!(snapshot.referenced_buffers.iter().all(|descriptor| {
            let buffer = hub.get_buffer(&descriptor.buffer_id, true).unwrap();
            buffer.bytes.len() == usize::try_from(descriptor.byte_length).unwrap()
        }));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn maps_r2022b_visual_properties_without_losing_modes_or_indices() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        {
            let mut session = session.lock().unwrap();
            let model::GraphicsResponse::Handle(axes) = session
                .execute(model::GraphicsRequest::CurrentAxes)
                .unwrap()
                .response
            else {
                unreachable!()
            };
            session
                .execute(model::GraphicsRequest::PlotStyled {
                    axes: Some(axes),
                    lines: vec![model::LineInput {
                        x: model::NumericData::from_f64(vec![1.0, 2.0, 3.0]),
                        y: model::NumericData::from_f64(vec![4.0, 5.0, 6.0]),
                        z: None,
                    }],
                    properties: vec![
                        model::GraphicsPropertyUpdate::Marker(model::Marker::Circle),
                        model::GraphicsPropertyUpdate::MarkerIndices {
                            values: vec![3, 1, 3, 8],
                            value_class: model::MarkerIndicesClass::Double,
                        },
                    ],
                })
                .unwrap();
            session
                .execute(model::GraphicsRequest::SetAxesProperties {
                    axes: Some(axes),
                    properties: vec![
                        model::GraphicsPropertyUpdate::XTick(vec![0.0, 5.0, f64::INFINITY]),
                        model::GraphicsPropertyUpdate::XTickLabel(vec![
                            "$x$".encode_utf16().collect(),
                        ]),
                        model::GraphicsPropertyUpdate::TickLabelInterpreter(
                            model::Interpreter::Latex,
                        ),
                        model::GraphicsPropertyUpdate::FontSize(12.0),
                        model::GraphicsPropertyUpdate::LineWidth(1.0),
                        model::GraphicsPropertyUpdate::Box(true),
                    ],
                })
                .unwrap();
            session
                .execute(model::GraphicsRequest::SetColormap {
                    axes: Some(axes),
                    colors: vec![[0.0, 0.25, 0.5], [0.5, 0.75, 1.0]],
                })
                .unwrap();
            session
                .execute(model::GraphicsRequest::SetText {
                    axes: Some(axes),
                    role: model::TextRole::Title,
                    code_units: "$x^2$".encode_utf16().collect(),
                    properties: vec![model::GraphicsPropertyUpdate::Interpreter(
                        model::Interpreter::Latex,
                    )],
                })
                .unwrap();
            session
                .execute(model::GraphicsRequest::Legend {
                    axes: Some(axes),
                    labels_utf16: vec!["literal_$".encode_utf16().collect()],
                    properties: vec![
                        model::GraphicsPropertyUpdate::Interpreter(model::Interpreter::None),
                        model::GraphicsPropertyUpdate::Location(model::LegendLocation::Northwest),
                    ],
                })
                .unwrap();
        }

        let deltas = session.lock().unwrap().take_pending_deltas();
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        let figure = hub.initialize().unwrap().figures.remove(0);
        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        snapshot.validate(wire::GraphicsLimits::default()).unwrap();

        let axes = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::Axes2d { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            axes.x_tick
                .iter()
                .map(|tick| tick.get())
                .collect::<Vec<_>>(),
            vec![0.0, 5.0, f64::INFINITY]
        );
        assert_eq!(axes.x_tick_mode, wire::AxesTickMode::Manual);
        assert_eq!(axes.x_tick_label_mode, wire::AxesTickMode::Manual);
        assert_eq!(axes.x_tick_label_code_units.len(), 1);
        assert!(axes.box_enabled);
        assert_eq!(axes.tick_label_interpreter, wire::TextInterpreter::Latex);
        assert_eq!(axes.font_size_css_px.to_bits(), 16.0_f64.to_bits());
        assert_eq!(axes.line_width_css_px.to_bits(), (4.0_f64 / 3.0).to_bits());
        assert_eq!(
            axes.colormap.as_deref(),
            Some([[0.0, 0.25, 0.5], [0.5, 0.75, 1.0]].as_slice())
        );

        let line = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::LineSeries { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            line.marker_indices
                .as_ref()
                .unwrap()
                .iter()
                .map(|index| index.get())
                .collect::<Vec<_>>(),
            vec![3, 1, 3, 8]
        );
        assert!(snapshot.objects.iter().any(|object| matches!(
            object,
            wire::GraphicsObject::Text { properties, .. }
                if properties.interpreter == wire::TextInterpreter::Latex
        )));
        assert!(snapshot.objects.iter().any(|object| matches!(
            object,
            wire::GraphicsObject::Legend { properties, .. }
                if properties.interpreter == wire::TextInterpreter::None
                    && properties.location == wire::LegendLocation::NorthWest
        )));
    }

    #[test]
    fn new_figure_batch_waits_for_discovery_snapshot_before_publishing_deltas() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        let deltas = {
            let mut session = session.lock().unwrap();
            session
                .execute(model::GraphicsRequest::Figure { number: None })
                .unwrap();
            session
                .execute(model::GraphicsRequest::Plot {
                    lines: vec![model::LineInput {
                        x: model::NumericData::from_f64(vec![1.0, 2.0]),
                        y: model::NumericData::from_f64(vec![2.0, 1.0]),
                        z: None,
                    }],
                })
                .unwrap();
            session.take_pending_deltas()
        };
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].base_revision, 0);
        assert!(deltas[1].base_revision > 0);
        assert_eq!(deltas[0].figure_id, deltas[1].figure_id);

        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        assert!(hub.events_after(0).unwrap().is_empty());
        let initialized = hub.initialize().unwrap();
        assert_eq!(initialized.figures.len(), 1);
        assert_eq!(initialized.figures[0].revision, 2);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn semantic_limits_use_one_model_request_revision_and_journal_delta() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        session
            .lock()
            .unwrap()
            .execute(model::GraphicsRequest::Plot {
                lines: vec![model::LineInput {
                    x: model::NumericData::from_f64(vec![1.0, 2.0, 3.0]),
                    y: model::NumericData::from_f64(vec![4.0, 5.0, 6.0]),
                    z: None,
                }],
            })
            .unwrap();
        let deltas = session.lock().unwrap().take_pending_deltas();
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        let figure = hub.initialize().unwrap().figures.remove(0);

        let outcome = hub
            .set_axes_limits(
                &figure.figure_id,
                None,
                figure.revision,
                [-2.0, 8.0],
                [0.25, 16.0],
            )
            .unwrap();
        assert_eq!(outcome.committed_revision, figure.revision + 1);
        assert_eq!(outcome.event_cursor, 1);
        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        assert_eq!(snapshot.revision, outcome.committed_revision);
        let axes = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::Axes2d { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(axes.x_limits, [-2.0, 8.0]);
        assert_eq!(axes.y_limits, [0.25, 16.0]);
        assert_eq!(axes.x_limits_mode, wire::AxesLimitMode::Manual);
        assert_eq!(axes.y_limits_mode, wire::AxesLimitMode::Manual);
        let events = hub.events_after(0).unwrap();
        assert!(matches!(
            events.as_slice(),
            [SequencedGraphicsEvent {
                sequence: 1,
                event: wire::Event::FigureDelta(wire::FigureDelta {
                    base_revision,
                    revision,
                    operations,
                    ..
                }),
            }] if *base_revision == figure.revision
                && *revision == outcome.committed_revision
                && matches!(operations.as_slice(), [wire::DeltaOperation::UpsertObject {
                    object: wire::GraphicsObject::Axes2d { .. }
                }])
        ));

        let before_conflict = hub.get_snapshot(&figure.figure_id).unwrap();
        let conflict = hub
            .set_axes_limits(
                &figure.figure_id,
                None,
                figure.revision,
                [0.0, 1.0],
                [0.0, 1.0],
            )
            .unwrap_err();
        assert_eq!(conflict.category(), wire::ErrorCategory::RevisionConflict);
        assert_eq!(
            hub.get_snapshot(&figure.figure_id).unwrap(),
            before_conflict
        );
        assert_eq!(hub.events_after(0).unwrap().len(), 1);

        let invalid = hub
            .set_axes_limits(
                &figure.figure_id,
                None,
                outcome.committed_revision,
                [1.0, 1.0],
                [0.0, 1.0],
            )
            .unwrap_err();
        assert_eq!(invalid.category(), wire::ErrorCategory::InvalidRequest);
        assert_eq!(
            hub.get_snapshot(&figure.figure_id).unwrap(),
            before_conflict
        );
        assert_eq!(hub.events_after(0).unwrap().len(), 1);

        let unknown = hub
            .set_axes_limits("missing-figure", None, 1, [0.0, 1.0], [0.0, 1.0])
            .unwrap_err();
        assert_eq!(unknown.category(), wire::ErrorCategory::UnknownFigure);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn semantic_limits_can_target_one_axes_in_a_multi_axes_figure() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        let (second_id, deltas) = {
            let mut session = session.lock().unwrap();
            let model::GraphicsResponse::Handle(figure) = session
                .execute(model::GraphicsRequest::Figure { number: None })
                .unwrap()
                .response
            else {
                panic!("Figure must return a handle");
            };
            for position in [[0.13, 0.11, 0.334, 0.815], [0.57, 0.11, 0.334, 0.815]] {
                session
                    .execute(model::GraphicsRequest::CreateAxes {
                        figure: Some(figure),
                        properties: vec![model::GraphicsPropertyUpdate::Position(position)],
                    })
                    .unwrap();
            }
            let second = session.children(figure).unwrap()[1];
            (
                session.object_id(second).unwrap().to_owned(),
                session.take_pending_deltas(),
            )
        };
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        let figure = hub.initialize().unwrap().figures.remove(0);
        let outcome = hub
            .set_axes_limits(
                &figure.figure_id,
                Some(&second_id),
                figure.revision,
                [-4.0, 4.0],
                [10.0, 20.0],
            )
            .unwrap();

        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        assert_eq!(snapshot.revision, outcome.committed_revision);
        let axes = snapshot
            .objects
            .iter()
            .filter_map(|object| match object {
                wire::GraphicsObject::Axes2d { fields, properties } => {
                    Some((&fields.id, properties))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(axes.len(), 2);
        assert!(axes.iter().any(|(id, properties)| {
            id.as_str() == second_id
                && properties.x_limits == [-4.0, 4.0]
                && properties.y_limits == [10.0, 20.0]
        }));
        assert!(axes.iter().any(|(id, properties)| {
            id.as_str() != second_id
                && properties.x_limits != [-4.0, 4.0]
                && properties.y_limits != [10.0, 20.0]
        }));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn camera_gesture_uses_one_model_request_revision_and_journal_delta() {
        let session = Arc::new(Mutex::new(model::GraphicsSession::new()));
        session
            .lock()
            .unwrap()
            .execute(model::GraphicsRequest::Surface {
                axes: None,
                surface: model::SurfaceInput {
                    x: model::NumericData::from_f64(vec![1.0, 2.0]),
                    y: model::NumericData::from_f64(vec![1.0, 2.0]),
                    z: model::NumericData::from_f64(vec![1.0, 2.0, 3.0, 4.0]),
                    c: model::NumericData::from_f64(vec![1.0, 2.0, 3.0, 4.0]),
                    rows: 2,
                    columns: 2,
                },
                style: model::SurfaceStyle::Surf,
                properties: Vec::new(),
            })
            .unwrap();
        let deltas = session.lock().unwrap().take_pending_deltas();
        let hub = RuntimeGraphicsHub::new(Arc::clone(&session));
        hub.publish(deltas).unwrap();
        let figure = hub.initialize().unwrap().figures.remove(0);

        let outcome = hub
            .set_axes_camera(&figure.figure_id, None, figure.revision, [55.0, 24.0], 0.8)
            .unwrap();
        assert_eq!(outcome.committed_revision, figure.revision + 1);
        assert_eq!(outcome.event_cursor, 1);
        let snapshot = hub.get_snapshot(&figure.figure_id).unwrap();
        let axes = snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                wire::GraphicsObject::Axes2d { properties, .. } => Some(properties),
                _ => None,
            })
            .unwrap();
        assert_eq!(axes.view, [55.0, 24.0]);
        assert_eq!(axes.camera_scale, 0.8);
        assert!(matches!(
            hub.events_after(0).unwrap().as_slice(),
            [SequencedGraphicsEvent {
                sequence: 1,
                event: wire::Event::FigureDelta(wire::FigureDelta {
                    base_revision,
                    revision,
                    operations,
                    ..
                }),
            }] if *base_revision == figure.revision
                && *revision == outcome.committed_revision
                && matches!(operations.as_slice(), [wire::DeltaOperation::UpsertObject {
                    object: wire::GraphicsObject::Axes2d { .. }
                }])
        ));

        let stale = hub
            .set_axes_camera(&figure.figure_id, None, figure.revision, [0.0, 30.0], 1.0)
            .unwrap_err();
        assert_eq!(stale.category(), wire::ErrorCategory::RevisionConflict);
        let invalid = hub
            .set_axes_camera(
                &figure.figure_id,
                None,
                outcome.committed_revision,
                [0.0, 95.0],
                1.0,
            )
            .unwrap_err();
        assert_eq!(invalid.category(), wire::ErrorCategory::InvalidRequest);
    }
}
