use std::sync::Arc;

use crate::{DataDType, DataResource, GraphicsError, IsoNormalsInput, NumericData};

struct RectilinearVolume {
    axes: [Vec<f64>; 3],
    values: Vec<f64>,
    shape: [usize; 3],
}

pub(crate) fn decode_resource_f64(resource: &DataResource) -> Result<Vec<f64>, GraphicsError> {
    let bytes = resource.bytes();
    let expected = usize::try_from(resource.descriptor().byte_length)
        .map_err(|_| GraphicsError::invalid_state("graphics resource exceeds host limits"))?;
    if bytes.len() != expected {
        return Err(GraphicsError::invalid_state(
            "graphics resource byte length is inconsistent",
        ));
    }
    match resource.descriptor().dtype {
        DataDType::F32 => bytes
            .chunks_exact(4)
            .map(|chunk| {
                let raw: [u8; 4] = chunk.try_into().map_err(|_| {
                    GraphicsError::invalid_state("graphics f32 resource is truncated")
                })?;
                Ok(f64::from(f32::from_le_bytes(raw)))
            })
            .collect(),
        DataDType::F64 => bytes
            .chunks_exact(8)
            .map(|chunk| {
                let raw: [u8; 8] = chunk.try_into().map_err(|_| {
                    GraphicsError::invalid_state("graphics f64 resource is truncated")
                })?;
                Ok(f64::from_le_bytes(raw))
            })
            .collect(),
    }
}

pub(crate) fn compute_vertex_normals(
    input: &IsoNormalsInput,
    vertices: &[f64],
    vertex_rows: u64,
) -> Result<NumericData, GraphicsError> {
    let vertex_rows = usize::try_from(vertex_rows)
        .map_err(|_| GraphicsError::invalid_input("Patch vertex count exceeds host limits"))?;
    let expected_vertices = vertex_rows
        .checked_mul(3)
        .ok_or_else(|| GraphicsError::invalid_input("Patch vertex count overflowed"))?;
    if vertices.len() != expected_vertices {
        return Err(GraphicsError::invalid_state(
            "Patch Vertices resource does not match its N-by-3 descriptor",
        ));
    }
    let volume = RectilinearVolume::from_input(input)?;
    let mut normals = Vec::new();
    normals
        .try_reserve_exact(expected_vertices)
        .map_err(|_| GraphicsError::invalid_input("isonormals output allocation failed"))?;
    normals.resize(expected_vertices, 0.0);
    for vertex in 0..vertex_rows {
        let point = [
            vertices[vertex],
            vertices[vertex + vertex_rows],
            vertices[vertex + vertex_rows * 2],
        ];
        if point.iter().any(|value| !value.is_finite()) {
            return Err(GraphicsError::invalid_input(
                "isonormals Patch vertices must be finite",
            ));
        }
        let gradient = volume.gradient_at(point)?;
        for component in 0..3 {
            // MATLAB R2022b stores the negative scalar-field gradient in
            // VertexNormals; it does not normalize those property values.
            normals[vertex + component * vertex_rows] = -gradient[component];
        }
    }
    Ok(NumericData::from_f64(Arc::<[f64]>::from(normals)))
}

impl RectilinearVolume {
    fn from_input(input: &IsoNormalsInput) -> Result<Self, GraphicsError> {
        let shape = input
            .shape
            .map(|extent| {
                usize::try_from(extent).map_err(|_| {
                    GraphicsError::invalid_input("isonormals volume extent exceeds host limits")
                })
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let shape: [usize; 3] = shape.try_into().map_err(|_| {
            GraphicsError::invalid_input("isonormals volume must be three-dimensional")
        })?;
        if shape.iter().any(|extent| *extent < 2) {
            return Err(GraphicsError::invalid_input(
                "isonormals volume must be at least 2-by-2-by-2",
            ));
        }
        let element_count = shape.into_iter().try_fold(1_usize, |count, extent| {
            count.checked_mul(extent).ok_or_else(|| {
                GraphicsError::invalid_input("isonormals volume element count overflowed")
            })
        })?;
        let values = numeric_values_f64(&input.values);
        if values.len() != element_count {
            return Err(GraphicsError::invalid_input(
                "isonormals scalar data does not match the declared volume shape",
            ));
        }
        let coordinate_fields = match (&input.x, &input.y, &input.z) {
            (None, None, None) => None,
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => {
                return Err(GraphicsError::invalid_input(
                    "isonormals coordinates must provide X, Y, and Z together",
                ));
            }
        };
        let axes = if let Some(fields) = coordinate_fields {
            let fields = fields.map(numeric_values_f64);
            if fields.iter().any(|field| field.len() != element_count) {
                return Err(GraphicsError::invalid_input(
                    "isonormals coordinate data does not match the scalar volume shape",
                ));
            }
            let axes = extract_meshgrid_axes(&fields, shape);
            validate_rectilinear_fields(&fields, &axes, shape)?;
            axes
        } else {
            [
                one_based_axis(shape[1]),
                one_based_axis(shape[0]),
                one_based_axis(shape[2]),
            ]
        };
        for axis in &axes {
            validate_axis(axis)?;
        }
        Ok(Self {
            axes,
            values,
            shape,
        })
    }

    fn gradient_at(&self, point: [f64; 3]) -> Result<[f64; 3], GraphicsError> {
        let intervals = [
            locate_interval(&self.axes[0], point[0]),
            locate_interval(&self.axes[1], point[1]),
            locate_interval(&self.axes[2], point[2]),
        ];
        let mut gradient = [0.0; 3];
        for z_corner in 0..=1 {
            for y_corner in 0..=1 {
                for x_corner in 0..=1 {
                    let indices = [
                        intervals[0].0 + x_corner,
                        intervals[1].0 + y_corner,
                        intervals[2].0 + z_corner,
                    ];
                    let weight = corner_weight(intervals, [x_corner, y_corner, z_corner]);
                    for (component, value) in gradient.iter_mut().enumerate() {
                        *value += weight * self.derivative(indices, component);
                    }
                }
            }
        }
        if gradient.iter().any(|value| !value.is_finite()) {
            return Err(GraphicsError::invalid_input(
                "isonormals could not compute finite gradients at every Patch vertex",
            ));
        }
        Ok(gradient)
    }

    fn derivative(&self, indices: [usize; 3], component: usize) -> f64 {
        let coordinate_index = indices[component];
        let last = self.axes[component].len() - 1;
        let (before, after) = if coordinate_index == 0 {
            (0, 1)
        } else if coordinate_index == last {
            (last - 1, last)
        } else {
            (coordinate_index - 1, coordinate_index + 1)
        };
        let mut low = indices;
        let mut high = indices;
        low[component] = before;
        high[component] = after;
        let denominator = self.axes[component][after] - self.axes[component][before];
        (self.value(high) - self.value(low)) / denominator
    }

    fn value(&self, indices: [usize; 3]) -> f64 {
        // MATLAB volume dimensions are rows(Y), columns(X), pages(Z).
        let [x, y, z] = indices;
        self.values[y + self.shape[0] * (x + self.shape[1] * z)]
    }
}

fn numeric_values_f64(data: &NumericData) -> Vec<f64> {
    let mut values = Vec::with_capacity(data.len());
    data.visit_f64(|value| values.push(value));
    values
}

#[allow(clippy::cast_precision_loss)]
fn one_based_axis(length: usize) -> Vec<f64> {
    (1..=length).map(|value| value as f64).collect()
}

fn extract_meshgrid_axes(fields: &[Vec<f64>; 3], shape: [usize; 3]) -> [Vec<f64>; 3] {
    let [rows, columns, pages] = shape;
    let x = (0..columns)
        .map(|column| fields[0][rows * column])
        .collect();
    let y = (0..rows).map(|row| fields[1][row]).collect();
    let z = (0..pages)
        .map(|page| fields[2][rows * columns * page])
        .collect();
    [x, y, z]
}

fn validate_rectilinear_fields(
    fields: &[Vec<f64>; 3],
    axes: &[Vec<f64>; 3],
    shape: [usize; 3],
) -> Result<(), GraphicsError> {
    let [rows, columns, pages] = shape;
    for page in 0..pages {
        for column in 0..columns {
            for row in 0..rows {
                let index = row + rows * (column + columns * page);
                let expected = [axes[0][column], axes[1][row], axes[2][page]];
                if fields
                    .iter()
                    .zip(expected)
                    .any(|(field, expected)| !approximately_equal(field[index], expected))
                {
                    return Err(GraphicsError::invalid_input(
                        "isonormals currently requires meshgrid-style rectilinear coordinates",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn approximately_equal(actual: f64, expected: f64) -> bool {
    actual.to_bits() == expected.to_bits()
        || (actual - expected).abs()
            <= f64::EPSILON * 32.0 * actual.abs().max(expected.abs()).max(1.0)
}

fn validate_axis(axis: &[f64]) -> Result<(), GraphicsError> {
    if axis.iter().any(|value| !value.is_finite()) {
        return Err(GraphicsError::invalid_input(
            "isonormals coordinates must be finite",
        ));
    }
    let ascending = axis[1] > axis[0];
    if axis.windows(2).any(|pair| {
        if ascending {
            pair[1] <= pair[0]
        } else {
            pair[1] >= pair[0]
        }
    }) {
        return Err(GraphicsError::invalid_input(
            "isonormals coordinates must be strictly monotonic on every axis",
        ));
    }
    Ok(())
}

fn locate_interval(axis: &[f64], value: f64) -> (usize, f64) {
    let ascending = axis[1] > axis[0];
    let upper = if ascending {
        axis.partition_point(|candidate| *candidate <= value)
    } else {
        axis.partition_point(|candidate| *candidate >= value)
    };
    let lower = upper.saturating_sub(1).min(axis.len() - 2);
    let fraction = ((value - axis[lower]) / (axis[lower + 1] - axis[lower])).clamp(0.0, 1.0);
    (lower, fraction)
}

fn corner_weight(intervals: [(usize, f64); 3], corner: [usize; 3]) -> f64 {
    intervals
        .into_iter()
        .zip(corner)
        .map(|((_index, fraction), side)| if side == 0 { 1.0 - fraction } else { fraction })
        .product()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn computes_r2022b_negative_gradient_in_double_precision() {
        let axis = [-1.0, 0.0, 1.0];
        let mut x = Vec::new();
        let mut y = Vec::new();
        let mut z = Vec::new();
        let mut values = Vec::new();
        for z_value in axis {
            for x_value in axis {
                for y_value in axis {
                    x.push(x_value);
                    y.push(y_value);
                    z.push(z_value);
                    values.push(x_value * x_value + y_value * y_value + z_value * z_value);
                }
            }
        }
        let input = IsoNormalsInput {
            x: Some(NumericData::from_f64(Arc::from(x))),
            y: Some(NumericData::from_f64(Arc::from(y))),
            z: Some(NumericData::from_f64(Arc::from(z))),
            values: NumericData::from_f32(Arc::from(
                values
                    .into_iter()
                    .map(|value| value as f32)
                    .collect::<Vec<_>>(),
            )),
            shape: [3, 3, 3],
        };
        let normals = compute_vertex_normals(&input, &[0.5, 0.0, 0.0], 1).unwrap();
        let NumericData::F64(normals) = normals else {
            panic!("MATLAB isonormals always exposes double VertexNormals")
        };
        // R2022b-compatible finite differences use one-sided endpoint slopes;
        // trilinear interpolation at x=.5 therefore yields -.5 here.
        assert_eq!(normals.as_ref(), &[-0.5, 0.0, 0.0]);
    }
}
