# OpenMat graphics protocol v4

This document defines logarithmic and directed Axes rendering on top of
`openmat-graphics-v3`. Graphics v4 retains every snapshot, delta, binary,
interaction, lifecycle, limit, error, and response rule from
[`graphics-v1.md`](graphics-v1.md), [`graphics-v2.md`](graphics-v2.md), and
[`graphics-v3.md`](graphics-v3.md), except for the additive Axes fields below.

## Version selection and discovery

The exact identifier is `openmat-graphics-v4` and the WebSocket endpoint is
`/graphics/v4`. New Figure discovery uses `schemaVersion: 4`, this identifier,
and this endpoint. The server continues to serve the frozen v1, v2, and v3
endpoints. Every endpoint accepts and emits only its matching envelope
identifier.

Like v3, every `setAxesLimits` and `setAxesCamera` request must contain the
explicit `axesId` of a live Axes owned by the requested Figure.

## Axes scene fields

An `axes2d` object adds these properties:

```json
{
  "xScale": "linear",
  "yScale": "log",
  "zScale": "linear",
  "xDirection": "normal",
  "yDirection": "reverse",
  "zDirection": "normal",
  "visible": true
}
```

`xScale`, `yScale`, and `zScale` are either `linear` or `log`.
`xDirection`, `yDirection`, and `zDirection` are either `normal` or `reverse`.
`visible` is Boolean. The defaults are `linear`, `normal`, and `true`, so an
older stored scene can be read without changing its appearance.

Every logarithmic axis must have finite, strictly increasing, positive limits.
Zero, negative, NaN, and infinite data coordinates on a logarithmic axis do
not produce geometry. A line is split at each such coordinate; isolated
markers and scatter instances are omitted; surface and Patch cells touching an
invalid coordinate form holes. Original source indices and data values remain
unchanged for valid data-cursor results.

`reverse` changes only the axis-space mapping. Limits and explicit ticks remain
in increasing data order. Automatic logarithmic ticks use powers of ten in
data space and are placed through the same scale-and-direction transform as
the geometry.

When `visible` is false, the plot box, rulers, ticks, tick labels, grid, title,
and axis labels are not rendered. Plot children and independent objects such
as a colorbar remain visible when their own visibility permits it.

## Compatibility gate

The v1, v2, and v3 scene schemas do not represent logarithmic scales, reversed
directions, or hidden Axes. A snapshot or delta containing any of those
semantics is rejected on an older endpoint instead of being rendered with a
different meaning. Graphics v4 does not change the OMGP binary prefix or the
negotiated resource limits.
