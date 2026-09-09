pub const REQUIRED_MAX_BIND_GROUPS: u32 = 4;
pub const REQUIRED_MAX_VERTEX_BUFFERS: u32 = 4;
pub const REQUIRED_MAX_VERTEX_ATTRIBUTES: u32 = 8;
pub const REQUIRED_MAX_UNIFORM_BUFFER_BINDING_SIZE: u64 = 64 << 10;
pub const REQUIRED_MAX_STORAGE_BUFFER_BINDING_SIZE: u64 = 128 << 20;
pub const REQUIRED_MAX_BUFFER_SIZE: u64 = 256 << 20;
pub const REQUIRED_MAX_TEXTURE_DIMENSION_2D: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitName {
    BindGroups,
    VertexBuffers,
    VertexAttributes,
    UniformBufferBindingSize,
    StorageBufferBindingSize,
    BufferSize,
    TextureDimension2d,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitViolation {
    pub limit: LimitName,
    pub required: u64,
    pub available: u64,
}

#[must_use]
pub fn required_device_limits() -> wgpu::Limits {
    wgpu::Limits {
        max_bind_groups: REQUIRED_MAX_BIND_GROUPS,
        max_vertex_buffers: REQUIRED_MAX_VERTEX_BUFFERS,
        max_vertex_attributes: REQUIRED_MAX_VERTEX_ATTRIBUTES,
        max_uniform_buffer_binding_size: REQUIRED_MAX_UNIFORM_BUFFER_BINDING_SIZE,
        max_storage_buffer_binding_size: REQUIRED_MAX_STORAGE_BUFFER_BINDING_SIZE,
        max_buffer_size: REQUIRED_MAX_BUFFER_SIZE,
        max_texture_dimension_2d: REQUIRED_MAX_TEXTURE_DIMENSION_2D,
        ..wgpu::Limits::default()
    }
}

#[must_use]
pub fn check_required_limits(available: &wgpu::Limits) -> Vec<LimitViolation> {
    let checks = [
        (
            LimitName::BindGroups,
            u64::from(REQUIRED_MAX_BIND_GROUPS),
            u64::from(available.max_bind_groups),
        ),
        (
            LimitName::VertexBuffers,
            u64::from(REQUIRED_MAX_VERTEX_BUFFERS),
            u64::from(available.max_vertex_buffers),
        ),
        (
            LimitName::VertexAttributes,
            u64::from(REQUIRED_MAX_VERTEX_ATTRIBUTES),
            u64::from(available.max_vertex_attributes),
        ),
        (
            LimitName::UniformBufferBindingSize,
            REQUIRED_MAX_UNIFORM_BUFFER_BINDING_SIZE,
            available.max_uniform_buffer_binding_size,
        ),
        (
            LimitName::StorageBufferBindingSize,
            REQUIRED_MAX_STORAGE_BUFFER_BINDING_SIZE,
            available.max_storage_buffer_binding_size,
        ),
        (
            LimitName::BufferSize,
            REQUIRED_MAX_BUFFER_SIZE,
            available.max_buffer_size,
        ),
        (
            LimitName::TextureDimension2d,
            u64::from(REQUIRED_MAX_TEXTURE_DIMENSION_2D),
            u64::from(available.max_texture_dimension_2d),
        ),
    ];

    checks
        .into_iter()
        .filter_map(|(limit, required, actual)| {
            (actual < required).then_some(LimitViolation {
                limit,
                required,
                available: actual,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_each_insufficient_contract_limit() {
        let mut available = required_device_limits();
        available.max_bind_groups = REQUIRED_MAX_BIND_GROUPS - 1;
        available.max_vertex_buffers = REQUIRED_MAX_VERTEX_BUFFERS - 1;
        available.max_vertex_attributes = REQUIRED_MAX_VERTEX_ATTRIBUTES - 1;
        available.max_uniform_buffer_binding_size = REQUIRED_MAX_UNIFORM_BUFFER_BINDING_SIZE - 1;
        available.max_storage_buffer_binding_size = REQUIRED_MAX_STORAGE_BUFFER_BINDING_SIZE - 1;
        available.max_buffer_size = REQUIRED_MAX_BUFFER_SIZE - 1;
        available.max_texture_dimension_2d = REQUIRED_MAX_TEXTURE_DIMENSION_2D - 1;

        let failures = check_required_limits(&available);

        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.limit)
                .collect::<Vec<_>>(),
            vec![
                LimitName::BindGroups,
                LimitName::VertexBuffers,
                LimitName::VertexAttributes,
                LimitName::UniformBufferBindingSize,
                LimitName::StorageBufferBindingSize,
                LimitName::BufferSize,
                LimitName::TextureDimension2d,
            ]
        );
    }

    #[test]
    fn exact_required_profile_passes() {
        assert!(check_required_limits(&required_device_limits()).is_empty());
    }
}
