# Graphics v2 semantic-limit synchronization

## Scope and baseline

This implementation was developed from integration baseline
`pre-publication baseline`. It adds the smallest versioned
control-plane extension needed to commit a completed 2D navigation gesture.
The frozen `openmat-graphics-v1` specification, scene DTOs, Plot HIR, binary
OMGP framing, and kernel protocol envelopes are unchanged.

## Version boundary

`openmat-plot-protocol` retains `PROTOCOL` as the v1 compatibility alias and
adds explicit `PROTOCOL_V1`, `PROTOCOL_V2`, and `GraphicsProtocol` values.
Version-aware decode entry points require the envelope identifier selected by
the WebSocket route. The legacy decode entry points continue to mean v1.

The server serves both `/graphics/v1` and `/graphics/v2`. Newly created Figure
discovery payloads use schema version 2, `openmat-graphics-v2`, and
`/graphics/v2`. The same session-owned hub and immutable-buffer store back both
routes. A v1 endpoint treats `setAxesLimits` as an unsupported request; a v2
endpoint rejects a v1 envelope before attachment.

The Web discovery parser accepts the exact v1 tuple and the exact v2 tuple. It
rejects crossed schema/protocol/endpoint combinations. `GraphicsV1Client`
keeps its exported compatibility name, but binds its outgoing envelopes,
incoming-envelope validation, and selected URL to the discovered protocol.
Existing callers that pre-resolve `/graphics/v1` from the Kernel URL are
upgraded to the discovered `/graphics/v2` endpoint inside the client.

## Commit flow

```text
completed browser gesture
  -> one setAxesLimits(figureId, expectedRevision, xLimits, yLimits)
  -> /graphics/v2 validates envelope and bounded payload
  -> owning GraphicsSession validates Figure, revision, and both limit pairs
  -> one staged model transaction updates X/Y limits and both modes
  -> response { committedRevision }
  -> ordinary journaled figureDelta
  -> every mirror applies delta or uses resyncFigure on a gap
```

Both limit pairs must be finite and strictly increasing. The owning Figure must
exist and its current revision must equal `expectedRevision`. Validation occurs
before model mutation. A successful transaction advances the Figure exactly
once and emits the complete Axes upsert through the existing journal. The
WebSocket sends the correlated success response before draining the journal,
preserving a deterministic response-before-delta order.

The Web client validates `committedRevision == expectedRevision + 1`, but does
not edit a snapshot or advance its render revision from the acknowledgement.
Only a valid next-base `figureDelta` changes the authoritative local scene.
This avoids an acknowledgement/delta race and keeps reconnect behavior on the
existing snapshot and resynchronization path.

## Preserved safety properties

- Attachment tokens remain 256-bit server capabilities, constant-time
  compared, and redacted from Debug values, failures, and client-visible
  messages.
- Text, binary, buffer, resident, and live-object limits are unchanged and
  continue to be negotiated before post-initialize traffic.
- Unknown requests remain correlated `graphics.unsupportedRequest` failures.
- Stale mutations remain `graphics.revisionConflict` and publish no delta.
- Client-to-server binary frames remain unsupported.
- The v2 addition does not execute or generate MATLAB source.

## Integration boundary

The protocol client now exposes
`setAxesLimits(figureId, expectedRevision, xLimits, yLimits)`. The coordinator
still needs to add that method to the `FigureGraphicsClient` interface in
`FigureWindow.tsx` and call it once from the gesture-completion integration.
Pointer-frequency transforms remain local to the Plot Engine; this method must
not be called for each pointer or wheel event.

## Verification

Coverage includes:

- Rust protocol tests for exact v1/v2 selection, v1 unsupported behavior,
  invalid limit pairs, and v2 response identifiers;
- a real loopback TCP/WebSocket v2 lifecycle covering attach, one semantic
  commit, response-before-delta ordering, manual X/Y modes, duplicate request
  IDs, stale revision conflict, unknown request rejection, and shutdown;
- the existing real v1 TCP/WebSocket lifecycle to retain endpoint
  compatibility;
- Web fake-socket tests for exact v2 discovery selection, one request per API
  call, response validation without optimistic snapshot mutation, convergence
  after the ordinary delta, and v1 rejection of the v2-only method.
