use openmat_plot_mir::{
    AxesPoint3D, AxesVector3D, CssPx, LinePattern3D, LineSegment3D, LineSegmentBatch3D, Rgba,
    SurfaceColorInterpolation, SurfaceMesh3D, SurfaceVertex3D,
};

use crate::GeometryError;

/// Finite, strictly increasing data limits used to normalize 3D geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataBounds3D {
    pub x: [f64; 2],
    pub y: [f64; 2],
    pub z: [f64; 2],
}

impl DataBounds3D {
    /// Creates validated 3D limits.
    ///
    /// # Errors
    /// Returns [`GeometryError`] when any axis is non-finite or not strictly increasing.
    pub fn new(x: [f64; 2], y: [f64; 2], z: [f64; 2]) -> Result<Self, GeometryError> {
        validate_bounds("X", x)?;
        validate_bounds("Y", y)?;
        validate_bounds("Z", z)?;
        Ok(Self { x, y, z })
    }

    /// Normalizes one data-space point into the unit Axes cube.
    ///
    /// # Errors
    /// Returns [`GeometryError`] when the normalized point cannot be represented.
    pub fn normalize_point(self, x: f64, y: f64, z: f64) -> Result<AxesPoint3D, GeometryError> {
        let normalized = [
            (x - self.x[0]) / (self.x[1] - self.x[0]),
            (y - self.y[0]) / (self.y[1] - self.y[0]),
            (z - self.z[0]) / (self.z[1] - self.z[0]),
        ];
        narrow_axes_point(normalized)
    }
}

/// Resolved surface colors. Colormap and `CLim` semantics remain in the HIR layer;
/// geometry consumes only the resulting colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceColors<'a> {
    Uniform(Rgba),
    /// One MATLAB `CData` color per source vertex, but held constant over each
    /// grid cell using the first vertex in positive X/Y directions.
    Flat(&'a [Rgba]),
    /// One color per source vertex, smoothly interpolated across triangles.
    Interp(&'a [Rgba]),
}

/// A zero-copy, column-major structured surface grid.
///
/// `x` and `y` are either rectilinear vectors or full matrices shaped like `z`.
/// Matrices use MATLAB column-major order (`row + column * rows`). Non-finite
/// X/Y/Z samples form holes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceGridView<'a> {
    pub rows: usize,
    pub columns: usize,
    pub x: &'a [f64],
    pub y: &'a [f64],
    pub z: &'a [f64],
    pub colors: SurfaceColors<'a>,
}

impl SurfaceGridView<'_> {
    fn validate(self) -> Result<usize, GeometryError> {
        if self.rows == 0 || self.columns == 0 {
            return Err(GeometryError::InvalidGridShape {
                rows: self.rows,
                columns: self.columns,
            });
        }
        let elements = self
            .rows
            .checked_mul(self.columns)
            .ok_or(GeometryError::ArithmeticOverflow("surface element count"))?;
        let vector_coordinates = self.x.len() == self.columns && self.y.len() == self.rows;
        let matrix_coordinates = self.x.len() == elements && self.y.len() == elements;
        if !vector_coordinates && !matrix_coordinates {
            return Err(GeometryError::FieldLengthMismatch {
                field: "surface X/Y coordinates",
                expected: elements,
                actual: self.x.len().max(self.y.len()),
            });
        }
        validate_length("surface Z matrix", elements, self.z.len())?;
        match self.colors {
            SurfaceColors::Uniform(color) => color.validate()?,
            SurfaceColors::Flat(colors) | SurfaceColors::Interp(colors) => {
                validate_length("surface color matrix", elements, colors.len())?;
                for &color in colors {
                    color.validate()?;
                }
            }
        }
        Ok(elements)
    }

    fn color(self, source_index: usize) -> Rgba {
        match self.colors {
            SurfaceColors::Uniform(color) => color,
            SurfaceColors::Flat(colors) | SurfaceColors::Interp(colors) => colors[source_index],
        }
    }

    fn coordinates(self, row: usize, column: usize, source_index: usize) -> (f64, f64) {
        if self.x.len() == self.columns {
            (self.x[column], self.y[row])
        } else {
            (self.x[source_index], self.y[source_index])
        }
    }
}

/// Tessellates a structured surface into render-ready triangles.
///
/// Every cell with four finite corners emits two counter-clockwise triangles.
/// Cells touching NaN or infinity emit no partial face. Vertex normals are the
/// normalized sum of adjacent triangle normals in axes-local space.
///
/// # Errors
/// Returns [`GeometryError`] for invalid shape, lengths, bounds, colors,
/// arithmetic overflow, or more than `u32::MAX` finite vertices.
pub fn tessellate_surface_grid(
    grid: SurfaceGridView<'_>,
    bounds: DataBounds3D,
) -> Result<SurfaceMesh3D, GeometryError> {
    let elements = grid.validate()?;
    let mut source_to_vertex = vec![None; elements];
    let mut vertices = Vec::with_capacity(elements);

    for column in 0..grid.columns {
        for row in 0..grid.rows {
            let source_index = source_index(row, column, grid.rows)?;
            let (x, y) = grid.coordinates(row, column, source_index);
            let z = grid.z[source_index];
            if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                continue;
            }
            let vertex_index = u32::try_from(vertices.len())
                .map_err(|_| GeometryError::ArithmeticOverflow("surface vertex index"))?;
            source_to_vertex[source_index] = Some(vertex_index);
            vertices.push(SurfaceVertex3D {
                position: bounds.normalize_point(x, y, z)?,
                normal: AxesVector3D::default(),
                color: grid.color(source_index),
            });
        }
    }

    let cell_count = grid
        .rows
        .saturating_sub(1)
        .checked_mul(grid.columns.saturating_sub(1))
        .ok_or(GeometryError::ArithmeticOverflow("surface cell count"))?;
    let index_capacity = cell_count
        .checked_mul(6)
        .ok_or(GeometryError::ArithmeticOverflow("surface index count"))?;
    let mut indices = Vec::with_capacity(index_capacity);
    for column in 0..grid.columns.saturating_sub(1) {
        for row in 0..grid.rows.saturating_sub(1) {
            let a = source_to_vertex[source_index(row, column, grid.rows)?];
            let b = source_to_vertex[source_index(row + 1, column, grid.rows)?];
            let c = source_to_vertex[source_index(row, column + 1, grid.rows)?];
            let d = source_to_vertex[source_index(row + 1, column + 1, grid.rows)?];
            if let (Some(a), Some(b), Some(c), Some(d)) = (a, b, c, d) {
                indices.extend([a, c, d, a, d, b]);
            }
        }
    }

    accumulate_normals(&mut vertices, &indices);
    let color_interpolation = if matches!(grid.colors, SurfaceColors::Interp(_)) {
        SurfaceColorInterpolation::Smooth
    } else {
        SurfaceColorInterpolation::Flat
    };
    let mesh = SurfaceMesh3D {
        vertices,
        indices,
        color_interpolation,
    };
    mesh.validate()?;
    Ok(mesh)
}

/// Builds each finite row and column edge once for the 3D screen-space stroke
/// pipeline. Flat colors use the first endpoint in the positive X/Y direction,
/// matching MATLAB's structured `CData` edge rule.
///
/// # Errors
/// Returns [`GeometryError`] for invalid shape, bounds, colors, or arithmetic.
pub fn surface_grid_edges(
    grid: SurfaceGridView<'_>,
    bounds: DataBounds3D,
    width_css_px: CssPx,
    pattern: LinePattern3D,
) -> Result<LineSegmentBatch3D, GeometryError> {
    grid.validate()?;
    let horizontal = grid
        .rows
        .checked_mul(grid.columns.saturating_sub(1))
        .ok_or(GeometryError::ArithmeticOverflow(
            "surface horizontal edge count",
        ))?;
    let vertical = grid
        .columns
        .checked_mul(grid.rows.saturating_sub(1))
        .ok_or(GeometryError::ArithmeticOverflow(
            "surface vertical edge count",
        ))?;
    let mut segments = Vec::with_capacity(
        horizontal
            .checked_add(vertical)
            .ok_or(GeometryError::ArithmeticOverflow("surface edge count"))?,
    );

    for column in 0..grid.columns {
        for row in 0..grid.rows {
            if column + 1 < grid.columns {
                push_surface_edge(&mut segments, grid, bounds, row, column, row, column + 1)?;
            }
            if row + 1 < grid.rows {
                push_surface_edge(&mut segments, grid, bounds, row, column, row + 1, column)?;
            }
        }
    }
    let batch = LineSegmentBatch3D {
        segments,
        width_css_px,
        pattern,
    };
    batch.validate()?;
    Ok(batch)
}

#[allow(clippy::too_many_arguments)]
fn push_surface_edge(
    segments: &mut Vec<LineSegment3D>,
    grid: SurfaceGridView<'_>,
    bounds: DataBounds3D,
    start_row: usize,
    start_column: usize,
    end_row: usize,
    end_column: usize,
) -> Result<(), GeometryError> {
    let start_index = source_index(start_row, start_column, grid.rows)?;
    let end_index = source_index(end_row, end_column, grid.rows)?;
    let start_xy = grid.coordinates(start_row, start_column, start_index);
    let end_xy = grid.coordinates(end_row, end_column, end_index);
    let start = (start_xy.0, start_xy.1, grid.z[start_index]);
    let end = (end_xy.0, end_xy.1, grid.z[end_index]);
    if [start.0, start.1, start.2, end.0, end.1, end.2]
        .iter()
        .all(|value| value.is_finite())
    {
        segments.push(LineSegment3D {
            start: bounds.normalize_point(start.0, start.1, start.2)?,
            end: bounds.normalize_point(end.0, end.1, end.2)?,
            color: grid.color(start_index),
        });
    }
    Ok(())
}

fn validate_bounds(axis: &'static str, bounds: [f64; 2]) -> Result<(), GeometryError> {
    let span = bounds[1] - bounds[0];
    if bounds[0].is_finite() && bounds[1].is_finite() && span.is_finite() && span > 0.0 {
        Ok(())
    } else {
        Err(GeometryError::DegenerateBounds(axis))
    }
}

fn validate_length(
    field: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), GeometryError> {
    if expected == actual {
        Ok(())
    } else {
        Err(GeometryError::FieldLengthMismatch {
            field,
            expected,
            actual,
        })
    }
}

fn source_index(row: usize, column: usize, rows: usize) -> Result<usize, GeometryError> {
    column
        .checked_mul(rows)
        .and_then(|base| base.checked_add(row))
        .ok_or(GeometryError::ArithmeticOverflow("surface source index"))
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_axes_point(values: [f64; 3]) -> Result<AxesPoint3D, GeometryError> {
    let narrowed = [values[0] as f32, values[1] as f32, values[2] as f32];
    if narrowed.iter().all(|value| value.is_finite()) {
        AxesPoint3D::new(narrowed[0], narrowed[1], narrowed[2]).map_err(GeometryError::from)
    } else {
        Err(GeometryError::ArithmeticOverflow(
            "surface coordinate normalization",
        ))
    }
}

fn accumulate_normals(vertices: &mut [SurfaceVertex3D], indices: &[u32]) {
    let mut sums = vec![[0.0_f64; 3]; vertices.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            usize::try_from(triangle[0]).expect("validated u32 index"),
            usize::try_from(triangle[1]).expect("validated u32 index"),
            usize::try_from(triangle[2]).expect("validated u32 index"),
        ];
        let normal = face_normal(
            vertices[a].position,
            vertices[b].position,
            vertices[c].position,
        );
        for index in [a, b, c] {
            sums[index][0] += normal[0];
            sums[index][1] += normal[1];
            sums[index][2] += normal[2];
        }
    }
    for (vertex, sum) in vertices.iter_mut().zip(sums) {
        vertex.normal = normalized_normal(sum);
    }
}

#[allow(clippy::cast_possible_truncation)]
fn normalized_normal(sum: [f64; 3]) -> AxesVector3D {
    let length = sum[0].hypot(sum[1]).hypot(sum[2]);
    let normal = if length.is_finite() && length > f64::EPSILON {
        [sum[0] / length, sum[1] / length, sum[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    };
    AxesVector3D {
        x: normal[0] as f32,
        y: normal[1] as f32,
        z: normal[2] as f32,
    }
}

fn face_normal(a: AxesPoint3D, b: AxesPoint3D, c: AxesPoint3D) -> [f64; 3] {
    let ab = [
        f64::from(b.x - a.x),
        f64::from(b.y - a.y),
        f64::from(b.z - a.z),
    ];
    let ac = [
        f64::from(c.x - a.x),
        f64::from(c.y - a.y),
        f64::from(c.z - a.z),
    ];
    [
        ab[1].mul_add(ac[2], -ab[2] * ac[1]),
        ab[2].mul_add(ac[0], -ab[0] * ac[2]),
        ab[0].mul_add(ac[1], -ab[1] * ac[0]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> DataBounds3D {
        DataBounds3D::new([10.0, 20.0], [-2.0, 2.0], [100.0, 200.0]).unwrap()
    }

    #[test]
    fn column_major_grid_emits_two_ccw_triangles_and_positive_normals() {
        let mesh = tessellate_surface_grid(
            SurfaceGridView {
                rows: 2,
                columns: 2,
                x: &[10.0, 20.0],
                y: &[-2.0, 2.0],
                z: &[100.0, 100.0, 100.0, 100.0],
                colors: SurfaceColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            },
            bounds(),
        )
        .unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, [0, 2, 3, 0, 3, 1]);
        assert!(mesh.vertices.iter().all(|vertex| vertex.normal.z > 0.999));
        assert_eq!(
            mesh.vertices[3].position,
            AxesPoint3D::new(1.0, 1.0, 0.0).unwrap()
        );
    }

    #[test]
    fn nonfinite_samples_remove_whole_touching_cells_without_fake_vertices() {
        let mesh = tessellate_surface_grid(
            SurfaceGridView {
                rows: 2,
                columns: 3,
                x: &[10.0, 15.0, 20.0],
                y: &[-2.0, 2.0],
                z: &[100.0, 100.0, f64::NAN, 100.0, 200.0, 200.0],
                colors: SurfaceColors::Uniform(Rgba::new(1.0, 1.0, 1.0, 1.0).unwrap()),
            },
            bounds(),
        )
        .unwrap();
        assert_eq!(mesh.vertices.len(), 5);
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn flat_colors_keep_source_order_and_first_vertex_for_both_cell_triangles() {
        let colors = [
            Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
            Rgba::new(0.0, 1.0, 0.0, 1.0).unwrap(),
            Rgba::new(0.0, 0.0, 1.0, 1.0).unwrap(),
            Rgba::new(1.0, 1.0, 0.0, 1.0).unwrap(),
        ];
        let mesh = tessellate_surface_grid(
            SurfaceGridView {
                rows: 2,
                columns: 2,
                x: &[10.0, 20.0],
                y: &[-2.0, 2.0],
                z: &[100.0, 100.0, 100.0, 100.0],
                colors: SurfaceColors::Flat(&colors),
            },
            bounds(),
        )
        .unwrap();
        assert_eq!(
            mesh.vertices
                .iter()
                .map(|vertex| vertex.color)
                .collect::<Vec<_>>(),
            colors
        );
        assert_eq!(mesh.indices, [0, 2, 3, 0, 3, 1]);
        assert_eq!(mesh.indices[0], mesh.indices[3]);
        assert_eq!(mesh.vertices[0].color, colors[0]);
    }

    #[test]
    fn grid_edges_are_unique_and_use_each_positive_direction_start_color() {
        let colors = [
            Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap(),
            Rgba::new(0.0, 1.0, 0.0, 1.0).unwrap(),
            Rgba::new(0.0, 0.0, 1.0, 1.0).unwrap(),
            Rgba::new(1.0, 1.0, 0.0, 1.0).unwrap(),
        ];
        let edges = surface_grid_edges(
            SurfaceGridView {
                rows: 2,
                columns: 2,
                x: &[10.0, 20.0],
                y: &[-2.0, 2.0],
                z: &[100.0, 100.0, 100.0, 100.0],
                colors: SurfaceColors::Flat(&colors),
            },
            bounds(),
            CssPx::new(0.5).unwrap(),
            LinePattern3D::Solid,
        )
        .unwrap();
        assert_eq!(edges.segments.len(), 4);
        assert_eq!(
            edges
                .segments
                .iter()
                .map(|segment| segment.color)
                .collect::<Vec<_>>(),
            [colors[0], colors[0], colors[1], colors[2]]
        );
        assert!(
            edges
                .segments
                .iter()
                .all(|segment| segment.start != segment.end)
        );
    }

    #[test]
    fn grid_shape_lengths_and_bounds_fail_structurally() {
        let error = tessellate_surface_grid(
            SurfaceGridView {
                rows: 2,
                columns: 2,
                x: &[0.0],
                y: &[0.0, 1.0],
                z: &[0.0; 4],
                colors: SurfaceColors::Uniform(Rgba::TRANSPARENT),
            },
            DataBounds3D::new([0.0, 1.0], [0.0, 1.0], [0.0, 1.0]).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            GeometryError::FieldLengthMismatch {
                field: "surface X/Y coordinates",
                expected: 4,
                actual: 2
            }
        ));
        assert_eq!(
            DataBounds3D::new([1.0, 1.0], [0.0, 1.0], [0.0, 1.0]),
            Err(GeometryError::DegenerateBounds("X"))
        );
        assert_eq!(
            DataBounds3D::new([-f64::MAX, f64::MAX], [0.0, 1.0], [0.0, 1.0]),
            Err(GeometryError::DegenerateBounds("X"))
        );
    }

    #[test]
    fn full_coordinate_matrices_preserve_warped_mesh_positions() {
        let mesh = tessellate_surface_grid(
            SurfaceGridView {
                rows: 2,
                columns: 2,
                x: &[10.0, 11.0, 19.0, 20.0],
                y: &[-2.0, 1.0, -1.0, 2.0],
                z: &[100.0; 4],
                colors: SurfaceColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            },
            bounds(),
        )
        .unwrap();
        assert!((mesh.vertices[1].position.x - 0.1).abs() < 1.0e-12);
        assert!((mesh.vertices[1].position.y - 0.75).abs() < 1.0e-12);
        assert!((mesh.vertices[2].position.x - 0.9).abs() < 1.0e-12);
        assert!((mesh.vertices[2].position.y - 0.25).abs() < 1.0e-12);
    }
}
