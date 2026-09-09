#![doc = "Plot geometry generation, clipping, stroke tessellation, and LOD planning."]
#![forbid(unsafe_code)]

mod camera3d;
mod clip;
mod error;
mod frame;
mod input;
mod lod;
mod marker;
mod patch3d;
mod plotbox3d;
mod ruler3d;
mod runs;
mod stroke;
mod surface3d;

pub use camera3d::{
    Camera3D, OrbitCamera3D, Point3D, Projection3D, Ray3D, TriangleHit3D, intersect_triangle_3d,
};
pub use clip::clip_line_runs;
pub use error::GeometryError;
pub use frame::{MirFrameBuilder, ScatterStyle};
pub use input::{NumericView, SeriesGeometryInput};
pub use lod::{
    LinearLodView, LodBreakKind, LodCacheKey, LodOutput, LodResourceRevision, LodSample,
    LodSourceKey, LodStatistics, LodViewKey, min_max_lod,
};
pub use marker::{
    MarkerStyle, marker_batch, marker_batch_at_indices, marker_batch_rotated,
    marker_batch_rotated_at_indices,
};
pub use patch3d::{
    PatchColors, PatchEdge2D, PatchMeshView, patch_mesh_edges, patch_mesh_edges_2d,
    tessellate_patch_mesh, tessellate_patch_mesh_2d,
};
pub use plotbox3d::{PlotBoxFit3D, PlotBoxFitMode3D, fit_plot_box_3d};
pub use ruler3d::{RulerEdge3D, ruler_edge_3d, select_projected_rulers_3d};
pub use runs::{
    AxesLineRun, DataPoint, LineRun, axes_line_runs, line_runs_from_lod, split_line_runs,
};
pub use stroke::{StrokeTessellation, tessellate_stroke};
pub use surface3d::{
    DataBounds3D, SurfaceColors, SurfaceGridView, surface_grid_edges, tessellate_surface_grid,
};
