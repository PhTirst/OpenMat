# Plot Engine v1 implementation design

Status: implementation contract for RFC 0005 and `openmat-graphics-v1`.

## Crate boundaries

The first implementation uses these dependency-ordered crates:

```text
openmat-graphics-model
  session-owned object arena, generational handles, HIR snapshots and deltas

openmat-plot-protocol
  graphics-v1 text models, binary framing, validation, negotiated limits

openmat-plot-mir
  backend-neutral render and overlay operations

openmat-plot-layout
  f64 axes limits, tick selection, viewport transforms, overlay layout intent

openmat-plot-geometry
  line-run splitting, stroke tessellation, markers, clipping, LOD interfaces

openmat-plot-wgpu
  MIR draw-list compilation, WebGPU/native wgpu resources and rendering

openmat-plot-web
  wasm-bindgen lifecycle, canvas, pointer/resize bridge, binary ingestion
```

Low-level model, protocol, MIR, layout, and geometry crates do not depend on
kernel, server, runtime, React, `web-sys`, or `wgpu`. `openmat-plot-wgpu`
depends on MIR and consumes prepared geometry. `openmat-plot-web` is the only
crate that exports `wasm-bindgen` browser entry points. The root workspace and
shared dependency versions are coordinated centrally.

Type ownership is intentionally not shared. `openmat-graphics-model` owns the
internal arena and semantic HIR. `openmat-plot-protocol` owns wire DTOs,
validation, and framing and does not depend on the model crate. A kernel/server
adapter performs the explicit one-way conversion. Layout and geometry own
small `SeriesLayoutInput` and `SeriesGeometryInput` views and do not depend on
either HIR or protocol DTOs.

## Runtime integration

`openmat-value` adds `GraphicsHandle` and homogeneous `GraphicsHandleArray`
value categories containing opaque object slots, generations, and one public
graphics class tag. They are handle values: language copy preserves each
identity. The first array form is `N x 1`, matching the Line handles returned
for matrix columns. Mixed-class and higher-dimensional graphics arrays are
structured unsupported boundaries rather than ordinary object or numeric
coercions.

The interpreter owns one `GraphicsSession`. Built-ins receive a narrow
`GraphicsService` through `BuiltinContext`, analogous to the existing output,
cancellation, numerical-provider, and language-copy services. Standalone
built-in tests without a graphics service return a structured host-service
error; they never create process-global Figure state.

An atomic graphics operation returns both its language result and a committed
`GraphicsNotice { figure_id, revision, discovery }`. The runtime output
boundary adds a graphics notice event without embedding numerical buffers or
an attachment token. The Server owns token generation and validation, registers
the session's shared graphics hub, and decorates discovery notices with the
`application/vnd.openmat.figure+json` descriptor. Ordinary updates use the
graphics Delta channel and do not emit a second discovery Display.

Graphics state is session-scoped, not workspace-scoped. `clear` does not close
Figures; kernel shutdown does. `close`, `clf`, and `cla` perform explicit arena
transactions. The atomic boundary is one graphics built-in call, not the whole
source execution: a later unrelated language error never rolls back an earlier
successful plot. Validation failure or cancellation before commit publishes no
partial HIR or data resource. Once committed, state remains committed even if
delivery of its notice fails; reconnect snapshot recovery is authoritative.

## Generational arena

Internal object identity is:

```text
GraphicsHandle {
    slot: u32,
    generation: u32,
    class: GraphicsClass,
}
```

Slot zero and generation zero are invalid. A free-list may reuse a slot only
after incrementing its generation with checked arithmetic. Generation
exhaustion permanently retires the slot. The public wire identifier is an
opaque session-specific encoding and must not expose a stable Rust memory
address.

The arena enforces the allowed parent matrix:

| Parent | Allowed children |
| --- | --- |
| session | Figure |
| Figure | Axes2D |
| Axes2D | LineSeries, ScatterSeries, Text, Legend |
| series/text/legend | none |

Deleting Figure recursively deletes its descendants. Deleting Axes2D deletes
its descendants. Reparenting is not exposed in v1. Current Figure and Axes
pointers are cleared or deterministically advanced when their targets are
deleted.

## Plot API transaction rules

`figure()` selects an existing numbered Figure when applicable or creates one;
automatic numbering uses the lowest available positive number. `figure(n)`
selects or creates the positive integer `n`. `gcf` creates a Figure when none
exists, and `gca` creates both a Figure and Axes when needed. `axes()` creates
or selects Axes2D. Any plotting call obtains a current Figure and Axes according
to those measured rules.

Figure and Axes each retain `nextPlot`. The first tranche supports their
measured defaults and Axes `replace`/`add` transition used by `hold`; it does
not claim the complete `newplot`, `replacechildren`, or reset-property matrix.
A replacing plot transaction clears the appropriate Axes children before
adding new series; an adding transaction preserves them and advances color
order. One user call that creates multiple series publishes one Figure
revision.

`plot(Y)` and `plot(X,Y)` first normalize accepted inputs into a list of series
descriptors without mutating graphics state. Matrix orientation, scalar cases,
complex rejection, empty inputs, row/column vectors, column pairing, requested
outputs, and error conditions are validated in this planning phase. `LineSpec`
and name/value style parsing are not accepted in v1. Only after planning are
data resources and objects committed.

The exact first API matrix is:

| API | Supported v1 form | Required result |
| --- | --- | --- |
| `figure` | `figure()`, `figure(n)` | create/select Figure and return scalar handle |
| `axes` | `axes()`, `axes(h)` | create Axes or select a valid Axes handle |
| `gcf`, `gca` | zero inputs | return current object, creating measured defaults |
| `plot` | `plot(Y)`, `plot(X,Y)` | real single/double scalar, vector, or matrix; return `N x 1` Line handles (`0 x 1` for no series) |
| `scatter` | `scatter(X,Y)` | equal-length real vectors; return scalar Scatter handle |
| `hold` | `hold on`, `hold off`, optional Axes target | set Axes add/replace state |
| `ishold` | zero inputs or one Axes | logical scalar |
| labels | `title(s)`, `xlabel(s)`, `ylabel(s)` | Unicode-preserving text scalar; return Text handle |
| `legend` | text scalar labels for current series | create/update and return Legend handle |
| limits | `xlim()`, `ylim()`, or one finite increasing pair | query or set semantic limits |
| `grid` | `grid on`, `grid off` | set both basic grid flags |
| clearing | `cla`, `clf`, optional valid target | delete the measured descendant set |
| `close` | zero inputs or one Figure | delete Figure; zero inputs targets current Figure |

Other argument counts, target kinds, modes such as `auto`/`tight`, `reset`,
`close all`, and property argument forms are structured unsupported in v1.

## HIR representation

HIR mirrors the wire object kinds but uses strongly typed Rust structs and
enums. It stores source `f32`/`f64` bytes in immutable, copy-on-write data
resources. In v1 each series owns independent contiguous `1 x N` X/Y resources;
matrix columns are copied during the atomic planning/commit boundary. Strided
views are deferred so the wire `DataRef` remains unambiguous.

Snapshot serialization traverses Figure children in stable display order.
Delta generation records full-object upserts, deletions, and child reorderings
inside one transaction. A bounded revision journal may serve reconnect deltas;
if the requested base revision is no longer retained, the service returns a
complete snapshot.

## Layout and precision

All data bounds, automatic limits, tick selection, and world-to-axes transforms
use `f64`. The canonical auto-limit algorithm is supplied by
`openmat-plot-layout` and runs on the kernel side when a transaction changes
auto-limited series. The Graphics Object Model stores the resulting limits and
modes; the browser receives them rather than independently inventing observable
limits. The layout stage produces an affine axes-local transform:

```text
normalized = (source - origin_f64) * scale_f64
```

Geometry is converted to `f32` only after this subtraction and scaling. Tests
include values near `1e12` with unit-scale differences, subnormal spans,
constant vectors, signed zero, NaN-separated runs, infinities, empty vectors,
and non-square viewports.

The first auto-limit implementation is deterministic and separately tested
against authored R2022b observations. Renderer viewport resize does not mutate
kernel-owned data; client-local pan/zoom creates a renderer-only view transform
and is not an R2022b-compatible property mutation. A future explicit
synchronization request may commit limits to the kernel.

## MIR and geometry

The first MIR includes:

```text
BeginViewport
GridLineBatch
StrokeMesh
MarkerBatch
SetClipRect
EndViewport
```

Every operation includes a stable draw order and optional picking identifier.
Stroke geometry defines width in CSS pixels, joins, caps, dash phase, and NaN
run boundaries. Markers use instancing where the backend permits it, but MIR
does not require a particular GPU instancing API.

The overlay plan includes axis lines, tick positions and formatted labels,
title/x-label/y-label, legend box and entries, accessible descriptions, and
the shared CSS-pixel viewport rectangle. Exact text is UTF-16 code units;
lossy Unicode display text may be derived only at the rendering boundary.

Font measurement is an explicit second-pass interface:

```text
FontKey { family_utf16, size_css_px, weight, style }
TextMeasureRequest { key, code_units, font_revision }
TextMetrics { width, height, advance, ascent, descent } // CSS pixels
```

The browser increments `font_revision` and invalidates cached metrics after
font loading completion or a font/theme change. Device-pixel ratio does not
change CSS-pixel metrics but does invalidate the device-space render target.

LOD never drops an isolated extremum merely because it lies between sampled
indices. The initial large-line strategy is a per-screen-column min/max
envelope with preserved first/last and NaN run boundaries. LOD output is a view
cache and never changes source HIR data.

## wgpu and Web integration

The v1 renderer requires browser WebGPU and requests no optional WebGPU
features. Its minimum adapter profile is: four bind groups, four vertex
buffers, eight vertex attributes, a 64 KiB uniform binding, a 128 MiB storage
binding, a 256 MiB maximum buffer, an 8192 two-dimensional texture dimension,
`u32` index buffers, and an `rgba8unorm` or `bgra8unorm` presentation format.
Single-sample rendering is required; four-sample MSAA is optional. Initial line
and scatter rendering does not require compute or indirect drawing. The WASM
build enables `wgpu`'s WebGPU backend and does not enable its WebGL backend.
Adapter absence, insufficient limits, device creation failure, surface loss,
outdated surface, timeout, and device loss are typed renderer states.

The browser Figure component dynamically loads the Plot WASM bundle on first
use. This keeps the existing IDE startup path independent of the renderer's
bundle size. A `ResizeObserver` supplies CSS size and device-pixel ratio. Canvas
reconfiguration and render scheduling are imperative. React stores only window
and connection state.

SVG overlay nodes use the layout core's stable keys. High-frequency camera or
viewport changes update a dedicated overlay controller rather than serializing
all geometry or routing every coordinate through React state.

## Verification strategy

Each layer has an independent gate:

- graphics model: arena generation, hierarchy, transaction rollback, hold and
  deletion semantics;
- protocol: serde round trips, malformed text, binary prefix/length/overflow,
  chunk gaps/overlaps, resource limits, revision conflict and reconnect;
- layout: exact or tolerance-based deterministic bounds/ticks/transforms;
- geometry: finite output, clipping, NaN runs, joins, winding, index bounds and
  LOD extrema preservation;
- wgpu: shader/pipeline creation, resize, device-loss state machine, readback
  invariants where supported;
- Web: protocol-to-WASM lifecycle, WebGPU unsupported UI, multiple Figures,
  close/reconnect and accessible overlay;
- compatibility: MATLAB R2022b black-box property observations, not screenshot
  identity;
- performance: bounded resident bytes, first-frame time, million-point upload,
  steady pan latency, and no full-buffer resend for style-only deltas.

Browser smoke tests require an actual WebGPU adapter. Environments without one
report that gate as unavailable rather than passing through a mock renderer.
