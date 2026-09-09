# RFC 0009: Layout constraints and transactional designer editing

Status: Implemented; additive extension of RFCs 0006–0008

## Scope and ownership

This phase supplies practical sizing and selection tools for the existing React
designer and Rust-backed M class runtime. The same renderer evaluates design
definitions and native snapshots. It does not introduce a second UI object model,
execute M in WASM, or change class inheritance, exposure, signals or lifecycle.

Layout stays in `.omui`; user properties and methods stay in the paired `.m`
class. New optional constraints use `layoutVersion="2"`, independently of the
XML document's v1/v2/v3 ownership format. See
[`ui-layout-v2.md`](../../spec/protocol/ui-layout-v2.md) for exact semantics.
Legacy definitions retain their prior defaults and serialize without the marker
until extended fields or ScrollPanel are used. Older clients reject the new
fields; use a matching updated web client and native server for these documents.

## Sizing

Each axis supports default, fixed, content and fill modes. Default retains
legacy Width/Height/Grow behavior. Explicit modes allow an independently fixed
sidebar, a growing plot, and a content-height heading in one window. Minimum and
maximum constraints bound each axis. Flex layouts distribute remaining space
using GrowX or GrowY on their main axis; grid proportions belong to tracks.

Grid tracks accept bounded positive pixel sizes, `auto`, and fractions such as
`1fr` or `2fr`. They are parsed as a small grammar before conversion to CSS;
arbitrary CSS is not accepted. ScrollPanel adds a separately configurable
vertical, horizontal or two-axis scrolling surface and a dedicated palette icon.
Its children use the same four layout arrangements as other containers.

The preview width/height fields and bottom-right handle edit the saved root
window dimensions. During a drag, zoom stays fixed and a transient document is
rendered. Release commits once; Escape discards the preview. The running app uses
the saved dimensions or subsequent M changes. Editing remains disabled while
running; restart after changing the definition.

## Selection and transactions

Click selects one component; Shift, Ctrl or Cmd click toggles membership. Dragging
the empty area of a container selects intersecting direct children. Composite
internals remain selectable only in their own definition. Root selection is
exclusive; ancestor selections subsume descendants for structural operations.

The multi-inspector exposes the intersection of writable, type-compatible
properties. Numeric constraints and choices are intersected too. Different values
show a mixed placeholder or indeterminate checkbox; merely focusing/blurring an
unchanged mixed editor does not overwrite values. One edit validates the entire
result and produces one history entry.

Absolute-layout siblings can align edges/centers, distribute nonnegative equal
gaps, move together, or match the first-selected component's width/height. Dragging
snaps to an 8-pixel grid and nearby sibling/container edges or centers, with guide
lines. Alt temporarily disables snapping. Arrow keys nudge one pixel; Shift uses
eight pixels. Grid arrows move one cell when free, while flex arrows reorder.
Text editors, form inputs and panel resize separators retain their keyboard input.

Grid drops use the pointer's actual row/column; a single same-span occupant swaps
positions, otherwise placement advances to free cells. Groups originating in one
grid preserve relative cell offsets. Row/column drops change child order. A drop
hint names its container and insertion position. Structural operations reject
cycles, incompatible parents, composite internals and overlapping grid wrappers.

Wrap selection creates a row, column, grid or scroll container while retaining
child identities and event bindings. Duplicate creates fresh IDs and names and
remaps XML receiver connections within the copied subtree. Delete disconnects
remaining XML bindings to deleted receivers. Arbitrary user M source references
are not rewritten by these operations. Every drag, wrap, delete, duplication,
alignment or bulk edit is one undo/redo transaction.

## Acceptance and boundaries

`examples/app-designer/LayoutLab.omui` and `LayoutLab.m` demonstrate a 260-pixel
scrolling parameter column, a growing plot and a 150-pixel bottom log, with
content-height headings. The real M startup and button methods compute a signal,
publish a figure and update the log. At 1100×720 and 800×500 the fixed tracks stay
fixed while the center shrinks and the parameter panel scrolls.

Automated checks cover format/validation parity, immutable editing operations,
group identity, mixed properties, save/reopen, pointer preview/cancel/commit,
one-step undo and native layout snapshots. Browser verification uses the native
server for M callbacks/plots and pointer input for resize, absolute dragging,
grid swapping, marquee selection and wrapping.

This phase does not add breakpoints, arbitrary anchors/constraint equations,
touch multi-selection, runtime inspection, code hot replacement, data bindings
or background compute jobs. Native Position keeps the previous bottom-left
absolute-position behavior; the layout editor uses top-left X/Y. Explicit size
modes constrain dimensions after the legacy Position/default style is applied.
