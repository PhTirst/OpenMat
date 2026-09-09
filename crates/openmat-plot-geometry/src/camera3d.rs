use std::f64::consts::{FRAC_PI_2, PI};

use openmat_plot_mir::{AxesPoint, AxesPoint3D, ViewProjection3D};

use crate::GeometryError;

const POLE_EPSILON_RADIANS: f64 = 1.0e-6;

/// A finite point or vector in the normalized, right-handed, Z-up 3D axes space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3D {
    /// Creates a finite 3D point.
    ///
    /// # Errors
    /// Returns [`GeometryError`] when any coordinate is non-finite.
    pub fn new(x: f64, y: f64, z: f64) -> Result<Self, GeometryError> {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(Self { x, y, z })
        } else {
            Err(GeometryError::NonFiniteCoordinate)
        }
    }

    fn subtract(self, other: Self) -> Vector3 {
        Vector3 {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }

    fn add_vector(self, vector: Vector3) -> Self {
        Self {
            x: self.x + vector.x,
            y: self.y + vector.y,
            z: self.z + vector.z,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vector3 {
    x: f64,
    y: f64,
    z: f64,
}

impl Vector3 {
    fn dot(self, other: Self) -> f64 {
        self.x
            .mul_add(other.x, self.y.mul_add(other.y, self.z * other.z))
    }

    fn cross(self, other: Self) -> Self {
        Self {
            x: self.y.mul_add(other.z, -self.z * other.y),
            y: self.z.mul_add(other.x, -self.x * other.z),
            z: self.x.mul_add(other.y, -self.y * other.x),
        }
    }

    fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    fn normalized(self, field: &'static str) -> Result<Self, GeometryError> {
        let length = self.length();
        if !length.is_finite() || length <= f64::EPSILON {
            return Err(GeometryError::InvalidCamera(field));
        }
        Ok(Self {
            x: self.x / length,
            y: self.y / length,
            z: self.z / length,
        })
    }

    fn scale(self, scalar: f64) -> Self {
        Self {
            x: self.x * scalar,
            y: self.y * scalar,
            z: self.z * scalar,
        }
    }
}

/// WebGPU-compatible 3D projection. Near/far distances are positive camera-space magnitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection3D {
    Perspective {
        vertical_fov_radians: f64,
        near: f64,
        far: f64,
    },
    Orthographic {
        vertical_span: f64,
        near: f64,
        far: f64,
    },
}

impl Projection3D {
    fn matrix(self, aspect_ratio: f64) -> Result<Matrix4, GeometryError> {
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            return Err(GeometryError::InvalidProjection("aspect ratio"));
        }
        match self {
            Self::Perspective {
                vertical_fov_radians,
                near,
                far,
            } => {
                if !vertical_fov_radians.is_finite()
                    || vertical_fov_radians <= 0.0
                    || vertical_fov_radians >= PI
                {
                    return Err(GeometryError::InvalidProjection("vertical field of view"));
                }
                validate_depth_range(near, far)?;
                let focal = 1.0 / (vertical_fov_radians * 0.5).tan();
                Ok(Matrix4::from_columns([
                    [focal / aspect_ratio, 0.0, 0.0, 0.0],
                    [0.0, focal, 0.0, 0.0],
                    [0.0, 0.0, far / (near - far), -1.0],
                    [0.0, 0.0, near * far / (near - far), 0.0],
                ]))
            }
            Self::Orthographic {
                vertical_span,
                near,
                far,
            } => {
                if !vertical_span.is_finite() || vertical_span <= 0.0 {
                    return Err(GeometryError::InvalidProjection("orthographic span"));
                }
                validate_depth_range(near, far)?;
                let horizontal_span = vertical_span * aspect_ratio;
                if !horizontal_span.is_finite() {
                    return Err(GeometryError::ArithmeticOverflow(
                        "orthographic horizontal span",
                    ));
                }
                Ok(Matrix4::from_columns([
                    [2.0 / horizontal_span, 0.0, 0.0, 0.0],
                    [0.0, 2.0 / vertical_span, 0.0, 0.0],
                    [0.0, 0.0, 1.0 / (near - far), 0.0],
                    [0.0, 0.0, near / (near - far), 1.0],
                ]))
            }
        }
    }
}

/// An explicit right-handed, Z-up camera pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3D {
    pub eye: Point3D,
    pub target: Point3D,
    pub up: Point3D,
    pub projection: Projection3D,
}

/// A normalized world-space ray used by renderer-independent 3D picking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray3D {
    pub origin: Point3D,
    pub direction: [f64; 3],
}

/// The nearest positive ray/triangle intersection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleHit3D {
    pub distance: f64,
    pub barycentric: [f64; 3],
}

impl Camera3D {
    fn basis(self) -> Result<(Vector3, Vector3, Vector3), GeometryError> {
        let forward = self
            .target
            .subtract(self.eye)
            .normalized("eye and target")?;
        let up = Vector3 {
            x: self.up.x,
            y: self.up.y,
            z: self.up.z,
        }
        .normalized("up vector")?;
        let right = forward.cross(up).normalized("up direction")?;
        let corrected_up = right.cross(forward);
        Ok((right, corrected_up, forward))
    }

    /// Builds a column-major axes-to-WebGPU-clip transform.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for a coincident eye/target, parallel up
    /// vector, invalid projection, non-finite arithmetic, or f32 overflow.
    pub fn view_projection(self, aspect_ratio: f64) -> Result<ViewProjection3D, GeometryError> {
        let (right, corrected_up, forward) = self.basis()?;
        let eye = Vector3 {
            x: self.eye.x,
            y: self.eye.y,
            z: self.eye.z,
        };
        let view = Matrix4::from_columns([
            [right.x, corrected_up.x, -forward.x, 0.0],
            [right.y, corrected_up.y, -forward.y, 0.0],
            [right.z, corrected_up.z, -forward.z, 0.0],
            [
                -right.dot(eye),
                -corrected_up.dot(eye),
                forward.dot(eye),
                1.0,
            ],
        ]);
        self.projection
            .matrix(aspect_ratio)?
            .multiply(view)
            .to_mir()
    }

    /// Creates a world-space picking ray from an axes-local pointer (`x` right,
    /// `y` up). The pointer is not clamped, allowing callers to reject or retain
    /// out-of-viewport gestures explicitly.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for an invalid camera, pointer, aspect ratio,
    /// or projection.
    pub fn ray_from_axes(
        self,
        pointer: AxesPoint,
        aspect_ratio: f64,
    ) -> Result<Ray3D, GeometryError> {
        pointer.validate()?;
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            return Err(GeometryError::InvalidProjection("aspect ratio"));
        }
        let (right, up, forward) = self.basis()?;
        let x_ndc = f64::from(pointer.x).mul_add(2.0, -1.0);
        let y_ndc = f64::from(pointer.y).mul_add(2.0, -1.0);
        let (origin, direction) = match self.projection {
            Projection3D::Perspective {
                vertical_fov_radians,
                near,
                far,
            } => {
                self.projection.matrix(aspect_ratio)?;
                validate_depth_range(near, far)?;
                let half_height = (vertical_fov_radians * 0.5).tan();
                let direction = Vector3 {
                    x: forward.x
                        + right.x * x_ndc * half_height * aspect_ratio
                        + up.x * y_ndc * half_height,
                    y: forward.y
                        + right.y * x_ndc * half_height * aspect_ratio
                        + up.y * y_ndc * half_height,
                    z: forward.z
                        + right.z * x_ndc * half_height * aspect_ratio
                        + up.z * y_ndc * half_height,
                }
                .normalized("picking ray")?;
                (self.eye, direction)
            }
            Projection3D::Orthographic {
                vertical_span,
                near,
                far,
            } => {
                self.projection.matrix(aspect_ratio)?;
                validate_depth_range(near, far)?;
                let origin = self
                    .eye
                    .add_vector(right.scale(x_ndc * vertical_span * aspect_ratio * 0.5))
                    .add_vector(up.scale(y_ndc * vertical_span * 0.5));
                (origin, forward)
            }
        };
        Ok(Ray3D {
            origin,
            direction: [direction.x, direction.y, direction.z],
        })
    }
}

/// Intersects a normalized picking ray with one axes-local triangle using the
/// Moller-Trumbore test. Back faces remain pickable, matching the initial
/// double-sided surface renderer.
#[must_use]
pub fn intersect_triangle_3d(ray: Ray3D, triangle: [AxesPoint3D; 3]) -> Option<TriangleHit3D> {
    let origin = [ray.origin.x, ray.origin.y, ray.origin.z];
    let first_vertex = point_array(triangle[0]);
    let second_vertex = point_array(triangle[1]);
    let third_vertex = point_array(triangle[2]);
    let first_edge = subtract_array(second_vertex, first_vertex);
    let second_edge = subtract_array(third_vertex, first_vertex);
    let ray_cross_second_edge = cross_array(ray.direction, second_edge);
    let determinant = dot_array(first_edge, ray_cross_second_edge);
    if !determinant.is_finite() || determinant.abs() <= 1.0e-12 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let origin_offset = subtract_array(origin, first_vertex);
    let second_weight = dot_array(origin_offset, ray_cross_second_edge) * inverse;
    if !(0.0..=1.0).contains(&second_weight) {
        return None;
    }
    let origin_cross_first_edge = cross_array(origin_offset, first_edge);
    let third_weight = dot_array(ray.direction, origin_cross_first_edge) * inverse;
    if third_weight < 0.0 || second_weight + third_weight > 1.0 {
        return None;
    }
    let distance = dot_array(second_edge, origin_cross_first_edge) * inverse;
    if !distance.is_finite() || distance < 0.0 {
        return None;
    }
    Some(TriangleHit3D {
        distance,
        barycentric: [
            1.0 - second_weight - third_weight,
            second_weight,
            third_weight,
        ],
    })
}

fn point_array(point: AxesPoint3D) -> [f64; 3] {
    [f64::from(point.x), f64::from(point.y), f64::from(point.z)]
}

fn subtract_array(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot_array(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0].mul_add(right[0], left[1].mul_add(right[1], left[2] * right[2]))
}

fn cross_array(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1].mul_add(right[2], -left[2] * right[1]),
        left[2].mul_add(right[0], -left[0] * right[2]),
        left[0].mul_add(right[1], -left[1] * right[0]),
    ]
}

/// Renderer-local orbit camera state shared by mouse, touch, and native hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera3D {
    target: Point3D,
    distance: f64,
    azimuth_radians: f64,
    elevation_radians: f64,
    projection: Projection3D,
}

impl OrbitCamera3D {
    /// Creates a finite orbit camera. Elevation is clamped away from the poles
    /// to preserve a stable Z-up basis.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for non-positive distance or non-finite angles.
    pub fn new(
        target: Point3D,
        distance: f64,
        azimuth_radians: f64,
        elevation_radians: f64,
        projection: Projection3D,
    ) -> Result<Self, GeometryError> {
        if !distance.is_finite() || distance <= 0.0 {
            return Err(GeometryError::InvalidCamera("orbit distance"));
        }
        if !azimuth_radians.is_finite() || !elevation_radians.is_finite() {
            return Err(GeometryError::InvalidCamera("orbit angles"));
        }
        Ok(Self {
            target,
            distance,
            azimuth_radians: wrap_angle(azimuth_radians),
            elevation_radians: clamp_elevation(elevation_radians),
            projection,
        })
    }

    #[must_use]
    pub const fn target(self) -> Point3D {
        self.target
    }

    #[must_use]
    pub const fn distance(self) -> f64 {
        self.distance
    }

    #[must_use]
    pub const fn azimuth_radians(self) -> f64 {
        self.azimuth_radians
    }

    #[must_use]
    pub const fn elevation_radians(self) -> f64 {
        self.elevation_radians
    }

    #[must_use]
    pub const fn projection(self) -> Projection3D {
        self.projection
    }

    #[must_use]
    pub fn eye(self) -> Point3D {
        let horizontal = self.elevation_radians.cos();
        self.target.add_vector(
            Vector3 {
                x: horizontal * self.azimuth_radians.cos(),
                y: horizontal * self.azimuth_radians.sin(),
                z: self.elevation_radians.sin(),
            }
            .scale(self.distance),
        )
    }

    #[must_use]
    pub fn camera(self) -> Camera3D {
        Camera3D {
            eye: self.eye(),
            target: self.target,
            up: Point3D {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            projection: self.projection,
        }
    }

    /// Applies an orbit delta in radians.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for non-finite input.
    pub fn orbit(
        &mut self,
        delta_azimuth_radians: f64,
        delta_elevation_radians: f64,
    ) -> Result<(), GeometryError> {
        if !delta_azimuth_radians.is_finite() || !delta_elevation_radians.is_finite() {
            return Err(GeometryError::InvalidCamera("orbit delta"));
        }
        self.azimuth_radians = wrap_angle(self.azimuth_radians + delta_azimuth_radians);
        self.elevation_radians = clamp_elevation(self.elevation_radians + delta_elevation_radians);
        Ok(())
    }

    /// Multiplies the eye-to-target distance. Factors below one dolly inward.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for non-positive, non-finite, or overflowing input.
    pub fn dolly(&mut self, factor: f64) -> Result<(), GeometryError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(GeometryError::InvalidCamera("dolly factor"));
        }
        let distance = self.distance * factor;
        if !distance.is_finite() || distance <= f64::EPSILON {
            return Err(GeometryError::ArithmeticOverflow("orbit distance"));
        }
        self.distance = distance;
        Ok(())
    }

    /// Changes the projection scale without rebuilding scene geometry. A
    /// perspective camera changes its field of view, while an orthographic
    /// camera changes its vertical span.
    ///
    /// # Errors
    /// Returns [`GeometryError`] for non-positive or non-finite input.
    pub fn zoom(&mut self, factor: f64) -> Result<(), GeometryError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(GeometryError::InvalidCamera("zoom factor"));
        }
        match &mut self.projection {
            Projection3D::Perspective {
                vertical_fov_radians,
                ..
            } => {
                *vertical_fov_radians = (*vertical_fov_radians * factor)
                    .clamp(1.0_f64.to_radians(), 170.0_f64.to_radians());
            }
            Projection3D::Orthographic { vertical_span, .. } => {
                *vertical_span = (*vertical_span * factor).clamp(1.0e-4, 1.0e4);
            }
        }
        Ok(())
    }

    /// Produces the current renderer matrix.
    ///
    /// # Errors
    /// Returns the same validation failures as [`Camera3D::view_projection`].
    pub fn view_projection(self, aspect_ratio: f64) -> Result<ViewProjection3D, GeometryError> {
        self.camera().view_projection(aspect_ratio)
    }
}

fn validate_depth_range(near: f64, far: f64) -> Result<(), GeometryError> {
    if near.is_finite() && far.is_finite() && near > 0.0 && far > near {
        Ok(())
    } else {
        Err(GeometryError::InvalidProjection("depth range"))
    }
}

fn wrap_angle(angle: f64) -> f64 {
    (angle + PI).rem_euclid(2.0 * PI) - PI
}

fn clamp_elevation(elevation: f64) -> f64 {
    elevation.clamp(
        -FRAC_PI_2 + POLE_EPSILON_RADIANS,
        FRAC_PI_2 - POLE_EPSILON_RADIANS,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Matrix4 {
    columns: [[f64; 4]; 4],
}

impl Matrix4 {
    const fn from_columns(columns: [[f64; 4]; 4]) -> Self {
        Self { columns }
    }

    fn multiply(self, right: Self) -> Self {
        let mut result = [[0.0; 4]; 4];
        for (column, output_column) in result.iter_mut().enumerate() {
            for (row, output) in output_column.iter_mut().enumerate() {
                *output = (0..4)
                    .map(|index| self.columns[index][row] * right.columns[column][index])
                    .sum();
            }
        }
        Self::from_columns(result)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn to_mir(self) -> Result<ViewProjection3D, GeometryError> {
        let mut columns = [[0.0_f32; 4]; 4];
        for (source_column, output_column) in self.columns.into_iter().zip(&mut columns) {
            for (source, output) in source_column.into_iter().zip(output_column) {
                let narrowed = source as f32;
                if !narrowed.is_finite() {
                    return Err(GeometryError::ArithmeticOverflow(
                        "3D view-projection matrix",
                    ));
                }
                *output = narrowed;
            }
        }
        ViewProjection3D::new(columns).map_err(GeometryError::from)
    }

    #[cfg(test)]
    fn transform(self, point: [f64; 4]) -> [f64; 4] {
        let mut result = [0.0; 4];
        for (row, output) in result.iter_mut().enumerate() {
            *output = (0..4)
                .map(|column| self.columns[column][row] * point[column])
                .sum();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective() -> Projection3D {
        Projection3D::Perspective {
            vertical_fov_radians: 60.0_f64.to_radians(),
            near: 0.1,
            far: 10.0,
        }
    }

    #[test]
    fn perspective_uses_webgpu_zero_to_one_depth() {
        let projection = perspective().matrix(1.0).unwrap();
        let near = projection.transform([0.0, 0.0, -0.1, 1.0]);
        let far = projection.transform([0.0, 0.0, -10.0, 1.0]);
        assert!((near[2] / near[3]).abs() < 1.0e-12);
        assert!((far[2] / far[3] - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn orbit_camera_is_z_up_bounded_and_deterministic() {
        let mut orbit = OrbitCamera3D::new(
            Point3D::new(0.5, 0.5, 0.5).unwrap(),
            3.0,
            -37.5_f64.to_radians(),
            30.0_f64.to_radians(),
            perspective(),
        )
        .unwrap();
        let first = orbit.view_projection(16.0 / 9.0).unwrap();
        orbit.orbit(2.0 * PI, PI).unwrap();
        assert!(orbit.elevation_radians() < FRAC_PI_2);
        orbit.dolly(0.5).unwrap();
        assert!((orbit.distance() - 1.5).abs() < f64::EPSILON);
        assert_ne!(first, orbit.view_projection(16.0 / 9.0).unwrap());
    }

    #[test]
    fn orbit_zoom_changes_both_projection_kinds_without_moving_the_eye() {
        let target = Point3D::new(0.5, 0.5, 0.5).unwrap();
        let mut perspective_camera =
            OrbitCamera3D::new(target, 3.0, 0.0, 0.5, perspective()).unwrap();
        let perspective_before = perspective_camera.view_projection(1.0).unwrap();
        let eye_before = perspective_camera.eye();
        perspective_camera.zoom(0.5).unwrap();
        assert_eq!(perspective_camera.eye(), eye_before);
        assert_ne!(
            perspective_camera.view_projection(1.0).unwrap(),
            perspective_before
        );

        let mut orthographic_camera = OrbitCamera3D::new(
            target,
            3.0,
            0.0,
            0.5,
            Projection3D::Orthographic {
                vertical_span: 2.0,
                near: 0.1,
                far: 10.0,
            },
        )
        .unwrap();
        let orthographic_before = orthographic_camera.view_projection(1.0).unwrap();
        orthographic_camera.zoom(2.0).unwrap();
        assert_ne!(
            orthographic_camera.view_projection(1.0).unwrap(),
            orthographic_before
        );
        assert_eq!(
            orthographic_camera.zoom(f64::NAN),
            Err(GeometryError::InvalidCamera("zoom factor"))
        );
    }

    #[test]
    fn invalid_camera_and_projection_fail_before_matrix_publication() {
        let camera = Camera3D {
            eye: Point3D::new(1.0, 1.0, 1.0).unwrap(),
            target: Point3D::new(1.0, 1.0, 1.0).unwrap(),
            up: Point3D::new(0.0, 0.0, 1.0).unwrap(),
            projection: perspective(),
        };
        assert_eq!(
            camera.view_projection(1.0),
            Err(GeometryError::InvalidCamera("eye and target"))
        );
        let invalid_projection = Projection3D::Perspective {
            vertical_fov_radians: 0.0,
            near: 0.1,
            far: 10.0,
        };
        assert!(matches!(
            invalid_projection.matrix(1.0),
            Err(GeometryError::InvalidProjection(_))
        ));
    }

    #[test]
    fn center_ray_hits_front_triangle_and_reports_barycentric_coordinates() {
        let camera = Camera3D {
            eye: Point3D::new(0.5, 0.5, 2.0).unwrap(),
            target: Point3D::new(0.5, 0.5, 0.0).unwrap(),
            up: Point3D::new(0.0, 1.0, 0.0).unwrap(),
            projection: perspective(),
        };
        let ray = camera
            .ray_from_axes(AxesPoint::new(0.5, 0.5).unwrap(), 1.0)
            .unwrap();
        assert!((ray.direction[2] + 1.0).abs() < 1.0e-12);
        let hit = intersect_triangle_3d(
            ray,
            [
                AxesPoint3D::new(0.0, 0.0, 0.0).unwrap(),
                AxesPoint3D::new(1.0, 0.0, 0.0).unwrap(),
                AxesPoint3D::new(0.5, 1.0, 0.0).unwrap(),
            ],
        )
        .unwrap();
        assert!((hit.distance - 2.0).abs() < 1.0e-12);
        assert!((hit.barycentric.iter().sum::<f64>() - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn view_matrix_places_target_on_camera_forward_axis() {
        let camera = Camera3D {
            eye: Point3D::new(0.5, -2.5, 0.5).unwrap(),
            target: Point3D::new(0.5, 0.5, 0.5).unwrap(),
            up: Point3D::new(0.0, 0.0, 1.0).unwrap(),
            projection: Projection3D::Orthographic {
                vertical_span: 2.0,
                near: 0.1,
                far: 10.0,
            },
        };
        let matrix = camera.view_projection(1.0).unwrap().columns();
        assert!(matrix.iter().flatten().all(|value| value.is_finite()));
    }

    #[test]
    fn vector_scaling_supports_orbit_eye_construction() {
        let point = Point3D::new(1.0, 2.0, 3.0).unwrap();
        let moved = point.add_vector(
            Vector3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            }
            .scale(2.0),
        );
        assert_eq!(moved, Point3D::new(3.0, 2.0, 3.0).unwrap());
    }
}
