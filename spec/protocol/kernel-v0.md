# OpenMat kernel protocol v0

This document freezes the wave-one logical protocol shared by the mock web UI,
kernel, and server. Transport bindings may use JSON text frames initially.

## Envelope

Every message contains:

- `protocol`: the string `openmat-kernel-v0`.
- `sessionId`: opaque UTF-8 string.
- `messageId`: unique opaque UTF-8 string.
- `kind`: `request`, `response`, or `event`.

A response also contains `replyTo`, referring to the request `messageId`.
Unknown fields and unknown event types must be ignored for forward compatibility.

## Requests

- `initialize`: negotiate capabilities and implementation versions.
- `execute`: submit UTF-8 code, a source name, and execution mode (`file`,
  `cell`, or `repl`).
- `interrupt`: request cooperative interruption of the active execution.
- `inspect`: request a bounded preview of a workspace value.
- `listWorkspace`: request variable summaries without transferring values.
- `shutdown`: request orderly kernel termination.

Requests other than control requests execute sequentially within a session.

## Responses

Responses carry `ok`. Successful responses carry `result`; failures carry an
`error` object with stable OpenMat category, human-readable message, and optional
source ranges. Error prose is not a compatibility ABI.

## Events

- `status`: `starting`, `idle`, `busy`, `interrupted`, or `dead`.
- `stream`: `stdout` or `stderr` UTF-8 text.
- `display`: a typed display item with MIME-keyed representations.
- `diagnostic`: structured compiler or runtime diagnostic.
- `workspaceDelta`: added, changed, and removed variable summaries.

Large binary values are not part of v0. Matrix inspection returns bounded JSON
previews with class, dimensions, selected range, and truncation metadata.

