use std::error::Error;
use std::fmt::{self, Display, Formatter};

use openmat_plot_mir::{MarkerShape, MirError};

use crate::{DeviceLoss, LimitViolation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    MeshVertex,
    MeshIndex,
    MarkerInstance,
    ScreenLine,
    LineSegment3D,
    SurfaceVertex3D,
    SurfaceIndex3D,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CompileError {
    InvalidMir(MirError),
    UnsupportedMirCommand,
    UnsupportedMarkerShape(MarkerShape),
    BufferLimitExceeded {
        kind: BufferKind,
        requested_bytes: u64,
        maximum_bytes: u64,
    },
    ArithmeticOverflow(BufferKind),
    TooManyVertices,
    TooManyMarkerInstances,
    TooManyLineSegments,
}

impl Display for CompileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMir(error) => write!(formatter, "invalid Plot MIR: {error}"),
            Self::UnsupportedMirCommand => {
                formatter.write_str("MIR command is newer than this renderer")
            }
            Self::UnsupportedMarkerShape(shape) => {
                write!(
                    formatter,
                    "marker shape {shape:?} is not supported by this renderer"
                )
            }
            Self::BufferLimitExceeded {
                kind,
                requested_bytes,
                maximum_bytes,
            } => {
                let (requested_mib, requested_hundredths) = mib_parts(*requested_bytes);
                let (maximum_mib, maximum_hundredths) = mib_parts(*maximum_bytes);
                write!(
                    formatter,
                    "{kind:?} buffer requires {requested_mib}.{requested_hundredths:02} MiB ({requested_bytes} bytes), exceeding the configured {maximum_mib}.{maximum_hundredths:02} MiB ({maximum_bytes} bytes) limit"
                )
            }
            Self::ArithmeticOverflow(kind) => {
                write!(formatter, "{kind:?} buffer size arithmetic overflowed")
            }
            Self::TooManyVertices => formatter.write_str("mesh vertex count exceeds u32 range"),
            Self::TooManyMarkerInstances => {
                formatter.write_str("marker instance count exceeds u32 range")
            }
            Self::TooManyLineSegments => {
                formatter.write_str("instanced line count exceeds u32 range")
            }
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidMir(error) => Some(error),
            _ => None,
        }
    }
}

impl From<MirError> for CompileError {
    fn from(error: MirError) -> Self {
        Self::InvalidMir(error)
    }
}

#[derive(Debug)]
pub enum RendererInitError {
    AdapterUnavailable(wgpu::RequestAdapterError),
    InsufficientLimits(Vec<LimitViolation>),
    DeviceCreation(wgpu::RequestDeviceError),
    PipelineCreation(wgpu::Error),
    DeviceLost(DeviceLoss),
}

impl Display for RendererInitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdapterUnavailable(_) => {
                formatter.write_str("no WebGPU/wgpu adapter is available")
            }
            Self::InsufficientLimits(violations) => write!(
                formatter,
                "adapter misses {} required renderer limit(s)",
                violations.len()
            ),
            Self::DeviceCreation(error) => {
                write!(formatter, "wgpu device creation failed: {error}")
            }
            Self::PipelineCreation(error) => {
                write!(formatter, "wgpu render pipeline creation failed: {error}")
            }
            Self::DeviceLost(loss) => {
                write!(
                    formatter,
                    "wgpu device was lost during initialization: {loss}"
                )
            }
        }
    }
}

impl Error for RendererInitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AdapterUnavailable(error) => Some(error),
            Self::DeviceCreation(error) => Some(error),
            Self::PipelineCreation(error) => Some(error),
            Self::InsufficientLimits(_) | Self::DeviceLost(_) => None,
        }
    }
}

#[derive(Debug)]
pub enum SurfaceCreateError {
    Creation(wgpu::CreateSurfaceError),
}

impl Display for SurfaceCreateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Creation(error) => write!(formatter, "wgpu surface creation failed: {error}"),
        }
    }
}

impl Error for SurfaceCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Creation(error) => Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceConfigureError {
    DeviceLost(DeviceLoss),
    DimensionsExceedLimit {
        width: u32,
        height: u32,
        maximum_dimension: u32,
    },
    NoCompatibleFormat {
        available: Vec<wgpu::TextureFormat>,
    },
    NoPresentMode,
    NoAlphaMode,
    SurfaceMustBeRecreated,
}

impl Display for SurfaceConfigureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost(loss) => write!(formatter, "renderer device was lost: {loss}"),
            Self::DimensionsExceedLimit {
                width,
                height,
                maximum_dimension,
            } => write!(
                formatter,
                "surface dimensions {width}x{height} exceed adapter limit {maximum_dimension}"
            ),
            Self::NoCompatibleFormat { available } => write!(
                formatter,
                "surface has no rgba8unorm or bgra8unorm format (available: {available:?})"
            ),
            Self::NoPresentMode => formatter.write_str("surface reports no presentation mode"),
            Self::NoAlphaMode => formatter.write_str("surface reports no alpha mode"),
            Self::SurfaceMustBeRecreated => {
                formatter.write_str("lost surface must be recreated before configuration")
            }
        }
    }
}

impl Error for SurfaceConfigureError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadError {
    DeviceLost(DeviceLoss),
    MissingBuffer(BufferKind),
    InvalidViewport,
    BufferExceedsDeviceLimit {
        kind: BufferKind,
        size_bytes: u64,
        maximum_bytes: u64,
    },
}

impl Display for UploadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost(loss) => write!(formatter, "renderer device was lost: {loss}"),
            Self::MissingBuffer(kind) => {
                write!(
                    formatter,
                    "compiled frame is missing its {kind:?} GPU buffer"
                )
            }
            Self::InvalidViewport => formatter.write_str("compiled frame viewport is invalid"),
            Self::BufferExceedsDeviceLimit {
                kind,
                size_bytes,
                maximum_bytes,
            } => {
                let (size_mib, size_hundredths) = mib_parts(*size_bytes);
                let (maximum_mib, maximum_hundredths) = mib_parts(*maximum_bytes);
                write!(
                    formatter,
                    "{kind:?} buffer uses {size_mib}.{size_hundredths:02} MiB ({size_bytes} bytes), exceeding the device {maximum_mib}.{maximum_hundredths:02} MiB ({maximum_bytes} bytes) limit"
                )
            }
        }
    }
}

impl Error for UploadError {}

fn mib_parts(bytes: u64) -> (u64, u64) {
    const BYTES_PER_MIB: u64 = 1 << 20;
    let mut whole = bytes / BYTES_PER_MIB;
    let mut hundredths = ((bytes % BYTES_PER_MIB) * 100 + BYTES_PER_MIB / 2) / BYTES_PER_MIB;
    if hundredths == 100 {
        whole += 1;
        hundredths = 0;
    }
    (whole, hundredths)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    DeviceLost(DeviceLoss),
    SurfaceNotConfigured,
    SurfaceSuspended,
    SurfaceLost,
    SurfaceOutdated,
    SurfaceTimeout,
    SurfaceOccluded,
    SurfaceValidation,
    UnsupportedSurfaceFormat(wgpu::TextureFormat),
}

impl Display for RenderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceLost(loss) => write!(formatter, "renderer device was lost: {loss}"),
            Self::SurfaceNotConfigured => formatter.write_str("surface is not configured"),
            Self::SurfaceSuspended => {
                formatter.write_str("surface has zero extent and is suspended")
            }
            Self::SurfaceLost => formatter.write_str("surface was lost and must be recreated"),
            Self::SurfaceOutdated => {
                formatter.write_str("surface is outdated and must be reconfigured")
            }
            Self::SurfaceTimeout => formatter.write_str("surface texture acquisition timed out"),
            Self::SurfaceOccluded => formatter.write_str("surface is occluded"),
            Self::SurfaceValidation => {
                formatter.write_str("surface texture acquisition failed validation")
            }
            Self::UnsupportedSurfaceFormat(format) => {
                write!(
                    formatter,
                    "surface format {format:?} has no renderer pipeline"
                )
            }
        }
    }
}

impl Error for RenderError {}
