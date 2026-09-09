use crate::{GeometryError, SeriesGeometryInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodBreakKind {
    NaN,
    Infinity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LodSample {
    Point {
        source_index: usize,
        x: f64,
        y: f64,
    },
    Break {
        source_index: usize,
        kind: LodBreakKind,
    },
}

/// Immutable data-resource identity used by an LOD cache.
///
/// `revision` is explicit because not every caller uses immutable buffer identifiers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LodResourceRevision {
    pub identity: String,
    pub revision: u64,
}

impl LodResourceRevision {
    #[must_use]
    pub fn new(identity: impl Into<String>, revision: u64) -> Self {
        Self {
            identity: identity.into(),
            revision,
        }
    }
}

/// Identity of the source x/y resources. `x == None` denotes implicit `1..=N` x data.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LodSourceKey {
    pub x: Option<LodResourceRevision>,
    pub y: LodResourceRevision,
}

/// Canonical view-dependent portion of an LOD cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LodViewKey {
    visible_x_min_bits: u64,
    visible_x_max_bits: u64,
    device_columns: u32,
}

impl LodViewKey {
    #[must_use]
    pub const fn visible_x_min_bits(self) -> u64 {
        self.visible_x_min_bits
    }

    #[must_use]
    pub const fn visible_x_max_bits(self) -> u64 {
        self.visible_x_max_bits
    }

    #[must_use]
    pub const fn device_columns(self) -> u32 {
        self.device_columns
    }
}

/// Complete key for an exact LOD selection cache entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LodCacheKey {
    pub source: LodSourceKey,
    pub view: LodViewKey,
}

impl LodCacheKey {
    #[must_use]
    pub const fn new(source: LodSourceKey, view: LinearLodView) -> Self {
        Self {
            source,
            view: view.cache_key(),
        }
    }
}

/// Linear x view used to assign source points to physical screen columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearLodView {
    visible_x_min: f64,
    visible_x_max: f64,
    css_width: f64,
    device_pixel_ratio: f64,
    device_columns: u32,
}

impl LinearLodView {
    /// Creates a finite, increasing view and derives `ceil(css_width * DPR)` columns.
    ///
    /// # Errors
    /// Returns [`GeometryError::NonFiniteCoordinate`] for non-finite values,
    /// [`GeometryError::DegenerateViewport`] for non-positive dimensions or range,
    /// and [`GeometryError::ArithmeticOverflow`] when the physical width exceeds `u32`.
    pub fn new(
        visible_x_min: f64,
        visible_x_max: f64,
        css_width: f64,
        device_pixel_ratio: f64,
    ) -> Result<Self, GeometryError> {
        if !visible_x_min.is_finite()
            || !visible_x_max.is_finite()
            || !css_width.is_finite()
            || !device_pixel_ratio.is_finite()
        {
            return Err(GeometryError::NonFiniteCoordinate);
        }
        if visible_x_min >= visible_x_max || css_width <= 0.0 || device_pixel_ratio <= 0.0 {
            return Err(GeometryError::DegenerateViewport);
        }
        let physical_width = css_width * device_pixel_ratio;
        if !physical_width.is_finite() || physical_width > f64::from(u32::MAX) {
            return Err(GeometryError::ArithmeticOverflow("LOD viewport width"));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let device_columns = physical_width.ceil() as u32;
        Ok(Self {
            visible_x_min: canonical_zero(visible_x_min),
            visible_x_max: canonical_zero(visible_x_max),
            css_width,
            device_pixel_ratio,
            device_columns: device_columns.max(1),
        })
    }

    #[must_use]
    pub const fn visible_x_range(self) -> [f64; 2] {
        [self.visible_x_min, self.visible_x_max]
    }

    #[must_use]
    pub const fn css_width(self) -> f64 {
        self.css_width
    }

    #[must_use]
    pub const fn device_pixel_ratio(self) -> f64 {
        self.device_pixel_ratio
    }

    #[must_use]
    pub const fn device_columns(self) -> u32 {
        self.device_columns
    }

    #[must_use]
    pub const fn cache_key(self) -> LodViewKey {
        LodViewKey {
            visible_x_min_bits: self.visible_x_min.to_bits(),
            visible_x_max_bits: self.visible_x_max.to_bits(),
            device_columns: self.device_columns,
        }
    }

    fn column(self, x: f64) -> Option<u32> {
        if x < self.visible_x_min || x > self.visible_x_max {
            return None;
        }
        let span = self.visible_x_max - self.visible_x_min;
        let normalized = if span.is_finite() {
            (x - self.visible_x_min) / span
        } else {
            let half_min = self.visible_x_min * 0.5;
            ((x * 0.5) - half_min) / ((self.visible_x_max * 0.5) - half_min)
        };
        let scaled = normalized * f64::from(self.device_columns);
        if scaled >= f64::from(self.device_columns) {
            return Some(self.device_columns - 1);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(scaled.max(0.0).floor() as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodStatistics {
    pub source_points_visited: usize,
    pub bucket_updates: usize,
    pub finite_runs: usize,
    pub invalid_runs: usize,
    pub touched_columns: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LodOutput {
    pub samples: Vec<LodSample>,
    pub statistics: LodStatistics,
}

impl LodOutput {
    /// Conservative structural bound for this selection's output.
    ///
    /// Each touched column contributes at most first/min/max/last, each finite run
    /// contributes at most two off-screen endpoints, and each invalid run contributes
    /// at most its two boundary markers.
    #[must_use]
    pub fn structural_output_bound(&self) -> Option<usize> {
        self.statistics
            .touched_columns
            .checked_mul(4)?
            .checked_add(self.statistics.finite_runs.checked_mul(2)?)?
            .checked_add(self.statistics.invalid_runs.checked_mul(2)?)
    }
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    source_index: usize,
    y: f64,
}

#[derive(Clone, Copy, Debug)]
struct ColumnEnvelope {
    first: Candidate,
    minimum: Candidate,
    maximum: Candidate,
    last: Candidate,
}

impl ColumnEnvelope {
    const fn new(candidate: Candidate) -> Self {
        Self {
            first: candidate,
            minimum: candidate,
            maximum: candidate,
            last: candidate,
        }
    }

    fn update(&mut self, candidate: Candidate) {
        if candidate.y.total_cmp(&self.minimum.y).is_lt() {
            self.minimum = candidate;
        }
        if candidate.y.total_cmp(&self.maximum.y).is_gt() {
            self.maximum = candidate;
        }
        self.last = candidate;
    }

    const fn contains(self, source_index: usize) -> bool {
        self.first.source_index == source_index
            || self.minimum.source_index == source_index
            || self.maximum.source_index == source_index
            || self.last.source_index == source_index
    }
}

/// Builds a deterministic per-device-column min/max envelope.
///
/// Every finite run is scanned twice. The first pass retains at most four candidates
/// per touched screen column; the second emits those candidates in original source
/// order. Run endpoints and compact non-finite boundaries are preserved. The source
/// views are only read and are never mutated.
///
/// # Errors
/// Returns [`GeometryError`] for invalid views, implicit-index overflow, or scratch
/// allocation overflow.
pub fn min_max_lod(
    input: SeriesGeometryInput<'_>,
    view: LinearLodView,
) -> Result<LodOutput, GeometryError> {
    input.validate()?;
    let column_count = usize::try_from(view.device_columns)
        .map_err(|_| GeometryError::ArithmeticOverflow("LOD column count"))?;
    let mut columns = Vec::new();
    columns
        .try_reserve_exact(column_count)
        .map_err(|_| GeometryError::ArithmeticOverflow("LOD column storage"))?;
    columns.resize(column_count, None);

    let mut output = LodOutput::default();
    let mut touched = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let (x, y) = read_point(input, cursor, &mut output.statistics)?;
        if x.is_finite() && y.is_finite() {
            let start = cursor;
            cursor = collect_finite_run(
                input,
                cursor,
                view,
                &mut columns,
                &mut touched,
                &mut output.statistics,
            )?;
            emit_finite_run(input, start, cursor, view, &columns, &mut output)?;
            output.statistics.finite_runs += 1;
            output.statistics.touched_columns += touched.len();
            for column in touched.drain(..) {
                columns[column] = None;
            }
        } else {
            cursor = emit_invalid_run(input, cursor, &mut output)?;
        }
    }
    debug_assert!(
        output
            .structural_output_bound()
            .is_some_and(|bound| output.samples.len() <= bound)
    );
    Ok(output)
}

fn collect_finite_run(
    input: SeriesGeometryInput<'_>,
    mut cursor: usize,
    view: LinearLodView,
    columns: &mut [Option<ColumnEnvelope>],
    touched: &mut Vec<usize>,
    statistics: &mut LodStatistics,
) -> Result<usize, GeometryError> {
    while cursor < input.len() {
        let (x, y) = read_point(input, cursor, statistics)?;
        if !x.is_finite() || !y.is_finite() {
            break;
        }
        if let Some(column) = view.column(x) {
            let column = usize::try_from(column)
                .map_err(|_| GeometryError::ArithmeticOverflow("LOD screen column"))?;
            let candidate = Candidate {
                source_index: cursor,
                y: canonical_zero(y),
            };
            if let Some(envelope) = &mut columns[column] {
                envelope.update(candidate);
            } else {
                columns[column] = Some(ColumnEnvelope::new(candidate));
                touched.push(column);
            }
            statistics.bucket_updates += 1;
        }
        cursor += 1;
    }
    Ok(cursor)
}

fn emit_finite_run(
    input: SeriesGeometryInput<'_>,
    start: usize,
    end: usize,
    view: LinearLodView,
    columns: &[Option<ColumnEnvelope>],
    output: &mut LodOutput,
) -> Result<(), GeometryError> {
    for source_index in start..end {
        let (x, y) = read_point(input, source_index, &mut output.statistics)?;
        let run_endpoint = source_index == start || source_index + 1 == end;
        let envelope_candidate = view.column(x).is_some_and(|column| {
            usize::try_from(column)
                .ok()
                .and_then(|column| columns[column])
                .is_some_and(|envelope| envelope.contains(source_index))
        });
        if run_endpoint || envelope_candidate {
            output.samples.push(LodSample::Point {
                source_index,
                x: canonical_zero(x),
                y: canonical_zero(y),
            });
        }
    }
    Ok(())
}

fn emit_invalid_run(
    input: SeriesGeometryInput<'_>,
    start: usize,
    output: &mut LodOutput,
) -> Result<usize, GeometryError> {
    let (first_x, first_y) = read_point(input, start, &mut output.statistics)?;
    let first_kind = break_kind(first_x, first_y);
    let mut cursor = start + 1;
    let mut last = (start, first_kind);
    while cursor < input.len() {
        let (x, y) = read_point(input, cursor, &mut output.statistics)?;
        if x.is_finite() && y.is_finite() {
            break;
        }
        last = (cursor, break_kind(x, y));
        cursor += 1;
    }
    output.samples.push(LodSample::Break {
        source_index: start,
        kind: first_kind,
    });
    if last.0 != start {
        output.samples.push(LodSample::Break {
            source_index: last.0,
            kind: last.1,
        });
    }
    output.statistics.invalid_runs += 1;
    Ok(cursor)
}

fn read_point(
    input: SeriesGeometryInput<'_>,
    index: usize,
    statistics: &mut LodStatistics,
) -> Result<(f64, f64), GeometryError> {
    statistics.source_points_visited = statistics
        .source_points_visited
        .checked_add(1)
        .ok_or(GeometryError::ArithmeticOverflow("LOD operation count"))?;
    input.point(index)
}

const fn break_kind(x: f64, y: f64) -> LodBreakKind {
    if x.is_nan() || y.is_nan() {
        LodBreakKind::NaN
    } else {
        LodBreakKind::Infinity
    }
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NumericView;
    use crate::runs::line_runs_from_lod;

    fn view(min: f64, max: f64, css_width: f64, dpr: f64) -> LinearLodView {
        LinearLodView::new(min, max, css_width, dpr).unwrap()
    }

    fn explicit<'a>(x: &'a [f64], y: &'a [f64]) -> SeriesGeometryInput<'a> {
        SeriesGeometryInput {
            x: Some(NumericView::F64(x)),
            y: NumericView::F64(y),
        }
    }

    fn point_indices(output: &LodOutput) -> Vec<usize> {
        output
            .samples
            .iter()
            .filter_map(|sample| match sample {
                LodSample::Point { source_index, .. } => Some(*source_index),
                LodSample::Break { .. } => None,
            })
            .collect()
    }

    #[test]
    fn empty_and_single_point_inputs_are_stable() {
        let empty = min_max_lod(
            SeriesGeometryInput {
                x: None,
                y: NumericView::F64(&[]),
            },
            view(0.0, 2.0, 10.0, 1.0),
        )
        .unwrap();
        assert!(empty.samples.is_empty());

        let one = min_max_lod(
            SeriesGeometryInput {
                x: None,
                y: NumericView::F64(&[7.0]),
            },
            view(0.0, 2.0, 10.0, 1.0),
        )
        .unwrap();
        assert_eq!(point_indices(&one), vec![0]);
    }

    #[test]
    fn isolated_spike_and_column_extrema_survive() {
        let x: Vec<f64> = (0_u32..100).map(|value| f64::from(value) / 100.0).collect();
        let mut y = vec![0.0; x.len()];
        y[40] = 1.0e9;
        y[41] = -1.0e9;
        let output = min_max_lod(explicit(&x, &y), view(0.0, 1.0, 1.0, 1.0)).unwrap();
        let indices = point_indices(&output);
        assert!(indices.contains(&0));
        assert!(indices.contains(&40));
        assert!(indices.contains(&41));
        assert!(indices.contains(&99));
        assert_eq!(indices, {
            let mut sorted = indices.clone();
            sorted.sort_unstable();
            sorted
        });
    }

    #[test]
    fn repeated_and_non_monotonic_x_are_globally_bounded_by_columns() {
        let x = [0.1, 0.9, 0.1, 0.9, 0.1, 0.9, 0.1, 0.9, 0.1, 0.9];
        let y = [0.0, 1.0, 2.0, 3.0, -20.0, 5.0, 30.0, 7.0, 8.0, 9.0];
        let output = min_max_lod(explicit(&x, &y), view(0.0, 1.0, 2.0, 1.0)).unwrap();
        let indices = point_indices(&output);
        assert!(indices.contains(&4));
        assert!(indices.contains(&6));
        assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(output.samples.len() <= 4 * 2 + 2);
        assert!(output.samples.len() <= output.structural_output_bound().unwrap());
    }

    #[test]
    fn finite_run_endpoints_and_compact_nan_infinity_boundaries_survive() {
        let x = [0.0, 0.1, f64::NAN, f64::INFINITY, 0.8, 0.9, 1.0];
        let y = [0.0, 1.0, 2.0, f64::NAN, 3.0, f64::INFINITY, 4.0];
        let output = min_max_lod(explicit(&x, &y), view(0.0, 1.0, 2.0, 1.0)).unwrap();
        assert_eq!(point_indices(&output), vec![0, 1, 4, 6]);
        assert!(output.samples.contains(&LodSample::Break {
            source_index: 2,
            kind: LodBreakKind::NaN,
        }));
        assert!(output.samples.contains(&LodSample::Break {
            source_index: 3,
            kind: LodBreakKind::NaN,
        }));
        assert!(output.samples.contains(&LodSample::Break {
            source_index: 5,
            kind: LodBreakKind::Infinity,
        }));
        let runs = line_runs_from_lod(&output.samples);
        assert_eq!(
            runs.iter().map(|run| run.points.len()).collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
    }

    #[test]
    fn visible_range_keeps_run_endpoints_but_ignores_offscreen_extrema() {
        let x = [-10.0, -5.0, 0.25, 0.75, 5.0, 10.0];
        let y = [1000.0, -2000.0, 2.0, 3.0, 4000.0, -5000.0];
        let output = min_max_lod(explicit(&x, &y), view(0.0, 1.0, 1.0, 1.0)).unwrap();
        assert_eq!(point_indices(&output), vec![0, 2, 3, 5]);
    }

    #[test]
    fn huge_subnormal_and_signed_zero_values_map_deterministically() {
        let huge_x = [-f64::MAX, -1.0, 0.0, 1.0, f64::MAX];
        let huge_y = [-0.0, f64::MIN_POSITIVE, -f64::MIN_POSITIVE, 1.0, 2.0];
        let first = min_max_lod(
            explicit(&huge_x, &huge_y),
            view(-f64::MAX, f64::MAX, 4.0, 1.0),
        )
        .unwrap();
        let second = min_max_lod(
            explicit(&huge_x, &huge_y),
            view(-f64::MAX, f64::MAX, 4.0, 1.0),
        )
        .unwrap();
        assert_eq!(first.samples, second.samples);
        assert!(first.samples.iter().all(|sample| match sample {
            LodSample::Point { x, y, .. } => {
                (*x != 0.0 || x.to_bits() == 0) && (*y != 0.0 || y.to_bits() == 0)
            }
            LodSample::Break { .. } => true,
        }));

        let tiny = f64::from_bits(1);
        let tiny_x = [0.0, tiny, tiny * 2.0];
        let tiny_y = [1.0, 4.0, 2.0];
        let tiny_output =
            min_max_lod(explicit(&tiny_x, &tiny_y), view(0.0, tiny * 2.0, 2.0, 1.0)).unwrap();
        assert_eq!(point_indices(&tiny_output), vec![0, 1, 2]);
    }

    #[test]
    fn css_width_and_dpr_control_physical_column_budget() {
        let a = view(0.0, 1.0, 320.0, 2.0);
        let b = view(0.0, 1.0, 640.0, 1.0);
        let c = view(0.0, 1.0, 320.0, 1.0);
        assert_eq!(a.device_columns(), 640);
        assert_eq!(a.cache_key(), b.cache_key());
        assert_ne!(a.cache_key(), c.cache_key());

        let source = LodSourceKey {
            x: Some(LodResourceRevision::new("x-buffer", 3)),
            y: LodResourceRevision::new("y-buffer", 7),
        };
        assert_eq!(
            LodCacheKey::new(source.clone(), a),
            LodCacheKey::new(source.clone(), b)
        );
        assert_ne!(
            LodCacheKey::new(source.clone(), a),
            LodCacheKey::new(
                LodSourceKey {
                    y: LodResourceRevision::new("y-buffer", 8),
                    ..source
                },
                a
            )
        );
    }

    #[test]
    fn multiple_viewport_widths_obey_their_structural_bounds() {
        let x: Vec<f64> = (0_u32..10_000).map(f64::from).collect();
        let y: Vec<f64> = (0_u32..10_000)
            .map(|index| f64::from((index.wrapping_mul(7919)) % 997))
            .collect();
        for (css_width, dpr) in [(1.0, 1.0), (7.25, 1.0), (127.5, 2.0), (320.0, 3.0)] {
            let view = view(0.0, 9_999.0, css_width, dpr);
            let output = min_max_lod(explicit(&x, &y), view).unwrap();
            let column_bound = usize::try_from(view.device_columns()).unwrap() * 4 + 2;
            assert!(output.samples.len() <= column_bound);
            assert!(output.samples.len() <= output.structural_output_bound().unwrap());
        }
    }

    #[test]
    fn million_points_have_linear_work_and_width_bounded_output() {
        const COUNT: usize = 1_000_000;
        let x: Vec<f64> = (0_u32..1_000_000).map(f64::from).collect();
        let mut y = vec![0.0; COUNT];
        y[543_210] = 1.0e100;
        let output = min_max_lod(explicit(&x, &y), view(0.0, 999_999.0, 800.0, 2.0)).unwrap();
        assert_eq!(output.statistics.bucket_updates, COUNT);
        assert!(output.statistics.source_points_visited <= COUNT * 2 + 2);
        assert!(output.samples.len() <= 4 * 1_600 + 2);
        assert!(output.samples.len() <= output.structural_output_bound().unwrap());
        assert!(point_indices(&output).contains(&543_210));
    }

    #[test]
    fn invalid_view_parameters_are_rejected() {
        assert_eq!(
            LinearLodView::new(1.0, 1.0, 10.0, 1.0),
            Err(GeometryError::DegenerateViewport)
        );
        assert_eq!(
            LinearLodView::new(0.0, 1.0, f64::NAN, 1.0),
            Err(GeometryError::NonFiniteCoordinate)
        );
    }
}
