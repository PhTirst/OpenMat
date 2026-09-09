# OpenMat graphics protocol v2

This document defines the minimal interaction extension to the frozen
`openmat-graphics-v1` contract. Graphics v2 is a strict superset: unless this
document says otherwise, every discovery field, text message, Figure HIR
snapshot, delta, resource rule, binary frame, limit, lifecycle rule, and error
category has exactly the meaning specified by
[`graphics-v1.md`](graphics-v1.md). Implementations reuse the same scene and
binary representations; they do not copy or fork Plot HIR.

## Version selection and discovery

The exact v2 protocol identifier is `openmat-graphics-v2` and its WebSocket
endpoint is `/graphics/v2`. The server continues to serve `/graphics/v1` with
the frozen v1 behavior. An endpoint accepts only its matching envelope
identifier; protocol mixing is `graphics.invalidRequest` and does not attach a
session.

New Figure discovery defaults to this v2 shape:

```json
{
  "schemaVersion": 2,
  "graphicsProtocol": "openmat-graphics-v2",
  "endpoint": "/graphics/v2",
  "attachToken": "opaque-session-capability",
  "figureId": "figure-opaque-id",
  "revision": 1
}
```

The MIME type remains `application/vnd.openmat.figure+json`. A client selects
the protocol and endpoint declared by discovery. It may retain a v1 parser for
older servers, but must not silently reinterpret an unknown version. Token
entropy, lifetime, constant-time validation, redaction, origin restrictions,
and single-active-attachment behavior are unchanged.

## Atomic semantic limits request

After successful v2 initialization, one completed pan, wheel-zoom, or box-zoom
gesture may send at most one `setAxesLimits` request:

```json
{
  "protocol": "openmat-graphics-v2",
  "sessionId": "session-1",
  "messageId": "limits-1",
  "kind": "request",
  "request": {
    "type": "setAxesLimits",
    "figureId": "figure-id",
    "expectedRevision": 4,
    "xLimits": [-2.0, 8.0],
    "yLimits": [0.25, 16.0]
  }
}
```

`figureId` is a non-empty opaque identifier owned by the attached graphics
session. `expectedRevision` is a positive JSON-safe integer and must equal the
Figure's current revision. `xLimits` and `yLimits` each contain exactly two
finite numbers in strictly increasing order. Validation of the figure,
revision, both pairs, and revision capacity happens before mutation.

The owning `GraphicsSession` selects the existing Axes associated with the
target Figure and atomically assigns both resolved limit pairs. Both
`xLimitsMode` and `yLimitsMode` become `manual`. The operation is one graphics
transaction: it either commits all four property changes and advances the
Figure revision exactly once, or changes no model state, revision, or journal
entry. It does not execute or generate MATLAB source.

An unknown or non-live Figure returns `graphics.unknownFigure`. A stale
`expectedRevision` returns `graphics.revisionConflict`. Invalid limit values or
a target without an applicable 2D Axes return `graphics.invalidRequest`.
Revision exhaustion retains the v1 no-mutation behavior. `setAxesLimits` sent
to `/graphics/v1` receives `graphics.unsupportedRequest`.

On success, the server returns:

```json
{
  "protocol": "openmat-graphics-v2",
  "sessionId": "session-1",
  "messageId": "server-response-1",
  "kind": "response",
  "replyTo": "limits-1",
  "ok": true,
  "result": {
    "type": "setAxesLimits",
    "committedRevision": 5
  }
}
```

The success response is sent after the model commit and before publishing that
request's `figureDelta` on the connection. `committedRevision` equals the
delta's `revision`; the delta has the client's `expectedRevision` as
`baseRevision` and contains the complete upserted `axes2d` object. The client
does not synthesize a snapshot or optimistically edit axes properties from the
response. It advances its authoritative scene by applying the ordinary delta,
or uses the ordinary `resyncFigure` recovery if a revision gap is observed.
The owning journal makes the same committed delta available after reconnect,
so every client mirror converges through the existing snapshot/delta rules.

Unknown v2 request types still receive `graphics.unsupportedRequest`. Text and
binary frames retain the negotiated v1 maxima. The OMGP binary prefix keeps
protocol major `1`, minor `0`, because v2 adds no binary representation and
uses the frozen v1 binary contract verbatim.
