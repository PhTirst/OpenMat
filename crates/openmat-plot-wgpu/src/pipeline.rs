use std::borrow::Cow;

use crate::{
    LINE_SEGMENT_3D_STRIDE, MARKER_INSTANCE_STRIDE, MESH_VERTEX_STRIDE, RendererInitError,
    SCREEN_LINE_STRIDE, SURFACE_VERTEX_3D_STRIDE,
};

const VIEW_UNIFORM_BYTES: u64 = 144;
pub(crate) const SAMPLE_COUNT: u32 = 4;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SURFACE_WITH_EDGES_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 1,
    slope_scale: 1.0,
    clamp: 0.0,
};
const SURFACE_EDGE_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -1,
    slope_scale: 0.0,
    clamp: 0.0,
};

const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 32,
        shader_location: 3,
    },
];

const MARKER_ATTRIBUTES: [wgpu::VertexAttribute; 7] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 16,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 24,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 40,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 56,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 60,
        shader_location: 6,
    },
];

const SURFACE_3D_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 24,
        shader_location: 2,
    },
];

const LINE_SEGMENT_3D_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 40,
        shader_location: 3,
    },
];

const SCREEN_LINE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 2,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineKind {
    Mesh,
    Surface3D,
    Surface3DSmooth,
    Surface3DWithEdges,
    Surface3DSmoothWithEdges,
    Line3D,
    SurfaceEdge3D,
    Ruler3D,
    Tick3D,
    ScreenLine,
    CircleMarker,
    SquareMarker,
    DiamondMarker,
    UpTriangleMarker,
    DownTriangleMarker,
    PlusMarker,
    CrossMarker,
    HorizontalLineMarker,
    VerticalLineMarker,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PipelineDescriptorSummary {
    pub kind: PipelineKind,
    pub topology: wgpu::PrimitiveTopology,
    pub front_face: wgpu::FrontFace,
    pub cull_mode: Option<wgpu::Face>,
    pub blend: wgpu::BlendState,
    pub sample_count: u32,
    pub vertex_stride: u64,
    pub vertex_step_mode: wgpu::VertexStepMode,
    pub vertex_attribute_count: u32,
    pub depth_write_enabled: bool,
    pub depth_compare: Option<wgpu::CompareFunction>,
    pub depth_bias: wgpu::DepthBiasState,
}

#[must_use]
pub fn pipeline_descriptor_summary(kind: PipelineKind) -> PipelineDescriptorSummary {
    let (vertex_stride, vertex_step_mode, vertex_attribute_count) = match kind {
        PipelineKind::Mesh => (MESH_VERTEX_STRIDE, wgpu::VertexStepMode::Vertex, 4),
        PipelineKind::Surface3D
        | PipelineKind::Surface3DSmooth
        | PipelineKind::Surface3DWithEdges
        | PipelineKind::Surface3DSmoothWithEdges => {
            (SURFACE_VERTEX_3D_STRIDE, wgpu::VertexStepMode::Vertex, 3)
        }
        PipelineKind::Line3D
        | PipelineKind::SurfaceEdge3D
        | PipelineKind::Ruler3D
        | PipelineKind::Tick3D => (LINE_SEGMENT_3D_STRIDE, wgpu::VertexStepMode::Instance, 4),
        PipelineKind::ScreenLine => (SCREEN_LINE_STRIDE, wgpu::VertexStepMode::Instance, 3),
        PipelineKind::CircleMarker
        | PipelineKind::SquareMarker
        | PipelineKind::DiamondMarker
        | PipelineKind::UpTriangleMarker
        | PipelineKind::DownTriangleMarker
        | PipelineKind::PlusMarker
        | PipelineKind::CrossMarker
        | PipelineKind::HorizontalLineMarker
        | PipelineKind::VerticalLineMarker => {
            (MARKER_INSTANCE_STRIDE, wgpu::VertexStepMode::Instance, 7)
        }
    };
    let depth_compare = match kind {
        PipelineKind::Surface3D
        | PipelineKind::Surface3DSmooth
        | PipelineKind::Surface3DWithEdges
        | PipelineKind::Surface3DSmoothWithEdges
        | PipelineKind::Line3D
        | PipelineKind::SurfaceEdge3D
        | PipelineKind::CircleMarker
        | PipelineKind::SquareMarker
        | PipelineKind::DiamondMarker
        | PipelineKind::UpTriangleMarker
        | PipelineKind::DownTriangleMarker
        | PipelineKind::PlusMarker
        | PipelineKind::CrossMarker
        | PipelineKind::HorizontalLineMarker
        | PipelineKind::VerticalLineMarker => Some(wgpu::CompareFunction::LessEqual),
        PipelineKind::Mesh
        | PipelineKind::Ruler3D
        | PipelineKind::Tick3D
        | PipelineKind::ScreenLine => Some(wgpu::CompareFunction::Always),
    };
    PipelineDescriptorSummary {
        kind,
        topology: wgpu::PrimitiveTopology::TriangleList,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: None,
        blend: if matches!(
            kind,
            PipelineKind::Surface3D
                | PipelineKind::Surface3DSmooth
                | PipelineKind::Surface3DWithEdges
                | PipelineKind::Surface3DSmoothWithEdges
        ) {
            wgpu::BlendState::REPLACE
        } else {
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        },
        sample_count: SAMPLE_COUNT,
        vertex_stride,
        vertex_step_mode,
        vertex_attribute_count,
        depth_write_enabled: matches!(
            kind,
            PipelineKind::Surface3D
                | PipelineKind::Surface3DSmooth
                | PipelineKind::Surface3DWithEdges
                | PipelineKind::Surface3DSmoothWithEdges
        ),
        depth_compare,
        depth_bias: match kind {
            PipelineKind::Surface3DWithEdges | PipelineKind::Surface3DSmoothWithEdges => {
                SURFACE_WITH_EDGES_DEPTH_BIAS
            }
            PipelineKind::SurfaceEdge3D => SURFACE_EDGE_DEPTH_BIAS,
            _ => wgpu::DepthBiasState::default(),
        },
    }
}

#[must_use]
pub const fn mesh_vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: MESH_VERTEX_STRIDE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &MESH_ATTRIBUTES,
    }
}

#[must_use]
pub const fn marker_instance_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: MARKER_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &MARKER_ATTRIBUTES,
    }
}

#[must_use]
pub const fn surface_vertex_3d_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: SURFACE_VERTEX_3D_STRIDE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &SURFACE_3D_ATTRIBUTES,
    }
}

#[must_use]
pub const fn line_segment_3d_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: LINE_SEGMENT_3D_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &LINE_SEGMENT_3D_ATTRIBUTES,
    }
}

#[must_use]
pub const fn screen_line_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: SCREEN_LINE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &SCREEN_LINE_ATTRIBUTES,
    }
}

pub(crate) struct PipelineSet {
    pub(crate) mesh: wgpu::RenderPipeline,
    pub(crate) surface_3d: wgpu::RenderPipeline,
    pub(crate) surface_3d_smooth: wgpu::RenderPipeline,
    pub(crate) surface_3d_with_edges: wgpu::RenderPipeline,
    pub(crate) surface_3d_smooth_with_edges: wgpu::RenderPipeline,
    pub(crate) line_3d: wgpu::RenderPipeline,
    pub(crate) surface_edge_3d: wgpu::RenderPipeline,
    pub(crate) ruler_3d: wgpu::RenderPipeline,
    pub(crate) tick_3d: wgpu::RenderPipeline,
    pub(crate) screen_line: wgpu::RenderPipeline,
    pub(crate) circle_marker: wgpu::RenderPipeline,
    pub(crate) square_marker: wgpu::RenderPipeline,
    pub(crate) diamond_marker: wgpu::RenderPipeline,
    pub(crate) up_triangle_marker: wgpu::RenderPipeline,
    pub(crate) down_triangle_marker: wgpu::RenderPipeline,
    pub(crate) plus_marker: wgpu::RenderPipeline,
    pub(crate) cross_marker: wgpu::RenderPipeline,
    pub(crate) horizontal_line_marker: wgpu::RenderPipeline,
    pub(crate) vertical_line_marker: wgpu::RenderPipeline,
}

pub(crate) struct PipelineResources {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    rgba8unorm: PipelineSet,
    bgra8unorm: PipelineSet,
}

impl PipelineResources {
    pub(crate) async fn create(device: &wgpu::Device) -> Result<Self, RendererInitError> {
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let resources = Self::create_unchecked(device);
        if let Some(error) = error_scope.pop().await {
            return Err(RendererInitError::PipelineCreation(error));
        }
        Ok(resources)
    }

    fn create_unchecked(device: &wgpu::Device) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("openmat plot view uniform"),
            size: VIEW_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("openmat plot view bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(VIEW_UNIFORM_BYTES),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("openmat plot view bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("openmat plot pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("openmat plot shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("plot.wgsl"))),
        });
        let rgba8unorm = create_pipeline_set(
            device,
            &pipeline_layout,
            &shader,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let bgra8unorm = create_pipeline_set(
            device,
            &pipeline_layout,
            &shader,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        Self {
            uniform_buffer,
            bind_group,
            rgba8unorm,
            bgra8unorm,
        }
    }

    pub(crate) fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub(crate) fn for_format(&self, format: wgpu::TextureFormat) -> Option<&PipelineSet> {
        match format {
            wgpu::TextureFormat::Rgba8Unorm => Some(&self.rgba8unorm),
            wgpu::TextureFormat::Bgra8Unorm => Some(&self.bgra8unorm),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_lines)]
fn create_pipeline_set(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> PipelineSet {
    PipelineSet {
        mesh: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Mesh,
            "fs_solid",
        ),
        surface_3d: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Surface3D,
            "fs_surface_3d",
        ),
        surface_3d_smooth: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Surface3DSmooth,
            "fs_surface_3d_smooth",
        ),
        surface_3d_with_edges: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Surface3DWithEdges,
            "fs_surface_3d",
        ),
        surface_3d_smooth_with_edges: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Surface3DSmoothWithEdges,
            "fs_surface_3d_smooth",
        ),
        line_3d: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Line3D,
            "fs_line_3d",
        ),
        surface_edge_3d: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::SurfaceEdge3D,
            "fs_line_3d",
        ),
        ruler_3d: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Ruler3D,
            "fs_line_3d",
        ),
        tick_3d: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::Tick3D,
            "fs_line_3d",
        ),
        screen_line: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::ScreenLine,
            "fs_line_3d",
        ),
        circle_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::CircleMarker,
            "fs_circle_marker",
        ),
        square_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::SquareMarker,
            "fs_square_marker",
        ),
        diamond_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::DiamondMarker,
            "fs_diamond_marker",
        ),
        up_triangle_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::UpTriangleMarker,
            "fs_up_triangle_marker",
        ),
        down_triangle_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::DownTriangleMarker,
            "fs_down_triangle_marker",
        ),
        plus_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::PlusMarker,
            "fs_plus_marker",
        ),
        cross_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::CrossMarker,
            "fs_cross_marker",
        ),
        horizontal_line_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::HorizontalLineMarker,
            "fs_horizontal_line_marker",
        ),
        vertical_line_marker: create_pipeline(
            device,
            layout,
            shader,
            format,
            PipelineKind::VerticalLineMarker,
            "fs_vertical_line_marker",
        ),
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    kind: PipelineKind,
    fragment_entry: &'static str,
) -> wgpu::RenderPipeline {
    let summary = pipeline_descriptor_summary(kind);
    let mesh_buffers = [Some(mesh_vertex_buffer_layout())];
    let marker_buffers = [Some(marker_instance_buffer_layout())];
    let surface_3d_buffers = [Some(surface_vertex_3d_buffer_layout())];
    let line_3d_buffers = [Some(line_segment_3d_buffer_layout())];
    let screen_line_buffers = [Some(screen_line_buffer_layout())];
    let buffers = match kind {
        PipelineKind::Mesh => &mesh_buffers[..],
        PipelineKind::Surface3D
        | PipelineKind::Surface3DSmooth
        | PipelineKind::Surface3DWithEdges
        | PipelineKind::Surface3DSmoothWithEdges => &surface_3d_buffers[..],
        PipelineKind::Line3D
        | PipelineKind::SurfaceEdge3D
        | PipelineKind::Ruler3D
        | PipelineKind::Tick3D => &line_3d_buffers[..],
        PipelineKind::ScreenLine => &screen_line_buffers[..],
        PipelineKind::CircleMarker
        | PipelineKind::SquareMarker
        | PipelineKind::DiamondMarker
        | PipelineKind::UpTriangleMarker
        | PipelineKind::DownTriangleMarker
        | PipelineKind::PlusMarker
        | PipelineKind::CrossMarker
        | PipelineKind::HorizontalLineMarker
        | PipelineKind::VerticalLineMarker => &marker_buffers[..],
    };
    let vertex_entry = match kind {
        PipelineKind::Mesh => "vs_mesh",
        PipelineKind::Surface3D | PipelineKind::Surface3DWithEdges => "vs_surface_3d",
        PipelineKind::Surface3DSmooth | PipelineKind::Surface3DSmoothWithEdges => {
            "vs_surface_3d_smooth"
        }
        PipelineKind::Line3D | PipelineKind::SurfaceEdge3D => "vs_line_3d",
        PipelineKind::Ruler3D => "vs_ruler_3d",
        PipelineKind::Tick3D => "vs_tick_3d",
        PipelineKind::ScreenLine => "vs_screen_line",
        PipelineKind::CircleMarker
        | PipelineKind::SquareMarker
        | PipelineKind::DiamondMarker
        | PipelineKind::UpTriangleMarker
        | PipelineKind::DownTriangleMarker
        | PipelineKind::PlusMarker
        | PipelineKind::CrossMarker
        | PipelineKind::HorizontalLineMarker
        | PipelineKind::VerticalLineMarker => "vs_marker",
    };
    let target = [Some(wgpu::ColorTargetState {
        format,
        blend: Some(summary.blend),
        write_mask: wgpu::ColorWrites::ALL,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(pipeline_label(kind)),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: summary.topology,
            strip_index_format: None,
            front_face: summary.front_face,
            cull_mode: summary.cull_mode,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: summary
            .depth_compare
            .map(|depth_compare| wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(summary.depth_write_enabled),
                depth_compare: Some(depth_compare),
                stencil: wgpu::StencilState::default(),
                bias: summary.depth_bias,
            }),
        multisample: wgpu::MultisampleState {
            count: summary.sample_count,
            mask: u64::MAX,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &target,
        }),
        multiview_mask: None,
        cache: None,
    })
}

const fn pipeline_label(kind: PipelineKind) -> &'static str {
    match kind {
        PipelineKind::Mesh => "openmat plot mesh pipeline",
        PipelineKind::Surface3D => "openmat plot 3D flat surface pipeline",
        PipelineKind::Surface3DSmooth => "openmat plot 3D smooth surface pipeline",
        PipelineKind::Surface3DWithEdges => "openmat plot biased 3D flat surface pipeline",
        PipelineKind::Surface3DSmoothWithEdges => "openmat plot biased 3D smooth surface pipeline",
        PipelineKind::Line3D => "openmat plot 3D line pipeline",
        PipelineKind::SurfaceEdge3D => "openmat plot 3D surface edge pipeline",
        PipelineKind::Ruler3D => "openmat plot 3D ruler pipeline",
        PipelineKind::Tick3D => "openmat plot 3D tick pipeline",
        PipelineKind::ScreenLine => "openmat plot screen-space line pipeline",
        PipelineKind::CircleMarker => "openmat plot circle marker pipeline",
        PipelineKind::SquareMarker => "openmat plot square marker pipeline",
        PipelineKind::DiamondMarker => "openmat plot diamond marker pipeline",
        PipelineKind::UpTriangleMarker => "openmat plot up-triangle marker pipeline",
        PipelineKind::DownTriangleMarker => "openmat plot down-triangle marker pipeline",
        PipelineKind::PlusMarker => "openmat plot plus marker pipeline",
        PipelineKind::CrossMarker => "openmat plot cross marker pipeline",
        PipelineKind::HorizontalLineMarker => "openmat plot horizontal-line marker pipeline",
        PipelineKind::VerticalLineMarker => "openmat plot vertical-line marker pipeline",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_descriptor_is_multisampled_premultiplied_alpha_triangles() {
        let descriptor = pipeline_descriptor_summary(PipelineKind::Mesh);
        assert_eq!(descriptor.topology, wgpu::PrimitiveTopology::TriangleList);
        assert_eq!(descriptor.cull_mode, None);
        assert_eq!(
            descriptor.blend,
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
        );
        assert_eq!(descriptor.sample_count, SAMPLE_COUNT);
        assert_eq!(
            descriptor.depth_compare,
            Some(wgpu::CompareFunction::Always)
        );
        assert_eq!(descriptor.vertex_stride, 40);
        assert_eq!(descriptor.vertex_attribute_count, 4);
        assert_eq!(
            mesh_vertex_buffer_layout().step_mode,
            wgpu::VertexStepMode::Vertex
        );
    }

    #[test]
    fn marker_descriptor_supports_2d_and_depth_tested_3d_instances() {
        for kind in [
            PipelineKind::CircleMarker,
            PipelineKind::SquareMarker,
            PipelineKind::DiamondMarker,
            PipelineKind::UpTriangleMarker,
            PipelineKind::DownTriangleMarker,
            PipelineKind::PlusMarker,
            PipelineKind::CrossMarker,
            PipelineKind::HorizontalLineMarker,
            PipelineKind::VerticalLineMarker,
        ] {
            let descriptor = pipeline_descriptor_summary(kind);
            assert_eq!(descriptor.vertex_stride, 64);
            assert_eq!(descriptor.vertex_attribute_count, 7);
            assert_eq!(descriptor.vertex_step_mode, wgpu::VertexStepMode::Instance);
            assert_eq!(
                descriptor.depth_compare,
                Some(wgpu::CompareFunction::LessEqual)
            );
        }
        assert_eq!(
            marker_instance_buffer_layout().attributes,
            &MARKER_ATTRIBUTES
        );
    }

    #[test]
    fn surface_descriptor_is_depth_tested_triangle_geometry() {
        let descriptor = pipeline_descriptor_summary(PipelineKind::Surface3D);
        assert_eq!(descriptor.vertex_stride, SURFACE_VERTEX_3D_STRIDE);
        assert_eq!(descriptor.vertex_attribute_count, 3);
        assert!(descriptor.depth_write_enabled);
        assert_eq!(
            descriptor.depth_compare,
            Some(wgpu::CompareFunction::LessEqual)
        );
        assert_eq!(descriptor.blend, wgpu::BlendState::REPLACE);
        assert_eq!(descriptor.depth_bias, wgpu::DepthBiasState::default());
        assert_eq!(
            surface_vertex_3d_buffer_layout().attributes,
            &SURFACE_3D_ATTRIBUTES
        );
    }

    #[test]
    fn surface_wireframe_biases_fill_and_edges_without_disabling_occlusion() {
        for kind in [
            PipelineKind::Surface3DWithEdges,
            PipelineKind::Surface3DSmoothWithEdges,
        ] {
            let descriptor = pipeline_descriptor_summary(kind);
            assert_eq!(
                descriptor.depth_compare,
                Some(wgpu::CompareFunction::LessEqual)
            );
            assert!(descriptor.depth_write_enabled);
            assert_eq!(descriptor.depth_bias, SURFACE_WITH_EDGES_DEPTH_BIAS);
            assert!(descriptor.depth_bias.constant > 0);
            assert!(descriptor.depth_bias.slope_scale > 0.0);
        }

        let edge = pipeline_descriptor_summary(PipelineKind::SurfaceEdge3D);
        assert_eq!(edge.depth_compare, Some(wgpu::CompareFunction::LessEqual));
        assert!(!edge.depth_write_enabled);
        assert_eq!(edge.depth_bias, SURFACE_EDGE_DEPTH_BIAS);
        assert!(edge.depth_bias.constant < 0);
        assert_eq!(edge.depth_bias.slope_scale.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn line_3d_descriptor_uses_depth_tested_instanced_triangle_strokes() {
        let descriptor = pipeline_descriptor_summary(PipelineKind::Line3D);
        assert_eq!(descriptor.topology, wgpu::PrimitiveTopology::TriangleList);
        assert_eq!(descriptor.vertex_stride, LINE_SEGMENT_3D_STRIDE);
        assert_eq!(descriptor.vertex_attribute_count, 4);
        assert_eq!(descriptor.vertex_step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(
            descriptor.depth_compare,
            Some(wgpu::CompareFunction::LessEqual)
        );
        assert!(!descriptor.depth_write_enabled);
        assert_eq!(
            line_segment_3d_buffer_layout().attributes,
            &LINE_SEGMENT_3D_ATTRIBUTES
        );
    }

    #[test]
    fn overlay_line_descriptors_are_instanced_and_do_not_use_depth() {
        let tick = pipeline_descriptor_summary(PipelineKind::Tick3D);
        assert_eq!(tick.vertex_stride, LINE_SEGMENT_3D_STRIDE);
        assert_eq!(tick.vertex_step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(tick.depth_compare, Some(wgpu::CompareFunction::Always));

        let screen = pipeline_descriptor_summary(PipelineKind::ScreenLine);
        assert_eq!(screen.vertex_stride, SCREEN_LINE_STRIDE);
        assert_eq!(screen.vertex_attribute_count, 3);
        assert_eq!(screen.vertex_step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(screen.depth_compare, Some(wgpu::CompareFunction::Always));
        assert_eq!(
            screen_line_buffer_layout().attributes,
            &SCREEN_LINE_ATTRIBUTES
        );
    }

    #[test]
    fn shader_adds_high_and_low_coordinates_and_preserves_alpha() {
        let shader = include_str!("plot.wgsl");
        assert!(shader.contains("input.position_hi + input.position_lo"));
        assert!(shader.contains("input.center_hi + input.center_lo"));
        assert!(shader.contains("fill: vec4<f32>"));
        assert!(shader.contains("stroke: vec4<f32>"));
        assert!(shader.contains("fwidth(input.edge_distance_css_px)"));
        assert!(shader.contains("premultiply_with_coverage"));
        assert!(shader.contains("view_projection_3d"));
        assert!(shader.contains("vs_surface_3d"));
        assert!(shader.contains("fs_surface_3d"));
        assert!(shader.contains("vs_line_3d"));
        assert!(shader.contains("vs_tick_3d"));
        assert!(shader.contains("vs_screen_line"));
        assert!(shader.contains("fs_line_3d"));
        assert!(shader.contains("line_pattern_coverage"));
        assert!(shader.contains("@interpolate(linear) edge_distance_and_half_width_device_px"));
        assert!(shader.contains("@interpolate(linear) along_css_px"));
        assert!(shader.contains("output.color = input.color"));
        assert!(shader.contains("@interpolate(flat, first) color"));
        assert!(shader.contains("matlab_headlight_color"));
        assert!(shader.contains("SpecularColorReflectance=1"));
        assert!(shader.contains("view.lighting_3d.xyz - axes_position"));
        assert!(shader.contains("view.lighting_3d.w"));
        for entry in [
            "fs_circle_marker",
            "fs_square_marker",
            "fs_diamond_marker",
            "fs_up_triangle_marker",
            "fs_down_triangle_marker",
            "fs_plus_marker",
            "fs_cross_marker",
            "fs_horizontal_line_marker",
            "fs_vertical_line_marker",
        ] {
            assert!(shader.contains(entry));
        }
    }
}
