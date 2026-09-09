use openmat_plot_mir::{AxesPoint, AxesRect};

use crate::AxesLineRun;

#[must_use]
pub fn clip_line_runs(runs: &[AxesLineRun], clip: AxesRect) -> Vec<AxesLineRun> {
    let mut output = Vec::new();
    for run in runs {
        if run.points.len() == 1 {
            if clip.contains(run.points[0]) {
                output.push(run.clone());
            }
            continue;
        }
        let mut current = AxesLineRun::default();
        for segment in run.points.windows(2) {
            if let Some((start, end)) = clip_segment(segment[0], segment[1], clip) {
                if current.points.last().copied() != Some(start) {
                    if !current.points.is_empty() {
                        output.push(std::mem::take(&mut current));
                    }
                    current.points.push(start);
                }
                current.points.push(end);
            } else if !current.points.is_empty() {
                output.push(std::mem::take(&mut current));
            }
        }
        if !current.points.is_empty() {
            output.push(current);
        }
    }
    output
}

fn clip_segment(
    start: AxesPoint,
    end: AxesPoint,
    clip: AxesRect,
) -> Option<(AxesPoint, AxesPoint)> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let p = [-dx, dx, -dy, dy];
    let q = [
        start.x - clip.min.x,
        clip.max.x - start.x,
        start.y - clip.min.y,
        clip.max.y - start.y,
    ];
    let mut enter = 0.0_f32;
    let mut leave = 1.0_f32;
    for (&direction, &distance) in p.iter().zip(&q) {
        if direction == 0.0 {
            if distance < 0.0 {
                return None;
            }
        } else {
            let ratio = distance / direction;
            if direction < 0.0 {
                enter = enter.max(ratio);
            } else {
                leave = leave.min(ratio);
            }
            if enter > leave {
                return None;
            }
        }
    }
    Some((
        AxesPoint {
            x: start.x + enter * dx,
            y: start.y + enter * dy,
        },
        AxesPoint {
            x: start.x + leave * dx,
            y: start.y + leave * dy,
        },
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn clips_crossing_segments_and_splits_reentry() {
        let runs = vec![AxesLineRun {
            points: vec![
                AxesPoint { x: -1.0, y: 0.5 },
                AxesPoint { x: 0.5, y: 0.5 },
                AxesPoint { x: 2.0, y: 0.5 },
                AxesPoint { x: 0.5, y: 0.75 },
            ],
        }];
        let clipped = clip_line_runs(&runs, AxesRect::unit());
        assert_eq!(clipped.len(), 2);
        assert_eq!(clipped[0].points.first().unwrap().x, 0.0);
        assert_eq!(clipped[0].points.last().unwrap().x, 1.0);
        assert_eq!(clipped[1].points.first().unwrap().x, 1.0);
        assert!(
            clipped
                .iter()
                .flat_map(|run| &run.points)
                .all(|point| AxesRect::unit().contains(*point))
        );
    }
}
