use std::f32::consts::PI;

use openmat_plot_mir::{
    AxesPoint, CssSize, LineCap, LineJoin, Rgba, StrokeMesh, StrokeStyle, StrokeVertex,
};

use crate::{AxesLineRun, GeometryError};

/// Geometry padding consumed by derivative-based fragment coverage. One CSS
/// pixel covers at least one physical pixel for the browser DPRs supported by
/// the Web IDE and keeps the antialias fringe outside the nominal stroke.
const ANTIALIAS_FRINGE_CSS_PX: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct StrokeTessellation {
    pub mesh: StrokeMesh,
    /// Number of finite source runs represented by this mesh.
    pub run_count: usize,
}

/// Tessellates solid or dashed wide lines into finite, counter-clockwise triangles.
///
/// Dash lengths and phase are measured in CSS pixels. Phase advances continuously
/// through every non-degenerate segment of one finite run and restarts for the next run.
///
/// # Errors
/// Returns [`GeometryError`] for invalid styles, viewport degeneracy, or checked overflow.
pub fn tessellate_stroke(
    runs: &[AxesLineRun],
    viewport_css: CssSize,
    style: StrokeStyle,
    color: Rgba,
    round_segments: u16,
) -> Result<StrokeTessellation, GeometryError> {
    style.validate()?;
    if round_segments == 0 {
        return Err(GeometryError::InvalidRoundSegments);
    }
    let width = viewport_css.width.get();
    let height = viewport_css.height.get();
    if width == 0.0 || height == 0.0 {
        return Err(GeometryError::DegenerateViewport);
    }
    let mut builder = MeshBuilder::new(width, height);
    let dash_pattern = DashPattern::new(&style);
    let mut represented_runs = 0;
    for run in runs {
        let screen_points: Vec<Vec2> = run
            .points
            .iter()
            .map(|point| Vec2::new(point.x * width, point.y * height))
            .collect::<Result<_, _>>()?;
        let segments = non_degenerate_segments(&screen_points);
        if segments.is_empty() || style.width_css_px.get() == 0.0 {
            continue;
        }
        let index_count_before = builder.indices.len();
        if let Some(pattern) = &dash_pattern {
            let dashed = dash_segments(&segments, pattern, style.dash_phase_css_px.get())?;
            for dash in &dashed.runs {
                tessellate_run(&mut builder, dash, &style, round_segments)?;
            }
            for dot in dashed.dots {
                tessellate_dot(&mut builder, dot, &style, round_segments)?;
            }
        } else {
            tessellate_run(&mut builder, &segments, &style, round_segments)?;
        }
        if builder.indices.len() != index_count_before {
            represented_runs += 1;
        }
    }
    let mesh = StrokeMesh {
        vertices: builder.vertices,
        indices: builder.indices,
        color,
        style,
    };
    mesh.validate()?;
    Ok(StrokeTessellation {
        mesh,
        run_count: represented_runs,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vec2 {
    x: f32,
    y: f32,
}

impl Vec2 {
    fn new(x: f32, y: f32) -> Result<Self, GeometryError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryError::NonFiniteCoordinate);
        }
        Ok(Self { x, y })
    }

    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    fn scale(self, factor: f32) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }

    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    fn left_normal(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    start: Vec2,
    end: Vec2,
    direction: Vec2,
    normal: Vec2,
    length: f32,
}

impl Segment {
    fn new(start: Vec2, end: Vec2) -> Option<Self> {
        let delta = end.sub(start);
        let length = delta.length();
        if length <= f32::EPSILON {
            return None;
        }
        let direction = delta.scale(1.0 / length);
        Some(Self {
            start,
            end,
            direction,
            normal: direction.left_normal(),
            length,
        })
    }

    fn point_at(self, distance: f64) -> Vec2 {
        if distance <= 0.0 {
            return self.start;
        }
        if distance >= f64::from(self.length) {
            return self.end;
        }
        let ratio = (distance / f64::from(self.length)).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let ratio = ratio as f32;
        self.start.add(self.end.sub(self.start).scale(ratio))
    }
}

fn non_degenerate_segments(points: &[Vec2]) -> Vec<Segment> {
    points
        .windows(2)
        .filter_map(|pair| Segment::new(pair[0], pair[1]))
        .collect()
}

#[derive(Clone, Debug)]
struct DashPattern {
    lengths: Vec<f64>,
    total: f64,
}

impl DashPattern {
    fn new(style: &StrokeStyle) -> Option<Self> {
        if style.dash_pattern_css_px.is_empty() {
            return None;
        }
        let mut lengths: Vec<f64> = style
            .dash_pattern_css_px
            .iter()
            .map(|length| f64::from(length.get()))
            .collect();
        if lengths.len() % 2 == 1 {
            lengths.extend_from_within(..);
        }
        let total: f64 = lengths.iter().sum();
        debug_assert!(total > 0.0 && total.is_finite());
        Some(Self { lengths, total })
    }
}

#[derive(Clone, Copy, Debug)]
struct DashCursor<'a> {
    pattern: &'a DashPattern,
    index: usize,
    remaining: f64,
}

impl<'a> DashCursor<'a> {
    fn new(pattern: &'a DashPattern, phase: f32) -> Self {
        let mut cursor = Self {
            pattern,
            index: 0,
            remaining: pattern.lengths[0],
        };
        let mut phase = f64::from(phase) % pattern.total;
        while phase > 0.0 {
            if cursor.remaining == 0.0 {
                cursor.advance_entry();
            } else if phase < cursor.remaining {
                cursor.remaining -= phase;
                phase = 0.0;
            } else {
                phase -= cursor.remaining;
                cursor.advance_entry();
            }
        }
        cursor
    }

    const fn is_on(self) -> bool {
        self.index.is_multiple_of(2)
    }

    fn advance_entry(&mut self) {
        self.index = (self.index + 1) % self.pattern.lengths.len();
        self.remaining = self.pattern.lengths[self.index];
    }
}

#[derive(Clone, Debug, Default)]
struct DashedSegments {
    runs: Vec<Vec<Segment>>,
    dots: Vec<Vec2>,
}

fn dash_segments(
    segments: &[Segment],
    pattern: &DashPattern,
    phase: f32,
) -> Result<DashedSegments, GeometryError> {
    let mut cursor = DashCursor::new(pattern, phase);
    let mut output = DashedSegments::default();
    let mut current_run = Vec::new();
    for segment in segments {
        let mut consumed = 0.0;
        let segment_length = f64::from(segment.length);
        while consumed < segment_length {
            skip_zero_dash_entries(
                &mut cursor,
                segment.point_at(consumed),
                &mut current_run,
                &mut output,
            );
            let step = cursor.remaining.min(segment_length - consumed);
            let next_consumed = consumed + step;
            if next_consumed <= consumed {
                return Err(GeometryError::ArithmeticOverflow(
                    "dash subdivision progress",
                ));
            }
            if cursor.is_on()
                && let Some(piece) =
                    Segment::new(segment.point_at(consumed), segment.point_at(next_consumed))
            {
                current_run.push(piece);
            }
            consumed = next_consumed;
            cursor.remaining -= step;
            if cursor.remaining == 0.0 {
                let was_on = cursor.is_on();
                cursor.advance_entry();
                if was_on && !cursor.is_on() {
                    finish_dash_run(&mut current_run, &mut output.runs);
                }
            }
        }
    }
    if let Some(last) = segments.last() {
        skip_zero_dash_entries(&mut cursor, last.end, &mut current_run, &mut output);
    }
    finish_dash_run(&mut current_run, &mut output.runs);
    Ok(output)
}

fn skip_zero_dash_entries(
    cursor: &mut DashCursor<'_>,
    point: Vec2,
    current_run: &mut Vec<Segment>,
    output: &mut DashedSegments,
) {
    while cursor.remaining == 0.0 {
        if cursor.is_on() {
            finish_dash_run(current_run, &mut output.runs);
            output.dots.push(point);
        }
        cursor.advance_entry();
    }
}

fn finish_dash_run(current: &mut Vec<Segment>, runs: &mut Vec<Vec<Segment>>) {
    if !current.is_empty() {
        runs.push(std::mem::take(current));
    }
}

fn tessellate_run(
    builder: &mut MeshBuilder,
    segments: &[Segment],
    style: &StrokeStyle,
    round_segments: u16,
) -> Result<(), GeometryError> {
    let half_width = style.width_css_px.get() / 2.0;
    let outer_half_width = half_width + ANTIALIAS_FRINGE_CSS_PX;
    for (index, segment) in segments.iter().enumerate() {
        let mut start = segment.start;
        let mut end = segment.end;
        if style.cap == LineCap::Square && index == 0 {
            start = start.sub(segment.direction.scale(half_width));
        }
        if style.cap == LineCap::Square && index + 1 == segments.len() {
            end = end.add(segment.direction.scale(half_width));
        }
        let offset = segment.normal.scale(outer_half_width);
        builder.quad(
            StrokeSample::new(start.add(offset), outer_half_width),
            StrokeSample::new(start.sub(offset), -outer_half_width),
            StrokeSample::new(end.add(offset), outer_half_width),
            StrokeSample::new(end.sub(offset), -outer_half_width),
        )?;
    }
    for pair in segments.windows(2) {
        if pair[0].end == pair[1].start {
            tessellate_join(
                builder,
                pair[0],
                pair[1],
                style,
                outer_half_width,
                round_segments,
            )?;
        }
    }
    if style.cap == LineCap::Round {
        let first = segments[0];
        let last = segments[segments.len() - 1];
        let left = first.normal.scale(outer_half_width);
        builder.fan_arc(first.start, left, PI, outer_half_width, round_segments)?;
        let right = last.normal.scale(-outer_half_width);
        builder.fan_arc(last.end, right, PI, outer_half_width, round_segments)?;
    }
    Ok(())
}

fn tessellate_dot(
    builder: &mut MeshBuilder,
    center: Vec2,
    style: &StrokeStyle,
    round_segments: u16,
) -> Result<(), GeometryError> {
    let half_width = style.width_css_px.get() / 2.0;
    let outer_half_width = half_width + ANTIALIAS_FRINGE_CSS_PX;
    match style.cap {
        LineCap::Butt => Ok(()),
        LineCap::Round => {
            let right = Vec2 {
                x: outer_half_width,
                y: 0.0,
            };
            builder.fan_arc(center, right, PI, outer_half_width, round_segments)?;
            builder.fan_arc(
                center,
                right.scale(-1.0),
                PI,
                outer_half_width,
                round_segments,
            )
        }
        LineCap::Square => builder.quad(
            StrokeSample::new(
                center.add(Vec2 {
                    x: -half_width,
                    y: outer_half_width,
                }),
                outer_half_width,
            ),
            StrokeSample::new(
                center.add(Vec2 {
                    x: -half_width,
                    y: -outer_half_width,
                }),
                -outer_half_width,
            ),
            StrokeSample::new(
                center.add(Vec2 {
                    x: half_width,
                    y: outer_half_width,
                }),
                outer_half_width,
            ),
            StrokeSample::new(
                center.add(Vec2 {
                    x: half_width,
                    y: -outer_half_width,
                }),
                -outer_half_width,
            ),
        ),
    }
}

fn tessellate_join(
    builder: &mut MeshBuilder,
    previous: Segment,
    next: Segment,
    style: &StrokeStyle,
    outer_half_width: f32,
    round_segments: u16,
) -> Result<(), GeometryError> {
    let turn = previous.direction.cross(next.direction);
    if turn.abs() <= f32::EPSILON {
        return Ok(());
    }
    let side = if turn > 0.0 { 1.0 } else { -1.0 };
    let previous_offset = previous.normal.scale(outer_half_width * side);
    let next_offset = next.normal.scale(outer_half_width * side);
    let center = previous.end;
    let previous_outer = center.add(previous_offset);
    let next_outer = center.add(next_offset);
    let outer_distance = outer_half_width * side;
    let center_sample = StrokeSample::new(center, 0.0);
    let previous_sample = StrokeSample::new(previous_outer, outer_distance);
    let next_sample = StrokeSample::new(next_outer, outer_distance);
    match style.join {
        LineJoin::Bevel => builder.triangle(center_sample, previous_sample, next_sample)?,
        LineJoin::Round => {
            let angle = signed_angle(previous_offset, next_offset);
            builder.fan_arc(
                center,
                previous_offset,
                angle,
                outer_distance,
                round_segments,
            )?;
        }
        LineJoin::Miter => {
            let denominator = previous.direction.cross(next.direction);
            let delta = next_outer.sub(previous_outer);
            let distance = delta.cross(next.direction) / denominator;
            let miter = previous_outer.add(previous.direction.scale(distance));
            let ratio = miter.sub(center).length() / outer_half_width.max(f32::MIN_POSITIVE);
            if ratio.is_finite() && ratio <= style.miter_limit {
                let miter_sample = StrokeSample::new(miter, outer_distance);
                builder.triangle(center_sample, previous_sample, miter_sample)?;
                builder.triangle(center_sample, miter_sample, next_sample)?;
            } else {
                builder.triangle(center_sample, previous_sample, next_sample)?;
            }
        }
    }
    Ok(())
}

fn signed_angle(from: Vec2, to: Vec2) -> f32 {
    from.cross(to).atan2(from.dot(to))
}

struct MeshBuilder {
    viewport_width: f32,
    viewport_height: f32,
    vertices: Vec<StrokeVertex>,
    indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
struct StrokeSample {
    point: Vec2,
    edge_distance_css_px: f32,
}

impl StrokeSample {
    const fn new(point: Vec2, edge_distance_css_px: f32) -> Self {
        Self {
            point,
            edge_distance_css_px,
        }
    }
}

impl MeshBuilder {
    const fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn vertex(&mut self, sample: StrokeSample) -> Result<u32, GeometryError> {
        let position = AxesPoint::new(
            sample.point.x / self.viewport_width,
            sample.point.y / self.viewport_height,
        )?;
        let index = checked_index(self.vertices.len())?;
        self.vertices.push(StrokeVertex {
            position,
            edge_distance_css_px: sample.edge_distance_css_px,
        });
        Ok(index)
    }

    fn triangle(
        &mut self,
        a: StrokeSample,
        mut b: StrokeSample,
        mut c: StrokeSample,
    ) -> Result<(), GeometryError> {
        let area = b.point.sub(a.point).cross(c.point.sub(a.point));
        if area.abs() <= f32::EPSILON {
            return Ok(());
        }
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
        }
        let a = self.vertex(a)?;
        let b = self.vertex(b)?;
        let c = self.vertex(c)?;
        self.indices.extend_from_slice(&[a, b, c]);
        Ok(())
    }

    fn quad(
        &mut self,
        left_start: StrokeSample,
        right_start: StrokeSample,
        left_end: StrokeSample,
        right_end: StrokeSample,
    ) -> Result<(), GeometryError> {
        self.triangle(left_start, right_start, left_end)?;
        self.triangle(right_start, right_end, left_end)
    }

    fn fan_arc(
        &mut self,
        center: Vec2,
        start_offset: Vec2,
        angle: f32,
        outer_distance_css_px: f32,
        segment_count: u16,
    ) -> Result<(), GeometryError> {
        let step = angle / f32::from(segment_count);
        let mut previous = center.add(start_offset);
        for index in 1..=segment_count {
            let rotation = step * f32::from(index);
            let (sine, cosine) = rotation.sin_cos();
            let rotated = Vec2 {
                x: start_offset.x * cosine - start_offset.y * sine,
                y: start_offset.x * sine + start_offset.y * cosine,
            };
            let current = center.add(rotated);
            self.triangle(
                StrokeSample::new(center, 0.0),
                StrokeSample::new(previous, outer_distance_css_px),
                StrokeSample::new(current, outer_distance_css_px),
            )?;
            previous = current;
        }
        Ok(())
    }
}

fn checked_index(length: usize) -> Result<u32, GeometryError> {
    u32::try_from(length).map_err(|_| GeometryError::ArithmeticOverflow("mesh vertex index"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmat_plot_mir::{CssPx, LineCap, LineJoin};

    fn style(cap: LineCap, join: LineJoin) -> StrokeStyle {
        StrokeStyle::solid(CssPx::new(4.0).unwrap(), cap, join).unwrap()
    }

    fn color() -> Rgba {
        Rgba::new(0.1, 0.2, 0.3, 1.0).unwrap()
    }

    fn dashed_style(pattern: &[f32], phase: f32, cap: LineCap) -> StrokeStyle {
        let mut style = style(cap, LineJoin::Miter);
        style.dash_pattern_css_px = pattern
            .iter()
            .map(|&length| CssPx::new(length).unwrap())
            .collect();
        style.dash_phase_css_px = CssPx::new(phase).unwrap();
        style.validate().unwrap();
        style
    }

    fn horizontal_segments(length: f32) -> Vec<Segment> {
        non_degenerate_segments(&[Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: length, y: 0.0 }])
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
    }

    #[test]
    fn all_cap_and_join_variants_produce_finite_ccw_bounded_meshes() {
        let run = AxesLineRun {
            points: vec![
                AxesPoint { x: 0.1, y: 0.1 },
                AxesPoint { x: 0.5, y: 0.8 },
                AxesPoint { x: 0.9, y: 0.2 },
            ],
        };
        let viewport = CssSize::new(600.0, 200.0).unwrap();
        for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
            for join in [LineJoin::Miter, LineJoin::Bevel, LineJoin::Round] {
                let output = tessellate_stroke(
                    std::slice::from_ref(&run),
                    viewport,
                    style(cap, join),
                    color(),
                    8,
                )
                .unwrap();
                assert!(!output.mesh.indices.is_empty());
                assert!(
                    output.mesh.vertices.iter().all(
                        |vertex| vertex.position.x.is_finite() && vertex.position.y.is_finite()
                    )
                );
                assert!(
                    output
                        .mesh
                        .indices
                        .iter()
                        .all(|&index| usize::try_from(index).unwrap() < output.mesh.vertices.len())
                );
                for triangle in output.mesh.indices.chunks_exact(3) {
                    let a = output.mesh.vertices[triangle[0] as usize].position;
                    let b = output.mesh.vertices[triangle[1] as usize].position;
                    let c = output.mesh.vertices[triangle[2] as usize].position;
                    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
                    assert!(area > 0.0, "triangle winding must be CCW");
                }
            }
        }
    }

    #[test]
    fn empty_and_duplicate_only_runs_produce_empty_mesh() {
        let duplicate = AxesLineRun {
            points: vec![AxesPoint { x: 0.5, y: 0.5 }; 3],
        };
        let output = tessellate_stroke(
            &[duplicate],
            CssSize::new(100.0, 100.0).unwrap(),
            style(LineCap::Butt, LineJoin::Miter),
            color(),
            4,
        )
        .unwrap();
        assert!(output.mesh.vertices.is_empty());
        assert!(output.mesh.indices.is_empty());
    }

    #[test]
    fn analytic_antialias_fringe_extends_beyond_nominal_stroke() {
        let run = AxesLineRun {
            points: vec![AxesPoint { x: 0.1, y: 0.5 }, AxesPoint { x: 0.9, y: 0.5 }],
        };
        let output = tessellate_stroke(
            &[run],
            CssSize::new(100.0, 100.0).unwrap(),
            StrokeStyle::solid(CssPx::new(2.0).unwrap(), LineCap::Butt, LineJoin::Miter).unwrap(),
            color(),
            4,
        )
        .unwrap();

        let minimum_y = output
            .mesh
            .vertices
            .iter()
            .map(|vertex| vertex.position.y)
            .reduce(f32::min)
            .unwrap();
        let maximum_y = output
            .mesh
            .vertices
            .iter()
            .map(|vertex| vertex.position.y)
            .reduce(f32::max)
            .unwrap();
        assert!((minimum_y - 0.48).abs() < 1.0e-6);
        assert!((maximum_y - 0.52).abs() < 1.0e-6);
        assert!(
            output
                .mesh
                .vertices
                .iter()
                .any(|vertex| { (vertex.edge_distance_css_px - 2.0).abs() < f32::EPSILON })
        );
        assert!(
            output
                .mesh
                .vertices
                .iter()
                .any(|vertex| { (vertex.edge_distance_css_px + 2.0).abs() < f32::EPSILON })
        );
    }

    #[test]
    fn dash_pattern_and_phase_cut_at_deterministic_css_positions() {
        let style = dashed_style(&[10.0, 10.0], 5.0, LineCap::Butt);
        let pattern = DashPattern::new(&style).unwrap();
        let dashed = dash_segments(&horizontal_segments(60.0), &pattern, 5.0).unwrap();
        assert!(dashed.dots.is_empty());
        assert_eq!(dashed.runs.len(), 4);
        let extents: Vec<_> = dashed
            .runs
            .iter()
            .map(|run| (run.first().unwrap().start.x, run.last().unwrap().end.x))
            .collect();
        for ((actual_start, actual_end), (expected_start, expected_end)) in extents
            .into_iter()
            .zip([(0.0, 5.0), (15.0, 25.0), (35.0, 45.0), (55.0, 60.0)])
        {
            assert_close(actual_start, expected_start);
            assert_close(actual_end, expected_end);
        }
    }

    #[test]
    fn dash_phase_is_continuous_across_polyline_vertices() {
        let style = dashed_style(&[20.0, 10.0], 0.0, LineCap::Butt);
        let pattern = DashPattern::new(&style).unwrap();
        let segments = non_degenerate_segments(&[
            Vec2 { x: 0.0, y: 0.0 },
            Vec2 { x: 15.0, y: 0.0 },
            Vec2 { x: 15.0, y: 15.0 },
        ]);
        let dashed = dash_segments(&segments, &pattern, 0.0).unwrap();
        assert_eq!(dashed.runs.len(), 1);
        assert_eq!(dashed.runs[0].len(), 2);
        assert_eq!(dashed.runs[0][0].end, Vec2 { x: 15.0, y: 0.0 });
        assert_eq!(dashed.runs[0][1].start, Vec2 { x: 15.0, y: 0.0 });
        assert_eq!(dashed.runs[0][1].end, Vec2 { x: 15.0, y: 5.0 });
    }

    #[test]
    fn zero_length_on_entries_make_round_dots_and_dash_dot_sequences() {
        let dot_style = dashed_style(&[0.0, 10.0], 0.0, LineCap::Round);
        let dot_pattern = DashPattern::new(&dot_style).unwrap();
        let dots = dash_segments(&horizontal_segments(25.0), &dot_pattern, 0.0).unwrap();
        assert!(dots.runs.is_empty());
        assert_eq!(
            dots.dots,
            vec![
                Vec2 { x: 0.0, y: 0.0 },
                Vec2 { x: 10.0, y: 0.0 },
                Vec2 { x: 20.0, y: 0.0 }
            ]
        );

        let dash_dot_style = dashed_style(&[10.0, 5.0, 0.0, 5.0], 0.0, LineCap::Round);
        let dash_dot_pattern = DashPattern::new(&dash_dot_style).unwrap();
        let dash_dot = dash_segments(&horizontal_segments(40.0), &dash_dot_pattern, 0.0).unwrap();
        assert_eq!(dash_dot.runs.len(), 2);
        assert_eq!(
            dash_dot.dots,
            vec![Vec2 { x: 15.0, y: 0.0 }, Vec2 { x: 35.0, y: 0.0 }]
        );

        let output = tessellate_stroke(
            &[AxesLineRun {
                points: vec![AxesPoint { x: 0.0, y: 0.5 }, AxesPoint { x: 0.25, y: 0.5 }],
            }],
            CssSize::new(100.0, 100.0).unwrap(),
            dot_style,
            color(),
            8,
        )
        .unwrap();
        assert_eq!(output.run_count, 1);
        assert!(!output.mesh.indices.is_empty());
    }

    #[test]
    fn finite_runs_restart_phase_and_mesh_output_is_deterministic() {
        let runs = [
            AxesLineRun {
                points: vec![
                    AxesPoint { x: 0.0, y: 0.25 },
                    AxesPoint { x: 0.15, y: 0.25 },
                ],
            },
            AxesLineRun {
                points: vec![
                    AxesPoint { x: 0.5, y: 0.75 },
                    AxesPoint { x: 0.65, y: 0.75 },
                ],
            },
        ];
        let style = dashed_style(&[10.0, 10.0], 0.0, LineCap::Square);
        let first = tessellate_stroke(
            &runs,
            CssSize::new(100.0, 100.0).unwrap(),
            style.clone(),
            color(),
            8,
        )
        .unwrap();
        let second = tessellate_stroke(
            &runs,
            CssSize::new(100.0, 100.0).unwrap(),
            style,
            color(),
            8,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.run_count, 2);
        let minimum_x = first
            .mesh
            .vertices
            .iter()
            .map(|vertex| vertex.position.x)
            .reduce(f32::min)
            .unwrap();
        assert!(minimum_x < 0.0, "square cap should extend the first dash");
        assert!(
            first
                .mesh
                .vertices
                .iter()
                .any(|vertex| vertex.position.x > 0.47 && vertex.position.x < 0.53)
        );
    }

    #[test]
    fn checked_index_rejects_usize_overflow() {
        assert_eq!(
            checked_index(usize::MAX),
            Err(GeometryError::ArithmeticOverflow("mesh vertex index"))
        );
    }
}
