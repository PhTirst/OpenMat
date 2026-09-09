# OpenMat graphics protocol v3

This document defines the multi-Axes interaction extension to
`openmat-graphics-v2`. Graphics v3 is a strict scene and binary superset: all
snapshot, delta, buffer, lifecycle, limit, error, and response rules retain the
meaning defined by [`graphics-v1.md`](graphics-v1.md) and
[`graphics-v2.md`](graphics-v2.md), except for the explicit interaction target
defined below.

## Version selection and discovery

The exact identifier is `openmat-graphics-v3` and the WebSocket endpoint is
`/graphics/v3`. New Figure discovery uses `schemaVersion: 3`, this identifier,
and this endpoint. The server continues to serve the frozen v1 and v2
endpoints, and each endpoint accepts only its matching envelope identifier.

## Explicit Axes interaction target

Every v3 `setAxesLimits` and `setAxesCamera` request must include a non-empty
opaque `axesId` next to `figureId`:

```json
{
  "type": "setAxesLimits",
  "figureId": "figure-id",
  "axesId": "axes-id",
  "expectedRevision": 4,
  "xLimits": [-2.0, 8.0],
  "yLimits": [0.25, 16.0]
}
```

The target must be a live Axes whose direct parent is the requested Figure.
The server validates both identifiers, ownership, revision, and all property
values before mutation. A successful request changes only the named Axes and
advances the owning Figure by exactly one revision. A missing, unknown,
wrong-class, or cross-Figure `axesId` is `graphics.invalidRequest` and changes
no state or journal entry.

The v2 forms remain unchanged and must omit `axesId`; sending it to the v2
endpoint is `graphics.invalidRequest`. Conversely, omitting `axesId` from a v3
interaction request is `graphics.invalidRequest`. Response and subsequent
`figureDelta` ordering are unchanged from v2.

Graphics v3 adds no scene or binary representation. OMGP therefore keeps the
v1 binary prefix and negotiated limits unchanged.
