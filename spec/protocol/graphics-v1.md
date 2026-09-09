# OpenMat graphics protocol v1

This document freezes the first Plot Engine control and binary data contract.
It is independent of the frozen kernel-v0, kernel-v1, and kernel-v2 protocols.
The normal Web binding is a WebSocket at `/graphics/v1` on the same native
server that owns the kernel session.

## Discovery and attachment

After the client and kernel negotiate the string
`application/vnd.openmat.figure+json` through the existing
`displayMimeTypes` capability, the Server announces a newly discovered Figure
through a normal kernel display event. Ordinary Figure changes are not display
events; they use `figureDelta` after attachment. The MIME UTF-8 JSON string has
exactly this v1 meaning:

```json
{
  "schemaVersion": 1,
  "graphicsProtocol": "openmat-graphics-v1",
  "endpoint": "/graphics/v1",
  "attachToken": "opaque-session-capability",
  "figureId": "figure-opaque-id",
  "revision": 1
}
```

`endpoint` is an origin-relative WebSocket path unless an embedding host has
already supplied an authenticated graphics URL. `attachToken`, `figureId`, and
all other identifiers are opaque non-empty UTF-8 strings. The Server is the
only token generator and validator. The token has at least 256 bits of
cryptographic entropy, authorizes one live kernel graphics session, must not
appear in logs, and expires on kernel session teardown or explicit rotation.
A rotated token causes a new discovery display. Figure creation still succeeds
when the MIME was not negotiated, but no discovery display is emitted.

The first graphics message is an `initialize` request carrying the attachment
token. No snapshot, buffer, or mutation operation is accepted before successful
attachment. V1 permits one active graphics connection per kernel session. A
second active attachment receives `graphics.sessionInUse`. Disconnect releases
that connection's leases; the same token may reconnect while the kernel session
remains alive. Origin and loopback restrictions match the native kernel
endpoint.

## Text envelope

Every text message is UTF-8 JSON no larger than 1,048,576 encoded bytes and
contains:

- `protocol`: exactly `openmat-graphics-v1`;
- `sessionId`: the opaque kernel session identifier;
- `messageId`: a unique opaque identifier on the connection;
- `kind`: `request`, `response`, or `event`.

A response contains `replyTo`, exactly matching one request `messageId`, and
`ok`. Successful responses contain `result`; failed responses contain `error`.
An error contains a stable OpenMat-owned `category` and non-normative `message`.
Unknown object fields and unknown event types are ignored. Unknown request
types receive `graphics.unsupportedRequest`.

Structural counts, byte lengths, offsets, revisions, and dimensions are JSON
safe integers in `0..9007199254740991`. Implementations check every sum and
product before allocating or slicing.

## Initialization

```json
{
  "protocol": "openmat-graphics-v1",
  "sessionId": "session-1",
  "messageId": "graphics-init-1",
  "kind": "request",
  "request": {
    "type": "initialize",
    "attachToken": "opaque-session-capability",
    "client": { "name": "openmat-web", "version": "0.1.0" },
    "capabilities": {
      "renderBackend": "webgpu",
      "maxTextFrameBytes": 1048576,
      "maxBinaryFrameBytes": 8388608,
      "maxBufferBytes": 268435456,
      "maxResidentBytes": 536870912
    }
  }
}
```

The server returns the common limits, its implementation identity, and the
figures currently visible to the attached session. `renderBackend` is exactly
`webgpu` in v1. Absence of a WebGPU adapter is a client renderer result and does
not weaken the transport to WebGL.

A successful initialization response is:

```json
{
  "protocol": "openmat-graphics-v1",
  "sessionId": "session-1",
  "messageId": "server-response-1",
  "kind": "response",
  "replyTo": "graphics-init-1",
  "ok": true,
  "result": {
    "type": "initialize",
    "implementation": { "name": "openmat-server", "version": "0.1.0" },
    "capabilities": {
      "renderBackend": "webgpu",
      "maxTextFrameBytes": 1048576,
      "maxBinaryFrameBytes": 8388608,
      "maxBufferBytes": 268435456,
      "maxResidentBytes": 536870912,
      "maxObjects": 100000
    },
    "figures": [
      { "figureId": "figure-opaque-id", "revision": 1 }
    ]
  }
}
```

A failed response has no `result` and contains
`"error":{"category":"...","message":"..."}`. The error message and all
logging redact the attachment token.

The negotiated maxima never exceed:

- 1 MiB per text frame;
- 8 MiB per complete binary WebSocket message;
- 256 MiB per immutable buffer;
- 512 MiB resident graphics data per session;
- 100,000 live graphics objects per session.

Hosts may negotiate stricter nonzero bounds. Limit exhaustion returns a
structured error and does not expose a partial object or buffer.

## Requests

After initialization the client may send:

- `getSnapshot { figureId }`: request one complete current Figure HIR snapshot;
- `getBuffer { bufferId }`: begin transfer of one immutable data resource;
- `releaseBuffer { bufferId }`: release the client's lease after GPU upload or
  replacement;
- `closeFigure { figureId, expectedRevision }`: request the same object deletion
  as an interactive Figure close;
- `resyncFigure { figureId, knownRevision }`: request a snapshot after a delta
  gap;
- `shutdown {}`: detach this graphics connection without shutting down the
  kernel.

Requests are correlated independently and may be in flight together. Figure
mutations are serialized by the owning kernel session. `closeFigure` rejects a
stale `expectedRevision` with `graphics.revisionConflict`.

Every post-initialize request uses this exact shape:

```json
{
  "protocol": "openmat-graphics-v1",
  "sessionId": "session-1",
  "messageId": "request-2",
  "kind": "request",
  "request": { "type": "getSnapshot", "figureId": "figure-id" }
}
```

The required members by type are:

| Type | Required request members | Successful `result` |
| --- | --- | --- |
| `getSnapshot` | `figureId` | `{ "type":"getSnapshot", "snapshot": FigureSnapshot }` |
| `resyncFigure` | `figureId`, `knownRevision` | `{ "type":"resyncFigure", "snapshot": FigureSnapshot }` |
| `getBuffer` | `bufferId` | `BufferTransferResult` followed by binary chunks |
| `releaseBuffer` | `bufferId` | `{ "type":"releaseBuffer", "released": boolean }` |
| `closeFigure` | `figureId`, `expectedRevision` | `{ "type":"closeFigure", "closedRevision": integer }` |
| `shutdown` | none | `{ "type":"shutdown" }` |

`getSnapshot` and `resyncFigure` return the snapshot directly in their response;
they do not also emit `figureSnapshot`. `closeFigure` sends its success response
before the resulting `figureClosed` event. `getBuffer` sends its success text
response before any binary chunk for that transfer.

## Events and revisions

After initialization the server may emit:

- `figureDelta`: ordered mutations from one exact base revision;
- `figureClosed`: deletion of one Figure and all descendants;
- `bufferReleased`: confirmation that an unreferenced resource was reclaimed;
- `sessionClosed`: terminal graphics-session notification.

Every event uses the common envelope and an `event` object. `figureClosed`
contains `figureId` and `closedRevision`; `bufferReleased` contains `bufferId`;
`sessionClosed` contains no additional required member. A client ignores an
unknown event `type` but validates every known event completely.

A Figure revision starts at one and increases by exactly one for every atomic
observable graphics transaction. A delta contains `baseRevision` and
`revision == baseRevision + 1`. An object's `objectRevision` starts at one and
increases by exactly one only when that object is upserted. Reordering a parent
upserts and advances the parent. Exhausting the JSON-safe revision range rejects
the transaction without mutation. A client applies a delta only when its
current revision equals `baseRevision`; otherwise it requests `resyncFigure`
and retains the last complete render until a snapshot arrives.

Initialization, snapshots, and mutations are serialized by the session
registry. The revisions returned in the initialize result are the exact bases
for later deltas; if a mutation wins the serialization order first, initialize
reports the new revision rather than emitting an unapplyable earlier base.

One transaction either publishes its complete object changes and immutable
buffer references or publishes nothing. Referenced buffers may still be in
flight; the client retains the previous drawable resource until the complete
replacement is validated.

## Object identifiers and hierarchy

Wire object identifiers are opaque strings. Every serialized object also has a
positive `generation`. Parent and child identifiers refer to objects in the
same Figure. A snapshot has one Figure root, no cycles, no duplicate object
identifiers, exactly one parent for every non-root object, and child order that
matches draw and legend order.

V1 object kinds are:

- `figure`;
- `axes2d`;
- `lineSeries`;
- `scatterSeries`;
- `text`;
- `legend`.

Every object contains:

```json
{
  "id": "opaque-object-id",
  "generation": 1,
  "objectRevision": 1,
  "kind": "lineSeries",
  "parentId": "axes-id",
  "children": [],
  "properties": {}
}
```

The Figure root has `parentId: null`. Series and text objects have no children.
Unknown object kinds make a v1 snapshot or delta unsupported rather than being
silently reinterpreted.

## First-tranche properties

The first schema uses typed property sets even though they occupy the common
`properties` member.

All listed members are required unless marked optional. Unknown members are
ignored for forward compatibility but a producer always emits the complete v1
set. There are no implicit wire defaults.

`figure` properties are:

| Member | Type and invariant |
| --- | --- |
| `number` | positive safe integer |
| `nameCodeUnits` | array of integers in `0..65535` |
| `visible` | Boolean |
| `backgroundRgba` | four finite numbers in `[0,1]` |
| `initialLogicalSizeCssPixels` | two positive finite numbers |
| `nextPlot` | `add` or `new`; the basic v1 API produces `add` |

`initialLogicalSizeCssPixels` is the kernel's initial window-size intent, not
the current browser viewport. Browser resize is local and does not mutate this
property in v1.

`axes2d` properties are:

| Member | Type and invariant |
| --- | --- |
| `positionNormalized` | four finite numbers `[x,y,width,height]`; all nonnegative, width/height positive, and the rectangle lies within `[0,1]^2` |
| `xScale`, `yScale` | exactly `linear` in v1 |
| `xLimits`, `yLimits` | two finite strictly increasing numbers, already resolved by the kernel |
| `xLimitsMode`, `yLimitsMode` | `auto` or `manual` |
| `nextPlot` | `replace` or `add` |
| `gridX`, `gridY` | Boolean |
| `colorOrderIndex` | positive safe integer |
| `titleId`, `xLabelId`, `yLabelId` | object identifier string or JSON `null` |

The three optional-target members are required on the wire and use explicit
`null`; omission is invalid.

`lineSeries` properties require `xData` and `yData` DataRefs with equal `1 x N`
shapes, `colorRgba`, positive finite `lineWidthCssPx`, `lineStyle`, `marker`,
positive finite `markerSizeCssPx`, `displayNameCodeUnits`, Boolean `visible`,
and Boolean `clipping`. `lineStyle` is one of `solid`, `dash`, `dot`, or
`dashDot`. `marker` is one of `none`, `circle`, `plus`, `star`, `point`,
`cross`, `square`, `diamond`, `triangleUp`, `triangleDown`, `triangleLeft`,
`triangleRight`, `pentagram`, or `hexagram`. The basic v1 plot API produces
`solid` and `none`.

`scatterSeries` requires equal-shape `xData` and `yData`, `sizeData` and
`colorData` as DataRef or explicit `null`, positive finite default marker size,
fill and stroke RGBA colors, a non-`none` marker enum, `displayNameCodeUnits`,
Boolean `visible`, and Boolean `clipping`. Non-null size/color resources must
contain either one value or exactly N values.

`text` properties contain exact `codeUnits` as integers in `0..65535`, `role`,
two-number `anchorNormalized`, horizontal and vertical alignment, `colorRgba`,
`fontFamilyCodeUnits`, positive finite `fontSizeCssPx`, integer `fontWeight` in
`1..1000`, `fontStyle`, and Boolean `visible`. UTF-16 surrogate code units are
preserved even when a rendering surface must display a replacement glyph. V1
roles are `title`, `xLabel`, `yLabel`, and `annotation`; horizontal alignment is
`left`, `center`, or `right`; vertical alignment is `top`, `middle`, `baseline`,
or `bottom`; font style is `normal` or `italic`.

`legend` properties contain `seriesIds`, an equal-length `labelCodeUnits` array
of UTF-16 arrays, `location`, Boolean `visible`, `backgroundRgba`,
`borderRgba`, `fontFamilyCodeUnits`, positive finite `fontSizeCssPx`, and
`fontWeight`. V1 location is one of `best`, `north`, `south`, `east`, `west`,
`northEast`, `northWest`, `southEast`, or `southWest`. The renderer does not
infer legend membership from traversal when the explicit list exists.

Colors are four finite JSON numbers in `[0,1]`, interpreted as unpremultiplied
sRGB plus alpha. CSS and device pixel spaces are never confused: layout intent
uses CSS pixels, while the renderer multiplies by the current device pixel
ratio.

## Snapshot and delta shape

A `FigureSnapshot` embedded in a successful snapshot response contains:

```json
{
  "type": "figureSnapshot",
  "figureId": "figure-id",
  "revision": 4,
  "rootId": "figure-object-id",
  "objects": [],
  "referencedBuffers": []
}
```

`referencedBuffers` contains each `DataRef` used by the snapshot exactly once.
All references in object properties must resolve to that set.

A delta event contains an ordered `operations` array. V1 operations are:

- `upsertObject { object }`;
- `deleteObject { id, generation }`;
- `reorderChildren { parentId, children }`;
- `setRoot { rootId }`.

The complete delta event envelope is:

```json
{
  "protocol": "openmat-graphics-v1",
  "sessionId": "session-1",
  "messageId": "event-9",
  "kind": "event",
  "event": {
    "type": "figureDelta",
    "figureId": "figure-id",
    "baseRevision": 4,
    "revision": 5,
    "operations": [],
    "addedBuffers": [],
    "releasedBufferIds": []
  }
}
```

`addedBuffers` lists every newly referenced DataRef exactly once.
`releasedBufferIds` lists resources whose live Figure reference count reached
zero in this transaction. A released resource may remain resident while the
connection holds a lease. The two lists contain no duplicate identifier and do
not overlap.

Upsert replaces the complete typed property set of one object; v1 does not
merge arbitrary individual JSON fields. A delete recursively removes the
object's complete subtree, so deleting an Axes also deletes its series, text,
and legend descendants. Every resulting scene must satisfy the snapshot
invariants.

## DataRef

Large numerical properties use an immutable resource descriptor:

```json
{
  "bufferId": "opaque-buffer-id",
  "dtype": "f64",
  "shape": [1, 1000],
  "order": "columnMajor",
  "endianness": "little",
  "byteOffset": 0,
  "byteLength": 8000
}
```

V1 `dtype` is `f32` or `f64`. Data is real. Series resources have exactly the
shape `[1,N]` and own independent contiguous bytes; v1 has no strided or matrix
slice DataRef. `order` is exactly `columnMajor`, `endianness` exactly `little`,
and `byteOffset` exactly zero. `byteLength` equals checked N times the dtype
width. Empty data uses `[1,0]`, `byteLength: 0`, and requires no binary payload.
NaN, positive and negative infinity, and signed zero preserve their IEEE-754
bit patterns.

A `bufferId` names immutable bytes for its lifetime. Updating source data
creates a new identifier; it never mutates bytes already leased by a client.
The server retains a resource until both its live-Figure reference count is zero
and the active client has no lease, subject to session teardown and hard
resource limits.

One connection has at most one idempotent lease per buffer. A lease begins when
the successful `getBuffer` text response is sent, before its first chunk. A
failed or cancelled transfer removes the newly acquired lease. Repeating
`getBuffer` reuses the existing lease. `releaseBuffer` is idempotent and returns
`released: true` only when it removed a lease. Disconnect implicitly releases
all leases. A later Figure may reference the same immutable buffer again and
therefore prevent reclamation independently of client leases.

## Buffer transfer response

A successful `getBuffer` text response precedes its binary chunks:

```json
{
  "type": "getBuffer",
  "transferId": "opaque-transfer-id",
  "bufferId": "opaque-buffer-id",
  "totalBytes": 8000,
  "chunkBytes": 8000,
  "chunkCount": 1
}
```

The server sends exactly `chunkCount` chunks whose non-overlapping ordered
ranges cover `0..totalBytes`. Different transfers may be interleaved, so binary
headers carry `transferId`. WebSocket ordering is retained but is not used as a
substitute for offset validation.

For a zero-byte buffer, `totalBytes`, `chunkBytes`, and `chunkCount` are zero;
the server sends no binary message.

## Binary message format

Binary transfer is server-to-client only. Each complete WebSocket binary
message is:

| Offset | Width | Meaning |
| ---: | ---: | --- |
| 0 | 4 | ASCII magic `OMGP` |
| 4 | 2 | little-endian protocol major, exactly `1` |
| 6 | 2 | little-endian protocol minor, exactly `0` |
| 8 | 4 | little-endian UTF-8 JSON header byte length |
| 12 | 4 | little-endian payload byte length |
| 16 | variable | JSON header bytes |
| after header | variable | raw payload bytes |

The complete message length equals `16 + headerLength + payloadLength` and is
within the negotiated binary-frame limit. The header is at most 65,536 bytes
and has this meaning:

```json
{
  "type": "bufferChunk",
  "transferId": "opaque-transfer-id",
  "bufferId": "opaque-buffer-id",
  "offset": 0,
  "totalBytes": 8000,
  "final": true
}
```

`payloadLength` equals the chunk range length. `final` is true exactly for the
chunk ending at `totalBytes`; it does not waive coverage validation. A malformed
prefix, unsupported version, invalid UTF-8/JSON header, length mismatch,
unknown transfer, duplicate range, overlap, gap, or descriptor mismatch causes
the client to discard the local transfer and close/reconnect the graphics
channel; it cannot rely on a text error from the malformed sender. Partially
received data never becomes visible to HIR or the renderer. If a client sends a
binary message, the Server closes the connection because v1 defines no
client-to-server binary frame.

## Error categories

V1 reserves these stable categories:

- `graphics.invalidRequest`;
- `graphics.unsupportedRequest`;
- `graphics.unauthorized`;
- `graphics.sessionInUse`;
- `graphics.unknownFigure`;
- `graphics.unknownBuffer`;
- `graphics.revisionConflict`;
- `graphics.payloadLimit`;
- `graphics.residentLimit`;
- `graphics.invalidScene`;
- `graphics.invalidBinaryFrame`;
- `graphics.sessionClosed`.

Error prose and browser WebGPU error text are not compatibility ABIs.

## Server session registry

The native Server owns a thread-safe `GraphicsSessionRegistry`. A kernel
connection creates one shared graphics hub, registers it under an unguessable
token before emitting any discovery display, and unregisters it on kernel
session shutdown. Registry lookup never exposes the token in a diagnostic.

The hub owns the Graphics Object Model reference, revision journal, immutable
buffer store, and active-attachment/lease state. The graphics WebSocket borrows
the hub; it does not create a second graphics model. Closing only the graphics
socket detaches the active client and releases its leases but preserves Figures.
Closing the owning kernel socket tears down the hub, invalidates the token,
cancels transfers, and emits `sessionClosed` when the graphics socket can still
receive it. Reconnection uses the same live hub and current revisions.

Token rotation atomically unregisters the old token before publishing the new
discovery descriptor. V1 does not allow simultaneous read-only observers or
multiple renderer clients.

## Lifecycle and recovery

Closing a Figure invalidates its object handles and emits `figureClosed`.
Detaching the browser does not delete Figures. Kernel shutdown deletes the
graphics arena, invalidates every attachment token, cancels transfers, and
releases all buffers.

WebGPU device loss affects only the browser renderer. The client recreates its
device, requests snapshots as needed, and re-fetches immutable buffers. It does
not ask the kernel to re-execute source code. A browser without WebGPU may stay
attached for lifecycle updates but reports that the Figure cannot be rendered.

## V1 non-goals

- WebGL or WebGL2 transport and renderer negotiation;
- shared native/WASM pointers or a Rust ABI boundary;
- complex, integer, sparse, GPU-resident, or distributed DataRef values;
- callbacks and arbitrary client-to-kernel property mutation;
- 3D camera events;
- image, contour, surface, patch, or volume payloads;
- compressed buffers or cross-session resource sharing.
