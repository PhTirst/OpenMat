# Plot Engine 2D language closure

This tranche extends the retained graphics object model and language builtins
against MATLAB R2022b observations. Octave behavior is not used as a
compatibility oracle.

## Transaction and lifetime contract

Every mutating request still passes through one
`GraphicsSession::execute` staged clone. Property names, target handles, target
classes, data shapes, and every value are validated before the staged state is
committed. A failed request therefore changes no Figure revision, object,
resource, color-order index, or pending delta. A successful request increments
the owning Figure revision once and propagates complete object upserts through
the existing delta format; no field-level delta or new Rust ABI is introduced.

Handles retain session-local slot, generation, and public class identity.
`isgraphics` reports false for stale handles without throwing. `==` and `~=`
compare graphics identity and support scalar expansion over a homogeneous
handle column. Clearing or replacement invalidates removed generations.

## Figure and target selection

No-argument `figure`, with or without parentheses, always creates a fresh
Figure using the smallest unused positive number. `figure(n)` selects an
existing Figure numbered `n` or creates that explicit number. Bare `gcf`,
`gca`, and `ishold` use their MATLAB zero-input meaning while explicit calls
remain callable. `gcf`, `plot`, and other current-target operations reuse the
current live Figure and create one only when none exists.

`plot`, `scatter`, `title`, `xlabel`, `ylabel`, `legend`, `xlim`, `ylim`, and
`grid` accept an optional Axes handle as their first argument. The target must
be a live Axes owned by this graphics session. Explicit targeting does not
change unrelated Figures and preserves hold, replacement, color-order, and
line-style-order behavior.

## Properties and MATLAB values

`get` and `set` use ASCII case-insensitive exact names or unique prefixes. An
ambiguous prefix is rejected. A scalar graphics value, including an internal
`1 x 1` `GraphicsArray`, returns the property value directly; a multi-object
handle column returns an equally shaped cell column.

The closed property set covers:

- Figure/Axes hierarchy: `Parent`, `Children`.
- Axes styling and state: `ColorOrder`, `LineStyleOrder`,
  `ColorOrderIndex`, `XLim`, `YLim`, `XLimMode`, `YLimMode`, `XGrid`,
  `YGrid`, `ZGrid`, `XTick`, `YTick`, their tick/tick-label modes and labels,
  `TickLabelInterpreter`, `FontSize`, `LineWidth`, and `Box`.
- Line: `XData`, `YData`, `LineWidth`, `Color`, `LineStyle`, `Marker`,
  `MarkerSize`, `MarkerIndices`, and `Visible`.
- Scatter: scalar `SizeData`, scalar RGB `CData`, `MarkerFaceColor`,
  `MarkerEdgeColor`, `Marker`, `XData`, `YData`, and `Visible`.
- Text/Legend: `String`, `Color`, `FontSize`, `Interpreter`, and `Location`
  where applicable.

Changing Line or Scatter `XData`/`YData` recomputes automatic limits after both
vectors have been validated and installed. Manual limits remain unchanged.
`SetAxesLimits` validates a Figure handle and both finite, strictly increasing
limit pairs before one staged update sets X/Y limits and both modes to
`Manual`; the resulting delta contains one complete Axes upsert and one Figure
revision increment.

Axes tick and label state is resolved in the model. Explicit ticks or labels
set only their corresponding mode to `Manual`; automatic positions may coexist
with manual labels. `xticks` and `xticklabels` route to the same strong property
updates used by `set`/`get`. Axes `FontSize` and `LineWidth` are typographic
points, and `box` updates the semantic Axes outline rather than a frontend flag.

Line `MarkerIndices` retain one-based original-data positions, order,
duplicates, empty values, and indices beyond the current data length. An
automatic provenance bit distinguishes the factory `1..=N` value from an
explicitly assigned equal vector so later data updates match R2022b.

`class` returns a MATLAB char row at the language boundary. Graphics public
classes remain `matlab.ui.Figure`, `matlab.graphics.axis.Axes`,
`matlab.graphics.chart.primitive.Line`,
`matlab.graphics.chart.primitive.Scatter`,
`matlab.graphics.primitive.Text`, and
`matlab.graphics.illustration.Legend`. Dot reads such as `h.Parent` are routed
to the same model query as `get(h,'Parent')`; dot writes remain outside this
closure.

## LineSpec and marker mapping

LineSpec recognizes `-`, `--`, `:`, and `-.`; the eight MATLAB short colors;
and the following marker spellings. Name/value `Marker` accepts the same short
symbols and canonical long names.

| MATLAB spelling | Model marker | Protocol boundary |
| --- | --- | --- |
| `none` | `None` | `None` |
| `.` | `Point` | `Point` |
| `o` | `Circle` | `Circle` |
| `+` | `Plus` | `Plus` |
| `*` | `Star` | `Star` |
| `x` | `Cross` | `Cross` |
| `s`, `square` | `Square` | `Square` |
| `d`, `diamond` | `Diamond` | `Diamond` |
| `^` | `TriangleUp` | `TriangleUp` |
| `v` | `TriangleDown` | `TriangleDown` |
| `>` | `TriangleRight` | `TriangleRight` |
| `<` | `TriangleLeft` | `TriangleLeft` |

The server/protocol conversion is a coordination boundary outside this task's
authorized paths. It must map the model variants one-to-one as shown and must
not collapse markers to `None`/`Circle`.

## Conformance boundary

The integrated implementation passes all 13 `graphics_2d_*` OpenMat runner
cases against the captured MATLAB R2022b reference. The CLI normalizes finite
double observations with the oracle's `%.17g` convention, including exponent
spelling and signed zero, while the graphics model continues to retain the
original IEEE-754 values. This keeps comparison formatting out of the model and
renderer rather than rounding graphics properties.
