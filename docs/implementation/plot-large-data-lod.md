# Plot Engine 2D large-data LOD core

Status: geometry core and public exports implemented; frame integration and Web cache
ownership remain integration-task boundaries.

## Selection contract

`min_max_lod` consumes read-only `SeriesGeometryInput` views plus a
`LinearLodView`. The view validates a finite increasing visible X range and derives
the number of physical screen columns as
`ceil(css_width * device_pixel_ratio)`. The source HIR arrays and decoded source
buffers are never rewritten, reordered, or replaced by the selection.

The selector treats every maximal finite span as an independent line run. For each
run it retains:

- the run's original first and last point, even when they are outside the visible X
  range, so subsequent clipping keeps the run topology;
- the first, minimum-Y, maximum-Y, and last source point in every touched physical
  screen column;
- the first and last marker of every contiguous non-finite span, including their
  actual `NaN` or `Infinity` kind.

Candidates are emitted only during a second forward scan of the same run. Output is
therefore in strictly original source order without sorting by X. Repeated X and
non-monotonic X revisit the same bounded column table instead of creating new
consecutive buckets. Equal extrema retain the earliest source point. Geometry-output
signed zeros are canonicalized to positive zero; the immutable source buffer still
preserves its original IEEE-754 bits.

The visible maximum maps to the final column. A view spanning `-f64::MAX` to
`f64::MAX` uses a half-scaled affine ratio when the direct span would overflow;
subnormal finite spans use the ordinary subtraction-first affine mapping.

## Complexity and output bound

Let `n` be the input length, `C = ceil(css_width * DPR)`, `R` the number of finite
runs, `B` the number of contiguous non-finite runs, and `T` the sum of touched
columns over all finite runs. The implementation performs two forward visits per
finite point plus constant boundary lookahead, so time is `O(n)`. Scratch storage is
`O(C)`, independently of `n`.

The exact structural bound exposed by `LodOutput::structural_output_bound` is
`4*T + 2*R + 2*B`; because `T <= R*C`, a single ordinary finite line produces at
most `4*C + 2` samples. Multiple NaN-separated runs necessarily add their preserved
topology; claiming a bound independent of run count would contradict the requirement
to preserve every break. The million-point regression checks source-visit count,
bucket-update count, spike retention, and the 1600-column output bound. It deliberately
does not use a fragile sub-millisecond wall-clock threshold.

## Cache key and reuse

`LodCacheKey` consists of:

- `LodSourceKey`: independent X and Y `LodResourceRevision` values. X is `None`
  for implicit `1..=N` coordinates. An immutable protocol `bufferId` can use that ID
  as `identity` and revision zero; a mutable upstream resource must increment its
  revision whenever its bytes change.
- `LodViewKey`: canonical finite visible-X endpoint bits plus the derived physical
  column count.

The selection depends on physical column count, not on the CSS-width/DPR factorization.
For example, 320 CSS pixels at DPR 2 and 640 CSS pixels at DPR 1 share a 640-column
selection. The later axes-to-CSS transform and render target still use their actual
CSS size and DPR.

An exact cached selection is reusable only when both source resource identities and
revisions, visible X range, and physical column count match. Ordinary pan or zoom
changes the range and must select again from the immutable source. A coarser cached
envelope must never be used as the source of a zoomed result because extrema omitted
inside the old buckets cannot be recovered. Implementations may retain decoded source
buffers and the `O(C)` scratch allocation across pan/zoom, but this initial key does
not claim hierarchical or tile-cache reuse.

Y limits, colors, line width, joins, caps, visibility, picking IDs, and draw order do
not affect point selection. In particular, a style-only delta must reuse the LOD source
cache: it should invalidate only style/tessellation/MIR products as appropriate. A new
X or Y data resource identity/revision invalidates the LOD entry.

## Integration-task wiring

This task intentionally did not edit `lib.rs`, `frame.rs`, or `plot-web`. The
coordinator must make these exact changes:

1. In `frame.rs`, factor the body after `split_line_runs` into a narrow
   `MirFrameBuilder::add_line_runs(&[LineRun], ...)` entry. It should call the existing
   `axes_line_runs`, `clip_line_runs`, and `tessellate_stroke` sequence. Keep the
   current `add_line` as the non-LOD convenience path.
2. In the Web scene/cache owner, create `LinearLodView` from semantic X limits, the
   axes viewport CSS width, and the current DPR. Build `LodSourceKey` from the exact
   X/Y `DataRef.buffer_id` values (revision zero while those IDs remain immutable),
   look up or run `min_max_lod`, convert its ordered samples with
   `line_runs_from_lod`, and pass those runs to `add_line_runs` before axes mapping,
   clipping, and stroke tessellation.
3. Keep the cache above `PreparedScene`/one-frame construction so resize and
   renderer-local pan/zoom can reuse entries. Data-resource release must evict entries
   referencing the released identity. Style-only object upserts must not evict them.

The LOD path is for line strokes. Marker rendering remains source-point based unless a
separate marker-density policy is explicitly designed; silently applying the line
envelope to markers would change scatter/marker semantics.

## Verification coverage

Unit tests cover empty and single-point inputs, repeated and non-monotonic X, visible
range filtering, finite-run endpoints, compact NaN/Inf boundaries, isolated positive
and negative spikes, huge and subnormal ranges, signed zero, multiple CSS-width/DPR
combinations, cache-key equality/invalidation, reconstruction of line runs, and a
million-point deterministic complexity regression.
