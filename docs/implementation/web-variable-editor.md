# Web Variable Editor grid contract

The OpenMat Web Variable Editor presents a MATLAB-style bounded view of a
kernel `inspect` result. `double`, `single`, `logical`, and fixed-width integer
matrix cells are editable; aggregate and other value classes remain read-only. The
presentation grid is intentionally not the inspection range.

## Data coordinates and presentation filler

The kernel-selected range remains the only source of array coordinates and
values. Its rows and columns are rendered in column-major order, with one-based
headers. Paging and N-D slice controls continue to create bounded, in-range
`MatrixRange` requests.

For a small result, the table grows to a minimum presentation viewport of 12
rows by 8 columns. Positions outside the selected range are filler cells. They:

- have no value lookup or column-major index;
- have no click, selection, paging, or inspect handler;
- are `aria-hidden` and use `role="presentation"`;
- do not contribute accessible `cell`, `rowheader`, or `columnheader` roles;
- never change the loaded/omitted element counts.

A shaped-empty matrix uses the same blank grid while retaining its exact empty
shape message and any structurally valid column headers. Aggregate shaped-empty
previews keep their v2 schema-specific empty view.

## Header and sizing rules

The column-header rail is sticky at the top of the table scroller. The
row-header rail is sticky at the left. Their intersection is a separate,
labelled corner cell with the highest stacking order, so both rails remain
visually outside the data cells during two-axis scrolling.

Ordinary data columns use a 104 px desktop default (88 px at the existing
mobile breakpoint), a 48 px row-header gutter, and 29 px grid rows. The table
uses fixed layout and `max-content` width, so one or two data columns do not
expand to fill the editor. Ordinary values stay on one line and use ellipsis;
the complete formatted value remains in the DOM and is exposed through both
`title` and its accessible label.

Top-level cell and struct columns use a dedicated 320 px desktop width (260 px
at the mobile breakpoint). Their outer cell is bounded to 204 px high and the
complete exact-value view scrolls inside it. Exact leaf values use the same
single-line ellipsis plus full `title`/accessible text. Nested cell and struct
structure, declared field order, complete exact nodes, and internal tables are
not flattened or coerced.

## Versioned edit behavior

The Web product requires `openmat-kernel-v3`. It preserves:

- bounded v2-compatible inspect payloads and negotiated limits inside the v3 envelope;
- bounded row/column paging and one-position N-D slice navigation;
- one-based ranges and column-major value traversal;
- kernel truncation/omission reporting;
- exact single, complex, char, string, integer, cell, and struct rendering;
- stale-response cancellation through the existing editor request sequence.

Each inspect response carries the current workspace revision. A numeric edit
sends its absolute one-based N-D indices, canonical real/imaginary components,
and that expected revision. The kernel checks and writes on its sequential
request path. A mismatch returns `workspace.revisionConflict`; the editor keeps
the typed value and offers an explicit reload instead of retrying the write.
Successful writes refresh the visible bounded range and update workspace
metadata. Integer components are range-checked and transported as exact decimal
text, including the complete `int64` and `uint64` domains. Copy-on-write
detachment and floating-point real-to-complex promotion happen in the runtime,
not in the browser.

## Verification boundary

Focused DOM tests distinguish real accessible cells from filler, cover 2x2,
1x1, shaped-empty, long-value, sticky-header, bounded-aggregate, and no-handler
contracts, numeric parsing and commit behavior, conflict recovery, and the v3
protocol behavior. CSS positioning
and size rules are asserted as stylesheet contracts because jsdom does not
perform layout or sticky-scroll geometry. The grid is an R2022b-oriented visual
interpretation, not a pixel-for-pixel claim against proprietary MATLAB source,
tests, documentation, or messages.
