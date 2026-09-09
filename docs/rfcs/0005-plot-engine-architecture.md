# RFC 0005: Plot Engine architecture and WebGPU boundary

Status: Accepted

## Decision

OpenMat will develop a Web-native, MATLAB-style scientific Plot Engine as a
post-release-one subsystem. This does not change RFC 0001's statement that full
MATLAB graphics compatibility is outside the release-one language boundary.
The first Plot Engine tranche is a deliberately bounded 2D graphics vertical
slice.

The browser renderer is Rust compiled to WebAssembly with `wasm-bindgen`. It
uses `wgpu` with the browser WebGPU backend. There is no WebGL or WebGL2
fallback. A browser that cannot create a WebGPU adapter receives a stable,
user-visible unsupported-renderer result while the kernel session remains
usable. Native builds use the same `wgpu` renderer through the platform backend
selected by `wgpu`; no Three.js scene abstraction is introduced.

The core Rust crates compile for both ordinary native targets and
`wasm32-unknown-unknown`. WebAssembly is an adapter and deployment target, not
the canonical ownership boundary of the graphics model.

## Architecture

```text
MATLAB plot built-ins
  -> session-owned Graphics Object Model
  -> revisioned Plot HIR snapshot or delta
  -> openmat-graphics-v1 control and binary data protocol
  -> browser Plot Engine mirror
  -> layout, level-of-detail selection, and tessellation
  -> platform-neutral Plot MIR and overlay plan
  -> wgpu draw list                  -> SVG overlay
  -> WebGPU or native wgpu backend   -> text, ticks, legend, controls
```

React owns Figure windows, menus, accessibility, layout integration, and
lifecycle hooks. React does not generate geometry, issue GPU draw calls, or
store the authoritative graphics hierarchy. Pointer, wheel, resize, and device
events are forwarded to the Plot Engine adapter. Per-frame pan, zoom, picking,
and overlay updates do not require React reconciliation.

## Graphics object semantics

The Graphics Object Model precedes Plot HIR. A plot call is not an immediate
draw command: it creates or mutates session-owned objects and returns an opaque
handle with aliasing semantics. The first hierarchy is:

```text
Figure
  -> Axes2D
       -> LineSeries
       -> ScatterSeries
       -> Text
       -> Legend
```

Every object has a stable slot and generation. Deleting an object invalidates
all handles carrying the old generation; slot reuse never makes a stale handle
valid. Figure and object revisions are monotonic safe JSON integers. Graphics
handles are a dedicated runtime value category backed by the graphics arena,
not Rust references and not ordinary user `classdef` object records.

The session owns current-figure and current-axes state. Copying a graphics
handle preserves identity. The first tranche supports scalar handles and
homogeneous `N x 1` graphics-handle arrays because MATLAB R2022b returns one
Line handle per plotted column. Source numerical values are copied across the
language assignment boundary into immutable graphics data resources; later
mutation of the source array does not mutate an existing series.

## HIR and MIR boundary

Plot HIR contains scientific meaning and retained object properties. It uses
stable object identifiers and immutable `DataRef` resources. HIR includes axes
scales and kernel-resolved semantic limits, series data and style, exact UTF-16
text intent, legend membership, visibility, clipping intent, and parent/child
order. It contains no DOM nodes, WASM pointers, `wgpu` resources, or backend
handles.

The kernel Graphics Object Model is authoritative for MATLAB-observable axes
limits and limit modes. The browser consumes those limits for ticks and
projection; renderer-local navigation may apply a temporary view transform but
is explicitly outside the R2022b compatibility surface and does not alter a
subsequent language-level `xlim` or `ylim` query.

Plot MIR contains platform-neutral render operations such as clipped stroke
paths, marker instances, triangle meshes, image quads, grid lines, and picking
identifiers. It also produces an overlay plan containing positioned text,
ticks, legend entries, and accessibility labels. MIR contains no serialized
`wgpu` handles. A backend draw-list compiler may turn MIR into pipeline keys,
buffer ranges, bind groups, scissor rectangles, and render passes.

Logical `Polyline` operations do not require hardware line primitives. Width,
join, cap, dash, and discontinuity semantics are tessellated into triangles
where necessary.

## Process and data boundary

The native kernel and browser WebAssembly module do not share an address
space. `TensorRef` or `DataRef` therefore means an opaque protocol resource,
never a native or WASM pointer. The graphics protocol uses bounded JSON control
messages and canonical little-endian binary frames. Browser ingestion may
require one CPU copy into WASM memory before geometry generation and one GPU
upload; the project does not describe this boundary as universally zero-copy.

The frozen kernel-v0, kernel-v1, and kernel-v2 wire contracts remain text-only.
A small MIME display representation announces a newly discovered Figure and
grants an unguessable, session-scoped attachment token. It is repeated only
for token rotation or explicit reattachment, not for every Figure delta.
Snapshot, delta, and binary data traffic then uses the independently versioned
`/graphics/v1` endpoint defined in `spec/protocol/graphics-v1.md`. The new MIME
string uses the existing `displayMimeTypes` negotiation; no kernel envelope
shape changes.

Rust enum layout and Rust ABI are never a process boundary. Tauri and native
hosts reuse the same semantic protocol even if a same-process embedding later
adds an optimized adapter.

## Numerical precision

Graphics data preserves its source `f32` or `f64` representation and MATLAB
column-major ordering at the data boundary. WebGPU shaders do not provide a
portable concrete `f64` type. Layout and axes transforms therefore run in Rust
`f64`; coordinates are translated relative to an axes-local origin and scaled
before conversion to GPU `f32`. Implementations must not cast absolute source
coordinates directly to `f32` when that would collapse visible differences.

NaN separates line runs. Infinity, empty inputs, degenerate spans, signed zero,
large offsets with small spans, and log-domain rejection are explicit test
categories. The first tranche implements linear axes only; the representation
reserves a scale enum for later logarithmic axes.

## First vertical slice

The first language/API tranche contains scalar and homogeneous row-vector
graphics handles and these ordinary functions or command forms:

- `figure`, `axes`, `gcf`, and `gca`;
- `plot(Y)` and `plot(X,Y)` for real `single` and `double` vectors and matrices;
- `scatter(X,Y)` for real vectors;
- `hold on`, `hold off`, and `ishold` query behavior;
- `title`, `xlabel`, `ylabel`, and `legend` for text scalar inputs;
- `xlim`, `ylim`, `grid`, `cla`, `clf`, and `close` in their basic forms.

`LineSpec`, style name/value arguments, public `get`/`set` or dot-property
mutation, complex plot overloads, datetime and categorical axes, callbacks, UI
controls, property listeners, animation timing, subplots, tiled layout,
images, contouring, and 3D are later tranches. The object model must permit
those additions without changing object identity.

## Text and export

The Web implementation uses SVG for axes text, titles, labels, legend, and a
bounded number of annotations. HTML is reserved for controls and tooltips. The
Rust layout core selects ticks and formats labels; the browser supplies measured
font metrics when exact layout needs them. GPU and overlay layers share the
same viewport and device-pixel-ratio transform.

SVG/DOM is not part of Plot MIR. A future native text backend and SVG/PDF export
backend consume the same overlay plan. Pixel screenshots are useful OpenMat
regressions but are not the MATLAB semantic compatibility oracle.

## Compatibility and validation

MATLAB R2022b behavior is measured with locally authored black-box programs.
Compatibility tests compare a normalized semantic projection: Figure/Axes/
series roles, public class names where implemented, data, supported property
values, limits, ordering, and handle lifetime. Internal Rust type names and
arena layout are not compared to MATLAB internals. Tests do not copy MATLAB
source, tests, documentation, or messages and do not require cross-product
pixel equality.

Deterministic HIR-to-MIR tests, protocol negative tests, geometry invariants,
browser WebGPU smoke tests, device-loss tests, and large-data performance
budgets are required before a tranche is called complete.
