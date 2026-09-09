# Plot interaction and picking foundation

## Scope

This slice adds the first renderer-local 2D interaction loop without changing
graphics-v1, Kernel messages, React Figure state, or immutable source arrays.
`DataRect` and `ViewTransform2D` live in Plot MIR so CPU tests and future native
frontends use the same math. WGPU retains one compiled/uploaded frame per
Figure, while Device, Queue, uniform binding, and all render pipelines remain
shared per browser worker/page.

## Coordinate contract

- **CSS pixels** are canvas-local browser event coordinates with X right and Y
  down. Pan deltas, wheel anchors, box selections, and pick tolerance use this
  space.
- **Device pixels** are the independently rounded canvas backing dimensions.
  Mapping uses the actual `DeviceRect`; it does not assume that CSS coordinates
  multiplied by DPR are integral.
- **Axes coordinates** are normalized with X right and Y up. Immutable MIR/GPU
  vertices are normalized against the semantic home limits.
- **Data coordinates** are finite `f64` semantic X/Y values. Current limits must
  have positive non-overflowing spans.

`ViewTransform2D` maps source axes through home/current semantic limits into
view axes. The GPU receives only a four-value affine uniform. Home, pan,
pointer-anchored wheel zoom, and viewport-clamped box zoom replace that small
state and redraw the retained `GpuFrame`; they do not recreate a Device or
Pipeline and do not recompile or re-upload geometry.

Home is an explicit reset target, not a navigation boundary. Pan may move beyond
the original data domain, and anchored wheel/box zoom operates on that current
view. Zoom-in has a relative minimum span of `1e-12` of home. Non-finite values,
non-positive zoom factors, zero data
spans, degenerate viewports, zero-area boxes, and f64/f32 arithmetic overflow
return typed errors.

## Picking

`PickingId` reserves zero for no hit. During draw-list compilation, a
`PickingIndex` is built beside the GPU payload:

- grid/source segments are indexed directly;
- tessellated stroke quads are deterministically reduced back to centerline
  segment candidates;
- Marker instances are indexed by center and CSS-pixel radius.

Queries transform candidates through the current local view and compute distance
in CSS pixels, so tolerance is stable across DPR. Equal-distance candidates
prefer later draw order, then lower stable picking ID and primitive index. The
WASM adapter assigns per-frame IDs to visible line/scatter objects and returns
the graphics-v1 object ID without modifying the protocol.

## Browser imperative API

The narrow `PlotFigure` API is:

- `beginInteraction()`
- `panBy(dxCssPx, dyCssPx)`
- `wheelZoom(xCssPx, yCssPx, deltaY)`
- `boxZoom(x1CssPx, y1CssPx, x2CssPx, y2CssPx)`
- `homeView()`
- `pick(xCssPx, yCssPx, toleranceCssPx)`
- `endInteraction()`

`endInteraction()` returns `{ xLimits, yLimits, changed }`. It is a structured
handoff point only; this slice does not update Kernel state or graphics-v1.
`plot-interaction.ts` owns pointer capture, pan deltas, box state, and wheel
coalescing outside React. Its commit callback runs once at pointer end or after
the wheel quiet period.

Snapshot/delta changes and resize invalidate only the Figure-local retained GPU
frame. Dispose clears that frame, picking map, buffers, and pending gesture
state. A lost shared device makes local redraw fail with `deviceLost`; the shared
context cache will not reuse it for a later Figure.

## Marker coverage

All existing MIR marker shapes compile and render through instanced WebGPU
pipelines: Circle, Square, Diamond, UpTriangle, DownTriangle, Plus, and Cross.
Each uses the same premultiplied fill/stroke and derivative-based antialias
coverage contract. No scene/model/protocol change is part of this slice.

## Integration boundary

The coordinating UI task still needs to instantiate
`createPlotInteractionController` for each Figure canvas, render any desired box
selection/tooltip overlay, and decide when an `endInteraction()` result should
be committed as semantic `xlim`/`ylim` to Kernel. That commit must be one
gesture-end operation, not a stream of pointer-move deltas.
