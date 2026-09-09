//! Platform-neutral Plot MIR rendering through the single WebGPU/wgpu path.

#![forbid(unsafe_code)]

mod compiler;
mod error;
mod interaction;
mod lifecycle;
mod limits;
mod picking;
mod pipeline;
mod renderer;

pub use compiler::{
    CompiledFrame, DrawCommand, DrawKind, DrawListCompiler, FrameCompileOptions, INDEX_STRIDE,
    LINE_SEGMENT_3D_STRIDE, MARKER_INSTANCE_STRIDE, MESH_VERTEX_STRIDE, SCREEN_LINE_STRIDE,
    SURFACE_VERTEX_3D_STRIDE,
};
pub use error::{
    BufferKind, CompileError, RenderError, RendererInitError, SurfaceConfigureError,
    SurfaceCreateError, UploadError,
};
pub use interaction::{DataPoint, DataRect, InteractionError, ViewTransform2D};
pub use lifecycle::{DeviceLoss, RendererLifecycle, RendererState, SurfaceExtent, SurfaceState};
pub use limits::{
    LimitName, LimitViolation, REQUIRED_MAX_BIND_GROUPS, REQUIRED_MAX_BUFFER_SIZE,
    REQUIRED_MAX_STORAGE_BUFFER_BINDING_SIZE, REQUIRED_MAX_TEXTURE_DIMENSION_2D,
    REQUIRED_MAX_UNIFORM_BUFFER_BINDING_SIZE, REQUIRED_MAX_VERTEX_ATTRIBUTES,
    REQUIRED_MAX_VERTEX_BUFFERS, check_required_limits, required_device_limits,
};
pub use picking::{PickHit, PickKind, PickTolerance, PickingIndex};
pub use pipeline::{
    PipelineDescriptorSummary, PipelineKind, line_segment_3d_buffer_layout,
    marker_instance_buffer_layout, mesh_vertex_buffer_layout, pipeline_descriptor_summary,
    screen_line_buffer_layout, surface_vertex_3d_buffer_layout,
};
pub use renderer::{
    GpuFrame, RenderOutcome, RenderSurface, Renderer, RendererRequest, ResizeOutcome,
    select_surface_format,
};
