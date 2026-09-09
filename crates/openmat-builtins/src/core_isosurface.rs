use std::collections::HashMap;

use openmat_array::{ArrayData, DenseArray, Shape};
use openmat_runtime::{BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinResult};
use openmat_value::{FieldName, StructArray, Value};

use crate::{aggregate_error, array_error, expect_argument_count_range, expect_max_outputs};

const CANCELLATION_CHECK_INTERVAL: usize = 4_096;
const TETRAHEDRA: [[usize; 4]; 6] = [
    [0, 1, 2, 6],
    [0, 2, 3, 6],
    [0, 3, 7, 6],
    [0, 7, 4, 6],
    [0, 4, 5, 6],
    [0, 5, 1, 6],
];
const CORNERS: [[usize; 3]; 8] = [
    [0, 0, 0],
    [0, 1, 0],
    [1, 1, 0],
    [1, 0, 0],
    [0, 0, 1],
    [0, 1, 1],
    [1, 1, 1],
    [1, 0, 1],
];

#[derive(Clone)]
struct RealVolume {
    shape: [usize; 3],
    values: Vec<f64>,
}

#[derive(Default)]
struct IsoMesh {
    vertices: Vec<[f64; 3]>,
    faces: Vec<[u32; 3]>,
    edge_vertices: HashMap<(usize, usize), u32>,
}

pub(super) fn isosurface_builtin(
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
) -> BuiltinResult {
    expect_argument_count_range("isosurface", arguments, 2, 5)?;
    expect_max_outputs("isosurface", context, 2)?;
    if !matches!(arguments.len(), 2 | 5) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            "`isosurface` currently accepts V/isovalue or X/Y/Z/V/isovalue",
        ));
    }
    context.check_cancelled()?;
    let (coordinates, field, iso_position) = if arguments.len() == 5 {
        let x = real_volume("isosurface", 1, &arguments[0])?;
        let y = real_volume("isosurface", 2, &arguments[1])?;
        let z = real_volume("isosurface", 3, &arguments[2])?;
        let field = real_volume("isosurface", 4, &arguments[3])?;
        if x.shape != field.shape || y.shape != field.shape || z.shape != field.shape {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Domain,
                "`isosurface` X, Y, Z, and V arrays must have identical three-dimensional shapes",
            ));
        }
        (Some([x, y, z]), field, 5)
    } else {
        (None, real_volume("isosurface", 1, &arguments[0])?, 2)
    };
    let iso_value = real_scalar("isosurface", iso_position, &arguments[iso_position - 1])?;
    if !iso_value.is_finite() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "`isosurface` isovalue must be finite",
        ));
    }
    let mesh = extract_isosurface(coordinates.as_ref(), &field, iso_value, context)?;
    context.check_cancelled()?;
    isosurface_outputs(&mesh, context.requested_outputs())
}

fn real_volume(name: &str, position: usize, value: &Value) -> Result<RealVolume, BuiltinError> {
    let (shape, values) = match value {
        Value::Array(ArrayData::F64(array)) => {
            (array.shape().dimensions(), array.as_slice().to_vec())
        }
        Value::Array(ArrayData::F32(array)) => (
            array.shape().dimensions(),
            array
                .as_slice()
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
        ),
        _ => {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::Type,
                format!("input {position} to `{name}` must be a real double or single volume"),
            ));
        }
    };
    if shape.len() != 3 || shape.iter().any(|extent| *extent < 2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be at least 2-by-2-by-2"),
        ));
    }
    let shape = [
        usize::try_from(shape[0]).map_err(|_| volume_size_error())?,
        usize::try_from(shape[1]).map_err(|_| volume_size_error())?,
        usize::try_from(shape[2]).map_err(|_| volume_size_error())?,
    ];
    Ok(RealVolume { shape, values })
}

fn real_scalar(name: &str, position: usize, value: &Value) -> Result<f64, BuiltinError> {
    value
        .as_real_number()
        .or_else(|| {
            value
                .as_complex_single()
                .filter(|value| value.im == 0.0)
                .map(|value| f64::from(value.re))
        })
        .ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Type,
                format!("input {position} to `{name}` must be a real numeric scalar"),
            )
        })
}

fn extract_isosurface(
    coordinates: Option<&[RealVolume; 3]>,
    field: &RealVolume,
    iso_value: f64,
    context: &BuiltinContext<'_>,
) -> Result<IsoMesh, BuiltinError> {
    let [rows, columns, pages] = field.shape;
    let mut mesh = IsoMesh::default();
    for page in 0..pages - 1 {
        for column in 0..columns - 1 {
            for row in 0..rows - 1 {
                let cube_number = row + rows * (column + columns * page);
                if cube_number.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    context.check_cancelled()?;
                }
                let mut offsets = [0_usize; 8];
                let mut positions = [[0.0_f64; 3]; 8];
                let mut values = [0.0_f64; 8];
                for (corner, [delta_row, delta_column, delta_page]) in
                    CORNERS.iter().copied().enumerate()
                {
                    let grid_row = row + delta_row;
                    let grid_column = column + delta_column;
                    let grid_page = page + delta_page;
                    let offset = grid_row + rows * (grid_column + columns * grid_page);
                    offsets[corner] = offset;
                    values[corner] = field.values[offset];
                    positions[corner] = if let Some(coordinates) = coordinates {
                        [
                            coordinates[0].values[offset],
                            coordinates[1].values[offset],
                            coordinates[2].values[offset],
                        ]
                    } else {
                        [
                            index_coordinate(grid_column)?,
                            index_coordinate(grid_row)?,
                            index_coordinate(grid_page)?,
                        ]
                    };
                }
                if values.iter().any(|value| !value.is_finite())
                    || positions.iter().flatten().any(|value| !value.is_finite())
                {
                    continue;
                }
                for tetrahedron in TETRAHEDRA {
                    polygonize_tetrahedron(
                        &mut mesh,
                        tetrahedron,
                        offsets,
                        positions,
                        values,
                        iso_value,
                    )?;
                }
            }
        }
    }
    Ok(mesh)
}

fn polygonize_tetrahedron(
    mesh: &mut IsoMesh,
    tetrahedron: [usize; 4],
    offsets: [usize; 8],
    positions: [[f64; 3]; 8],
    values: [f64; 8],
    iso_value: f64,
) -> Result<(), BuiltinError> {
    let mut inside = Vec::with_capacity(4);
    let mut outside = Vec::with_capacity(4);
    for corner in tetrahedron {
        if values[corner] < iso_value {
            inside.push(corner);
        } else {
            outside.push(corner);
        }
    }
    if inside.is_empty() || outside.is_empty() {
        return Ok(());
    }
    let outward = sub(
        mean_points(&outside, &positions),
        mean_points(&inside, &positions),
    );
    match (inside.as_slice(), outside.as_slice()) {
        ([inside], [outside_a, outside_b, outside_c]) => {
            let triangle = [
                edge_vertex(
                    mesh, *inside, *outside_a, offsets, positions, values, iso_value,
                )?,
                edge_vertex(
                    mesh, *inside, *outside_b, offsets, positions, values, iso_value,
                )?,
                edge_vertex(
                    mesh, *inside, *outside_c, offsets, positions, values, iso_value,
                )?,
            ];
            push_oriented_triangle(mesh, triangle, outward);
        }
        ([inside_a, inside_b, inside_c], [outside]) => {
            let triangle = [
                edge_vertex(
                    mesh, *outside, *inside_a, offsets, positions, values, iso_value,
                )?,
                edge_vertex(
                    mesh, *outside, *inside_b, offsets, positions, values, iso_value,
                )?,
                edge_vertex(
                    mesh, *outside, *inside_c, offsets, positions, values, iso_value,
                )?,
            ];
            push_oriented_triangle(mesh, triangle, outward);
        }
        ([inside_a, inside_b], [outside_a, outside_b]) => {
            let ac = edge_vertex(
                mesh, *inside_a, *outside_a, offsets, positions, values, iso_value,
            )?;
            let ad = edge_vertex(
                mesh, *inside_a, *outside_b, offsets, positions, values, iso_value,
            )?;
            let bc = edge_vertex(
                mesh, *inside_b, *outside_a, offsets, positions, values, iso_value,
            )?;
            let bd = edge_vertex(
                mesh, *inside_b, *outside_b, offsets, positions, values, iso_value,
            )?;
            push_oriented_triangle(mesh, [ac, ad, bd], outward);
            push_oriented_triangle(mesh, [ac, bd, bc], outward);
        }
        _ => unreachable!("a tetrahedron has four corners"),
    }
    Ok(())
}

fn edge_vertex(
    mesh: &mut IsoMesh,
    first: usize,
    second: usize,
    offsets: [usize; 8],
    positions: [[f64; 3]; 8],
    values: [f64; 8],
    iso_value: f64,
) -> Result<u32, BuiltinError> {
    let key = if offsets[first] < offsets[second] {
        (offsets[first], offsets[second])
    } else {
        (offsets[second], offsets[first])
    };
    if let Some(index) = mesh.edge_vertices.get(&key) {
        return Ok(*index);
    }
    let denominator = values[second] - values[first];
    let fraction = if denominator == 0.0 {
        0.5
    } else {
        ((iso_value - values[first]) / denominator).clamp(0.0, 1.0)
    };
    let position = [
        positions[first][0] + fraction * (positions[second][0] - positions[first][0]),
        positions[first][1] + fraction * (positions[second][1] - positions[first][1]),
        positions[first][2] + fraction * (positions[second][2] - positions[first][2]),
    ];
    let index = u32::try_from(mesh.vertices.len()).map_err(|_| volume_size_error())?;
    mesh.vertices.push(position);
    mesh.edge_vertices.insert(key, index);
    Ok(index)
}

fn push_oriented_triangle(mesh: &mut IsoMesh, mut triangle: [u32; 3], outward: [f64; 3]) {
    let a = mesh.vertices[triangle[0] as usize];
    let b = mesh.vertices[triangle[1] as usize];
    let c = mesh.vertices[triangle[2] as usize];
    let normal = cross(sub(b, a), sub(c, a));
    let area_squared = dot(normal, normal);
    if !area_squared.is_finite() || area_squared <= f64::EPSILON {
        return;
    }
    if dot(normal, outward) < 0.0 {
        triangle.swap(1, 2);
    }
    mesh.faces.push(triangle);
}

fn mean_points(indices: &[usize], positions: &[[f64; 3]; 8]) -> [f64; 3] {
    let mut result = [0.0; 3];
    for index in indices {
        for (component, value) in result.iter_mut().enumerate() {
            *value += positions[*index][component];
        }
    }
    let divisor = match indices.len() {
        1 => 1.0,
        2 => 2.0,
        3 => 3.0,
        _ => unreachable!("inside and outside tetrahedron sets contain one to three corners"),
    };
    result.map(|value| value / divisor)
}

fn index_coordinate(index: usize) -> Result<f64, BuiltinError> {
    u32::try_from(index)
        .map(|index| f64::from(index) + 1.0)
        .map_err(|_| volume_size_error())
}

const fn sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

const fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

const fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn isosurface_outputs(mesh: &IsoMesh, requested_outputs: usize) -> BuiltinResult {
    let face_count = mesh.faces.len();
    let vertex_count = mesh.vertices.len();
    let mut face_values = Vec::with_capacity(face_count * 3);
    for column in 0..3 {
        face_values.extend(mesh.faces.iter().map(|face| f64::from(face[column]) + 1.0));
    }
    let mut vertex_values = Vec::with_capacity(vertex_count * 3);
    for column in 0..3 {
        vertex_values.extend(mesh.vertices.iter().map(|vertex| vertex[column]));
    }
    let faces = real_matrix(face_count, 3, face_values)?;
    let vertices = real_matrix(vertex_count, 3, vertex_values)?;
    if requested_outputs == 2 {
        return Ok(vec![faces, vertices]);
    }
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    let fields = ["vertices", "faces"]
        .into_iter()
        .map(FieldName::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| aggregate_error("isosurface", &error))?;
    StructArray::from_columns(shape, fields, vec![vec![vertices], vec![faces]])
        .map(Value::Struct)
        .map(|value| vec![value])
        .map_err(|error| aggregate_error("isosurface", &error))
}

fn real_matrix(rows: usize, columns: usize, values: Vec<f64>) -> Result<Value, BuiltinError> {
    let rows = u64::try_from(rows).map_err(|_| volume_size_error())?;
    let columns = u64::try_from(columns).map_err(|_| volume_size_error())?;
    let shape = Shape::new([rows, columns]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, values)
        .map(ArrayData::F64)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn volume_size_error() -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        "`isosurface` volume or generated mesh is too large for this host",
    )
}
