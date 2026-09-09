use openmat_plot_mir::{
    CssPoint, DeviceRect, Lighting3D, MarkerShape, Rgba, RulerSelection3D, ViewProjection3D,
    Viewport,
};
use wgpu::util::DeviceExt;

use crate::lifecycle::{SurfaceEvent, SurfaceLifecycle};
use crate::pipeline::{DEPTH_FORMAT, PipelineResources, PipelineSet, SAMPLE_COUNT};
use crate::{
    BufferKind, CompiledFrame, DataRect, DeviceLoss, DrawCommand, DrawKind, InteractionError,
    PickHit, PickTolerance, PickingIndex, RenderError, RendererInitError, RendererLifecycle,
    RendererState, SurfaceConfigureError, SurfaceCreateError, SurfaceExtent, SurfaceState,
    UploadError, ViewTransform2D, check_required_limits, required_device_limits,
};

pub struct RendererRequest {
    instance: wgpu::Instance,
    lifecycle: RendererLifecycle,
}

impl Default for RendererRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererRequest {
    #[must_use]
    pub fn new() -> Self {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = renderer_backends();
        Self::from_instance(wgpu::Instance::new(descriptor))
    }

    #[must_use]
    pub fn from_instance(instance: wgpu::Instance) -> Self {
        Self {
            instance,
            lifecycle: RendererLifecycle::default(),
        }
    }

    #[must_use]
    pub fn state(&self) -> RendererState {
        self.lifecycle.state()
    }

    /// Creates a safe native-window or browser-canvas surface owned by this request context.
    ///
    /// # Errors
    /// Returns [`SurfaceCreateError`] when wgpu cannot create the surface.
    pub fn create_surface<'window>(
        &self,
        target: impl Into<wgpu::SurfaceTarget<'window>>,
    ) -> Result<RenderSurface<'window>, SurfaceCreateError> {
        self.instance
            .create_surface(target)
            .map(RenderSurface::new)
            .map_err(SurfaceCreateError::Creation)
    }

    /// Requests the minimum Plot Engine device profile and creates both presentation pipelines.
    ///
    /// # Errors
    /// Returns a typed adapter, limits, device, pipeline, or device-loss error.
    pub async fn initialize(
        &self,
        compatible_surface: Option<&RenderSurface<'_>>,
    ) -> Result<Renderer, RendererInitError> {
        self.lifecycle.transition(RendererState::RequestingAdapter);
        let adapter_result = self
            .instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: false,
                compatible_surface: compatible_surface.map(RenderSurface::raw),
                apply_limit_buckets: false,
            })
            .await;
        let adapter = match adapter_result {
            Ok(adapter) => adapter,
            Err(error) => {
                self.lifecycle.transition(RendererState::AdapterUnavailable);
                return Err(RendererInitError::AdapterUnavailable(error));
            }
        };

        self.lifecycle.transition(RendererState::CheckingLimits);
        let limits = adapter.limits();
        let violations = check_required_limits(&limits);
        if !violations.is_empty() {
            self.lifecycle.transition(RendererState::InsufficientLimits);
            return Err(RendererInitError::InsufficientLimits(violations));
        }

        self.lifecycle.transition(RendererState::RequestingDevice);
        let device_result = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("OpenMat Plot Engine device"),
                required_features: wgpu::Features::empty(),
                required_limits: required_device_limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await;
        let (device, queue) = match device_result {
            Ok(result) => result,
            Err(error) => {
                self.lifecycle
                    .transition(RendererState::DeviceCreationFailed);
                return Err(RendererInitError::DeviceCreation(error));
            }
        };

        let loss_lifecycle = self.lifecycle.clone();
        device.set_device_lost_callback(move |reason, diagnostic| {
            loss_lifecycle.transition(RendererState::DeviceLost(DeviceLoss { reason, diagnostic }));
        });

        self.lifecycle.transition(RendererState::CreatingPipelines);
        let pipelines = match PipelineResources::create(&device).await {
            Ok(pipelines) => pipelines,
            Err(error) => {
                if let RendererState::DeviceLost(loss) = self.lifecycle.state() {
                    return Err(RendererInitError::DeviceLost(loss));
                }
                self.lifecycle
                    .transition(RendererState::PipelineCreationFailed);
                return Err(error);
            }
        };
        self.lifecycle
            .mark_ready()
            .map_err(RendererInitError::DeviceLost)?;

        Ok(Renderer {
            adapter,
            device,
            queue,
            pipelines,
            lifecycle: self.lifecycle.clone(),
            limits,
        })
    }
}

pub struct Renderer {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: PipelineResources,
    lifecycle: RendererLifecycle,
    limits: wgpu::Limits,
}

impl Renderer {
    #[must_use]
    pub fn state(&self) -> RendererState {
        self.lifecycle.state()
    }

    #[must_use]
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    #[must_use]
    pub const fn limits(&self) -> &wgpu::Limits {
        &self.limits
    }

    /// Uploads deterministic compiler payloads into immutable frame GPU buffers.
    ///
    /// # Errors
    /// Returns [`UploadError`] if the device was lost, a buffer exceeds the device limit, or
    /// a compiler invariant is broken.
    #[allow(clippy::too_many_lines)]
    pub fn upload(&self, compiled: &CompiledFrame) -> Result<GpuFrame, UploadError> {
        self.ensure_ready().map_err(UploadError::DeviceLost)?;
        self.check_upload_size(BufferKind::MeshVertex, compiled.mesh_vertex_bytes())?;
        self.check_upload_size(BufferKind::MeshIndex, compiled.mesh_index_bytes())?;
        self.check_upload_size(BufferKind::MarkerInstance, compiled.marker_instance_bytes())?;
        self.check_upload_size(
            BufferKind::SurfaceVertex3D,
            compiled.surface_vertex_3d_bytes(),
        )?;
        self.check_upload_size(
            BufferKind::SurfaceIndex3D,
            compiled.surface_index_3d_bytes(),
        )?;
        self.check_upload_size(BufferKind::LineSegment3D, compiled.line_segment_3d_bytes())?;
        self.check_upload_size(BufferKind::ScreenLine, compiled.screen_line_bytes())?;

        let mesh_vertex = create_uploaded_buffer(
            &self.device,
            "openmat plot mesh vertices",
            compiled.mesh_vertex_bytes(),
            wgpu::BufferUsages::VERTEX,
        );
        let mesh_index = create_uploaded_buffer(
            &self.device,
            "openmat plot mesh indices",
            compiled.mesh_index_bytes(),
            wgpu::BufferUsages::INDEX,
        );
        let marker_instance = create_uploaded_buffer(
            &self.device,
            "openmat plot marker instances",
            compiled.marker_instance_bytes(),
            wgpu::BufferUsages::VERTEX,
        );
        let surface_vertex_3d = create_uploaded_buffer(
            &self.device,
            "openmat plot 3D surface vertices",
            compiled.surface_vertex_3d_bytes(),
            wgpu::BufferUsages::VERTEX,
        );
        let surface_index_3d = create_uploaded_buffer(
            &self.device,
            "openmat plot 3D surface indices",
            compiled.surface_index_3d_bytes(),
            wgpu::BufferUsages::INDEX,
        );
        let line_segment_3d = create_uploaded_buffer(
            &self.device,
            "openmat plot 3D line segments",
            compiled.line_segment_3d_bytes(),
            wgpu::BufferUsages::VERTEX,
        );
        let screen_line = create_uploaded_buffer(
            &self.device,
            "openmat plot screen-space lines",
            compiled.screen_line_bytes(),
            wgpu::BufferUsages::VERTEX,
        );

        for draw in compiled.draws() {
            match draw.kind {
                DrawKind::Triangles { .. } if mesh_vertex.is_none() || mesh_index.is_none() => {
                    return Err(UploadError::MissingBuffer(BufferKind::MeshVertex));
                }
                DrawKind::Markers { .. } if marker_instance.is_none() => {
                    return Err(UploadError::MissingBuffer(BufferKind::MarkerInstance));
                }
                DrawKind::SurfaceTriangles3D { .. }
                    if surface_vertex_3d.is_none() || surface_index_3d.is_none() =>
                {
                    return Err(UploadError::MissingBuffer(BufferKind::SurfaceVertex3D));
                }
                DrawKind::LineSegments3D { .. }
                | DrawKind::SurfaceEdgeSegments3D { .. }
                | DrawKind::RulerLines3D { .. }
                | DrawKind::TickMarks3D { .. }
                    if line_segment_3d.is_none() =>
                {
                    return Err(UploadError::MissingBuffer(BufferKind::LineSegment3D));
                }
                DrawKind::ScreenLines { .. } if screen_line.is_none() => {
                    return Err(UploadError::MissingBuffer(BufferKind::ScreenLine));
                }
                DrawKind::Triangles { .. }
                | DrawKind::Markers { .. }
                | DrawKind::SurfaceTriangles3D { .. }
                | DrawKind::LineSegments3D { .. }
                | DrawKind::SurfaceEdgeSegments3D { .. }
                | DrawKind::RulerLines3D { .. }
                | DrawKind::TickMarks3D { .. }
                | DrawKind::ScreenLines { .. } => {}
            }
        }

        let view = ViewTransform2D::new(
            compiled.viewport(),
            DataRect::new(0.0, 1.0, 0.0, 1.0).map_err(|_| UploadError::InvalidViewport)?,
        )
        .map_err(|_| UploadError::InvalidViewport)?;
        Ok(GpuFrame {
            viewport: compiled.viewport(),
            clear_color: compiled.clear_color(),
            draws: compiled.draws().to_vec(),
            view,
            picking: compiled.picking_index().clone(),
            mesh_vertex,
            mesh_index,
            marker_instance,
            surface_vertex_3d,
            surface_index_3d,
            line_segment_3d,
            screen_line,
            view_projection_3d: compiled.view_projection_3d(),
            lighting_3d: compiled.lighting_3d(),
            ruler_selection_3d: compiled.ruler_selection_3d(),
        })
    }

    fn check_upload_size(&self, kind: BufferKind, bytes: &[u8]) -> Result<(), UploadError> {
        let size = match u64::try_from(bytes.len()) {
            Ok(size) => size,
            Err(_) => u64::MAX,
        };
        if size > self.limits.max_buffer_size {
            return Err(UploadError::BufferExceedsDeviceLimit {
                kind,
                size_bytes: size,
                maximum_bytes: self.limits.max_buffer_size,
            });
        }
        Ok(())
    }

    fn ensure_ready(&self) -> Result<(), DeviceLoss> {
        match self.state() {
            RendererState::DeviceLost(loss) => Err(loss),
            _ => Ok(()),
        }
    }
}

pub struct GpuFrame {
    viewport: Viewport,
    clear_color: Rgba,
    draws: Vec<DrawCommand>,
    view: ViewTransform2D,
    picking: PickingIndex,
    mesh_vertex: Option<wgpu::Buffer>,
    mesh_index: Option<wgpu::Buffer>,
    marker_instance: Option<wgpu::Buffer>,
    surface_vertex_3d: Option<wgpu::Buffer>,
    surface_index_3d: Option<wgpu::Buffer>,
    line_segment_3d: Option<wgpu::Buffer>,
    screen_line: Option<wgpu::Buffer>,
    view_projection_3d: Option<ViewProjection3D>,
    lighting_3d: Option<Lighting3D>,
    ruler_selection_3d: Option<RulerSelection3D>,
}

impl GpuFrame {
    #[must_use]
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }

    #[must_use]
    pub const fn view_transform(&self) -> ViewTransform2D {
        self.view
    }

    #[must_use]
    pub const fn view_projection_3d(&self) -> Option<ViewProjection3D> {
        self.view_projection_3d
    }

    #[must_use]
    pub const fn ruler_selection_3d(&self) -> Option<RulerSelection3D> {
        self.ruler_selection_3d
    }

    /// Replaces only renderer-local 3D camera state. Immutable source and GPU
    /// geometry buffers remain untouched during pointer-frequency orbit input.
    pub fn set_view_projection_3d(&mut self, matrix: ViewProjection3D) {
        self.view_projection_3d = Some(matrix);
    }

    /// Replaces only renderer-local ruler edge selection. Candidate geometry
    /// remains in immutable GPU buffers during pointer-frequency orbit input.
    pub fn set_ruler_selection_3d(&mut self, selection: RulerSelection3D) {
        self.ruler_selection_3d = Some(selection);
    }

    /// Rebinds the immutable normalized source geometry to semantic home limits.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid limits or viewport state.
    pub fn set_home_limits(&mut self, limits: DataRect) -> Result<(), InteractionError> {
        self.view = ViewTransform2D::new(self.viewport, limits)?;
        Ok(())
    }

    pub fn home(&mut self) {
        self.view.home();
    }

    /// Applies a CSS-pixel pan without replacing GPU buffers.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid input or overflow.
    pub fn pan_css(&mut self, delta_x: f64, delta_y: f64) -> Result<(), InteractionError> {
        self.view.pan_css(delta_x, delta_y)
    }

    /// Applies anchored zoom without replacing GPU buffers.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid input or overflow.
    pub fn zoom_css(&mut self, anchor: CssPoint, factor: f64) -> Result<(), InteractionError> {
        self.view.zoom_css(anchor, factor)
    }

    /// Applies a clamped CSS box zoom without replacing GPU buffers.
    ///
    /// # Errors
    /// Returns [`InteractionError`] for invalid input or a zero-area box.
    pub fn box_zoom_css(
        &mut self,
        first: CssPoint,
        second: CssPoint,
    ) -> Result<(), InteractionError> {
        self.view.box_zoom_css(first, second)
    }

    #[must_use]
    pub fn pick(&self, pointer: CssPoint, tolerance: PickTolerance) -> Option<PickHit> {
        self.picking.pick(self.view, pointer, tolerance)
    }
}

pub struct RenderSurface<'window> {
    raw: wgpu::Surface<'window>,
    lifecycle: SurfaceLifecycle,
    configuration: Option<wgpu::SurfaceConfiguration>,
    multisample_target: Option<MultisampleTarget>,
}

struct MultisampleTarget {
    _color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl<'window> RenderSurface<'window> {
    fn new(raw: wgpu::Surface<'window>) -> Self {
        Self {
            raw,
            lifecycle: SurfaceLifecycle::default(),
            configuration: None,
            multisample_target: None,
        }
    }

    fn raw(&self) -> &wgpu::Surface<'window> {
        &self.raw
    }

    #[must_use]
    pub fn state(&self) -> SurfaceState {
        self.lifecycle.state()
    }

    #[must_use]
    pub fn format(&self) -> Option<wgpu::TextureFormat> {
        self.configuration
            .as_ref()
            .map(|configuration| configuration.format)
    }

    /// Configures or resizes the presentation surface. A zero extent suspends it without calling
    /// `wgpu::Surface::configure`.
    ///
    /// # Errors
    /// Returns [`SurfaceConfigureError`] for device loss, unsupported limits/formats, or a lost
    /// surface that must be recreated.
    pub fn configure(
        &mut self,
        renderer: &Renderer,
        extent: SurfaceExtent,
    ) -> Result<ResizeOutcome, SurfaceConfigureError> {
        if let RendererState::DeviceLost(loss) = renderer.state() {
            self.lifecycle.transition(SurfaceEvent::DeviceLost);
            return Err(SurfaceConfigureError::DeviceLost(loss));
        }
        if matches!(self.state(), SurfaceState::Lost { .. }) {
            return Err(SurfaceConfigureError::SurfaceMustBeRecreated);
        }
        if extent.is_zero() {
            self.configuration = None;
            self.multisample_target = None;
            self.lifecycle.transition(SurfaceEvent::Configure(extent));
            return Ok(ResizeOutcome::Suspended);
        }
        if extent.width > renderer.limits.max_texture_dimension_2d
            || extent.height > renderer.limits.max_texture_dimension_2d
        {
            return Err(SurfaceConfigureError::DimensionsExceedLimit {
                width: extent.width,
                height: extent.height,
                maximum_dimension: renderer.limits.max_texture_dimension_2d,
            });
        }

        let capabilities = self.raw.get_capabilities(&renderer.adapter);
        let Some(format) = select_surface_format(&capabilities.formats) else {
            return Err(SurfaceConfigureError::NoCompatibleFormat {
                available: capabilities.formats,
            });
        };
        let present_mode = capabilities
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Fifo)
            .or_else(|| capabilities.present_modes.first().copied())
            .ok_or(SurfaceConfigureError::NoPresentMode)?;
        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::Auto)
            .or_else(|| capabilities.alpha_modes.first().copied())
            .ok_or(SurfaceConfigureError::NoAlphaMode)?;
        let configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            // Plot colors are already sRGB-encoded MATLAB RGB values. A non-sRGB
            // UNORM format plus an explicit sRGB presentation space preserves
            // those numeric values without applying a second transfer function.
            color_space: matlab_surface_color_space(),
            width: extent.width,
            height: extent.height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        self.raw.configure(&renderer.device, &configuration);
        self.multisample_target = Some(create_multisample_target(&renderer.device, &configuration));
        self.configuration = Some(configuration);
        self.lifecycle.transition(SurfaceEvent::Configure(extent));
        Ok(ResizeOutcome::Configured {
            extent,
            format,
            state: self.state(),
        })
    }

    /// Clears and renders a previously uploaded frame, then presents it.
    ///
    /// # Errors
    /// Returns typed device and surface acquisition failures. Lost/outdated surfaces require
    /// recreation or configuration by the caller; timeouts and occlusion may be retried later.
    pub fn render(
        &mut self,
        renderer: &Renderer,
        frame: &GpuFrame,
    ) -> Result<RenderOutcome, RenderError> {
        self.render_frames(renderer, &[frame])
    }

    /// Clears and renders multiple Axes frames onto one Figure surface, then presents once.
    ///
    /// Every frame retains its own viewport, interaction transform, depth buffer clear, and
    /// camera uniform. Color is cleared by the first frame and loaded by subsequent frames.
    ///
    /// # Errors
    /// Returns the same typed device and surface failures as [`Self::render`], and rejects an
    /// empty frame slice as a surface validation error.
    pub fn render_frames(
        &mut self,
        renderer: &Renderer,
        frames: &[&GpuFrame],
    ) -> Result<RenderOutcome, RenderError> {
        if let RendererState::DeviceLost(loss) = renderer.state() {
            self.lifecycle.transition(SurfaceEvent::DeviceLost);
            return Err(RenderError::DeviceLost(loss));
        }
        match self.state() {
            SurfaceState::Unconfigured => return Err(RenderError::SurfaceNotConfigured),
            SurfaceState::Suspended { .. } => return Err(RenderError::SurfaceSuspended),
            SurfaceState::Lost { .. } => return Err(RenderError::SurfaceLost),
            SurfaceState::Outdated { .. } => return Err(RenderError::SurfaceOutdated),
            SurfaceState::ValidationFailed { .. } => return Err(RenderError::SurfaceValidation),
            SurfaceState::DeviceLost => {
                return Err(RenderError::DeviceLost(DeviceLoss {
                    reason: wgpu::DeviceLostReason::Unknown,
                    diagnostic: String::new(),
                }));
            }
            SurfaceState::Configured { .. }
            | SurfaceState::TimedOut { .. }
            | SurfaceState::Occluded { .. } => {}
        }
        let Some(configuration) = self.configuration.as_ref() else {
            return Err(RenderError::SurfaceNotConfigured);
        };
        let Some(pipelines) = renderer.pipelines.for_format(configuration.format) else {
            return Err(RenderError::UnsupportedSurfaceFormat(configuration.format));
        };
        let Some(multisample_target) = self.multisample_target.as_ref() else {
            return Err(RenderError::SurfaceNotConfigured);
        };
        if frames.is_empty() {
            return Err(RenderError::SurfaceValidation);
        }

        let (surface_texture, suboptimal) = match self.raw.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.lifecycle.transition(SurfaceEvent::Timeout);
                return Err(RenderError::SurfaceTimeout);
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.lifecycle.transition(SurfaceEvent::Occluded);
                return Err(RenderError::SurfaceOccluded);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.lifecycle.transition(SurfaceEvent::Outdated);
                return Err(RenderError::SurfaceOutdated);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.lifecycle.transition(SurfaceEvent::Lost);
                return Err(RenderError::SurfaceLost);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.lifecycle.transition(SurfaceEvent::ValidationFailed);
                return Err(RenderError::SurfaceValidation);
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let pass_resources = RenderPassResources {
            multisample_target,
            resolve_view: &view,
            configuration,
            bind_group: renderer.pipelines.bind_group(),
            pipelines,
        };
        for (index, frame) in frames.iter().enumerate() {
            let uniform_bytes = pack_view_uniform(
                configuration.width,
                configuration.height,
                frame.viewport,
                frame.view,
                frame.view_projection_3d,
                frame.lighting_3d,
                frame.ruler_selection_3d,
            )
            .map_err(|_| RenderError::SurfaceValidation)?;
            renderer
                .queue
                .write_buffer(renderer.pipelines.uniform_buffer(), 0, &uniform_bytes);
            let mut encoder =
                renderer
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("openmat plot axes frame encoder"),
                    });
            encode_render_pass(&mut encoder, &pass_resources, frame, index == 0);
            renderer.queue.submit([encoder.finish()]);
        }
        renderer.queue.present(surface_texture);

        self.lifecycle.transition(if suboptimal {
            SurfaceEvent::Suboptimal
        } else {
            SurfaceEvent::Acquired
        });
        Ok(RenderOutcome {
            presented: true,
            reconfigure_recommended: suboptimal,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeOutcome {
    Suspended,
    Configured {
        extent: SurfaceExtent,
        format: wgpu::TextureFormat,
        state: SurfaceState,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOutcome {
    pub presented: bool,
    pub reconfigure_recommended: bool,
}

#[must_use]
pub fn select_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|preferred| formats.contains(preferred))
}

const fn matlab_surface_color_space() -> wgpu::SurfaceColorSpace {
    wgpu::SurfaceColorSpace::Srgb
}

fn create_uploaded_buffer(
    device: &wgpu::Device,
    label: &'static str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> Option<wgpu::Buffer> {
    if bytes.is_empty() {
        None
    } else {
        Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            }),
        )
    }
}

struct RenderPassResources<'a> {
    multisample_target: &'a MultisampleTarget,
    resolve_view: &'a wgpu::TextureView,
    configuration: &'a wgpu::SurfaceConfiguration,
    bind_group: &'a wgpu::BindGroup,
    pipelines: &'a PipelineSet,
}

#[allow(clippy::too_many_lines)]
fn encode_render_pass(
    encoder: &mut wgpu::CommandEncoder,
    resources: &RenderPassResources<'_>,
    frame: &GpuFrame,
    clear_color: bool,
) {
    let multisample_target = resources.multisample_target;
    let resolve_view = resources.resolve_view;
    let configuration = resources.configuration;
    let bind_group = resources.bind_group;
    let pipelines = resources.pipelines;
    let attachment = [Some(wgpu::RenderPassColorAttachment {
        view: &multisample_target.color_view,
        depth_slice: None,
        resolve_target: Some(resolve_view),
        ops: wgpu::Operations {
            load: if clear_color {
                wgpu::LoadOp::Clear(to_wgpu_color(frame.clear_color))
            } else {
                wgpu::LoadOp::Load
            },
            store: wgpu::StoreOp::Store,
        },
    })];
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("openmat plot render pass"),
        color_attachments: &attachment,
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &multisample_target.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_bind_group(0, bind_group, &[]);

    for draw in &frame.draws {
        let Some(scissor) = clamp_scissor(
            draw.scissor,
            SurfaceExtent {
                width: configuration.width,
                height: configuration.height,
            },
        ) else {
            continue;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        match &draw.kind {
            DrawKind::Triangles { indices } => {
                let (Some(vertices), Some(mesh_indices)) = (&frame.mesh_vertex, &frame.mesh_index)
                else {
                    continue;
                };
                pass.set_pipeline(&pipelines.mesh);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(mesh_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(indices.clone(), 0, 0..1);
            }
            DrawKind::Markers { shape, instances } => {
                let Some(marker_instances) = &frame.marker_instance else {
                    continue;
                };
                let pipeline = match shape {
                    MarkerShape::Circle => &pipelines.circle_marker,
                    MarkerShape::Square => &pipelines.square_marker,
                    MarkerShape::Diamond => &pipelines.diamond_marker,
                    MarkerShape::UpTriangle => &pipelines.up_triangle_marker,
                    MarkerShape::DownTriangle => &pipelines.down_triangle_marker,
                    MarkerShape::Plus => &pipelines.plus_marker,
                    MarkerShape::Cross => &pipelines.cross_marker,
                    MarkerShape::HorizontalLine => &pipelines.horizontal_line_marker,
                    MarkerShape::VerticalLine => &pipelines.vertical_line_marker,
                };
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, marker_instances.slice(..));
                pass.draw(0..6, instances.clone());
            }
            DrawKind::SurfaceTriangles3D {
                indices,
                interpolation,
                edge_overlay,
            } => {
                let (Some(vertices), Some(surface_indices)) =
                    (&frame.surface_vertex_3d, &frame.surface_index_3d)
                else {
                    continue;
                };
                let pipeline = match (*interpolation, *edge_overlay) {
                    (openmat_plot_mir::SurfaceColorInterpolation::Flat, false) => {
                        &pipelines.surface_3d
                    }
                    (openmat_plot_mir::SurfaceColorInterpolation::Flat, true) => {
                        &pipelines.surface_3d_with_edges
                    }
                    (openmat_plot_mir::SurfaceColorInterpolation::Smooth, biased) => {
                        if biased {
                            &pipelines.surface_3d_smooth_with_edges
                        } else {
                            &pipelines.surface_3d_smooth
                        }
                    }
                };
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_index_buffer(surface_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(indices.clone(), 0, 0..1);
            }
            DrawKind::LineSegments3D { instances } => {
                let Some(line_segments) = &frame.line_segment_3d else {
                    continue;
                };
                pass.set_pipeline(&pipelines.line_3d);
                pass.set_vertex_buffer(0, line_segments.slice(..));
                pass.draw(0..6, instances.clone());
            }
            DrawKind::SurfaceEdgeSegments3D { instances } => {
                let Some(line_segments) = &frame.line_segment_3d else {
                    continue;
                };
                pass.set_pipeline(&pipelines.surface_edge_3d);
                pass.set_vertex_buffer(0, line_segments.slice(..));
                pass.draw(0..6, instances.clone());
            }
            DrawKind::RulerLines3D { instances } => {
                let Some(line_segments) = &frame.line_segment_3d else {
                    continue;
                };
                pass.set_pipeline(&pipelines.ruler_3d);
                pass.set_vertex_buffer(0, line_segments.slice(..));
                pass.draw(0..6, instances.clone());
            }
            DrawKind::TickMarks3D { instances } => {
                let Some(line_segments) = &frame.line_segment_3d else {
                    continue;
                };
                pass.set_pipeline(&pipelines.tick_3d);
                pass.set_vertex_buffer(0, line_segments.slice(..));
                pass.draw(0..6, instances.clone());
            }
            DrawKind::ScreenLines { instances } => {
                let Some(screen_lines) = &frame.screen_line else {
                    continue;
                };
                pass.set_pipeline(&pipelines.screen_line);
                pass.set_vertex_buffer(0, screen_lines.slice(..));
                pass.draw(0..6, instances.clone());
            }
        }
    }
}

fn create_multisample_target(
    device: &wgpu::Device,
    configuration: &wgpu::SurfaceConfiguration,
) -> MultisampleTarget {
    let color_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("openmat plot multisample color target"),
        size: wgpu::Extent3d {
            width: configuration.width,
            height: configuration.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: configuration.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("openmat plot multisample depth target"),
        size: wgpu::Extent3d {
            width: configuration.width,
            height: configuration.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
    MultisampleTarget {
        _color_texture: color_texture,
        color_view,
        _depth_texture: depth_texture,
        depth_view,
    }
}

fn clamp_scissor(rect: DeviceRect, extent: SurfaceExtent) -> Option<DeviceRect> {
    let right = rect.x.saturating_add(rect.width).min(extent.width);
    let bottom = rect.y.saturating_add(rect.height).min(extent.height);
    let x = rect.x.min(extent.width);
    let y = rect.y.min(extent.height);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    (width > 0 && height > 0).then_some(DeviceRect {
        x,
        y,
        width,
        height,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn pack_view_uniform(
    target_width: u32,
    target_height: u32,
    viewport: Viewport,
    view: ViewTransform2D,
    view_projection_3d: Option<ViewProjection3D>,
    lighting_3d: Option<Lighting3D>,
    ruler_selection_3d: Option<RulerSelection3D>,
) -> Result<[u8; 144], InteractionError> {
    let affine = view.source_to_view_affine()?;
    let mut values = vec![
        target_width as f32,
        target_height as f32,
        viewport.device.x as f32,
        viewport.device.y as f32,
        viewport.device.width as f32,
        viewport.device.height as f32,
        viewport.device_pixel_ratio.get() as f32,
        0.0,
        affine[0],
        affine[1],
        affine[2],
        affine[3],
    ];
    let matrix = view_projection_3d.unwrap_or_default().columns();
    values.extend(matrix.into_iter().flatten());
    let selection = ruler_selection_3d.unwrap_or_default();
    values.extend([
        f32::from(selection.x),
        f32::from(selection.y),
        f32::from(selection.z),
        0.0,
    ]);
    let lighting = lighting_3d.unwrap_or(Lighting3D {
        position: openmat_plot_mir::AxesPoint3D {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        enabled: false,
    });
    values.extend([
        lighting.position.x,
        lighting.position.y,
        lighting.position.z,
        if lighting.enabled { 1.0 } else { 0.0 },
    ]);
    let mut bytes = [0; 144];
    for (chunk, value) in bytes.chunks_exact_mut(4).zip(values) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

fn to_wgpu_color(color: Rgba) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color.red),
        g: f64::from(color.green),
        b: f64::from(color.blue),
        a: f64::from(color.alpha),
    }
}

#[cfg(target_arch = "wasm32")]
fn renderer_backends() -> wgpu::Backends {
    wgpu::Backends::BROWSER_WEBGPU
}

#[cfg(not(target_arch = "wasm32"))]
fn renderer_backends() -> wgpu::Backends {
    wgpu::Backends::DX12 | wgpu::Backends::METAL | wgpu::Backends::VULKAN
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;
    use std::time::Duration;

    struct ThreadWake(thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park_timeout(Duration::from_millis(100)),
        }
    }
}

#[cfg(test)]
mod tests {
    use openmat_plot_mir::{
        AxesPoint, AxesPoint3D, AxesVector3D, CssPx, CssRect, DevicePixelRatio, DrawOrder, LineCap,
        LineJoin, MirCommand, OverlayPlan, PlotFrame, RenderOperation, StrokeMesh, StrokeStyle,
        StrokeVertex, SurfaceMesh3D, SurfaceVertex3D,
    };

    use super::*;
    use crate::{DrawListCompiler, FrameCompileOptions};

    fn gpu_smoke_frame() -> CompiledFrame {
        let viewport = Viewport {
            css: CssRect::new(0.0, 0.0, 64.0, 64.0).unwrap(),
            device: DeviceRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            device_pixel_ratio: DevicePixelRatio::new(1.0).unwrap(),
        };
        let color = Rgba::new(0.25, 0.5, 0.75, 0.5).unwrap();
        let mesh = StrokeMesh {
            vertices: vec![
                StrokeVertex {
                    position: AxesPoint { x: 0.0, y: 0.0 },
                    edge_distance_css_px: 0.0,
                },
                StrokeVertex {
                    position: AxesPoint { x: 1.0, y: 0.0 },
                    edge_distance_css_px: 0.0,
                },
                StrokeVertex {
                    position: AxesPoint { x: 0.0, y: 1.0 },
                    edge_distance_css_px: 0.0,
                },
            ],
            indices: vec![0, 1, 2],
            color,
            style: StrokeStyle::solid(CssPx::new(1.0).unwrap(), LineCap::Butt, LineJoin::Miter)
                .unwrap(),
        };
        let frame = PlotFrame::new(
            vec![
                RenderOperation {
                    order: DrawOrder(0),
                    picking_id: None,
                    command: MirCommand::BeginViewport(viewport),
                },
                RenderOperation {
                    order: DrawOrder(1),
                    picking_id: None,
                    command: MirCommand::StrokeMesh(mesh),
                },
                RenderOperation {
                    order: DrawOrder(2),
                    picking_id: None,
                    command: MirCommand::EndViewport,
                },
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap()
    }

    fn gpu_surface_smoke_frame() -> CompiledFrame {
        let viewport = Viewport {
            css: CssRect::new(0.0, 0.0, 64.0, 64.0).unwrap(),
            device: DeviceRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            device_pixel_ratio: DevicePixelRatio::new(1.0).unwrap(),
        };
        let color = Rgba::new(0.25, 0.5, 0.75, 1.0).unwrap();
        let surface = SurfaceMesh3D {
            vertices: [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.5)]
                .into_iter()
                .map(|(x, y, z)| SurfaceVertex3D {
                    position: AxesPoint3D::new(x, y, z).unwrap(),
                    normal: AxesVector3D::new(0.0, 0.0, 1.0).unwrap(),
                    color,
                })
                .collect(),
            indices: vec![0, 1, 2],
            color_interpolation: openmat_plot_mir::SurfaceColorInterpolation::Flat,
        };
        let frame = PlotFrame::new(
            vec![
                RenderOperation {
                    order: DrawOrder(0),
                    picking_id: None,
                    command: MirCommand::BeginViewport(viewport),
                },
                RenderOperation {
                    order: DrawOrder(1),
                    picking_id: None,
                    command: MirCommand::SetViewProjection3D(ViewProjection3D::identity()),
                },
                RenderOperation {
                    order: DrawOrder(2),
                    picking_id: None,
                    command: MirCommand::SurfaceMesh3D(surface),
                },
                RenderOperation {
                    order: DrawOrder(3),
                    picking_id: None,
                    command: MirCommand::EndViewport,
                },
            ],
            OverlayPlan::default(),
        )
        .unwrap();
        DrawListCompiler::new(FrameCompileOptions::default())
            .compile(&frame)
            .unwrap()
    }

    #[test]
    fn format_selection_rejects_srgb_and_accepts_only_contract_formats() {
        assert_eq!(matlab_surface_color_space(), wgpu::SurfaceColorSpace::Srgb);
        assert_eq!(
            select_surface_format(&[
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureFormat::Rgba16Float,
            ]),
            None
        );
        assert_eq!(
            select_surface_format(&[
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Bgra8Unorm,
            ]),
            Some(wgpu::TextureFormat::Bgra8Unorm)
        );
    }

    #[test]
    fn scissor_is_clamped_to_surface_and_empty_regions_are_skipped() {
        let extent = SurfaceExtent {
            width: 100,
            height: 80,
        };
        assert_eq!(
            clamp_scissor(
                DeviceRect {
                    x: 90,
                    y: 70,
                    width: 20,
                    height: 20,
                },
                extent,
            ),
            Some(DeviceRect {
                x: 90,
                y: 70,
                width: 10,
                height: 10,
            })
        );
        assert_eq!(
            clamp_scissor(
                DeviceRect {
                    x: 101,
                    y: 0,
                    width: 10,
                    height: 10,
                },
                extent,
            ),
            None
        );
    }

    #[test]
    fn view_uniform_preserves_device_viewport_and_dpr() {
        let viewport = Viewport {
            css: CssRect::new(0.0, 0.0, 320.0, 200.0).unwrap(),
            device: DeviceRect {
                x: 10,
                y: 20,
                width: 640,
                height: 400,
            },
            device_pixel_ratio: DevicePixelRatio::new(2.0).unwrap(),
        };
        let mut view =
            ViewTransform2D::new(viewport, DataRect::new(0.0, 10.0, -5.0, 5.0).unwrap()).unwrap();
        view.zoom_css(openmat_plot_mir::CssPoint::new(160.0, 100.0).unwrap(), 0.5)
            .unwrap();
        let bytes = pack_view_uniform(
            800,
            600,
            viewport,
            view,
            None,
            None,
            Some(RulerSelection3D { x: 1, y: 2, z: 3 }),
        )
        .unwrap();
        let values: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        assert_eq!(
            &values[..12],
            &[
                800.0, 600.0, 10.0, 20.0, 640.0, 400.0, 2.0, 0.0, 2.0, 2.0, -0.5, -0.5
            ]
        );
        assert_eq!(
            &values[12..28],
            &ViewProjection3D::identity()
                .columns()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        );
        assert_eq!(&values[28..], &[1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    #[ignore = "requires a real native wgpu adapter; run explicitly for the GPU acceptance gate"]
    fn real_adapter_creates_device_and_wgsl_pipelines() {
        let request = RendererRequest::new();
        match block_on(request.initialize(None)) {
            Ok(renderer) => {
                eprintln!("OPENMAT_GPU_SMOKE=available {:?}", renderer.adapter_info());
                assert_eq!(renderer.state(), RendererState::Ready);
                let gpu_frame = renderer.upload(&gpu_smoke_frame()).unwrap();
                assert_eq!(gpu_frame.draw_count(), 1);
                let mut surface_frame = renderer.upload(&gpu_surface_smoke_frame()).unwrap();
                assert_eq!(surface_frame.draw_count(), 1);
                assert_eq!(
                    surface_frame.view_projection_3d(),
                    Some(ViewProjection3D::identity())
                );
                surface_frame.set_view_projection_3d(ViewProjection3D::identity());
            }
            Err(RendererInitError::AdapterUnavailable(error)) => {
                eprintln!("OPENMAT_GPU_SMOKE=unavailable adapter: {error}");
            }
            Err(RendererInitError::InsufficientLimits(violations)) => {
                eprintln!("OPENMAT_GPU_SMOKE=unavailable limits: {violations:?}");
            }
            Err(error) => panic!("real GPU pipeline creation failed: {error}"),
        }
    }
}
