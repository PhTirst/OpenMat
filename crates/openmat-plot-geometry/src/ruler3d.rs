use openmat_plot_mir::{AxesPoint3D, AxisDimension, RulerSelection3D, ViewProjection3D};

use crate::GeometryError;

const SILHOUETTE_EPSILON: f32 = 2.0e-5;
const EDGE_LENGTH_EPSILON: f32 = 1.0e-6;
const MIN_VISIBLE_RULER_NDC_LENGTH: f32 = 0.015;

/// One of the four parallel unit-cube edges that can host an axis ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerEdge3D {
    pub axis: AxisDimension,
    pub candidate: u8,
    pub start: AxesPoint3D,
    pub end: AxesPoint3D,
}

/// Returns a stable candidate edge in normalized axes coordinates.
///
/// Candidate bits enumerate the two coordinates orthogonal to `axis`: X uses
/// `y + 2*z`, Y uses `x + 2*z`, and Z uses `x + 2*y`.
///
/// # Errors
/// Returns [`GeometryError`] when `candidate` is outside `0..4`.
pub fn ruler_edge_3d(axis: AxisDimension, candidate: u8) -> Result<RulerEdge3D, GeometryError> {
    if candidate >= RulerSelection3D::CANDIDATE_COUNT {
        return Err(GeometryError::InvalidProjection("3D ruler candidate"));
    }
    let low = f32::from(candidate & 1);
    let high = f32::from((candidate >> 1) & 1);
    let (start, end) = match axis {
        AxisDimension::X => ([0.0, low, high], [1.0, low, high]),
        AxisDimension::Y => ([low, 0.0, high], [low, 1.0, high]),
        AxisDimension::Z => ([low, high, 0.0], [low, high, 1.0]),
    };
    Ok(RulerEdge3D {
        axis,
        candidate,
        start: AxesPoint3D::new(start[0], start[1], start[2])?,
        end: AxesPoint3D::new(end[0], end[1], end[2])?,
    })
}

/// Selects MATLAB-style ruler edges from the projected unit-cube silhouette.
///
/// X and Y rulers prefer the lowest visible silhouette edges while Z prefers
/// the leftmost visible edge. Because all Surface geometry is inside the axes
/// cube, labels offset along these edges' outward normals cannot cross the
/// projected Surface interior.
///
/// # Errors
/// Returns [`GeometryError`] if the matrix cannot project the complete cube.
pub fn select_projected_rulers_3d(
    matrix: ViewProjection3D,
) -> Result<RulerSelection3D, GeometryError> {
    let corners = projected_cube(matrix)?;
    Ok(RulerSelection3D {
        x: select_axis_candidate(AxisDimension::X, &corners)?,
        y: select_axis_candidate(AxisDimension::Y, &corners)?,
        z: select_axis_candidate(AxisDimension::Z, &corners)?,
    })
}

fn select_axis_candidate(
    axis: AxisDimension,
    corners: &[[f32; 2]; 8],
) -> Result<u8, GeometryError> {
    let mut best: Option<(f32, f32, u8)> = None;
    for candidate in 0..RulerSelection3D::CANDIDATE_COUNT {
        let (start_index, end_index) = edge_corner_indices(axis, candidate);
        let start = corners[start_index];
        let end = corners[end_index];
        let delta = [end[0] - start[0], end[1] - start[1]];
        if delta[0].hypot(delta[1]) <= EDGE_LENGTH_EPSILON
            || !is_silhouette_edge(start, end, corners)
        {
            continue;
        }
        let midpoint = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
        let score = match axis {
            AxisDimension::X | AxisDimension::Y => midpoint[1],
            AxisDimension::Z => midpoint[0],
        };
        let length = delta[0].hypot(delta[1]);
        if best.is_none_or(|(best_score, _, best_candidate)| {
            score < best_score - SILHOUETTE_EPSILON
                || ((score - best_score).abs() <= SILHOUETTE_EPSILON && candidate < best_candidate)
        }) {
            best = Some((score, length, candidate));
        }
    }
    best.map(|(_, length, candidate)| {
        if length < MIN_VISIBLE_RULER_NDC_LENGTH {
            RulerSelection3D::HIDDEN_CANDIDATE
        } else {
            candidate
        }
    })
    .ok_or(GeometryError::InvalidProjection(
        "projected 3D ruler silhouette",
    ))
}

fn is_silhouette_edge(start: [f32; 2], end: [f32; 2], corners: &[[f32; 2]; 8]) -> bool {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for point in corners {
        let relative = [point[0] - start[0], point[1] - start[1]];
        let cross = delta[0].mul_add(relative[1], -delta[1] * relative[0]);
        minimum = minimum.min(cross);
        maximum = maximum.max(cross);
    }
    minimum >= -SILHOUETTE_EPSILON || maximum <= SILHOUETTE_EPSILON
}

const fn edge_corner_indices(axis: AxisDimension, candidate: u8) -> (usize, usize) {
    let low = (candidate & 1) as usize;
    let high = ((candidate >> 1) & 1) as usize;
    match axis {
        AxisDimension::X => {
            let start = low * 2 + high * 4;
            (start, start + 1)
        }
        AxisDimension::Y => {
            let start = low + high * 4;
            (start, start + 2)
        }
        AxisDimension::Z => {
            let start = low + high * 2;
            (start, start + 4)
        }
    }
}

fn projected_cube(matrix: ViewProjection3D) -> Result<[[f32; 2]; 8], GeometryError> {
    let mut corners = [[0.0; 2]; 8];
    for (index, projected) in corners.iter_mut().enumerate() {
        let point = [
            if index & 1 == 0 { 0.0 } else { 1.0 },
            if index & 2 == 0 { 0.0 } else { 1.0 },
            if index & 4 == 0 { 0.0 } else { 1.0 },
        ];
        *projected = project_to_ndc(matrix, point)?;
    }
    Ok(corners)
}

fn project_to_ndc(matrix: ViewProjection3D, point: [f32; 3]) -> Result<[f32; 2], GeometryError> {
    let columns = matrix.columns();
    let vector = [point[0], point[1], point[2], 1.0];
    let mut clip = [0.0_f32; 4];
    for (row, output) in clip.iter_mut().enumerate() {
        *output = (0..4)
            .map(|column| columns[column][row] * vector[column])
            .sum();
    }
    if !clip.iter().all(|value| value.is_finite()) || clip[3].abs() <= f32::EPSILON {
        return Err(GeometryError::InvalidProjection("projected 3D ruler cube"));
    }
    let result = [clip[0] / clip[3], clip[1] / clip[3]];
    if result.iter().all(|value| value.is_finite()) {
        Ok(result)
    } else {
        Err(GeometryError::InvalidProjection("projected 3D ruler cube"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OrbitCamera3D, Point3D, Projection3D};

    fn camera(azimuth_degrees: f64) -> OrbitCamera3D {
        OrbitCamera3D::new(
            Point3D {
                x: 0.5,
                y: 0.5,
                z: 0.5,
            },
            3.0,
            azimuth_degrees.to_radians(),
            30_f64.to_radians(),
            Projection3D::Orthographic {
                vertical_span: 1.8,
                near: 0.1,
                far: 10.0,
            },
        )
        .unwrap()
    }

    #[test]
    fn candidate_mapping_covers_all_parallel_cube_edges() {
        let edge = ruler_edge_3d(AxisDimension::X, 3).unwrap();
        assert_eq!(
            [
                edge.start.x.to_bits(),
                edge.start.y.to_bits(),
                edge.start.z.to_bits()
            ],
            [0.0_f32.to_bits(), 1.0_f32.to_bits(), 1.0_f32.to_bits()]
        );
        assert_eq!(
            [
                edge.end.x.to_bits(),
                edge.end.y.to_bits(),
                edge.end.z.to_bits()
            ],
            [1.0_f32.to_bits(), 1.0_f32.to_bits(), 1.0_f32.to_bits()]
        );
        assert!(ruler_edge_3d(AxisDimension::Z, 4).is_err());
    }

    #[test]
    fn matlab_default_view_selects_only_projected_silhouette_edges() {
        let matrix = camera(-45.0).view_projection(4.0 / 3.0).unwrap();
        let selection = select_projected_rulers_3d(matrix).unwrap();
        let corners = projected_cube(matrix).unwrap();
        for axis in [AxisDimension::X, AxisDimension::Y, AxisDimension::Z] {
            let (start, end) = edge_corner_indices(axis, selection.candidate(axis));
            assert!(is_silhouette_edge(corners[start], corners[end], &corners));
        }
    }

    #[test]
    fn orbit_changes_ruler_edges_without_changing_geometry() {
        let first =
            select_projected_rulers_3d(camera(-45.0).view_projection(4.0 / 3.0).unwrap()).unwrap();
        let second =
            select_projected_rulers_3d(camera(135.0).view_projection(4.0 / 3.0).unwrap()).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn top_view_hides_the_degenerate_vertical_ruler() {
        let camera = OrbitCamera3D::new(
            Point3D {
                x: 0.5,
                y: 0.5,
                z: 0.5,
            },
            3.0,
            -45_f64.to_radians(),
            90_f64.to_radians(),
            Projection3D::Orthographic {
                vertical_span: 1.8,
                near: 0.1,
                far: 10.0,
            },
        )
        .unwrap();
        let selection =
            select_projected_rulers_3d(camera.view_projection(4.0 / 3.0).unwrap()).unwrap();
        assert_eq!(selection.z, RulerSelection3D::HIDDEN_CANDIDATE);
        assert!(selection.x < RulerSelection3D::CANDIDATE_COUNT);
        assert!(selection.y < RulerSelection3D::CANDIDATE_COUNT);
    }
}
