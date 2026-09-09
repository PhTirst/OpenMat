# Plot Engine 2D properties and styles

This tranche targets MATLAB R2022b behavior for a deliberately bounded set of
2D Line and Scatter properties. No compatibility conclusion in this tranche is
derived from Octave.

## Atomic model contract

`GraphicsSession::execute` remains the only commit boundary. `PlotStyled`
creates all Line objects and applies the parsed LineSpec/name-value assignments
inside one staged session clone. `SetProperties` validates every target,
property, and value before the staged clone is committed. An error therefore
leaves object contents, Figure revisions, resources, pending deltas, hold state,
and the color-order index unchanged.

A successful property change uses the existing complete-object
`GraphicsDeltaOperation::UpsertObject`; it does not add a field-level delta or a
Rust ABI boundary. Scalar Scatter `SizeData` and RGB `CData` reuse retained HIR
fields and allocate no new data resource.

## Supported Line forms

`plot` accepts these forms, with one optional LineSpec followed by zero or more
name/value pairs:

```matlab
plot(Y, ...)
plot(X, Y, ...)
plot(X, Y, '--o', 'LineWidth', 2, 'Color', [r g b])
```

The data-shape rules, replace/hold behavior, and color-order advancement are the
same as the first Plot Engine tranche. A LineSpec assignment is applied first;
later name/value assignments override it. The supported line styles are `-`,
`--`, `:`, and `-.`. The supported short colors are `r`, `g`, `b`, `c`, `m`,
`y`, `k`, and `w`. The marker symbols are `.`, `o`, `+`, `*`, `x`, `s`, `d`,
`^`, `v`, `>`, and `<`.

Line name/value and `set` properties are:

| Property | Accepted value |
| --- | --- |
| `LineWidth` | positive finite real double/single scalar |
| `Color` | finite RGB vector in `[0,1]`, or a supported short/long color name |
| `LineStyle` | style symbol or `solid`, `dashed`, `dotted`, `dash-dot` |
| `Marker` | `none`, a supported marker symbol, or its supported long name |
| `MarkerSize` | positive finite real double/single scalar |
| `Visible` | case-insensitive `on` or `off` |

Property names are ASCII case-insensitive. Duplicate assignments are legal and
the last value wins.

## Supported Scatter properties

`set` and `get` expose scalar forms that the retained Scatter HIR can represent:

| Property | Accepted value / retained meaning |
| --- | --- |
| `SizeData` | one nonnegative finite marker area; HIR stores its square-root diameter |
| `CData` | one RGB triplet; this tranche mirrors it to face and edge colors |
| `MarkerFaceColor` | one RGB triplet |
| `MarkerEdgeColor` | one RGB triplet |
| `Marker` | the same marker set as Line |
| `Visible` | case-insensitive `on` or `off` |

Per-point `SizeData` and `CData`, color modes such as `flat`/`auto`/`none`, and
alpha data remain outside this closure.

## Handle and output boundaries

`set(h,name,value,...)` accepts a scalar Line/Scatter handle or a homogeneous
`N x 1` graphics handle column. A nonempty update transaction must stay within
one Figure. Empty homogeneous handle columns are a no-op. Stale handles,
mixed-class model requests, cross-Figure updates, and Figure/Axes/Text/Legend
handles are rejected before commit. The public MATLAB class carried by every
handle is unchanged.

`get(h,name)` returns the MATLAB-style scalar value for a scalar handle. For a
graphics handle column it returns an `N x 1` cell column in the same handle
order, including `0 x 1` for an empty handle column. Numeric scalar properties
are doubles, colors are `1 x 3` doubles, and `LineStyle`, `Marker`, and
`Visible` are char rows. Existing `get(axes,'Children')` behavior is unchanged.

## Shared integration boundaries

The model marker enum now mirrors the marker shapes already present in
`openmat-plot-protocol`. The coordinator must extend
`crates/openmat-server/src/graphics_runtime.rs::marker` with these exact model to
wire mappings: `Point`, `Circle`, `Plus`, `Star`, `Cross`, `Square`, `Diamond`,
`TriangleUp`, `TriangleDown`, `TriangleRight`, and `TriangleLeft` (plus the
existing `None`). Until that shared conversion is updated, compiling the server
against this commit will correctly report a non-exhaustive match.

Dot syntax is also a shared runtime boundary. The compiler already lowers field
reads through `GetAggregateField` and assignments through `PlaceStep::Field`;
the runtime interpreter currently routes those paths only to struct/class
object property machinery. Supporting `h.LineWidth` and
`h.LineWidth = value` requires the interpreter's graphics value branches to
call the same graphics property query/update service used by `get`/`set`, while
preserving staged commit, handle-column shape, and assignment diagnostics. No
compiler or runtime file is changed in this tranche.
