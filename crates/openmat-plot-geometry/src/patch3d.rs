use std::collections::BTreeMap;

use openmat_plot_mir::{
    AxesPoint, AxesPoint3D, AxesVector3D, CssPx, LinePattern3D, LineSegment3D, LineSegmentBatch3D,
    Rgba, SurfaceColorInterpolation, SurfaceMesh3D, SurfaceVertex3D, TriangleMesh2D,
    TriangleVertex2D,
};

use crate::{DataBounds3D, GeometryError};

/// Resolved MATLAB Patch colors. Colormap lookup remains a HIR concern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PatchColors<'a> {
    Uniform(Rgba),
    FlatFaces(&'a [Rgba]),
    FlatVertices(&'a [Rgba]),
    InterpVertices(&'a [Rgba]),
}

/// A zero-copy, column-major MATLAB Faces/Vertices patch mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatchMeshView<'a> {
    pub face_rows: usize,
    pub face_columns: usize,
    pub faces: &'a [f64],
    pub vertex_rows: usize,
    pub vertex_columns: usize,
    pub vertices: &'a [f64],
    /// Optional N-by-3 MATLAB `VertexNormals` in column-major data coordinates.
    pub vertex_normals: Option<&'a [f64]>,
    pub colors: PatchColors<'a>,
    /// Average adjacent triangle normals for each source vertex.
    pub smooth_normals: bool,
}

/// One unique topological edge of a 2D Patch in normalized Axes coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatchEdge2D {
    pub start: AxesPoint,
    pub end: AxesPoint,
    pub color: Rgba,
}

impl PatchMeshView<'_> {
    fn validate(self) -> Result<(), GeometryError> {
        if self.face_rows == 0 || self.face_columns < 3 {
            return Err(GeometryError::InvalidPatch(
                "Faces must contain at least one row and three columns",
            ));
        }
        if self.vertex_rows == 0 || !(2..=3).contains(&self.vertex_columns) {
            return Err(GeometryError::InvalidPatch(
                "Vertices must be a non-empty N-by-2 or N-by-3 matrix",
            ));
        }
        validate_matrix_len(
            "patch Faces matrix",
            self.face_rows,
            self.face_columns,
            self.faces.len(),
        )?;
        validate_matrix_len(
            "patch Vertices matrix",
            self.vertex_rows,
            self.vertex_columns,
            self.vertices.len(),
        )?;
        if self.vertices.iter().any(|value| !value.is_finite()) {
            return Err(GeometryError::InvalidPatch(
                "Vertices must contain finite coordinates",
            ));
        }
        if let Some(normals) = self.vertex_normals {
            validate_matrix_len(
                "patch VertexNormals matrix",
                self.vertex_rows,
                3,
                normals.len(),
            )?;
            if self.vertex_columns != 3 || normals.iter().any(|value| !value.is_finite()) {
                return Err(GeometryError::InvalidPatch(
                    "VertexNormals must be a finite N-by-3 matrix for a 3D Patch",
                ));
            }
        }
        match self.colors {
            PatchColors::Uniform(color) => color.validate()?,
            PatchColors::FlatFaces(colors) => {
                validate_len("patch face colors", self.face_rows, colors.len())?;
                validate_colors(colors)?;
            }
            PatchColors::FlatVertices(colors) | PatchColors::InterpVertices(colors) => {
                validate_len("patch vertex colors", self.vertex_rows, colors.len())?;
                validate_colors(colors)?;
            }
        }
        for row in 0..self.face_rows {
            let face = self.face(row)?;
            if face.len() < 3 {
                return Err(GeometryError::InvalidPatch(
                    "each face must contain at least three vertices",
                ));
            }
        }
        Ok(())
    }

    fn face(self, row: usize) -> Result<Vec<usize>, GeometryError> {
        let mut indices = Vec::with_capacity(self.face_columns);
        let mut padding = false;
        for column in 0..self.face_columns {
            let source = matrix_index(row, column, self.face_rows)?;
            let value = self.faces[source];
            if value.is_nan() {
                padding = true;
                continue;
            }
            if padding {
                return Err(GeometryError::InvalidPatch(
                    "NaN face padding must be trailing",
                ));
            }
            if !value.is_finite() || value < 1.0 || value.fract() != 0.0 {
                return Err(GeometryError::InvalidPatch(
                    "Faces must contain positive one-based integer indices",
                ));
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let index = value as usize - 1;
            if index >= self.vertex_rows {
                return Err(GeometryError::InvalidPatch(
                    "Faces index exceeds the Vertices row count",
                ));
            }
            if indices.last().copied() != Some(index) {
                indices.push(index);
            }
        }
        if indices.len() > 1 && indices.first() == indices.last() {
            indices.pop();
        }
        Ok(indices)
    }

    fn point(self, index: usize, bounds: DataBounds3D) -> Result<AxesPoint3D, GeometryError> {
        let x = self.vertices[index];
        let y = self.vertices[index + self.vertex_rows];
        let z = if self.vertex_columns == 3 {
            self.vertices[index + self.vertex_rows * 2]
        } else {
            0.0
        };
        bounds.normalize_point(x, y, z)
    }

    fn color(self, face: usize, vertex: usize) -> Rgba {
        match self.colors {
            PatchColors::Uniform(color) => color,
            PatchColors::FlatFaces(colors) => colors[face],
            PatchColors::FlatVertices(colors) | PatchColors::InterpVertices(colors) => {
                colors[vertex]
            }
        }
    }

    fn explicit_normal(self, vertex: usize, bounds: DataBounds3D) -> Option<AxesVector3D> {
        let normals = self.vertex_normals?;
        let transformed = [
            normals[vertex] * (bounds.x[1] - bounds.x[0]),
            normals[vertex + self.vertex_rows] * (bounds.y[1] - bounds.y[0]),
            normals[vertex + self.vertex_rows * 2] * (bounds.z[1] - bounds.z[0]),
        ];
        let length_squared = transformed[0].mul_add(
            transformed[0],
            transformed[1].mul_add(transformed[1], transformed[2] * transformed[2]),
        );
        (length_squared > f64::EPSILON).then(|| normalized_vector(transformed))
    }
}

/// Tessellates arbitrary simple Patch polygons with ear clipping.
///
/// Concave faces are supported. Vertices are duplicated per emitted triangle so
/// MATLAB flat face colors remain exact and do not bleed across shared faces.
///
/// # Errors
/// Returns [`GeometryError`] for invalid connectivity, shapes, colors, bounds,
/// self-intersecting faces, or arithmetic overflow. Geometrically degenerate
/// faces are ignored so one zero-area face cannot suppress the remaining mesh.
pub fn tessellate_patch_mesh(
    patch: PatchMeshView<'_>,
    bounds: DataBounds3D,
) -> Result<SurfaceMesh3D, GeometryError> {
    patch.validate()?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut source_indices = Vec::new();

    for face_row in 0..patch.face_rows {
        let face = patch.face(face_row)?;
        let points = face
            .iter()
            .copied()
            .map(|index| patch.point(index, bounds))
            .collect::<Result<Vec<_>, _>>()?;
        for triangle in triangulate_face(&points)? {
            let first = u32::try_from(vertices.len())
                .map_err(|_| GeometryError::ArithmeticOverflow("patch vertex index"))?;
            let normal = triangle_normal(
                points[triangle[0]],
                points[triangle[1]],
                points[triangle[2]],
            );
            let flat_source = face[triangle[0]];
            for local in triangle {
                let source = face[local];
                let color_source = if matches!(patch.colors, PatchColors::FlatVertices(_)) {
                    flat_source
                } else {
                    source
                };
                vertices.push(SurfaceVertex3D {
                    position: points[local],
                    normal: patch.explicit_normal(source, bounds).unwrap_or(normal),
                    color: patch.color(face_row, color_source),
                });
                source_indices.push(source);
            }
            indices.extend([first, first + 1, first + 2]);
        }
    }

    if patch.smooth_normals && patch.vertex_normals.is_none() {
        smooth_patch_normals(&mut vertices, &source_indices, patch.vertex_rows);
    }

    let mesh = SurfaceMesh3D {
        vertices,
        indices,
        color_interpolation: if matches!(patch.colors, PatchColors::InterpVertices(_)) {
            SurfaceColorInterpolation::Smooth
        } else {
            SurfaceColorInterpolation::Flat
        },
    };
    mesh.validate()?;
    Ok(mesh)
}

fn smooth_patch_normals(
    vertices: &mut [SurfaceVertex3D],
    source_indices: &[usize],
    source_count: usize,
) {
    let mut sums = vec![[0.0_f64; 3]; source_count];
    for (vertex, source) in vertices.iter().zip(source_indices.iter().copied()) {
        sums[source][0] += f64::from(vertex.normal.x);
        sums[source][1] += f64::from(vertex.normal.y);
        sums[source][2] += f64::from(vertex.normal.z);
    }
    for (vertex, source) in vertices.iter_mut().zip(source_indices.iter().copied()) {
        vertex.normal = normalized_vector(sums[source]);
    }
}

/// Tessellates an N-by-2 Patch directly into the 2D triangle MIR.
///
/// # Errors
/// Returns [`GeometryError`] for invalid Patch data or bounds.
pub fn tessellate_patch_mesh_2d(
    patch: PatchMeshView<'_>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) -> Result<TriangleMesh2D, GeometryError> {
    if patch.vertex_columns != 2 {
        return Err(GeometryError::InvalidPatch(
            "2D Patch tessellation requires N-by-2 Vertices",
        ));
    }
    let mesh = tessellate_patch_mesh(patch, DataBounds3D::new(x_bounds, y_bounds, [-1.0, 1.0])?)?;
    let output = TriangleMesh2D {
        vertices: mesh
            .vertices
            .into_iter()
            .map(|vertex| {
                Ok(TriangleVertex2D {
                    position: AxesPoint::new(vertex.position.x, vertex.position.y)?,
                    color: vertex.color,
                })
            })
            .collect::<Result<Vec<_>, openmat_plot_mir::MirError>>()?,
        indices: mesh.indices,
    };
    output.validate()?;
    Ok(output)
}

/// Builds every topological Patch edge once for the screen-space AA line path.
///
/// # Errors
/// Returns [`GeometryError`] for invalid mesh data, colors, bounds, or width.
pub fn patch_mesh_edges(
    patch: PatchMeshView<'_>,
    bounds: DataBounds3D,
    width_css_px: CssPx,
    pattern: LinePattern3D,
) -> Result<LineSegmentBatch3D, GeometryError> {
    patch.validate()?;
    let mut edges = BTreeMap::<(usize, usize), (usize, usize, usize)>::new();
    for face_row in 0..patch.face_rows {
        let face = patch.face(face_row)?;
        for index in 0..face.len() {
            let start = face[index];
            let end = face[(index + 1) % face.len()];
            if start == end {
                continue;
            }
            let key = if start < end {
                (start, end)
            } else {
                (end, start)
            };
            edges.entry(key).or_insert((face_row, start, end));
        }
    }
    let mut segments = Vec::with_capacity(edges.len());
    for (_key, (face_row, start, end)) in edges {
        segments.push(LineSegment3D {
            start: patch.point(start, bounds)?,
            end: patch.point(end, bounds)?,
            color: patch.color(face_row, start),
        });
    }
    let batch = LineSegmentBatch3D {
        segments,
        width_css_px,
        pattern,
    };
    batch.validate()?;
    Ok(batch)
}

/// Builds unique N-by-2 Patch edges for 2D stroke tessellation.
///
/// # Errors
/// Returns [`GeometryError`] for invalid Patch data or bounds.
pub fn patch_mesh_edges_2d(
    patch: PatchMeshView<'_>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) -> Result<Vec<PatchEdge2D>, GeometryError> {
    if patch.vertex_columns != 2 {
        return Err(GeometryError::InvalidPatch(
            "2D Patch edges require N-by-2 Vertices",
        ));
    }
    let edges = patch_mesh_edges(
        patch,
        DataBounds3D::new(x_bounds, y_bounds, [-1.0, 1.0])?,
        CssPx::new(1.0)?,
        LinePattern3D::Solid,
    )?;
    edges
        .segments
        .into_iter()
        .map(|edge| {
            Ok(PatchEdge2D {
                start: AxesPoint::new(edge.start.x, edge.start.y)?,
                end: AxesPoint::new(edge.end.x, edge.end.y)?,
                color: edge.color,
            })
        })
        .collect::<Result<Vec<_>, openmat_plot_mir::MirError>>()
        .map_err(GeometryError::from)
}

fn triangulate_face(points: &[AxesPoint3D]) -> Result<Vec<[usize; 3]>, GeometryError> {
    if points.len() < 3 {
        return Err(GeometryError::InvalidPatch(
            "face has fewer than three points",
        ));
    }
    let projected = project_dominant_plane(points);
    let mut remaining = simplify_face_vertices(&projected);
    if remaining.len() < 3 {
        return Ok(Vec::new());
    }
    let simplified = remaining
        .iter()
        .map(|&index| projected[index])
        .collect::<Vec<_>>();
    let area = signed_area(&simplified);
    if !area.is_finite() {
        return Err(GeometryError::InvalidPatch("face area is not finite"));
    }
    if area.abs() <= 1.0e-12 {
        return Ok(Vec::new());
    }
    let orientation = area.signum();
    let mut triangles = Vec::with_capacity(points.len() - 2);
    let iteration_limit = points
        .len()
        .checked_mul(points.len())
        .ok_or(GeometryError::ArithmeticOverflow("patch ear clipping"))?;
    let mut iterations = 0;

    while remaining.len() > 3 {
        let mut ear = None;
        for cursor in 0..remaining.len() {
            let previous = remaining[(cursor + remaining.len() - 1) % remaining.len()];
            let current = remaining[cursor];
            let next = remaining[(cursor + 1) % remaining.len()];
            if cross2(projected[previous], projected[current], projected[next]) * orientation
                <= 1.0e-12
            {
                continue;
            }
            let contains_point = remaining.iter().copied().any(|candidate| {
                candidate != previous
                    && candidate != current
                    && candidate != next
                    && point_in_triangle(
                        projected[candidate],
                        projected[previous],
                        projected[current],
                        projected[next],
                        orientation,
                    )
            });
            if !contains_point {
                ear = Some((cursor, [previous, current, next]));
                break;
            }
        }
        let Some((cursor, triangle)) = ear else {
            return Err(GeometryError::InvalidPatch(
                "face is self-intersecting or cannot be triangulated",
            ));
        };
        triangles.push(triangle);
        remaining.remove(cursor);
        iterations += 1;
        if iterations > iteration_limit {
            return Err(GeometryError::InvalidPatch(
                "face triangulation exceeded its iteration limit",
            ));
        }
    }
    triangles.push([remaining[0], remaining[1], remaining[2]]);
    Ok(triangles)
}

fn simplify_face_vertices(points: &[[f64; 2]]) -> Vec<usize> {
    const EPSILON: f64 = 1.0e-12;
    let mut indices = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        if indices
            .last()
            .is_some_and(|&previous| points_are_close(points[previous], points[index], EPSILON))
        {
            continue;
        }
        indices.push(index);
    }
    if indices.len() > 1
        && points_are_close(
            points[indices[0]],
            points[*indices.last().unwrap()],
            EPSILON,
        )
    {
        indices.pop();
    }

    loop {
        if indices.len() <= 3 {
            break;
        }
        let removable = (0..indices.len()).find(|&cursor| {
            let previous = points[indices[(cursor + indices.len() - 1) % indices.len()]];
            let current = points[indices[cursor]];
            let next = points[indices[(cursor + 1) % indices.len()]];
            let incoming = [current[0] - previous[0], current[1] - previous[1]];
            let outgoing = [next[0] - current[0], next[1] - current[1]];
            cross2(previous, current, next).abs() <= EPSILON
                && incoming[0].mul_add(outgoing[0], incoming[1] * outgoing[1]) >= -EPSILON
        });
        let Some(cursor) = removable else {
            break;
        };
        indices.remove(cursor);
    }
    indices
}

fn points_are_close(left: [f64; 2], right: [f64; 2], epsilon: f64) -> bool {
    (left[0] - right[0]).abs() <= epsilon && (left[1] - right[1]).abs() <= epsilon
}

fn project_dominant_plane(points: &[AxesPoint3D]) -> Vec<[f64; 2]> {
    let mut normal = [0.0_f64; 3];
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        normal[0] += f64::from(current.y - next.y) * f64::from(current.z + next.z);
        normal[1] += f64::from(current.z - next.z) * f64::from(current.x + next.x);
        normal[2] += f64::from(current.x - next.x) * f64::from(current.y + next.y);
    }
    let drop_axis = if normal[0].abs() >= normal[1].abs() && normal[0].abs() >= normal[2].abs() {
        0
    } else if normal[1].abs() >= normal[2].abs() {
        1
    } else {
        2
    };
    points
        .iter()
        .map(|point| match drop_axis {
            0 => [f64::from(point.y), f64::from(point.z)],
            1 => [f64::from(point.x), f64::from(point.z)],
            _ => [f64::from(point.x), f64::from(point.y)],
        })
        .collect()
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(current, next)| current[0] * next[1] - next[0] * current[1])
        .sum::<f64>()
        * 0.5
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]).mul_add(c[1] - a[1], -(b[1] - a[1]) * (c[0] - a[0]))
}

fn point_in_triangle(
    point: [f64; 2],
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    orientation: f64,
) -> bool {
    cross2(a, b, point) * orientation >= -1.0e-12
        && cross2(b, c, point) * orientation >= -1.0e-12
        && cross2(c, a, point) * orientation >= -1.0e-12
}

fn triangle_normal(a: AxesPoint3D, b: AxesPoint3D, c: AxesPoint3D) -> AxesVector3D {
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
    let raw = [
        ab[1].mul_add(ac[2], -ab[2] * ac[1]),
        ab[2].mul_add(ac[0], -ab[0] * ac[2]),
        ab[0].mul_add(ac[1], -ab[1] * ac[0]),
    ];
    normalized_vector(raw)
}

fn normalized_vector(raw: [f64; 3]) -> AxesVector3D {
    let length = raw[0].hypot(raw[1]).hypot(raw[2]);
    let normalized = if length > f64::EPSILON {
        [raw[0] / length, raw[1] / length, raw[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    };
    #[allow(clippy::cast_possible_truncation)]
    AxesVector3D {
        x: normalized[0] as f32,
        y: normalized[1] as f32,
        z: normalized[2] as f32,
    }
}

fn matrix_index(row: usize, column: usize, rows: usize) -> Result<usize, GeometryError> {
    column
        .checked_mul(rows)
        .and_then(|base| base.checked_add(row))
        .ok_or(GeometryError::ArithmeticOverflow("patch matrix index"))
}

fn validate_matrix_len(
    field: &'static str,
    rows: usize,
    columns: usize,
    actual: usize,
) -> Result<(), GeometryError> {
    let expected = rows
        .checked_mul(columns)
        .ok_or(GeometryError::ArithmeticOverflow("patch matrix size"))?;
    validate_len(field, expected, actual)
}

fn validate_len(field: &'static str, expected: usize, actual: usize) -> Result<(), GeometryError> {
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

fn validate_colors(colors: &[Rgba]) -> Result<(), GeometryError> {
    for &color in colors {
        color.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> DataBounds3D {
        DataBounds3D::new([0.0, 2.0], [0.0, 2.0], [-1.0, 1.0]).unwrap()
    }

    #[test]
    fn concave_face_uses_ear_clipping_without_filling_the_notch() {
        let patch = PatchMeshView {
            face_rows: 1,
            face_columns: 5,
            faces: &[1.0, 2.0, 3.0, 4.0, 5.0],
            vertex_rows: 5,
            vertex_columns: 2,
            vertices: &[0.0, 2.0, 2.0, 1.0, 0.0, 0.0, 0.0, 2.0, 1.0, 2.0],
            vertex_normals: None,
            colors: PatchColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            smooth_normals: false,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert_eq!(mesh.indices.len(), 9);
        assert_eq!(mesh.vertices.len(), 9);
        assert_eq!(mesh.color_interpolation, SurfaceColorInterpolation::Flat);
    }

    #[test]
    fn duplicate_and_collinear_face_vertices_do_not_reject_the_patch() {
        let patch = PatchMeshView {
            face_rows: 1,
            face_columns: 6,
            faces: &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            vertex_rows: 6,
            vertex_columns: 2,
            vertices: &[
                0.0, 1.0, 1.0, 2.0, 2.0, 0.0, // x, including duplicate/collinear points
                0.0, 0.0, 0.0, 0.0, 2.0, 2.0, // y
            ],
            vertex_normals: None,
            colors: PatchColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            smooth_normals: false,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.vertices.len(), 6);
    }

    #[test]
    fn zero_area_face_is_ignored_without_hiding_valid_faces() {
        let patch = PatchMeshView {
            face_rows: 2,
            face_columns: 3,
            faces: &[1.0, 1.0, 2.0, 2.0, 3.0, 4.0],
            vertex_rows: 4,
            vertex_columns: 2,
            vertices: &[0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            vertex_normals: None,
            colors: PatchColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            smooth_normals: false,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(mesh.vertices.len(), 3);
    }

    #[test]
    fn shared_edges_are_emitted_once() {
        let patch = PatchMeshView {
            face_rows: 2,
            face_columns: 3,
            faces: &[1.0, 1.0, 2.0, 3.0, 3.0, 4.0],
            vertex_rows: 4,
            vertex_columns: 2,
            vertices: &[0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 2.0, 2.0],
            vertex_normals: None,
            colors: PatchColors::Uniform(Rgba::new(0.0, 0.0, 0.0, 1.0).unwrap()),
            smooth_normals: false,
        };
        let edges = patch_mesh_edges(
            patch,
            bounds(),
            CssPx::new(0.5).unwrap(),
            LinePattern3D::Solid,
        )
        .unwrap();
        assert_eq!(edges.segments.len(), 5);
    }

    #[test]
    fn face_colors_are_duplicated_per_triangle() {
        let red = Rgba::new(1.0, 0.0, 0.0, 1.0).unwrap();
        let blue = Rgba::new(0.0, 0.0, 1.0, 1.0).unwrap();
        let patch = PatchMeshView {
            face_rows: 2,
            face_columns: 3,
            faces: &[1.0, 1.0, 2.0, 3.0, 3.0, 4.0],
            vertex_rows: 4,
            vertex_columns: 2,
            vertices: &[0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 2.0, 2.0],
            vertex_normals: None,
            colors: PatchColors::FlatFaces(&[red, blue]),
            smooth_normals: false,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert!(mesh.vertices[..3].iter().all(|vertex| vertex.color == red));
        assert!(mesh.vertices[3..].iter().all(|vertex| vertex.color == blue));
    }

    #[test]
    fn smooth_normals_average_adjacent_faces_at_each_source_vertex() {
        let patch = PatchMeshView {
            face_rows: 2,
            face_columns: 3,
            faces: &[1.0, 1.0, 2.0, 3.0, 3.0, 4.0],
            vertex_rows: 4,
            vertex_columns: 3,
            vertices: &[
                0.0, 1.0, 0.0, 0.0, // x
                0.0, 0.0, 1.0, 0.0, // y
                0.0, 0.0, 0.0, 1.0, // z
            ],
            vertex_normals: None,
            colors: PatchColors::Uniform(Rgba::new(0.2, 0.4, 0.6, 1.0).unwrap()),
            smooth_normals: true,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert_eq!(mesh.vertices[0].normal, mesh.vertices[3].normal);
        assert!(mesh.vertices[0].normal.x > 0.0);
        assert!(mesh.vertices[0].normal.z > 0.0);
    }

    #[test]
    fn explicit_vertex_normals_override_topological_smoothing() {
        let patch = PatchMeshView {
            face_rows: 1,
            face_columns: 3,
            faces: &[1.0, 2.0, 3.0],
            vertex_rows: 3,
            vertex_columns: 3,
            vertices: &[0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            vertex_normals: Some(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, -1.0]),
            colors: PatchColors::Uniform(Rgba::new(0.8, 0.1, 0.2, 1.0).unwrap()),
            smooth_normals: true,
        };
        let mesh = tessellate_patch_mesh(patch, bounds()).unwrap();
        assert!(mesh.vertices.iter().all(|vertex| vertex.normal.z < -0.99));
    }
}
