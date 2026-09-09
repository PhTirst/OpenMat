# Plot Engine 3D public foundation

Status: implemented on 2026-08-26. This document records internal Plot Engine
contracts. It does not add MATLAB-visible `plot3`, `mesh`, `surf`, or `view`
behavior and does not change the accepted graphics wire protocols.

## Coordinate and camera contract

Plot HIR keeps language-facing `f64` source values and axis limits. Geometry
normalizes finite values in `f64` before narrowing them to the Plot MIR
axes-local unit cube. This avoids subtracting large, nearby source coordinates
in `f32` while keeping render resources compact.

The 3D world is right-handed and Z-up. Cameras look from `eye` to `target` and
support perspective and orthographic projections. Projection matrices are
column-major and use WebGPU clip depth `0..=1`; X and Y use the ordinary
`-1..=1` clip interval. The shader maps that camera clip rectangle into the
existing axes viewport, so titles, labels, and other Figure margins do not
silently change the camera aspect.

`OrbitCamera3D` stores target, distance, azimuth, elevation, and projection.
Elevation is kept away from the Z-up poles, preserving a stable camera basis.
Orbit and dolly operations update only the small renderer matrix. They do not
mutate source arrays or rebuild GPU geometry.

## MIR and geometry contract

Plot MIR adds:

- `AxesPoint3D` and `AxesVector3D`;
- finite `ViewProjection3D` matrices;
- `SurfaceVertex3D` with normalized position, normal, and resolved RGBA color;
- indexed `SurfaceMesh3D` triangle resources;
- `SetViewProjection3D` and `SurfaceMesh3D` commands.

A command stream sets at most one 3D view-projection matrix before its first 3D
draw. Surface indices are validated before compilation and retain
counter-clockwise winding, although the first pipeline renders both faces.

`SurfaceGridView` consumes MATLAB-style column-major structured data: X indexes
columns, Y indexes rows, and Z uses `row + column * rows`. A finite four-corner
cell emits two triangles. Any NaN or infinity in X, Y, or Z removes every face
touching that sample rather than manufacturing a finite replacement. Normals
are accumulated from adjacent faces. Colormap and `CLim` resolution remain a
HIR concern; geometry receives either one resolved color or one color per
source vertex.

The renderer-independent picking foundation creates perspective or orthographic
rays from axes-local pointer positions and intersects double-sided triangles.
It returns distance and barycentric coordinates without exposing GPU addresses
or persistent object identity.

## wgpu contract

The draw compiler packs 3D vertices and indices into buffers separate from the
existing 2D stroke and marker buffers. A surface vertex is 40 bytes:

- position: `Float32x3`;
- normal: `Float32x3`;
- color: `Float32x4`.

The surface pipeline uses triangle lists, a four-sample color target, a matching
`Depth32Float` attachment, depth writes, and `LessEqual` comparison. Resolved
flat colors use WGSL first-vertex flat interpolation, so both triangles of one
MATLAB grid cell select the same CData color without duplicating shared grid
vertices. Until MATLAB lighting objects and properties are modeled, the shader
does not invent a renderer-owned light. A `GpuFrame` can replace its
view-projection matrix without replacing immutable vertex or index buffers.

## Explicit next-layer boundaries

This foundation deliberately does not claim:

- MATLAB R2022b semantics for `plot3`, `scatter3`, `mesh`, `surf`, `view`, or
  camera properties;
- 3D axes boxes, ticks, labels, grids, legends, or color bars;
- graphics-model objects, Kernel built-ins, snapshots, deltas, or binary wire
  resources for 3D series;
- transparent-surface ordering, materials, MATLAB lighting semantics, texture
  mapping, or contour generation;
- a complete data-cursor policy or persistent picking identifiers.

Those features must be measured and integrated above this layer. They can reuse
the camera, normalization, mesh, depth, shader, and ray-intersection facilities
without changing the accepted 2D pipeline.
