# Plot Engine 2D completion milestone

Status: complete (2026-08-26). This milestone extends the accepted Plot Engine v1
architecture without introducing a second renderer or a WebGL fallback.

## Completion boundary

The milestone is complete only when one native Kernel session can create,
style, interact with, update, and close multiple 2D Figures through the Web IDE
without recreating the page-wide WebGPU device or losing MATLAB-visible
graphics state.

The required language surface is:

- `plot` with the common LineSpec forms and atomic name/value styling;
- scalar-style `scatter` with common marker size, marker, and color properties;
- `set` and `get` for the supported Line and Scatter properties;
- `title`, `xlabel`, `ylabel`, `legend`, `grid`, `xlim`, and `ylim` with the
  R2022b behavior accepted by the clean-room corpus;
- existing Figure, Axes, hold, clear, close, handle identity, and color-order
  behavior without regression.

Property validation precedes every graphics mutation. A bad property name,
value, LineSpec, target class, or stale handle must leave the Graphics Object
Model and its revision unchanged.

## Interaction ownership

Pan, wheel zoom, box zoom, and pointer picking have two deliberately separate
state rates:

1. Pointer-frequency changes are renderer-local view transforms. They do not
   serialize source arrays, run generated MATLAB source, or update React state
   for every pointer event.
2. Gesture completion commits at most one semantic limits mutation to the
   Kernel. The committed X/Y limits become authoritative, use manual limit
   mode, and are observable by a later `xlim` or `ylim` call.

Home restores the latest authoritative automatic or explicitly assigned view.
A reconnect discards an uncommitted transient transform and reconstructs the
view from the authoritative snapshot. Renderer-local navigation never changes
the source HIR arrays.

Picking identifiers are frame-local renderer data associated with opaque
graphics object identifiers. They are not persistent object identity and must
not expose Rust addresses, arena slots, or generations at the protocol
boundary. A data cursor result names the graphics object and source element,
then derives its displayed coordinates from the exact source data.

## Visual acceptance

Both Modern Light and Modern Dark must render:

- axes lines, major ticks, major grid lines, and clipped plot geometry;
- bounded tick labels with stable scientific notation;
- title, X label, Y label, and Unicode text;
- a real legend containing the visible named series in display order;
- correct CSS-pixel geometry at device pixel ratios 1, 1.25, 1.5, and 2;
- usable output in non-square and small resizable Figure windows.

Browser font metrics are authoritative for overlay placement. Layout remains a
two-pass operation: Rust produces measurement requests and semantic placement
intent; the browser returns measured CSS-pixel metrics; Rust completes the
overlay plan. Font load, theme, or font-property changes increment a font cache
revision. Device-pixel-ratio changes invalidate device rendering but do not
change CSS font metrics by themselves.

## Large-data acceptance

Line rendering uses a deterministic per-screen-column min/max envelope when
the visible source exceeds the direct-geometry budget. The LOD result must:

- preserve finite-run first and last points, local extrema, isolated spikes,
  source order, and NaN/Inf run boundaries;
- be bounded by viewport width and finite-run count rather than source length;
- use `f64` for range classification and axes transforms before device-space
  conversion;
- cache by source resource identity/revision, visible range, and viewport
  width;
- avoid invalidating source/LOD caches for color, width, marker, or visibility
  changes that do not alter X/Y data.

The acceptance workload includes one million points, repeated warm pan/zoom,
multiple simultaneous Figures, and a style-only delta. Unit tests use output
bounds and operation counts rather than fragile sub-millisecond deadlines. A
real WebGPU browser smoke records first-visible latency, warm interaction
latency, resident bytes, source-buffer upload count, and console errors.

## Delivery waves

Wave one develops independent foundations:

- Graphics Object Model properties and built-in parsing;
- renderer-local interaction and picking primitives;
- overlay layout and visual fidelity;
- large-line LOD;
- clean-room R2022b property and lifetime observations.

Wave two performs the shared integration:

- export new MIR/geometry APIs;
- connect property mutations through Runtime and graphics deltas;
- connect Figure toolbar and pointer events to the imperative WASM API;
- add the versioned semantic-limit mutation required at gesture completion;
- integrate LOD selection and cache invalidation in scene preparation;
- update conformance indexes and compatibility counts.

Wave three is the release gate: all locked Rust/Web tests, wasm32 checks,
Clippy with warnings denied, production Web build, conformance comparison, and
a real Windows WebGPU multi-Figure interaction/performance smoke.

Three-dimensional axes, cameras, surfaces, color maps, and color bars are not
part of this milestone. They begin only after these 2D contracts are stable.

## Release-gate evidence

The completed integration passes the 13-case MATLAB R2022b `graphics_2d_*`
differential corpus, all locked Rust workspace tests, Clippy across all targets
and features with warnings denied, the wasm32 Plot Engine check, 175 Web tests,
TypeScript checking, and the production Web/WASM build. A real Intel/Vulkan
adapter creates the wgpu device and shader pipelines.

The Windows browser smoke created three independent `graphics-v2` Figure
windows, committed box-zoom limits once per gesture, returned exact source
coordinates from the data cursor, and rendered a one-million-point sine curve
followed by a style-only revision with no browser console errors. The source
pair occupies 16,000,000 bytes; screen-column LOD keeps generated line geometry
viewport-bounded. A 4 MiB real TCP regression test verifies that graphics
buffer backpressure no longer disconnects Windows WebSocket clients.
