# RFC 0012: Simulation editor and native run service v0

Status: Implementation contract for the user-approved graphical simulation milestone.

## Scope

Add a React workbench view for editing, saving, checking and running RFC 0010
models. The first palette contains Constant, Sum, Gain, Integrator, UnitDelay
and Scope. This milestone connects the existing native Rust reference engine;
it does not implement the separately planned static m-function compiler, general
m JIT, SUNDIALS, subsystem execution or SLX writing. Existing kernel, workspace,
graphics and SLX contracts retain their meanings.

## Editor document

The `.omsim` file is UTF-8 JSON with `format: "openmat-simulation"`,
`schemaVersion: 1`, an RFC 0010 `model`, and `editor` metadata. Metadata contains
block labels, connection bend points and viewport. Stable block/port identities
are independent of labels, document ordering and React Flow objects. Connections
are identified by their source and destination block/port tuple. Copy remaps
block identities and only copies edges whose endpoints are both selected.
Unknown document versions and invalid metadata fail loading rather than being
silently discarded. RFC 0010 numeric JSON is also accepted and gets a layout.
Incomplete graphs can be saved; execution still requires native validation.
Deleting a block removes its incident connections. Gesture changes are one undo
transaction. Model results are not serialized into the authoring document.

Workspace saves use existing file revision checks. Desktop file picking/export
uses PlatformServices. A recoverable editor draft is kept independently of saved
files; dirty close/open operations require an explicit save or discard choice.

## Service boundary

Add `/simulation/v1` on the existing native WebSocket listener, with the same
origin validation as other endpoints. JSON messages have
`protocol: "openmat-simulation-v1"`, a nonempty `requestId` (maximum 128 bytes),
and `operation`. Operations are `catalog`, `check`, `run`, `cancel`, `importSlx`.
Check/run carry an RFC 0010 `model` and a caller `revision` string. Run requests
are immutable snapshots and the requestId identifies the run on that connection.
Cancel carries `runId`. SLX import carries `name` and `bytes` (an integer byte
array), never executable paths. Incoming messages are limited to 8 MiB and SLX
bytes to 2 MiB for this interactive service (the CLI keeps its own limits).

Responses contain `requestId`, `ok`, and `result` or `error`. A run first returns
its runId, revision, backend (`reference`), scopes and settings. Subsequent
messages carry `event`, `runId`, `revision` and an increasing sequence number.
`samples` contains ordered accepted frames; `finished`, `cancelled` and `failed`
are terminal. At most one run is active per connection. Disconnect cancels it.
The engine runs on a worker thread, with a bounded outgoing queue; cancellation
remains observable while that queue is full. No numerical work is performed in
React. Output is bounded to 100,000 samples / 2,000,000 values per run; limits
are explicit failures, not unnoticed truncation. Compilation/run errors include
the original block and port where available. The service does not accept native
library paths or execute imported model callbacks.

SLX import returns the existing structural inspection document, issues and, if
supported, its lowered runnable model. Unsupported models remain inspectable;
they are never substituted with zero-producing blocks. Import writes no files.

## Editor behavior

Use React Flow for viewport/nodes/edges, with OpenMat-owned authoring state,
history, palette and inspector. Keep visual edits entirely local. Use the
existing light/dark CSS tokens and platform services. Palette and model tree
have a vertical adjustable divider; sidebars and the result pane resize too.
Nodes expose distinct icons, named handles and orthogonal connections. Support
fan-out, editable routing, multi-selection, copy/paste, delete, context menus,
undo/redo and fit view. F5 runs the visible model; shortcuts do not consume
ordinary text editing commands in inputs or Monaco.

Native validation is authoritative. Run status/results retain the document
revision, so edits made after a run cannot make its results look current.
Run samples and chart drawing are separate from graph state. Scope refreshes
are batched; large traces are reduced for display while exports preserve the
retained samples. Backend availability and failures are visible; a mock is
never presented as a successful native run.

## Verification

Verify authoring round trips, copy identity/edge remapping, history, malformed
files, websocket lifecycle/cancellation/limits, SLX compatibility reporting and
native first-order feedback trajectories. Exercise real browser workflows for
drag/connect/edit/save/reopen/run, and inspect the chart, diagnostics and console.
Measure a few-hundred-node fixture independently from numerical run time.
Run relevant Rust and frontend checks and the public-source guard before focused
commits. Do not include dependency source, generated bundles or local results.
