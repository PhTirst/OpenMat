# Plot Engine 2D properties: MATLAB R2022b compatibility contract

Status: clean-room black-box measurements captured from the locally installed
MATLAB R2022b on 2026-08-26. The authored probes contain no MATLAB source,
tests, documentation, or diagnostic prose. Error observations retain only an
OpenMat-owned normalized category.

This contract covers the finite behaviors named below. It does not make
unmeasured graphics properties, callbacks, rendering pixels, interactive tools,
or later MATLAB releases part of the compatibility surface.

## Observation boundary

The conformance schema does not serialize graphics handles. Each probe instead
projects a handle through public language behavior:

- `class` and `size` identify the public object type and handle-array shape;
- `get` observes public property values;
- public `Parent` and `Children` relationships describe ownership;
- `isgraphics` observes lifetime after replacement or Figure closure.

Mixed values are represented by schema-v2 cells containing exact numeric,
logical, and UTF-16 character leaves. Properties whose public return value is
an enum-like wrapper are explicitly converted with `char` before observation.
No schema or runner extension is required for this tranche.

## `plot`: LineSpec, name/value, shape, and class

For a single series, `plot` returns a scalar `1 x 1` handle whose public class
is `matlab.graphics.chart.primitive.Line`. With LineSpec `m--s` and only
`LineWidth`/`MarkerSize` supplied as name/value pairs, the observed properties
are magenta `[1 0 1]`, dashed `'--'`, marker `'square'`, line width `1.75`, and
marker size `7`. This independently establishes LineSpec parsing rather than
inferring it from an overridden final state.

The probe starts with LineSpec `m--s` and follows it with explicit name/value
pairs. The final observed values are:

| Property | Final value |
| --- | --- |
| `Color` | `[0.125 0.25 0.5]` |
| `LineStyle` | `':'` |
| `LineWidth` | `2.5` |
| `Marker` | `'o'` |
| `MarkerSize` | `9` |

Thus later name/value pairs override the corresponding color, line style, and
marker selected by the earlier LineSpec. A compatible implementation must
apply the options in this observable order.

## `set`/`get` property-name matching and invalid inputs

Property names are case-insensitive. The following unambiguous prefixes were
accepted by `set` in R2022b:

| Authored spelling | Resolved property | Stored value |
| --- | --- | --- |
| `cOlOr` | `Color` | `[0.2 0.4 0.6]` |
| `lInEwId` | `LineWidth` | `3.25` |
| `LiNeStY` | `LineStyle` | `'-.'` |
| `mArKeR` | `Marker` | `'x'` |
| `MaRkErSi` | `MarkerSize` | `11` |

The values were read back with differently cased full property names, so both
the write and read sides participate in case-insensitive matching. The prefix
`Line` is not accepted because it is ambiguous between multiple Line
properties.

The structured error contract is deliberately message-free:

| Operation | Normalized category |
| --- | --- |
| `get(line, 'Line')` | `other` |
| `set(line, 'LineWidth', -1)` | `type-error` |
| `set(line, 'LineStyle', 'zigzag')` | `type-error` |

These categories are OpenMat conformance categories, not MATLAB identifiers or
messages. Implementations should match the category, not proprietary wording.

## `scatter` scalar properties

`scatter(1:3, [4 6 5], 36, [0.25 0.5 0.75], 'd')` returns a scalar `1 x 1`
handle of public class `matlab.graphics.chart.primitive.Scatter`. Its observed
public properties are:

- scalar `SizeData` equal to `36`;
- `CData` equal to the `1 x 3` RGB row `[0.25 0.5 0.75]`;
- `Marker` equal to the canonical name `'diamond'` rather than the input alias
  `'d'`.

## Hold, style inheritance, and replacement lifetime

With hold enabled, Axes `ColorOrder` set to red then dark green, and
`LineStyleOrder` set to `{'--'}`:

- `ColorOrderIndex` is `1` before plotting, `2` after the first Line, and wraps
  to `1` after the second Line;
- the first Line inherits `[1 0 0]` and the second inherits `[0 0.5 0]`;
- both Lines inherit `'--'`.

In a separate replacement probe, two held Lines are initially valid. After
`hold off` and another `plot` call, both old handles are invalid, the new Line
is valid, the Axes has exactly one child, and `ColorOrderIndex` is `2`. Closing
the Figure then invalidates the replacement Line as well.

This is an object-lifetime requirement. Deleted Lines must not remain valid
through a renderer cache, picking index, or legend bookkeeping structure.

## Text, legend, grid, and ownership

`title`, `xlabel`, and `ylabel` each return a
`matlab.graphics.primitive.Text` handle parented by the Axes. The observed
strings are preserved exactly. A title created with color `[0.1 0.2 0.3]` and
font size `14` returns those values through `get`.

Title and label decorations do not enter the measured public Axes `Children`
list: with one Line, its count remains one before and after all three Text
objects are created. Closing the owning Figure invalidates all three Text
handles.

A legend for two Lines is a scalar `1 x 1`
`matlab.graphics.illustration.Legend`. Its observed `String` value is the
two-element cell `{'ascending', 'descending'}`, its `Location` is
`'northwest'`, and its public parent is the Figure. At that point the Figure
has two public children (Axes and Legend), while the Axes has the two Lines.
The Lines and Legend are valid before closure; closing the Figure invalidates
the Legend.

For this 2D Axes, `grid(axes, 'on')` makes the observed `XGrid`, `YGrid`, and
`ZGrid` values all `'on'`. This contract records public state, not whether a Z
grid is visibly rasterized in a 2D view.

## Limits and mode transitions

After `xlim([10 20])` and `ylim([-2 2])`, both mode properties are `'manual'`.
Changing the surviving Line data from the unit interval to X `[100 200]` and Y
`[10 20]` leaves the queried limits exactly `[10 20 -2 2]` and leaves both
modes manual.

After explicitly selecting auto mode, a Line with X `[0 10]` and Y `[2 4]`
has queried limits `[0 10 2 4]`. Changing that same Line to X `[100 200]` and Y
`[-10 30]` changes the queried limits to `[100 200 -10 30]`; both modes remain
`'auto'`.

The compatibility boundary is therefore mode-sensitive: manual limits are
persistent stored state, while auto limits are derived from the current live
graphics data and must be refreshed when that data changes.

## Corpus mapping

| Case | Contract area |
| --- | --- |
| `graphics_2d_plot_linespec_properties` | direct LineSpec parsing and name/value properties |
| `graphics_2d_plot_linespec_namevalue` | LineSpec and name/value precedence, Line class/shape |
| `graphics_2d_set_get_property_names` | case-insensitive and abbreviated property names |
| `graphics_2d_get_ambiguous_property` | ambiguous-prefix rejection |
| `graphics_2d_set_invalid_linewidth` | invalid numeric property value |
| `graphics_2d_set_invalid_linestyle` | invalid enumerated property value |
| `graphics_2d_scatter_scalar_properties` | Scatter class and scalar public properties |
| `graphics_2d_hold_color_order_style` | ColorOrderIndex and inherited style |
| `graphics_2d_replace_invalidates_lines` | replacement children and handle lifetime |
| `graphics_2d_text_objects` | title/xlabel/ylabel class, properties, ownership, lifetime |
| `graphics_2d_legend_grid` | Legend class/ownership and grid state |
| `graphics_2d_limits_manual` | manual limits under data mutation |
| `graphics_2d_limits_auto_reacts` | auto-limit recomputation |

Repository-wide corpus totals and index tables are intentionally outside this
task's exclusive paths and must be updated by the integration coordinator.

## Marker indices, ticks, Axes style, and interpreters

The follow-up R2022b probes close the properties used by `plot_example_1.m`.

A factory-default Line exposes `MarkerIndices` as a `1 x N` `uint64` row
equal to `1:N`. While that value remains automatic, a data-length change
regenerates it. Any explicit assignment freezes the property, even if it equals
the old default. Empty, duplicate, unsorted, and indices greater than the
current data length are retained; row and column inputs normalize to a row.
Zero, negative, fractional, NaN, Inf, complex, and non-vector inputs are
rejected. Explicit double/single assignments retain their language class.

The model fields are `marker_indices: Vec<u64>`,
`marker_indices_class: MarkerIndicesClass`, and
`marker_indices_automatic: bool`. The indices address original one-based
source points; downstream skips indices greater than the current point count.

Axes retain resolved `x_ticks`/`y_ticks`, `x_tick_mode`/`y_tick_mode`,
`x_tick_labels_utf16`/`y_tick_labels_utf16`, and the corresponding label
modes. Assigning ticks changes only the tick mode to manual. Assigning labels
changes only the label mode to manual, so automatic positions may coexist with
manual labels. Limit changes preserve manual values. Switching tick mode to
auto recomputes positions without overwriting manual labels. Tick vectors may
be empty, outside the limits, or contain infinities; they reject NaN,
non-vector shapes, duplicates, and decreasing order. Label counts need not
match tick counts, and getters return a cell column.

Measured Axes defaults are `FontSize = 10`, `LineWidth = 0.5`,
`Box = 'off'`, and `TickLabelInterpreter = 'tex'`. Model fields are
`font_size_points`, `line_width_points`, `box_enabled`, and
`tick_label_interpreter`; the numeric units are MATLAB typographic points.
`box on/off`, optional-Axes forms, and no-mode toggle forms share that field.

`Interpreter` is a semantic enum `Tex | Latex | None`, defaulting to
`Tex` on Axes tick labels, Text, and Legend. Layout remains downstream.
Legend parses recognized trailing `Interpreter` and `Location` pairs in
either common order; labels precede that property tail or are supplied in one
cell array. A literal label equal to a property name must use the cell form.

All updates retain the staged-clone transaction contract: invalid input changes
no object, resource, pending delta, or Figure revision. The focused oracle
cases are `graphics_2d_marker_indices_semantics`,
`graphics_2d_axes_ticks_modes`, `graphics_2d_axes_style_box`, and
`graphics_2d_interpreter_legend_order`.

The integration task owns builtin registration and must add:

```rust
registry.register("xticks", graphics::xticks_builtin)?;
registry.register("xticklabels", graphics::xticklabels_builtin)?;
registry.register_bare_callable("box", graphics::box_builtin)?;
```
