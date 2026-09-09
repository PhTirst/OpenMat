struct ViewUniform {
    target_and_origin: vec4<f32>,
    viewport_and_dpr: vec4<f32>,
    source_to_view: vec4<f32>,
    view_projection_3d: mat4x4<f32>,
    ruler_selection_3d: vec4<f32>,
    lighting_3d: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> view: ViewUniform;

fn axes_to_pixel(position: vec2<f32>) -> vec2<f32> {
    let viewport_origin = view.target_and_origin.zw;
    let viewport_size = view.viewport_and_dpr.xy;
    let transformed = position * view.source_to_view.xy + view.source_to_view.zw;
    return viewport_origin + vec2<f32>(transformed.x, 1.0 - transformed.y) * viewport_size;
}

fn pixel_to_clip(position: vec2<f32>) -> vec4<f32> {
    let target_size = view.target_and_origin.xy;
    let normalized = position / target_size;
    return vec4<f32>(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0, 0.0, 1.0);
}

fn camera_clip_to_target_clip(camera_clip: vec4<f32>) -> vec4<f32> {
    let viewport_origin = view.target_and_origin.zw;
    let viewport_size = view.viewport_and_dpr.xy;
    let camera_ndc = camera_clip.xy / camera_clip.w;
    let axes_position = camera_ndc * 0.5 + vec2<f32>(0.5, 0.5);
    let pixel = viewport_origin
        + vec2<f32>(axes_position.x, 1.0 - axes_position.y) * viewport_size;
    let target_clip = pixel_to_clip(pixel);
    return vec4<f32>(
        target_clip.xy * camera_clip.w,
        camera_clip.z,
        camera_clip.w,
    );
}

struct MeshInput {
    @location(0) position_hi: vec2<f32>,
    @location(1) position_lo: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) edge_distance_and_half_width: vec2<f32>,
}

struct MeshOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) edge_distance_css_px: f32,
    @location(2) half_width_css_px: f32,
}

fn premultiply_with_coverage(color: vec4<f32>, coverage: f32) -> vec4<f32> {
    let alpha = color.a * clamp(coverage, 0.0, 1.0);
    return vec4<f32>(color.rgb * alpha, alpha);
}

@vertex
fn vs_mesh(input: MeshInput) -> MeshOutput {
    var output: MeshOutput;
    let axes_position = input.position_hi + input.position_lo;
    output.position = pixel_to_clip(axes_to_pixel(axes_position));
    output.color = input.color;
    output.edge_distance_css_px = input.edge_distance_and_half_width.x;
    output.half_width_css_px = input.edge_distance_and_half_width.y;
    return output;
}

@fragment
fn fs_solid(input: MeshOutput) -> @location(0) vec4<f32> {
    let pixel_width_css_px = max(fwidth(input.edge_distance_css_px), 0.0001);
    let half_transition = pixel_width_css_px * 0.5;
    let coverage = 1.0 - smoothstep(
        input.half_width_css_px - half_transition,
        input.half_width_css_px + half_transition,
        abs(input.edge_distance_css_px),
    );
    return premultiply_with_coverage(input.color, coverage);
}

struct Surface3DInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
}

struct Surface3DOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat, first) color: vec4<f32>,
}

fn matlab_headlight_color(
    base_color: vec3<f32>,
    axes_position: vec3<f32>,
    normal: vec3<f32>,
) -> vec3<f32> {
    if view.lighting_3d.w <= 0.0 {
        return base_color;
    }
    let normal_length = length(normal);
    let unit_normal = select(vec3<f32>(0.0, 0.0, 1.0), normal / max(normal_length, 0.000001), normal_length > 0.000001);
    let light_delta = view.lighting_3d.xyz - axes_position;
    let light_length = length(light_delta);
    let light_direction = select(vec3<f32>(0.0, 0.0, 1.0), light_delta / max(light_length, 0.000001), light_length > 0.000001);
    let facing_normal = select(-unit_normal, unit_normal, dot(unit_normal, light_direction) >= 0.0);
    let diffuse_cosine = max(dot(facing_normal, light_direction), 0.0);
    let reflected = reflect(-light_direction, facing_normal);
    let specular_cosine = max(dot(reflected, light_direction), 0.0);
    // MATLAB R2022b Patch defaults: AmbientStrength=.3,
    // DiffuseStrength=.6, SpecularStrength=.9, SpecularExponent=10, and
    // SpecularColorReflectance=1. camlight headlight is a local white light.
    let diffuse = base_color * (0.3 + 0.6 * diffuse_cosine);
    let specular = vec3<f32>(0.9 * pow(specular_cosine, 10.0));
    return clamp(diffuse + specular, vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs_surface_3d(input: Surface3DInput) -> Surface3DOutput {
    var output: Surface3DOutput;
    let camera_clip = view.view_projection_3d * vec4<f32>(input.position, 1.0);
    output.position = camera_clip_to_target_clip(camera_clip);
    output.color = vec4<f32>(
        matlab_headlight_color(input.color.rgb, input.position, input.normal),
        input.color.a,
    );
    return output;
}

@fragment
fn fs_surface_3d(input: Surface3DOutput) -> @location(0) vec4<f32> {
    return input.color;
}

struct Surface3DSmoothOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_surface_3d_smooth(input: Surface3DInput) -> Surface3DSmoothOutput {
    var output: Surface3DSmoothOutput;
    let camera_clip = view.view_projection_3d * vec4<f32>(input.position, 1.0);
    output.position = camera_clip_to_target_clip(camera_clip);
    output.color = vec4<f32>(
        matlab_headlight_color(input.color.rgb, input.position, input.normal),
        input.color.a,
    );
    return output;
}

@fragment
fn fs_surface_3d_smooth(input: Surface3DSmoothOutput) -> @location(0) vec4<f32> {
    return input.color;
}

struct Line3DInput {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) style: vec4<f32>,
}

struct Line3DOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(linear) edge_distance_and_half_width_device_px: vec2<f32>,
    @location(2) @interpolate(linear) along_css_px: f32,
    @location(3) @interpolate(flat, first) width_and_pattern: vec2<f32>,
}

fn target_clip_to_pixel(target_clip: vec4<f32>) -> vec2<f32> {
    let ndc = target_clip.xy / target_clip.w;
    return vec2<f32>(
        (ndc.x * 0.5 + 0.5) * view.target_and_origin.x,
        (0.5 - ndc.y * 0.5) * view.target_and_origin.y,
    );
}

fn build_line_output(
    start_clip: vec4<f32>,
    end_clip: vec4<f32>,
    color: vec4<f32>,
    width_css_px: f32,
    pattern: f32,
    vertex_index: u32,
) -> Line3DOutput {
    var endpoint_factors = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    var sides = array<f32, 6>(1.0, -1.0, -1.0, 1.0, -1.0, 1.0);
    let start_pixel = target_clip_to_pixel(start_clip);
    let end_pixel = target_clip_to_pixel(end_clip);
    let delta = end_pixel - start_pixel;
    let segment_length_device_px = length(delta);
    let direction = select(
        vec2<f32>(1.0, 0.0),
        delta / max(segment_length_device_px, 0.0001),
        segment_length_device_px > 0.0001,
    );
    let perpendicular = vec2<f32>(-direction.y, direction.x);
    let dpr = view.viewport_and_dpr.z;
    let requested_width_device_px = width_css_px * dpr;
    let half_width_device_px = max(requested_width_device_px, 1.0) * 0.5;
    let outer_half_width_device_px = half_width_device_px + 1.0;
    let endpoint_factor = endpoint_factors[vertex_index];
    let side = sides[vertex_index];
    var clip = select(start_clip, end_clip, endpoint_factor > 0.5);
    let offset_pixel = perpendicular * side * outer_half_width_device_px;
    clip.x += offset_pixel.x * 2.0 / view.target_and_origin.x * clip.w;
    clip.y -= offset_pixel.y * 2.0 / view.target_and_origin.y * clip.w;

    var output: Line3DOutput;
    output.position = clip;
    output.color = color;
    output.edge_distance_and_half_width_device_px = vec2<f32>(
        side * outer_half_width_device_px,
        half_width_device_px,
    );
    output.along_css_px = endpoint_factor * segment_length_device_px / dpr;
    output.width_and_pattern = vec2<f32>(width_css_px, pattern);
    return output;
}

fn hidden_line_output() -> Line3DOutput {
    var output: Line3DOutput;
    output.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    output.color = vec4<f32>(0.0);
    output.edge_distance_and_half_width_device_px = vec2<f32>(0.0);
    output.along_css_px = 0.0;
    output.width_and_pattern = vec2<f32>(0.0);
    return output;
}

fn ruler_dimension(selector: f32) -> u32 {
    return u32(round(selector)) / 4u;
}

fn ruler_candidate(selector: f32) -> u32 {
    return u32(round(selector)) % 4u;
}

fn ruler_visible(selector: f32) -> bool {
    let dimension = ruler_dimension(selector);
    let candidate = f32(ruler_candidate(selector));
    return abs(view.ruler_selection_3d[dimension] - candidate) < 0.5;
}

fn ruler_start(selector: f32) -> vec3<f32> {
    let dimension = ruler_dimension(selector);
    let candidate = ruler_candidate(selector);
    let low = f32(candidate & 1u);
    let high = f32((candidate >> 1u) & 1u);
    if dimension == 0u {
        return vec3<f32>(0.0, low, high);
    }
    if dimension == 1u {
        return vec3<f32>(low, 0.0, high);
    }
    return vec3<f32>(low, high, 0.0);
}

fn ruler_end(selector: f32) -> vec3<f32> {
    let dimension = ruler_dimension(selector);
    let candidate = ruler_candidate(selector);
    let low = f32(candidate & 1u);
    let high = f32((candidate >> 1u) & 1u);
    if dimension == 0u {
        return vec3<f32>(1.0, low, high);
    }
    if dimension == 1u {
        return vec3<f32>(low, 1.0, high);
    }
    return vec3<f32>(low, high, 1.0);
}

fn ruler_outward(selector: f32) -> vec2<f32> {
    let start_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(ruler_start(selector), 1.0),
    );
    let end_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(ruler_end(selector), 1.0),
    );
    let center_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(0.5, 0.5, 0.5, 1.0),
    );
    let start_pixel = target_clip_to_pixel(start_clip);
    let end_pixel = target_clip_to_pixel(end_clip);
    let center_pixel = target_clip_to_pixel(center_clip);
    let tangent_delta = end_pixel - start_pixel;
    let tangent_length = length(tangent_delta);
    let tangent = select(
        vec2<f32>(1.0, 0.0),
        tangent_delta / max(tangent_length, 0.0001),
        tangent_length > 0.0001,
    );
    let normal = vec2<f32>(-tangent.y, tangent.x);
    let midpoint = (start_pixel + end_pixel) * 0.5;
    return select(-normal, normal, dot(normal, midpoint - center_pixel) >= 0.0);
}

@vertex
fn vs_line_3d(input: Line3DInput, @builtin(vertex_index) vertex_index: u32) -> Line3DOutput {
    let start_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(input.start, 1.0),
    );
    let end_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(input.end, 1.0),
    );
    return build_line_output(
        start_clip,
        end_clip,
        input.color,
        input.style.x,
        input.style.y,
        vertex_index,
    );
}

@vertex
fn vs_ruler_3d(input: Line3DInput, @builtin(vertex_index) vertex_index: u32) -> Line3DOutput {
    if !ruler_visible(input.style.z) {
        return hidden_line_output();
    }
    let start_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(input.start, 1.0),
    );
    let end_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(input.end, 1.0),
    );
    return build_line_output(
        start_clip,
        end_clip,
        input.color,
        input.style.x,
        0.0,
        vertex_index,
    );
}

@vertex
fn vs_tick_3d(input: Line3DInput, @builtin(vertex_index) vertex_index: u32) -> Line3DOutput {
    if !ruler_visible(input.style.z) {
        return hidden_line_output();
    }
    let anchor_clip = camera_clip_to_target_clip(
        view.view_projection_3d * vec4<f32>(input.start, 1.0),
    );
    let outward = ruler_outward(input.style.z);
    let offset_pixel = outward * input.style.y * view.viewport_and_dpr.z;
    var end_clip = anchor_clip;
    end_clip.x += offset_pixel.x * 2.0 / view.target_and_origin.x * anchor_clip.w;
    end_clip.y -= offset_pixel.y * 2.0 / view.target_and_origin.y * anchor_clip.w;
    return build_line_output(
        anchor_clip,
        end_clip,
        input.color,
        input.style.x,
        0.0,
        vertex_index,
    );
}

struct ScreenLineInput {
    @location(0) start_and_end: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) style: vec4<f32>,
}

@vertex
fn vs_screen_line(
    input: ScreenLineInput,
    @builtin(vertex_index) vertex_index: u32,
) -> Line3DOutput {
    let dpr = view.viewport_and_dpr.z;
    let start_clip = pixel_to_clip(input.start_and_end.xy * dpr);
    let end_clip = pixel_to_clip(input.start_and_end.zw * dpr);
    return build_line_output(
        start_clip,
        end_clip,
        input.color,
        input.style.x,
        input.style.y,
        vertex_index,
    );
}

fn interval_coverage(value: f32, start: f32, end: f32, aa: f32) -> f32 {
    return smoothstep(start - aa, start + aa, value)
        * (1.0 - smoothstep(end - aa, end + aa, value));
}

fn line_pattern_coverage(along_css_px: f32, width_css_px: f32, pattern: f32) -> f32 {
    let scale = max(width_css_px, 1.0);
    let aa = max(fwidth(along_css_px) * 0.5, 0.01);
    if pattern < 0.5 {
        return 1.0;
    }
    if pattern < 1.5 {
        let cycle = 9.0 * scale;
        let phase = fract(along_css_px / cycle) * cycle;
        return interval_coverage(phase, 0.0, 6.0 * scale, aa);
    }
    if pattern < 2.5 {
        let cycle = 3.5 * scale;
        let phase = fract(along_css_px / cycle) * cycle;
        return interval_coverage(phase, 0.0, scale, aa);
    }
    let cycle = 13.0 * scale;
    let phase = fract(along_css_px / cycle) * cycle;
    return max(
        interval_coverage(phase, 0.0, 6.0 * scale, aa),
        interval_coverage(phase, 9.0 * scale, 10.0 * scale, aa),
    );
}

@fragment
fn fs_line_3d(input: Line3DOutput) -> @location(0) vec4<f32> {
    let edge_distance = abs(input.edge_distance_and_half_width_device_px.x);
    let half_width = input.edge_distance_and_half_width_device_px.y;
    let pixel_width = max(fwidth(edge_distance), 0.0001);
    let edge_coverage = 1.0 - smoothstep(
        half_width - pixel_width * 0.5,
        half_width + pixel_width * 0.5,
        edge_distance,
    );
    let dash_coverage = line_pattern_coverage(
        input.along_css_px,
        input.width_and_pattern.x,
        input.width_and_pattern.y,
    );
    return premultiply_with_coverage(input.color, edge_coverage * dash_coverage);
}

struct MarkerInput {
    @location(0) center_hi: vec2<f32>,
    @location(1) center_lo: vec2<f32>,
    @location(2) size_and_rotation: vec2<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) stroke: vec4<f32>,
    @location(5) stroke_width_css_px: f32,
    @location(6) dimension_mode: f32,
}

struct MarkerOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) stroke: vec4<f32>,
    @location(3) border_fraction: f32,
}

@vertex
fn vs_marker(input: MarkerInput, @builtin(vertex_index) vertex_index: u32) -> MarkerOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let local = corners[vertex_index];
    let sine = sin(input.size_and_rotation.y);
    let cosine = cos(input.size_and_rotation.y);
    let rotated = vec2<f32>(
        local.x * cosine - local.y * sine,
        local.x * sine + local.y * cosine,
    );
    let size_device_px = input.size_and_rotation.x * view.viewport_and_dpr.z;
    let half_size_device_px = max(size_device_px * 0.5, 0.0001);
    let outer_half_size_device_px = half_size_device_px + 1.0;
    var output: MarkerOutput;
    if input.dimension_mode > 0.5 {
        let camera_clip = view.view_projection_3d
            * vec4<f32>(input.center_hi, input.center_lo.x, 1.0);
        let center_clip = camera_clip_to_target_clip(camera_clip);
        let offset_ndc = vec2<f32>(
            rotated.x * outer_half_size_device_px * 2.0 / view.target_and_origin.x,
            -rotated.y * outer_half_size_device_px * 2.0 / view.target_and_origin.y,
        );
        output.position = center_clip
            + vec4<f32>(offset_ndc * center_clip.w, 0.0, 0.0);
    } else {
        let center_axes = input.center_hi + input.center_lo;
        let pixel = axes_to_pixel(center_axes) + rotated * outer_half_size_device_px;
        output.position = pixel_to_clip(pixel);
    }
    output.local_position = local * (outer_half_size_device_px / half_size_device_px);
    output.fill = input.fill;
    output.stroke = input.stroke;
    let requested_stroke_width_device_px =
        input.stroke_width_css_px * view.viewport_and_dpr.z;
    let stroke_width_device_px = select(
        0.0,
        max(requested_stroke_width_device_px, 1.0),
        requested_stroke_width_device_px > 0.0,
    );
    output.border_fraction = clamp(
        stroke_width_device_px / half_size_device_px,
        0.0,
        1.0,
    );
    return output;
}

fn marker_color(metric: f32, input: MarkerOutput) -> vec4<f32> {
    let fill_limit = 1.0 - input.border_fraction;
    let pixel_width = max(fwidth(metric), 0.0001);
    let half_transition = pixel_width * 0.5;
    let fill_coverage = 1.0 - smoothstep(
        fill_limit - half_transition,
        fill_limit + half_transition,
        metric,
    );
    let shape_coverage = 1.0 - smoothstep(
        1.0 - half_transition,
        1.0 + half_transition,
        metric,
    );
    let premultiplied_stroke = premultiply_with_coverage(input.stroke, 1.0);
    let premultiplied_fill = premultiply_with_coverage(input.fill, 1.0);
    return mix(premultiplied_stroke, premultiplied_fill, fill_coverage)
        * shape_coverage;
}

@fragment
fn fs_circle_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let radius = length(input.local_position);
    return marker_color(radius, input);
}

@fragment
fn fs_square_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let edge = max(abs(input.local_position.x), abs(input.local_position.y));
    return marker_color(edge, input);
}

@fragment
fn fs_diamond_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let edge = abs(input.local_position.x) + abs(input.local_position.y);
    return marker_color(edge, input);
}

@fragment
fn fs_up_triangle_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let point = input.local_position;
    let edge = max(2.0 * point.y, 1.7320508 * abs(point.x) - point.y);
    return marker_color(edge, input);
}

@fragment
fn fs_down_triangle_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let point = input.local_position;
    let edge = max(-2.0 * point.y, 1.7320508 * abs(point.x) + point.y);
    return marker_color(edge, input);
}

fn plus_metric(point: vec2<f32>) -> f32 {
    let arm_half_width = 0.35;
    let horizontal = max(abs(point.x), abs(point.y) / arm_half_width);
    let vertical = max(abs(point.x) / arm_half_width, abs(point.y));
    return min(horizontal, vertical);
}

@fragment
fn fs_plus_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    return marker_color(plus_metric(input.local_position), input);
}

@fragment
fn fs_cross_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let inverse_sqrt_two = 0.70710677;
    let point = vec2<f32>(
        (input.local_position.x + input.local_position.y) * inverse_sqrt_two,
        (input.local_position.y - input.local_position.x) * inverse_sqrt_two,
    );
    return marker_color(plus_metric(point), input);
}

@fragment
fn fs_horizontal_line_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let edge = max(abs(input.local_position.x), abs(input.local_position.y) / 0.25);
    return marker_color(edge, input);
}

@fragment
fn fs_vertical_line_marker(input: MarkerOutput) -> @location(0) vec4<f32> {
    let edge = max(abs(input.local_position.x) / 0.25, abs(input.local_position.y));
    return marker_color(edge, input);
}
