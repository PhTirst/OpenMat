# Plot Engine 2D visual fidelity

This implementation is based on integration commit
`pre-publication baseline`.

## Completed visual path

- Axes lowering now emits responsive linear ticks, axis marks, low-alpha major
  grids, exact UTF-16 tick labels, and measured title/x-label/y-label spacing.
  Tick density follows the axes CSS extent, so narrow and non-square canvases
  do not retain a desktop-sized fixed tick target.
- The axes palette is selected from Figure background luminance. Light Figures
  use MATLAB-like `#262626` ink and a 0.16-alpha grid; dark Figures use light
  ink and a 0.22-alpha grid. CSS keeps Modern Light scientific surfaces white
  and gives Modern Dark Figure chrome an explicit high-contrast boundary.
- A visible protocol `Legend` is lowered into the existing MIR `LegendPlan`.
  Layout measures all labels, places the box at the requested compass location
  (`best` currently resolves to northeast), and emits background, border,
  per-series swatches, and UTF-16 label text to the browser overlay.
- Resize observes CSS pixels and independently resamples DPR. Every effective
  CSS-size or DPR change resizes and re-renders the retained WASM Figure. The
  observer, window listener, font listener, and late async WASM load all have
  explicit disposal paths.
- SVG text uses the protocol font, alignment, baseline, rotation, color, and
  UTF-16 code units. The Figure canvas has an image role and its hidden caption
  includes graphics revision and Legend labels.

## Explicit axes and interpreted text

The V1 scene schema has additive, defaulted axes fields for `xTick`, `yTick`,
their labels and auto/manual modes, `box`, font size, axis line width, and the
tick-label interpreter. Missing fields in an older snapshot or delta decode to
the R2022b-compatible defaults: automatic ticks and labels, box off, a 10 pt
(40/3 CSS px) font, a 0.5 pt (2/3 CSS px) axis line, and the `tex` interpreter.

Tick values must be non-NaN and strictly increasing, but may be outside the
limits or infinite. Because JSON cannot encode non-finite numbers, the protocol
uses the exact strings `"Infinity"` and `"-Infinity"` for those two values.
Layout discards non-finite or non-visible ticks without modifying the retained
property. Manual tick-label arrays are deliberately not required to match the
tick count: non-empty arrays cycle by the tick's original property index,
longer arrays are naturally truncated, and an explicitly empty array produces
blank labels. `box` adds the top and right spines while preserving the bottom
and left axes.

Text, Legend, and tick labels carry a `tex`, `latex`, or `none` interpreter.
The browser retains the SVG `<text>` fast path for literal text and TeX strings
without markup. Math content is rendered by KaTeX in an SVG
`foreignObject`; its HTML is theme-colored, aligned at the MIR anchor, and
rotated around that same anchor. KaTeX is configured with `trust: false`.
Invalid markup produces KaTeX's non-throwing error rendering rather than
breaking the Figure render. This overlay does not introduce a WebGL path: plot
geometry remains on the existing WebGPU renderer.

## Browser-authoritative font measurement

`FigureWindow.tsx` exports a host-side two-stage contract:

1. `createSvgTextMeasurementController(svg, fontRevision, callback)` measures
   each `data-overlay-text-key`. SVG text uses the browser-selected font and
   Canvas `TextMetrics`; KaTeX HTML uses its laid-out DOM box. It remeasures
   after `document.fonts.ready` and `loadingdone`, and its `dispose()` removes
   the listener.
2. `PlotTextLayoutBridge.completeTextLayout(fontRevision, measurements)` is the
   renderer callback consumed by the controller when present. Measurement
   batches contain `widthCssPx`, `heightCssPx`, `ascentCssPx`, `descentCssPx`,
   and `advanceCssPx` for stable text keys. A fingerprint prevents a returned
   overlay from causing an effect loop.

The WASM Figure retains the `FirstLayoutPass`, validates measurement revision
and keys, completes the second pass, and returns an updated overlay DTO through
`completeTextLayout`. The same loop covers plain and KaTeX text, so measured
titles, rotated axis labels, Legend bounds, and interpreted tick labels all
feed back into stable layout.

## MarkerIndices property and rendering boundary

`LineSeries.markerIndices` is deliberately nullable:

- `null` or a missing legacy field means automatic mode. The renderer may use
  the currently selected LOD points, and the upstream model can keep extending
  the effective default as data grows.
- An array means manual mode. The array is preserved exactly, including an
  empty array, duplicates, unordered entries, and positive entries beyond the
  current X/Y length. Values use canonical positive decimal strings so every
  `u64` survives JavaScript without IEEE-754 rounding.

Manual marker geometry reads the original X/Y arrays using one-based indices,
independently of line LOD. It emits entries in property order, including
duplicates, and skips only points that are currently missing, non-finite, or
clipped. It never clamps, sorts, deduplicates, or rewrites the property.
Protocol decoding rejects zero and non-canonical/non-integer encodings; matrix,
complex, NaN, and Inf assignment rejection remains an upstream graphics-model
responsibility.

## Line and marker boundaries

`scene.rs` maps every wire line style into MIR `StrokeStyle`:

| Wire style | CSS dash pattern multiplied by `max(line width, 1px)` |
| --- | --- |
| `solid` | empty |
| `dash` | `[6, 3]` |
| `dot` | `[1, 2.5]`, round cap |
| `dashDot` | `[6, 3, 1, 3]`, round cap |

The scene no longer rejects non-solid styles, but the shared geometry boundary
still returns `GeometryError::UnsupportedDashPattern` at
`crates/openmat-plot-geometry/src/stroke.rs` before tessellation. Integration
must implement dash splitting/phase handling there; the existing MIR fields
`dash_pattern_css_px` and `dash_phase_css_px` are sufficient, so no MIR or
protocol change is needed.

Wire markers currently mapped to existing MIR are Point/Circle, Square,
Diamond, TriangleUp, TriangleDown, Plus, and Cross. The shared WebGPU compiler
at `crates/openmat-plot-wgpu/src/compiler.rs` currently accepts only Circle and
Square, so it must add pipelines/SDF handling for the five already-existing MIR
variants.

The remaining protocol markers need shared MIR work:

- Add `RightTriangle`, `LeftTriangle`, `Star`, `Pentagram`, and `Hexagram` to
  `openmat_plot_mir::MarkerShape`, then handle them in the WebGPU compiler and
  renderer pipeline selection. `Marker::None` remains a no-marker sentinel.
- Alternatively, Right/Left triangles can reuse a triangle primitive only
  after geometry exposes per-style marker rotation; today `marker_batch`
  hardcodes every `MarkerInstance.rotation_radians` to zero.

These changes intentionally remain outside this task's exclusive paths.
