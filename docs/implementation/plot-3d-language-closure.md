# Plot Engine 3D first MATLAB-visible closure

Status: surface closure implemented on 2026-08-26; line/scatter, aspect-ratio,
and browser data-cursor closure added on 2026-08-27.

This milestone connects the existing 3D geometry and wgpu foundation to the
OpenMat language, retained graphics model, graphics-v2 transport, browser scene
mirror, WASM lowering, and WebGPU renderer. The supported vertical slice is
deliberately small but uses real Kernel-created Figure state end to end.

## MATLAB-visible surface

- `surf(Z)`, `surf(X,Y,Z)`, and `surf(X,Y,Z,C)` for real structured matrices;
- `mesh(Z)`, `mesh(X,Y,Z)`, and `mesh(X,Y,Z,C)` with distinct MATLAB default
  face and edge properties;
- `view(az,el)`, `view([az el])`, `view(2)`, `view(3)`, and two-output
  `[az,el] = view`;
- `zlim`, `zlim([minimum maximum])`, `zlim auto`, and `zlim manual`;
- `zlabel` with the existing Text property path;
- `plot3(X,Y,Z)` with MATLAB Line objects, LineSpec/name-value styling,
  matrix-column expansion, source-index-preserving markers, and automatic 3D
  limits;
- `scatter3(X,Y,Z)`, scalar size/color, marker and `filled` forms, using the
  existing MATLAB Scatter object class;
- `daspect`, `pbaspect`, and the common `axis equal`, `axis normal`,
  `axis vis3d`, and `axis auto` forms for 3D Axes;
- Surface and Axes `get` coverage for structured data, Z/CLim, View,
  Projection, FaceColor, EdgeColor, CDataMapping, FaceAlpha, LineWidth, and
  visibility.

The retained model stores X as `1-by-columns`, Y as `rows-by-1`, and Z/C as
column-major `rows-by-columns` resources. graphics-v2 carries those shapes and
binary buffers without JSON numeric payloads. The browser resolves flat CData
through the current color limits, tessellates the structured grid into indexed
triangles, and sends a view-projection matrix plus `SurfaceMesh3D` to wgpu. For
a rendered 3D Figure, left-button dragging orbits the Z-up camera, the mouse
wheel changes projection scale, and Home or double-click restores the script's
`view` state. Pointer-frequency input replaces only the retained camera matrix;
it does not re-tessellate the Surface or upload its immutable buffers again.
Line3D strokes are tessellated as camera-aware screen-space quads and Scatter3D
markers are projected as depth-tested screen-space instances. Data Cursor uses
the retained source coordinates and reports X, Y, Z and the one-based source
point for Line, Scatter, and Surface objects without a color-ID render pass.
Default `FaceColor='flat'` follows MATLAB's per-grid-cell rule: both GPU
triangles use the CData color at the cell's first vertex in positive X/Y
directions. Color is not interpolated within the cell, and no implicit light is
applied before explicit MATLAB lighting semantics exist.

## Verified compatibility

Six clean-room cases are recorded from MATLAB R2022b and pass the OpenMat
differential runner:

- `graphics_3d_surf_defaults`;
- `graphics_3d_view_zlim_label`;
- `graphics_3d_mesh_defaults`;
- `graphics_3d_plot3_defaults`;
- `graphics_3d_scatter3_defaults`;
- `graphics_3d_axes_aspect_camera`.

They cover default coordinate/data shapes, default styles, limits, projection,
view angles, manual Z limits, Z labels, aspect modes, and the MATLAB Surface,
Line, and Scatter classes.

## Intentional first-slice boundaries

- One-output `T = view` is rejected until the MATLAB 4-by-4 transform matrix is
  implemented; returning an azimuth/elevation row would be observably wrong.
- `surf` and `mesh` use unique structured-grid edges, screen-space quad strokes,
  analytical coverage/MSAA, depth testing, and the dedicated coplanar edge-bias
  path completed by the preceding wireframe milestone.
- Completed browser orbit and zoom gestures commit one authoritative camera
  transaction to the Kernel. Data Cursor currently selects the nearest retained
  source vertex; interpolated triangle-face coordinates are intentionally not
  synthesized.
- Lighting, transparency ordering, `contour3`, per-point Scatter CData/SizeData
  arrays, `axis tight`, and MATLAB camera-position/target/up property APIs
  remain out of scope.
- The first surface renderer supports opaque faces only. NaN/Inf samples create
  holes rather than non-finite GPU geometry.
