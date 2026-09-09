use openmat_plot_mir::AxesPoint;

use crate::{GeometryError, LodSample, SeriesGeometryInput};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataPoint {
    pub source_index: usize,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineRun {
    pub points: Vec<DataPoint>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxesLineRun {
    pub points: Vec<AxesPoint>,
}

/// Rebuilds finite line runs from an ordered LOD selection.
///
/// Break samples are structural separators and are never forwarded to f32 geometry.
#[must_use]
pub fn line_runs_from_lod(samples: &[LodSample]) -> Vec<LineRun> {
    let mut runs = Vec::new();
    let mut current = LineRun::default();
    for sample in samples {
        match *sample {
            LodSample::Point { source_index, x, y } => {
                current.points.push(DataPoint { source_index, x, y });
            }
            LodSample::Break { .. } if !current.points.is_empty() => {
                runs.push(std::mem::take(&mut current));
            }
            LodSample::Break { .. } => {}
        }
    }
    if !current.points.is_empty() {
        runs.push(current);
    }
    runs
}

/// Splits a series at every NaN or infinity without converting source precision.
///
/// # Errors
/// Returns [`GeometryError`] for invalid series views or implicit-index overflow.
pub fn split_line_runs(input: SeriesGeometryInput<'_>) -> Result<Vec<LineRun>, GeometryError> {
    input.validate()?;
    let mut runs = Vec::new();
    let mut current = LineRun::default();
    for index in 0..input.y.len() {
        let (x, y) = input.point(index)?;
        if x.is_finite() && y.is_finite() {
            current.points.push(DataPoint {
                source_index: index,
                x: canonical_zero(x),
                y: canonical_zero(y),
            });
        } else if !current.points.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.points.is_empty() {
        runs.push(current);
    }
    Ok(runs)
}

/// Converts finite f64 runs through a caller-supplied axes-local transform.
///
/// # Errors
/// Returns [`GeometryError`] when mapping fails or produces a non-finite point.
pub fn axes_line_runs(
    runs: &[LineRun],
    mut mapper: impl FnMut(f64, f64) -> Result<AxesPoint, GeometryError>,
) -> Result<Vec<AxesLineRun>, GeometryError> {
    runs.iter()
        .map(|run| {
            let points = run
                .points
                .iter()
                .map(|point| mapper(point.x, point.y))
                .collect::<Result<Vec<_>, _>>()?;
            if points
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite())
            {
                return Err(GeometryError::NonFiniteCoordinate);
            }
            Ok(AxesLineRun { points })
        })
        .collect()
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NumericView;

    #[test]
    fn nan_and_infinity_split_runs_without_reaching_f32_geometry() {
        let values = [1.0, 2.0, f64::NAN, 3.0, f64::INFINITY, 4.0];
        let runs = split_line_runs(SeriesGeometryInput {
            x: None,
            y: NumericView::F64(&values),
        })
        .unwrap();
        assert_eq!(
            runs.iter().map(|run| run.points.len()).collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
        assert_eq!(runs[1].points[0].source_index, 3);
        assert_eq!(runs[2].points[0].source_index, 5);
    }

    #[test]
    fn empty_input_has_no_runs() {
        let runs = split_line_runs(SeriesGeometryInput {
            x: None,
            y: NumericView::F64(&[]),
        })
        .unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn lod_breaks_rebuild_runs_without_non_finite_geometry() {
        let samples = [
            LodSample::Point {
                source_index: 0,
                x: -0.0,
                y: 1.0,
            },
            LodSample::Break {
                source_index: 1,
                kind: crate::LodBreakKind::NaN,
            },
            LodSample::Break {
                source_index: 2,
                kind: crate::LodBreakKind::Infinity,
            },
            LodSample::Point {
                source_index: 3,
                x: 2.0,
                y: 3.0,
            },
        ];
        let runs = line_runs_from_lod(&samples);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].points[0].source_index, 0);
        assert_eq!(runs[1].points[0].source_index, 3);
    }
}
