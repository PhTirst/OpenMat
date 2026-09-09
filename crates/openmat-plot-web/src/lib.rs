//! Browser lifecycle and `wasm-bindgen` bridge for the `OpenMat` Plot Engine.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

use std::error::Error;
use std::fmt;

#[cfg(any(target_arch = "wasm32", test))]
use openmat_plot_wgpu::CompileError;
#[cfg(target_arch = "wasm32")]
use openmat_plot_wgpu::UploadError;

#[cfg(any(target_arch = "wasm32", test))]
mod scene;

#[cfg(any(target_arch = "wasm32", test))]
mod picking;

#[cfg(target_arch = "wasm32")]
use scene::{
    ProjectedOverlay, SceneCache, SourcePoint, SourcePoint3D, TextLayoutState,
    headlight_position_3d, prepare_scene_cached, projected_overlay_for_camera,
    resolved_view_projection_3d, source_points, source_points_3d,
};

/// Stable lifecycle states exposed to the browser adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlotWebState {
    /// No canvas or renderer has been initialized.
    Created,
    /// Canvas validation and renderer initialization are in progress.
    Initializing,
    /// Canvas and renderer are ready to accept frames.
    Ready,
    /// The browser does not expose WebGPU.
    UnsupportedWebGpu,
    /// The renderer crate has no usable browser entry point in this build.
    RendererUnavailable,
    /// A renderer or scene operation failed.
    Failed,
    /// The Figure has released its browser and renderer resources.
    Disposed,
}

impl PlotWebState {
    /// Returns the stable camel-case browser spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::UnsupportedWebGpu => "unsupportedWebGpu",
            Self::RendererUnavailable => "rendererUnavailable",
            Self::Failed => "failed",
            Self::Disposed => "disposed",
        }
    }
}

/// Stable categories for browser lifecycle failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlotWebErrorKind {
    /// CSS dimensions or device-pixel ratio were non-finite or non-positive.
    InvalidCanvasSize,
    /// CSS size multiplied by DPR did not fit a canvas device dimension.
    CanvasSizeOverflow,
    /// The browser does not expose a WebGPU implementation.
    UnsupportedWebGpu,
    /// `openmat-plot-wgpu` has not supplied its browser renderer API.
    RendererUnavailable,
    /// The adapter does not satisfy the frozen minimum renderer profile.
    InsufficientLimits,
    /// WebGPU device or pipeline creation failed.
    DeviceCreation,
    /// A canvas presentation surface could not be created or configured.
    Surface,
    /// The browser WebGPU device was lost.
    DeviceLost,
    /// Snapshot, delta, or immutable buffer input was invalid.
    InvalidScene,
    /// A configured CPU or GPU resource limit rejected otherwise valid content.
    ResourceLimit,
    /// An interaction coordinate, factor, or gesture was invalid.
    InvalidInteraction,
    /// An operation was attempted after disposal.
    Disposed,
}

impl PlotWebErrorKind {
    /// Returns the stable camel-case browser spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidCanvasSize => "invalidCanvasSize",
            Self::CanvasSizeOverflow => "canvasSizeOverflow",
            Self::UnsupportedWebGpu => "unsupportedWebGpu",
            Self::RendererUnavailable => "rendererUnavailable",
            Self::InsufficientLimits => "insufficientLimits",
            Self::DeviceCreation => "deviceCreation",
            Self::Surface => "surface",
            Self::DeviceLost => "deviceLost",
            Self::InvalidScene => "invalidScene",
            Self::ResourceLimit => "resourceLimit",
            Self::InvalidInteraction => "invalidInteraction",
            Self::Disposed => "disposed",
        }
    }
}

/// Typed browser adapter error whose message never contains attachment tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlotWebError {
    kind: PlotWebErrorKind,
    message: String,
}

impl PlotWebError {
    /// Creates a typed lifecycle error.
    #[must_use]
    pub fn new(kind: PlotWebErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> PlotWebErrorKind {
        self.kind
    }

    /// Returns the non-secret diagnostic text.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PlotWebError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PlotWebError {}

/// Validated CSS and device-pixel dimensions for one canvas target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSize {
    /// Logical width supplied by `ResizeObserver`, in CSS pixels.
    pub css_width: f64,
    /// Logical height supplied by `ResizeObserver`, in CSS pixels.
    pub css_height: f64,
    /// Browser device-pixel ratio used for this render target.
    pub device_pixel_ratio: f64,
    /// Backing canvas width in device pixels.
    pub device_width: u32,
    /// Backing canvas height in device pixels.
    pub device_height: u32,
}

impl CanvasSize {
    /// Validates a `ResizeObserver` CSS size and DPR without lossy wrapping.
    pub fn new(
        css_width: f64,
        css_height: f64,
        device_pixel_ratio: f64,
    ) -> Result<Self, PlotWebError> {
        if !css_width.is_finite()
            || !css_height.is_finite()
            || !device_pixel_ratio.is_finite()
            || css_width <= 0.0
            || css_height <= 0.0
            || device_pixel_ratio <= 0.0
        {
            return Err(PlotWebError::new(
                PlotWebErrorKind::InvalidCanvasSize,
                "Canvas CSS size and device-pixel ratio must be finite and positive.",
            ));
        }
        let device_width = checked_device_dimension(css_width, device_pixel_ratio)?;
        let device_height = checked_device_dimension(css_height, device_pixel_ratio)?;
        Ok(Self {
            css_width,
            css_height,
            device_pixel_ratio,
            device_width,
            device_height,
        })
    }
}

fn checked_device_dimension(css: f64, dpr: f64) -> Result<u32, PlotWebError> {
    let device = (css * dpr).round();
    if !device.is_finite() || device < 1.0 || device > f64::from(u32::MAX) {
        return Err(PlotWebError::new(
            PlotWebErrorKind::CanvasSizeOverflow,
            "Canvas device-pixel size is outside the supported range.",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(device as u32)
}

/// Platform-neutral lifecycle core used by native tests and the WASM wrapper.
#[derive(Debug)]
pub struct PlotWebLifecycle {
    state: PlotWebState,
    size: Option<CanvasSize>,
    last_error: Option<PlotWebError>,
}

impl Default for PlotWebLifecycle {
    fn default() -> Self {
        Self {
            state: PlotWebState::Created,
            size: None,
            last_error: None,
        }
    }
}

impl PlotWebLifecycle {
    /// Returns the current typed lifecycle state.
    #[must_use]
    pub const fn state(&self) -> PlotWebState {
        self.state
    }

    /// Returns the last validated canvas size, if any.
    #[must_use]
    pub const fn size(&self) -> Option<CanvasSize> {
        self.size
    }

    /// Returns the last typed error, if any.
    #[must_use]
    pub const fn last_error(&self) -> Option<&PlotWebError> {
        self.last_error.as_ref()
    }

    /// Begins asynchronous browser renderer initialization.
    pub fn begin_initialization(&mut self) -> Result<(), PlotWebError> {
        self.require_live()?;
        self.state = PlotWebState::Initializing;
        self.last_error = None;
        Ok(())
    }

    /// Records a successful renderer initialization.
    pub fn mark_ready(&mut self) -> Result<(), PlotWebError> {
        self.require_live()?;
        self.state = PlotWebState::Ready;
        self.last_error = None;
        Ok(())
    }

    /// Records a typed initialization or rendering failure.
    pub fn fail(&mut self, error: PlotWebError) {
        self.state = match error.kind() {
            PlotWebErrorKind::UnsupportedWebGpu => PlotWebState::UnsupportedWebGpu,
            PlotWebErrorKind::RendererUnavailable => PlotWebState::RendererUnavailable,
            PlotWebErrorKind::InvalidCanvasSize
            | PlotWebErrorKind::CanvasSizeOverflow
            | PlotWebErrorKind::InsufficientLimits
            | PlotWebErrorKind::DeviceCreation
            | PlotWebErrorKind::Surface
            | PlotWebErrorKind::DeviceLost
            | PlotWebErrorKind::InvalidScene
            | PlotWebErrorKind::ResourceLimit
            | PlotWebErrorKind::InvalidInteraction
            | PlotWebErrorKind::Disposed => PlotWebState::Failed,
        };
        self.last_error = Some(error);
    }

    /// Updates the validated canvas target size.
    pub fn resize(
        &mut self,
        css_width: f64,
        css_height: f64,
        device_pixel_ratio: f64,
    ) -> Result<CanvasSize, PlotWebError> {
        self.require_live()?;
        let size = CanvasSize::new(css_width, css_height, device_pixel_ratio)?;
        self.size = Some(size);
        Ok(size)
    }

    /// Marks this lifecycle permanently disposed. The operation is idempotent.
    pub fn dispose(&mut self) {
        self.state = PlotWebState::Disposed;
        self.size = None;
        self.last_error = None;
    }

    fn require_live(&self) -> Result<(), PlotWebError> {
        if self.state == PlotWebState::Disposed {
            Err(PlotWebError::new(
                PlotWebErrorKind::Disposed,
                "The Plot Figure has already been disposed.",
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn plot_compile_error(error: &CompileError) -> PlotWebError {
    let kind = match error {
        CompileError::BufferLimitExceeded { .. }
        | CompileError::ArithmeticOverflow(_)
        | CompileError::TooManyVertices
        | CompileError::TooManyMarkerInstances
        | CompileError::TooManyLineSegments => PlotWebErrorKind::ResourceLimit,
        CompileError::InvalidMir(_)
        | CompileError::UnsupportedMirCommand
        | CompileError::UnsupportedMarkerShape(_) => PlotWebErrorKind::InvalidScene,
    };
    PlotWebError::new(kind, format!("Plot MIR compilation failed: {error}."))
}

#[cfg(target_arch = "wasm32")]
fn plot_upload_error(error: &UploadError) -> PlotWebError {
    let kind = match error {
        UploadError::DeviceLost(_) => PlotWebErrorKind::DeviceLost,
        UploadError::BufferExceedsDeviceLimit { .. } => PlotWebErrorKind::ResourceLimit,
        UploadError::MissingBuffer(_) | UploadError::InvalidViewport => {
            PlotWebErrorKind::InvalidScene
        }
    };
    PlotWebError::new(kind, format!("Plot GPU upload failed: {error}."))
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use js_sys::{Object, Reflect, Uint8Array};
    use openmat_plot_geometry::{
        OrbitCamera3D, PlotBoxFit3D, Projection3D, select_projected_rulers_3d,
    };
    use openmat_plot_layout::{TextMeasurement, TextMetrics};
    use openmat_plot_mir::{CssPoint, PlotFrame, Rgba};
    use openmat_plot_protocol::{
        FigureDelta, FigureSnapshot, GraphicsLimits, GraphicsObject, SceneState,
    };
    use openmat_plot_wgpu::{
        DataPoint, DataRect, DrawListCompiler, FrameCompileOptions, GpuFrame, InteractionError,
        PickHit, PickKind, PickTolerance, RenderSurface, Renderer, RendererInitError,
        RendererRequest, RendererState, SurfaceExtent,
    };
    use serde::{Deserialize, Serialize};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use web_sys::HtmlCanvasElement;

    use crate::picking::assign_picking_ids;

    use super::{
        CanvasSize, PlotWebError, PlotWebErrorKind, PlotWebLifecycle, PlotWebState,
        ProjectedOverlay, SceneCache, SourcePoint, SourcePoint3D, TextLayoutState,
        headlight_position_3d, plot_compile_error, plot_upload_error, prepare_scene_cached,
        projected_overlay_for_camera, resolved_view_projection_3d, source_points, source_points_3d,
    };

    struct SharedRendererContext {
        request: RendererRequest,
        renderer: Renderer,
    }

    thread_local! {
        /// One WebGPU device and immutable pipeline set per browser worker/page.
        /// Figure windows retain only a canvas surface and an `Rc` to this context.
        static SHARED_RENDERER: RefCell<Option<Rc<SharedRendererContext>>> = const {
            RefCell::new(None)
        };
    }

    fn cached_renderer() -> Option<Rc<SharedRendererContext>> {
        SHARED_RENDERER.with(|slot| {
            let renderer = slot.borrow().clone();
            if renderer
                .as_ref()
                .is_some_and(|renderer| renderer.renderer.state() == RendererState::Ready)
            {
                renderer
            } else {
                *slot.borrow_mut() = None;
                None
            }
        })
    }

    fn create_surface(
        request: &RendererRequest,
        canvas: &HtmlCanvasElement,
    ) -> Result<RenderSurface<'static>, PlotWebError> {
        request
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| {
                PlotWebError::new(
                    PlotWebErrorKind::Surface,
                    format!("WebGPU canvas surface creation failed: {error}."),
                )
            })
    }

    /// The Figure-specific renderer dependency is isolated behind one surface
    /// and a reference to the page-wide WebGPU context.
    struct RendererAdapter {
        shared: Rc<SharedRendererContext>,
        surface: RenderSurface<'static>,
        frame: Option<GpuFrame>,
        camera_3d: Option<RendererCamera3D>,
        polar: bool,
        additional_frames: Vec<GpuFrame>,
        additional_cameras_3d: Vec<Option<RendererCamera3D>>,
        additional_polar: Vec<bool>,
        axes_ids: Vec<String>,
        active_axes: usize,
    }

    struct RendererFrameInput<'a> {
        axes_id: &'a str,
        frame: &'a PlotFrame,
        home: DataRect,
        camera_3d: Option<(OrbitCamera3D, f64, [f64; 3], PlotBoxFit3D)>,
        polar: bool,
    }

    #[derive(Clone, Copy, Debug)]
    struct RendererCamera3D {
        home: OrbitCamera3D,
        current: OrbitCamera3D,
        aspect_ratio: f64,
        axes_scale: [f64; 3],
        plot_box_fit: PlotBoxFit3D,
    }

    impl RendererAdapter {
        async fn initialize(canvas: &HtmlCanvasElement) -> Result<Self, PlotWebError> {
            if let Some(shared) = cached_renderer() {
                let surface = create_surface(&shared.request, canvas)?;
                return Ok(Self {
                    shared,
                    surface,
                    frame: None,
                    camera_3d: None,
                    polar: false,
                    additional_frames: Vec::new(),
                    additional_cameras_3d: Vec::new(),
                    additional_polar: Vec::new(),
                    axes_ids: Vec::new(),
                    active_axes: 0,
                });
            }

            let request = RendererRequest::new();
            let surface = create_surface(&request, canvas)?;
            let renderer = request
                .initialize(Some(&surface))
                .await
                .map_err(|error| renderer_init_error(&error))?;
            // The TypeScript loader serializes normal Figure creation. Keep a
            // second winner check so direct/concurrent wasm callers still use
            // whichever healthy context completed first.
            if let Some(shared) = cached_renderer() {
                let surface = create_surface(&shared.request, canvas)?;
                return Ok(Self {
                    shared,
                    surface,
                    frame: None,
                    camera_3d: None,
                    polar: false,
                    additional_frames: Vec::new(),
                    additional_cameras_3d: Vec::new(),
                    additional_polar: Vec::new(),
                    axes_ids: Vec::new(),
                    active_axes: 0,
                });
            }
            let shared = Rc::new(SharedRendererContext { request, renderer });
            SHARED_RENDERER.with(|slot| {
                *slot.borrow_mut() = Some(Rc::clone(&shared));
            });
            Ok(Self {
                shared,
                surface,
                frame: None,
                camera_3d: None,
                polar: false,
                additional_frames: Vec::new(),
                additional_cameras_3d: Vec::new(),
                additional_polar: Vec::new(),
                axes_ids: Vec::new(),
                active_axes: 0,
            })
        }

        fn resize(&mut self, size: CanvasSize) -> Result<(), PlotWebError> {
            // A resize changes the MIR viewport/tessellation. Discard only this
            // Figure's uploaded frame; the page-wide device/pipelines survive.
            self.frame = None;
            self.camera_3d = None;
            self.polar = false;
            self.additional_frames.clear();
            self.additional_cameras_3d.clear();
            self.additional_polar.clear();
            self.axes_ids.clear();
            self.active_axes = 0;
            self.surface
                .configure(
                    &self.shared.renderer,
                    SurfaceExtent {
                        width: size.device_width,
                        height: size.device_height,
                    },
                )
                .map(|_| ())
                .map_err(|error| {
                    PlotWebError::new(
                        PlotWebErrorKind::Surface,
                        format!("WebGPU canvas surface configuration failed: {error}."),
                    )
                })
        }

        fn render_frames(
            &mut self,
            clear: Rgba,
            inputs: &[RendererFrameInput<'_>],
        ) -> Result<(), PlotWebError> {
            if inputs.is_empty() {
                return Err(frame_required());
            }
            let mut frames = Vec::with_capacity(inputs.len());
            let mut cameras = Vec::with_capacity(inputs.len());
            let mut polar = Vec::with_capacity(inputs.len());
            let mut axes_ids = Vec::with_capacity(inputs.len());
            for input in inputs {
                axes_ids.push(input.axes_id.to_owned());
                polar.push(input.polar);
                let compiled = DrawListCompiler::new(FrameCompileOptions {
                    clear_color: clear,
                    ..FrameCompileOptions::default()
                })
                .compile(input.frame)
                .map_err(|error| plot_compile_error(&error))?;
                let mut gpu_frame = self
                    .shared
                    .renderer
                    .upload(&compiled)
                    .map_err(|error| plot_upload_error(&error))?;
                gpu_frame
                    .set_home_limits(input.home)
                    .map_err(interaction_error)?;
                frames.push(gpu_frame);
                cameras.push(input.camera_3d.map(
                    |(camera, aspect_ratio, axes_scale, plot_box_fit)| RendererCamera3D {
                        home: camera,
                        current: camera,
                        aspect_ratio,
                        axes_scale,
                        plot_box_fit,
                    },
                ));
            }
            let frame_refs = frames.iter().collect::<Vec<_>>();
            self.surface
                .render_frames(&self.shared.renderer, &frame_refs)
                .map(|_| ())
                .map_err(|error| {
                    PlotWebError::new(
                        PlotWebErrorKind::Surface,
                        format!("WebGPU canvas presentation failed: {error}."),
                    )
                })?;
            self.frame = Some(frames.remove(0));
            self.camera_3d = cameras.remove(0);
            self.polar = polar.remove(0);
            self.additional_frames = frames;
            self.additional_cameras_3d = cameras;
            self.additional_polar = polar;
            self.axes_ids = axes_ids;
            self.active_axes = 0;
            Ok(())
        }

        fn active_frame(&self) -> Option<&GpuFrame> {
            if self.active_axes == 0 {
                self.frame.as_ref()
            } else {
                self.additional_frames.get(self.active_axes - 1)
            }
        }

        fn active_frame_mut(&mut self) -> Option<&mut GpuFrame> {
            if self.active_axes == 0 {
                self.frame.as_mut()
            } else {
                self.additional_frames.get_mut(self.active_axes - 1)
            }
        }

        fn active_camera_3d(&self) -> Option<RendererCamera3D> {
            if self.active_axes == 0 {
                self.camera_3d
            } else {
                self.additional_cameras_3d
                    .get(self.active_axes - 1)
                    .copied()
                    .flatten()
            }
        }

        fn active_camera_3d_mut(&mut self) -> Option<&mut RendererCamera3D> {
            if self.active_axes == 0 {
                self.camera_3d.as_mut()
            } else {
                self.additional_cameras_3d
                    .get_mut(self.active_axes - 1)
                    .and_then(Option::as_mut)
            }
        }

        fn active_axes_id(&self) -> Option<&str> {
            self.axes_ids.get(self.active_axes).map(String::as_str)
        }

        fn active_is_polar(&self) -> bool {
            if self.active_axes == 0 {
                self.polar
            } else {
                self.additional_polar
                    .get(self.active_axes - 1)
                    .copied()
                    .unwrap_or(false)
            }
        }

        fn activate_at(&mut self, x: f32, y: f32) -> Result<(), PlotWebError> {
            let point = CssPoint::new(x, y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("pointer")))?;
            let Some(index) = self.axes_index_at(point) else {
                return Err(PlotWebError::new(
                    PlotWebErrorKind::InvalidInteraction,
                    "The pointer is outside every rendered Axes.",
                ));
            };
            self.active_axes = index;
            Ok(())
        }

        fn axes_index_at(&self, point: CssPoint) -> Option<usize> {
            let count = usize::from(self.frame.is_some()) + self.additional_frames.len();
            for index in (0..count).rev() {
                let frame = if index == 0 {
                    self.frame.as_ref()
                } else {
                    self.additional_frames.get(index - 1)
                };
                let Some(frame) = frame else {
                    continue;
                };
                let viewport = frame.view_transform().viewport();
                if viewport
                    .clamp_css_point(point)
                    .is_ok_and(|clamped| clamped == point)
                {
                    return Some(index);
                }
            }
            None
        }

        fn frame_at(&self, point: CssPoint) -> Option<&GpuFrame> {
            let index = self.axes_index_at(point)?;
            if index == 0 {
                self.frame.as_ref()
            } else {
                self.additional_frames.get(index - 1)
            }
        }

        fn begin_limits(&self) -> Result<DataRect, PlotWebError> {
            self.active_frame()
                .map(GpuFrame::view_transform)
                .map(openmat_plot_wgpu::ViewTransform2D::current_limits)
                .ok_or_else(frame_required)
        }

        fn current_limits(&self) -> Result<DataRect, PlotWebError> {
            self.begin_limits()
        }

        fn interaction_dimension(&self) -> &'static str {
            if self.active_camera_3d().is_some() {
                "3d"
            } else if self.active_is_polar() {
                "polar"
            } else {
                "2d"
            }
        }

        fn current_camera_3d(&self) -> Option<OrbitCamera3D> {
            self.active_camera_3d().map(|camera| camera.current)
        }

        fn begin_gesture(&self) -> Result<GestureStart, PlotWebError> {
            if let Some(camera) = self.active_camera_3d() {
                Ok(GestureStart::ThreeD(camera.current))
            } else if self.active_is_polar() {
                self.begin_limits().map(GestureStart::Polar)
            } else {
                self.begin_limits().map(GestureStart::TwoD)
            }
        }

        fn home(&mut self) -> Result<(), PlotWebError> {
            if let Some(camera) = self.active_camera_3d_mut() {
                camera.current = camera.home;
                self.update_camera_3d()?;
            } else if self.active_is_polar() {
                return Ok(());
            } else {
                self.active_frame_mut().ok_or_else(frame_required)?.home();
            }
            self.redraw()
        }

        fn pan(&mut self, delta_x: f64, delta_y: f64) -> Result<(), PlotWebError> {
            if self.active_is_polar() {
                return Ok(());
            }
            self.active_frame_mut()
                .ok_or_else(frame_required)?
                .pan_css(delta_x, delta_y)
                .map_err(interaction_error)?;
            self.redraw()
        }

        fn orbit(&mut self, delta_x: f64, delta_y: f64) -> Result<(), PlotWebError> {
            let camera = self.active_camera_3d_mut().ok_or_else(|| {
                PlotWebError::new(
                    PlotWebErrorKind::InvalidInteraction,
                    "Orbit is available only for a rendered 3D Figure.",
                )
            })?;
            camera
                .current
                .orbit(-delta_x * 0.01, delta_y * 0.01)
                .map_err(|_| camera_interaction_error())?;
            self.update_camera_3d()?;
            self.redraw()
        }

        fn zoom(&mut self, x: f32, y: f32, factor: f64) -> Result<(), PlotWebError> {
            if let Some(camera) = self.active_camera_3d_mut() {
                camera
                    .current
                    .zoom(factor)
                    .map_err(|_| camera_interaction_error())?;
                self.update_camera_3d()?;
                return self.redraw();
            }
            if self.active_is_polar() {
                return Ok(());
            }
            let point = CssPoint::new(x, y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("wheel anchor")))?;
            self.active_frame_mut()
                .ok_or_else(frame_required)?
                .zoom_css(point, factor)
                .map_err(interaction_error)?;
            self.redraw()
        }

        fn update_camera_3d(&mut self) -> Result<(), PlotWebError> {
            let camera = self.active_camera_3d().ok_or_else(frame_required)?;
            let matrix = resolved_view_projection_3d(
                camera.current,
                camera.aspect_ratio,
                camera.axes_scale,
                camera.plot_box_fit,
            )
            .map_err(|_| camera_interaction_error())?;
            let ruler_selection =
                select_projected_rulers_3d(matrix).map_err(|_| camera_interaction_error())?;
            let headlight_position =
                headlight_position_3d(camera.current).map_err(|_| camera_interaction_error())?;
            let frame = self.active_frame_mut().ok_or_else(frame_required)?;
            frame.set_view_projection_3d(matrix);
            frame.set_ruler_selection_3d(ruler_selection);
            // Full scene lowering derives the headlight from this same camera.
            // Keep it in sync during drag/zoom/home, before the kernel commit.
            frame.set_headlight_position_3d(headlight_position);
            Ok(())
        }

        fn box_zoom(
            &mut self,
            first_x: f32,
            first_y: f32,
            second_x: f32,
            second_y: f32,
        ) -> Result<(), PlotWebError> {
            if self.active_is_polar() {
                return Ok(());
            }
            let first = CssPoint::new(first_x, first_y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("box start")))?;
            let second = CssPoint::new(second_x, second_y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("box end")))?;
            self.active_frame_mut()
                .ok_or_else(frame_required)?
                .box_zoom_css(first, second)
                .map_err(interaction_error)?;
            self.redraw()
        }

        fn pick(&self, x: f32, y: f32, tolerance: f64) -> Result<Option<PickHit>, PlotWebError> {
            let point = CssPoint::new(x, y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("pick point")))?;
            let tolerance = PickTolerance::new(tolerance).map_err(interaction_error)?;
            let frame = self.frame_at(point).ok_or_else(frame_required)?;
            Ok(frame.pick(point, tolerance))
        }

        fn nearest_source_point(
            &self,
            x: f32,
            y: f32,
            points: &[SourcePoint],
        ) -> Result<Option<SourcePoint>, PlotWebError> {
            let pointer = CssPoint::new(x, y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("pick point")))?;
            let view = self
                .active_frame()
                .ok_or_else(frame_required)?
                .view_transform();
            Ok(points
                .iter()
                .filter_map(|source| {
                    let data = DataPoint::new(source.x, source.y).ok()?;
                    let css = view.data_to_css(data).ok()?;
                    let delta_x = f64::from(css.x - pointer.x);
                    let delta_y = f64::from(css.y - pointer.y);
                    Some((*source, delta_x.mul_add(delta_x, delta_y * delta_y)))
                })
                .min_by(|(left, left_distance), (right, right_distance)| {
                    left_distance
                        .total_cmp(right_distance)
                        .then(left.source_index.cmp(&right.source_index))
                })
                .map(|(source, _)| source))
        }

        fn nearest_source_point_3d(
            &self,
            x: f32,
            y: f32,
            tolerance: f64,
            points: &[SourcePoint3D],
        ) -> Result<Option<(SourcePoint3D, f64)>, PlotWebError> {
            if !tolerance.is_finite() || tolerance < 0.0 {
                return Err(invalid_interaction_coordinates());
            }
            let pointer = CssPoint::new(x, y)
                .map_err(|_| interaction_error(InteractionError::NonFinite("pick point")))?;
            let camera = self.active_camera_3d().ok_or_else(frame_required)?;
            let matrix = resolved_view_projection_3d(
                camera.current,
                camera.aspect_ratio,
                camera.axes_scale,
                camera.plot_box_fit,
            )
            .map_err(|_| camera_interaction_error())?;
            let viewport = self
                .active_frame()
                .ok_or_else(frame_required)?
                .view_transform()
                .viewport();
            let clamped = viewport
                .clamp_css_point(pointer)
                .map_err(|_| invalid_interaction_coordinates())?;
            if clamped != pointer {
                return Ok(None);
            }
            let columns = matrix.columns();
            let mut best: Option<(SourcePoint3D, f64, f64)> = None;
            for source in points {
                let point = source.normalized;
                let input = [
                    f64::from(point.x),
                    f64::from(point.y),
                    f64::from(point.z),
                    1.0,
                ];
                let mut clip = [0.0_f64; 4];
                for (column, value) in input.into_iter().enumerate() {
                    for (row, output) in clip.iter_mut().enumerate() {
                        *output += f64::from(columns[column][row]) * value;
                    }
                }
                if !clip.iter().all(|value| value.is_finite()) || clip[3] <= f64::EPSILON {
                    continue;
                }
                let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
                if !(0.0..=1.0).contains(&ndc[2]) {
                    continue;
                }
                let css_x = f64::from(viewport.css.origin.x)
                    + (ndc[0] * 0.5 + 0.5) * f64::from(viewport.css.size.width.get());
                let css_y = f64::from(viewport.css.origin.y)
                    + (0.5 - ndc[1] * 0.5) * f64::from(viewport.css.size.height.get());
                let distance = (css_x - f64::from(pointer.x)).hypot(css_y - f64::from(pointer.y));
                if distance > tolerance {
                    continue;
                }
                let replace = best
                    .as_ref()
                    .is_none_or(|(current, current_distance, depth)| {
                        distance.total_cmp(current_distance).is_lt()
                            || (distance.total_cmp(current_distance).is_eq()
                                && (ndc[2].total_cmp(depth).is_lt()
                                    || (ndc[2].total_cmp(depth).is_eq()
                                        && (source.object_id.as_str(), source.source_index)
                                            < (current.object_id.as_str(), current.source_index))))
                    });
                if replace {
                    best = Some((source.clone(), distance, ndc[2]));
                }
            }
            Ok(best.map(|(source, distance, _)| (source, distance)))
        }

        fn redraw(&mut self) -> Result<(), PlotWebError> {
            if self.shared.renderer.state() != RendererState::Ready {
                return Err(PlotWebError::new(
                    PlotWebErrorKind::DeviceLost,
                    "The shared Plot WebGPU device was lost.",
                ));
            }
            let frame = self.frame.as_ref().ok_or_else(frame_required)?;
            let mut frames = Vec::with_capacity(1 + self.additional_frames.len());
            frames.push(frame);
            frames.extend(self.additional_frames.iter());
            self.surface
                .render_frames(&self.shared.renderer, &frames)
                .map(|_| ())
                .map_err(|error| {
                    PlotWebError::new(
                        PlotWebErrorKind::Surface,
                        format!("Interactive WebGPU presentation failed: {error}."),
                    )
                })
        }

        #[allow(clippy::unused_self)] // Consuming self drops this Figure's surface and frame.
        fn dispose(self) {}
    }

    /// Browser-owned Figure canvas and validated scene mirror.
    #[wasm_bindgen]
    pub struct PlotFigure {
        canvas: Option<HtmlCanvasElement>,
        lifecycle: PlotWebLifecycle,
        renderer: Option<RendererAdapter>,
        scene: Option<SceneState>,
        buffers: HashMap<String, Vec<u8>>,
        picking_objects: HashMap<u64, String>,
        gesture_start: Option<GestureStart>,
        scene_cache: SceneCache,
        text_layout: Option<TextLayoutState>,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum GestureStart {
        TwoD(DataRect),
        Polar(DataRect),
        ThreeD(OrbitCamera3D),
    }

    #[wasm_bindgen]
    impl PlotFigure {
        /// Returns the current stable lifecycle state.
        #[wasm_bindgen(getter)]
        pub fn state(&self) -> String {
            self.lifecycle.state().as_str().to_owned()
        }

        /// Returns a typed error object or `null`.
        #[wasm_bindgen(js_name = lastError)]
        pub fn last_error(&self) -> JsValue {
            self.lifecycle
                .last_error()
                .map_or(JsValue::NULL, error_to_js)
        }

        /// Applies CSS size and DPR to the backing canvas and renderer surface.
        pub fn resize(
            &mut self,
            css_width: f64,
            css_height: f64,
            device_pixel_ratio: f64,
        ) -> Result<(), JsValue> {
            let size = self
                .lifecycle
                .resize(css_width, css_height, device_pixel_ratio)
                .map_err(|error| error_to_js(&error))?;
            let canvas = self.canvas.as_ref().ok_or_else(|| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::Disposed,
                    "The Plot Figure canvas has been disposed.",
                ))
            })?;
            canvas.set_width(size.device_width);
            canvas.set_height(size.device_height);
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(size).map_err(|error| error_to_js(&error))?;
            }
            Ok(())
        }

        /// Replaces the complete validated graphics-v1 Figure snapshot.
        #[wasm_bindgen(js_name = applySnapshot)]
        pub fn apply_snapshot(&mut self, value: JsValue) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let snapshot: FigureSnapshot = serde_wasm_bindgen::from_value(value).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Figure snapshot does not match graphics-v1.",
                ))
            })?;
            let scene = SceneState::new(snapshot, GraphicsLimits::default()).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Figure snapshot failed graphics-v1 validation.",
                ))
            })?;
            self.scene = Some(scene);
            if let Some(renderer) = &mut self.renderer {
                renderer.frame = None;
                renderer.camera_3d = None;
                renderer.additional_frames.clear();
                renderer.additional_cameras_3d.clear();
                renderer.axes_ids.clear();
                renderer.active_axes = 0;
            }
            self.picking_objects.clear();
            self.gesture_start = None;
            self.buffers.retain(|buffer_id, _| {
                self.scene.as_ref().is_some_and(|scene| {
                    scene
                        .snapshot()
                        .referenced_buffers
                        .iter()
                        .any(|data| data.buffer_id == *buffer_id)
                })
            });
            Ok(())
        }

        /// Applies one exact-next graphics-v1 Figure delta transactionally.
        #[wasm_bindgen(js_name = applyDelta)]
        pub fn apply_delta(&mut self, value: JsValue) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let delta: FigureDelta = serde_wasm_bindgen::from_value(value).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Figure delta does not match graphics-v1.",
                ))
            })?;
            let scene = self.scene.as_mut().ok_or_else(|| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "A Figure snapshot is required before a delta.",
                ))
            })?;
            scene.apply_delta(&delta).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Figure delta failed revision or scene validation.",
                ))
            })?;
            if let Some(renderer) = &mut self.renderer {
                renderer.frame = None;
                renderer.camera_3d = None;
                renderer.additional_frames.clear();
                renderer.additional_cameras_3d.clear();
                renderer.axes_ids.clear();
                renderer.active_axes = 0;
            }
            self.picking_objects.clear();
            self.gesture_start = None;
            Ok(())
        }

        /// Copies one complete immutable graphics-v1 buffer into WASM memory.
        #[wasm_bindgen(js_name = ingestBuffer)]
        #[allow(clippy::needless_pass_by_value)] // wasm-bindgen owns the JS typed-array argument.
        pub fn ingest_buffer(
            &mut self,
            buffer_id: String,
            bytes: Uint8Array,
        ) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let descriptor = self
                .scene
                .as_ref()
                .and_then(|scene| {
                    scene
                        .snapshot()
                        .referenced_buffers
                        .iter()
                        .find(|data| data.buffer_id == buffer_id)
                })
                .ok_or_else(|| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::InvalidScene,
                        "Immutable buffer is not referenced by the current Figure.",
                    ))
                })?;
            let expected = usize::try_from(descriptor.byte_length).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Immutable buffer length cannot be represented by this browser.",
                ))
            })?;
            if bytes.length() as usize != expected {
                return Err(error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Immutable buffer length does not match its DataRef.",
                )));
            }
            self.buffers.insert(buffer_id, bytes.to_vec());
            Ok(())
        }

        /// Lowers the current graphics-v1 HIR and immutable buffers to Plot MIR,
        /// uploads it, and presents the Figure through the shared wgpu renderer.
        pub fn render(&mut self) -> Result<JsValue, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let size = self.lifecycle.size().ok_or_else(|| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidCanvasSize,
                    "The Plot Figure must be resized before rendering.",
                ))
            })?;
            let scene = self.scene.as_ref().ok_or_else(|| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "A Figure snapshot is required before rendering.",
                ))
            })?;
            let mut prepared =
                prepare_scene_cached(scene.snapshot(), &self.buffers, size, &mut self.scene_cache)
                    .map_err(|error| error_to_js(&error))?;
            let mut next_picking_id = 1_u64;
            let mut picking_objects = HashMap::new();
            if let Some(axes_id) = prepared.axes_id.as_deref() {
                picking_objects.extend(
                    assign_picking_ids(
                        scene.snapshot(),
                        axes_id,
                        &mut prepared.frame,
                        &mut next_picking_id,
                    )
                    .map_err(|error| error_to_js(&error))?,
                );
            }
            for axes in &mut prepared.additional_axes {
                picking_objects.extend(
                    assign_picking_ids(
                        scene.snapshot(),
                        &axes.axes_id,
                        &mut axes.frame,
                        &mut next_picking_id,
                    )
                    .map_err(|error| error_to_js(&error))?,
                );
            }
            self.picking_objects = picking_objects;
            self.text_layout = prepared.text_layout_state();
            let first_home = semantic_home(scene.snapshot(), prepared.axes_id.as_deref())
                .map_err(|error| error_to_js(&error))?;
            let mut renderer_inputs = Vec::with_capacity(1 + prepared.additional_axes.len());
            renderer_inputs.push(RendererFrameInput {
                axes_id: prepared.axes_id.as_deref().unwrap_or(""),
                frame: &prepared.frame,
                home: first_home,
                camera_3d: prepared.camera_3d,
                polar: semantic_is_polar(scene.snapshot(), prepared.axes_id.as_deref()),
            });
            for axes in &prepared.additional_axes {
                renderer_inputs.push(RendererFrameInput {
                    axes_id: &axes.axes_id,
                    frame: &axes.frame,
                    home: semantic_home(scene.snapshot(), Some(&axes.axes_id))
                        .map_err(|error| error_to_js(&error))?,
                    camera_3d: axes.camera_3d,
                    polar: semantic_is_polar(scene.snapshot(), Some(&axes.axes_id)),
                });
            }
            self.renderer
                .as_mut()
                .ok_or_else(|| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::RendererUnavailable,
                        "The WebGPU renderer is unavailable.",
                    ))
                })?
                .render_frames(prepared.clear_color, &renderer_inputs)
                .map_err(|error| error_to_js(&error))?;
            self.gesture_start = None;
            serde_wasm_bindgen::to_value(&prepared.overlay_dto()).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "The Plot overlay plan could not be transferred to the browser.",
                ))
            })
        }

        /// Completes the retained overlay layout with browser-measured font metrics.
        #[wasm_bindgen(js_name = completeTextLayout)]
        pub fn complete_text_layout(
            &self,
            font_revision: u32,
            measurements: JsValue,
        ) -> Result<JsValue, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let measurements: Vec<BrowserTextMeasurementDto> =
                serde_wasm_bindgen::from_value(measurements).map_err(|_| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::InvalidScene,
                        "Browser text measurements have an invalid shape.",
                    ))
                })?;
            let measurements = measurements
                .into_iter()
                .map(BrowserTextMeasurementDto::into_layout)
                .collect::<Result<Vec<_>, _>>()?;
            let overlay = self
                .text_layout
                .as_ref()
                .ok_or_else(|| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::InvalidScene,
                        "The current Figure has no retained text layout.",
                    ))
                })?
                .complete(u64::from(font_revision), measurements)
                .map_err(|error| error_to_js(&error))?;
            serde_wasm_bindgen::to_value(&overlay).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "The measured Plot overlay could not be transferred to the browser.",
                ))
            })
        }

        /// Records the semantic view at the start of one local gesture.
        #[wasm_bindgen(js_name = beginInteraction)]
        pub fn begin_interaction(&mut self) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let start = self
                .renderer
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .begin_gesture()
                .map_err(|error| error_to_js(&error))?;
            self.gesture_start = Some(start);
            Ok(())
        }

        /// Selects the topmost Axes under a CSS-pixel pointer and records its semantic view.
        #[wasm_bindgen(js_name = beginInteractionAt)]
        pub fn begin_interaction_at(&mut self, x: f32, y: f32) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let renderer = self
                .renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            renderer
                .activate_at(x, y)
                .map_err(|error| error_to_js(&error))?;
            self.gesture_start = Some(
                renderer
                    .begin_gesture()
                    .map_err(|error| error_to_js(&error))?,
            );
            Ok(())
        }

        /// Returns whether the retained Figure uses 2D or 3D interaction.
        #[wasm_bindgen(js_name = interactionDimension)]
        pub fn interaction_dimension(&self) -> Result<String, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            Ok(self
                .renderer
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .interaction_dimension()
                .to_owned())
        }

        /// Returns the interaction dimension of the topmost Axes under a CSS-pixel pointer.
        #[wasm_bindgen(js_name = interactionDimensionAt)]
        pub fn interaction_dimension_at(&mut self, x: f32, y: f32) -> Result<String, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let renderer = self
                .renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            renderer
                .activate_at(x, y)
                .map_err(|error| error_to_js(&error))?;
            Ok(renderer.interaction_dimension().to_owned())
        }

        /// Reprojects the lightweight 3D text overlay after a local orbit or
        /// zoom without rebuilding or uploading GPU geometry.
        #[wasm_bindgen(js_name = currentOverlay)]
        pub fn current_overlay(&mut self) -> Result<JsValue, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let Some((camera, axes_id)) = self.renderer.as_ref().and_then(|renderer| {
                renderer
                    .current_camera_3d()
                    .zip(renderer.active_axes_id().map(str::to_owned))
            }) else {
                return Ok(JsValue::NULL);
            };
            let size = self
                .lifecycle
                .size()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            let snapshot = self
                .scene
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .snapshot()
                .clone();
            let prepared =
                prepare_scene_cached(&snapshot, &self.buffers, size, &mut self.scene_cache)
                    .map_err(|error| error_to_js(&error))?;
            let ProjectedOverlay {
                overlay,
                text_layout,
            } = projected_overlay_for_camera(&snapshot, size, &axes_id, camera, &prepared)
                .map_err(|error| error_to_js(&error))?;
            self.text_layout = Some(text_layout);
            serde_wasm_bindgen::to_value(&overlay).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "The projected 3D overlay could not be transferred to the browser.",
                ))
            })
        }

        /// Restores home limits and redraws the retained GPU frame.
        #[wasm_bindgen(js_name = homeView)]
        pub fn home_view(&mut self) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            self.renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .home()
                .map_err(|error| error_to_js(&error))
        }

        /// Pans the retained frame by a CSS-pixel pointer delta.
        #[wasm_bindgen(js_name = panBy)]
        pub fn pan_by(&mut self, delta_x: f64, delta_y: f64) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            self.renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .pan(delta_x, delta_y)
                .map_err(|error| error_to_js(&error))
        }

        /// Orbits the retained 3D camera by a CSS-pixel pointer delta.
        #[wasm_bindgen(js_name = orbitBy)]
        pub fn orbit_by(&mut self, delta_x: f64, delta_y: f64) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            self.renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .orbit(delta_x, delta_y)
                .map_err(|error| error_to_js(&error))
        }

        /// Applies pointer-anchored wheel zoom to the retained frame.
        #[wasm_bindgen(js_name = wheelZoom)]
        pub fn wheel_zoom(&mut self, x: f32, y: f32, delta_y: f64) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            if !delta_y.is_finite() {
                return Err(error_to_js(&interaction_error(
                    InteractionError::NonFinite("wheel delta"),
                )));
            }
            let factor = (delta_y * 0.001).clamp(-20.0, 20.0).exp();
            self.renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .zoom(x, y, factor)
                .map_err(|error| error_to_js(&error))
        }

        /// Applies one clamped CSS-pixel box zoom to the retained frame.
        #[wasm_bindgen(js_name = boxZoom)]
        pub fn box_zoom(
            &mut self,
            first_x: f32,
            first_y: f32,
            second_x: f32,
            second_y: f32,
        ) -> Result<(), JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            self.renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .box_zoom(first_x, first_y, second_x, second_y)
                .map_err(|error| error_to_js(&error))
        }

        /// Returns one structured semantic-limit result at gesture end. This is
        /// deliberately not a Kernel/protocol mutation.
        #[wasm_bindgen(js_name = endInteraction)]
        pub fn end_interaction(&mut self) -> Result<JsValue, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let start = self
                .gesture_start
                .take()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            let renderer = self
                .renderer
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            let result = match start {
                GestureStart::TwoD(start) => {
                    let current = renderer
                        .current_limits()
                        .map_err(|error| error_to_js(&error))?;
                    InteractionResultDto {
                        axes_id: renderer.active_axes_id().unwrap_or("").to_owned(),
                        dimension: "2d",
                        x_limits: current.x_limits(),
                        y_limits: current.y_limits(),
                        view: None,
                        camera_scale: None,
                        changed: start != current,
                    }
                }
                GestureStart::Polar(start) => {
                    let current = renderer
                        .current_limits()
                        .map_err(|error| error_to_js(&error))?;
                    InteractionResultDto {
                        axes_id: renderer.active_axes_id().unwrap_or("").to_owned(),
                        dimension: "polar",
                        x_limits: current.x_limits(),
                        y_limits: current.y_limits(),
                        view: None,
                        camera_scale: None,
                        changed: start != current,
                    }
                }
                GestureStart::ThreeD(start) => {
                    let current = renderer.active_camera_3d().ok_or_else(|| {
                        error_to_js(&PlotWebError::new(
                            PlotWebErrorKind::InvalidInteraction,
                            "The rendered Figure changed dimension during an interaction.",
                        ))
                    })?;
                    InteractionResultDto {
                        axes_id: renderer.active_axes_id().unwrap_or("").to_owned(),
                        dimension: "3d",
                        x_limits: renderer
                            .current_limits()
                            .map_err(|error| error_to_js(&error))?
                            .x_limits(),
                        y_limits: renderer
                            .current_limits()
                            .map_err(|error| error_to_js(&error))?
                            .y_limits(),
                        view: Some([
                            -current.current.azimuth_radians().to_degrees(),
                            current.current.elevation_radians().to_degrees(),
                        ]),
                        camera_scale: Some(camera_scale(current.current)),
                        changed: start != current.current,
                    }
                }
            };
            serde_wasm_bindgen::to_value(&result)
                .map_err(|_| error_to_js(&interaction_error(InteractionError::Overflow("result"))))
        }

        /// Picks the nearest retained line segment or Marker in CSS pixels.
        pub fn pick(&mut self, x: f32, y: f32, tolerance_css_px: f64) -> Result<JsValue, JsValue> {
            self.require_live().map_err(|error| error_to_js(&error))?;
            let scene = self
                .scene
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            let renderer = self
                .renderer
                .as_mut()
                .ok_or_else(|| error_to_js(&frame_required()))?;
            renderer
                .activate_at(x, y)
                .map_err(|error| error_to_js(&error))?;
            if renderer.active_camera_3d().is_some() {
                let source =
                    source_points_3d(scene.snapshot(), &self.buffers, renderer.active_axes_id())
                        .map_err(|error| error_to_js(&error))?;
                let Some((source, distance_css_px)) = renderer
                    .nearest_source_point_3d(x, y, tolerance_css_px, &source)
                    .map_err(|error| error_to_js(&error))?
                else {
                    return Ok(JsValue::NULL);
                };
                let source_index = u32::try_from(source.source_index).map_err(|_| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::InvalidScene,
                        "Picked source index exceeds the browser numeric contract.",
                    ))
                })?;
                let picking_id = scene
                    .snapshot()
                    .objects
                    .iter()
                    .position(|object| object.fields().id == source.object_id)
                    .and_then(|index| u32::try_from(index + 1).ok())
                    .ok_or_else(|| {
                        error_to_js(&PlotWebError::new(
                            PlotWebErrorKind::InvalidScene,
                            "Picked 3D object has no stable scene identifier.",
                        ))
                    })?;
                return serde_wasm_bindgen::to_value(&PickResultDto {
                    picking_id,
                    object_id: &source.object_id,
                    kind: source.kind,
                    primitive_index: source_index,
                    source_index,
                    distance_css_px,
                    x: source.x,
                    y: source.y,
                    z: Some(source.z),
                })
                .map_err(|_| {
                    error_to_js(&interaction_error(InteractionError::Overflow(
                        "3D pick result",
                    )))
                });
            }
            let hit = self
                .renderer
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .pick(x, y, tolerance_css_px)
                .map_err(|error| error_to_js(&error))?;
            let Some(hit) = hit else {
                return Ok(JsValue::NULL);
            };
            let picking_id = hit.picking_id.get();
            let object_id = self.picking_objects.get(&picking_id).ok_or_else(|| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Picking identifier has no graphics object mapping.",
                ))
            })?;
            let source = source_points(scene.snapshot(), &self.buffers, object_id)
                .map_err(|error| error_to_js(&error))?;
            let source = self
                .renderer
                .as_ref()
                .ok_or_else(|| error_to_js(&frame_required()))?
                .nearest_source_point(x, y, &source)
                .map_err(|error| error_to_js(&error))?
                .ok_or_else(|| {
                    error_to_js(&PlotWebError::new(
                        PlotWebErrorKind::InvalidScene,
                        "Picked graphics object has no finite source element.",
                    ))
                })?;
            let source_index = u32::try_from(source.source_index).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Picked source index exceeds the browser numeric contract.",
                ))
            })?;
            let picking_id = u32::try_from(picking_id).map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Picking identifier exceeds the browser numeric contract.",
                ))
            })?;
            serde_wasm_bindgen::to_value(&PickResultDto {
                picking_id,
                object_id,
                kind: match hit.kind {
                    PickKind::LineSegment => "lineSegment",
                    PickKind::Marker => "marker",
                },
                primitive_index: hit.primitive_index,
                source_index,
                distance_css_px: hit.distance_css_px,
                x: source.x,
                y: source.y,
                z: None,
            })
            .map_err(|_| {
                error_to_js(&interaction_error(InteractionError::Overflow(
                    "pick result",
                )))
            })
        }

        /// Releases renderer, scene, buffer, and canvas resources idempotently.
        pub fn dispose(&mut self) {
            if let Some(renderer) = self.renderer.take() {
                renderer.dispose();
            }
            if let Some(canvas) = self.canvas.take() {
                canvas.set_width(0);
                canvas.set_height(0);
            }
            self.scene = None;
            self.buffers.clear();
            self.picking_objects.clear();
            self.gesture_start = None;
            self.scene_cache = SceneCache::default();
            self.text_layout = None;
            self.lifecycle.dispose();
        }

        fn require_live(&self) -> Result<(), PlotWebError> {
            if self.lifecycle.state() == PlotWebState::Disposed {
                Err(PlotWebError::new(
                    PlotWebErrorKind::Disposed,
                    "The Plot Figure has already been disposed.",
                ))
            } else {
                Ok(())
            }
        }
    }

    /// Creates one browser Figure lifecycle around an existing canvas element.
    #[wasm_bindgen]
    pub async fn create_plot_figure(canvas: HtmlCanvasElement) -> Result<PlotFigure, JsValue> {
        let mut lifecycle = PlotWebLifecycle::default();
        lifecycle
            .begin_initialization()
            .map_err(|error| error_to_js(&error))?;
        let has_web_gpu = web_sys::window()
            .map(|window| window.navigator())
            .and_then(|navigator| Reflect::get(navigator.as_ref(), &JsValue::from_str("gpu")).ok())
            .is_some_and(|gpu| !gpu.is_null() && !gpu.is_undefined());
        if !has_web_gpu {
            lifecycle.fail(PlotWebError::new(
                PlotWebErrorKind::UnsupportedWebGpu,
                "WebGPU is unavailable. OpenMat Plot Engine requires WebGPU and does not fall back to WebGL.",
            ));
            return Ok(PlotFigure {
                canvas: Some(canvas),
                lifecycle,
                renderer: None,
                scene: None,
                buffers: HashMap::new(),
                picking_objects: HashMap::new(),
                gesture_start: None,
                scene_cache: SceneCache::default(),
                text_layout: None,
            });
        }
        let renderer = match RendererAdapter::initialize(&canvas).await {
            Ok(renderer) => {
                lifecycle
                    .mark_ready()
                    .map_err(|error| error_to_js(&error))?;
                Some(renderer)
            }
            Err(error) => {
                lifecycle.fail(error);
                None
            }
        };
        Ok(PlotFigure {
            canvas: Some(canvas),
            lifecycle,
            renderer,
            scene: None,
            buffers: HashMap::new(),
            picking_objects: HashMap::new(),
            gesture_start: None,
            scene_cache: SceneCache::default(),
            text_layout: None,
        })
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct InteractionResultDto {
        axes_id: String,
        dimension: &'static str,
        x_limits: [f64; 2],
        y_limits: [f64; 2],
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<[f64; 2]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        camera_scale: Option<f64>,
        changed: bool,
    }

    fn camera_scale(camera: OrbitCamera3D) -> f64 {
        match camera.projection() {
            Projection3D::Perspective {
                vertical_fov_radians,
                ..
            } => vertical_fov_radians.to_degrees() / 45.0,
            Projection3D::Orthographic { vertical_span, .. } => vertical_span / 1.8,
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PickResultDto<'a> {
        picking_id: u32,
        object_id: &'a str,
        kind: &'static str,
        primitive_index: u32,
        source_index: u32,
        distance_css_px: f64,
        x: f64,
        y: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        z: Option<f64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BrowserTextMeasurementDto {
        key: String,
        width_css_px: f64,
        height_css_px: f64,
        ascent_css_px: f64,
        descent_css_px: f64,
        advance_css_px: f64,
    }

    impl BrowserTextMeasurementDto {
        fn into_layout(self) -> Result<TextMeasurement, JsValue> {
            let metrics = TextMetrics::new(
                browser_metric(self.width_css_px)?,
                browser_metric(self.height_css_px)?,
                browser_metric(self.ascent_css_px)?,
                browser_metric(self.descent_css_px)?,
                browser_metric(self.advance_css_px)?,
            )
            .map_err(|_| {
                error_to_js(&PlotWebError::new(
                    PlotWebErrorKind::InvalidScene,
                    "Browser text measurements are inconsistent.",
                ))
            })?;
            Ok(TextMeasurement {
                key: self.key,
                metrics,
            })
        }
    }

    fn browser_metric(value: f64) -> Result<f32, JsValue> {
        if !value.is_finite() || value < 0.0 || value > f64::from(f32::MAX) {
            return Err(error_to_js(&PlotWebError::new(
                PlotWebErrorKind::InvalidScene,
                "A browser text metric is outside the supported range.",
            )));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(value as f32)
    }

    fn semantic_home(
        snapshot: &FigureSnapshot,
        axes_id: Option<&str>,
    ) -> Result<DataRect, PlotWebError> {
        snapshot
            .objects
            .iter()
            .find_map(|object| match object {
                GraphicsObject::Axes2d { fields, properties }
                    if axes_id.is_none_or(|axes_id| fields.id == axes_id) =>
                {
                    Some(DataRect::new(
                        properties.x_limits[0],
                        properties.x_limits[1],
                        properties.y_limits[0],
                        properties.y_limits[1],
                    ))
                }
                _ => None,
            })
            .transpose()
            .map_err(interaction_error)
            .map(|limits| {
                limits.unwrap_or(DataRect {
                    min: openmat_plot_wgpu::DataPoint { x: 0.0, y: 0.0 },
                    max: openmat_plot_wgpu::DataPoint { x: 1.0, y: 1.0 },
                })
            })
    }

    fn semantic_is_polar(snapshot: &FigureSnapshot, axes_id: Option<&str>) -> bool {
        snapshot.objects.iter().any(|object| {
            matches!(
                object,
                GraphicsObject::Axes2d { fields, properties }
                    if axes_id.is_none_or(|axes_id| fields.id == axes_id)
                        && properties.coordinate_system
                            == openmat_plot_protocol::AxesCoordinateSystem::Polar
            )
        })
    }

    fn frame_required() -> PlotWebError {
        PlotWebError::new(
            PlotWebErrorKind::InvalidInteraction,
            "Render the Figure before starting an interaction.",
        )
    }

    fn interaction_error(_error: InteractionError) -> PlotWebError {
        invalid_interaction_coordinates()
    }

    fn invalid_interaction_coordinates() -> PlotWebError {
        PlotWebError::new(
            PlotWebErrorKind::InvalidInteraction,
            "The Plot interaction coordinates or view limits are invalid.",
        )
    }

    fn camera_interaction_error() -> PlotWebError {
        PlotWebError::new(
            PlotWebErrorKind::InvalidInteraction,
            "The 3D camera interaction could not be represented.",
        )
    }

    fn error_to_js(error: &PlotWebError) -> JsValue {
        let object = Object::new();
        let _ = Reflect::set(
            object.as_ref(),
            &JsValue::from_str("kind"),
            &JsValue::from_str(error.kind().as_str()),
        );
        let _ = Reflect::set(
            object.as_ref(),
            &JsValue::from_str("message"),
            &JsValue::from_str(error.message()),
        );
        object.unchecked_into()
    }

    fn renderer_init_error(error: &RendererInitError) -> PlotWebError {
        let kind = match error {
            RendererInitError::AdapterUnavailable(_) => PlotWebErrorKind::UnsupportedWebGpu,
            RendererInitError::InsufficientLimits(_) => PlotWebErrorKind::InsufficientLimits,
            RendererInitError::DeviceCreation(_) | RendererInitError::PipelineCreation(_) => {
                PlotWebErrorKind::DeviceCreation
            }
            RendererInitError::DeviceLost(_) => PlotWebErrorKind::DeviceLost,
        };
        PlotWebError::new(
            kind,
            format!("Plot renderer initialization failed: {error}."),
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_size_keeps_css_and_device_pixels_distinct() {
        let size = CanvasSize::new(320.25, 200.5, 2.0).unwrap();
        assert_eq!(size.css_width.to_bits(), 320.25_f64.to_bits());
        assert_eq!(size.css_height.to_bits(), 200.5_f64.to_bits());
        assert_eq!(size.device_pixel_ratio.to_bits(), 2.0_f64.to_bits());
        assert_eq!(size.device_width, 641);
        assert_eq!(size.device_height, 401);
    }

    #[test]
    fn canvas_size_rejects_invalid_and_overflowing_inputs() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                CanvasSize::new(invalid, 100.0, 1.0).unwrap_err().kind(),
                PlotWebErrorKind::InvalidCanvasSize
            );
        }
        assert_eq!(
            CanvasSize::new(f64::from(u32::MAX), 100.0, 2.0)
                .unwrap_err()
                .kind(),
            PlotWebErrorKind::CanvasSizeOverflow
        );
    }

    #[test]
    fn lifecycle_preserves_typed_renderer_failure_and_disposal() {
        let mut lifecycle = PlotWebLifecycle::default();
        lifecycle.begin_initialization().unwrap();
        assert_eq!(lifecycle.state(), PlotWebState::Initializing);
        lifecycle.fail(PlotWebError::new(
            PlotWebErrorKind::RendererUnavailable,
            "renderer adapter pending",
        ));
        assert_eq!(lifecycle.state(), PlotWebState::RendererUnavailable);
        assert_eq!(
            lifecycle.last_error().map(PlotWebError::kind),
            Some(PlotWebErrorKind::RendererUnavailable)
        );
        lifecycle.resize(640.0, 480.0, 1.5).unwrap();
        lifecycle.dispose();
        lifecycle.dispose();
        assert_eq!(lifecycle.state(), PlotWebState::Disposed);
        assert_eq!(
            lifecycle.resize(640.0, 480.0, 1.0).unwrap_err().kind(),
            PlotWebErrorKind::Disposed
        );
    }

    #[test]
    fn compiler_resource_limit_keeps_exact_buffer_type_and_sizes() {
        let error = plot_compile_error(&CompileError::BufferLimitExceeded {
            kind: openmat_plot_wgpu::BufferKind::MeshVertex,
            requested_bytes: 523_458_480,
            maximum_bytes: 256 << 20,
        });
        assert_eq!(error.kind(), PlotWebErrorKind::ResourceLimit);
        assert!(error.message().contains("MeshVertex"));
        assert!(error.message().contains("499.21 MiB"));
        assert!(error.message().contains("256.00 MiB"));
        assert!(error.message().contains("523458480 bytes"));
        assert!(error.message().contains("268435456 bytes"));
    }
}
