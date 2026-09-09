# Plot Engine MATLAB R2022b black-box baseline

Status: authored compatibility measurements for the first OpenMat Plot Engine
tranche. These observations were collected from the locally installed MATLAB
R2022b 9.13 on 2026-08-25. They contain no MATLAB source, tests,
documentation, or diagnostic messages.

The reference process forced newly created MATLAB figures invisible for the
duration of each probe and closed all figures during cleanup. Visibility was a
harness concern; the programs observed ordinary public graphics behavior.

## Figure and axes selection

| Operation | Observed result |
| --- | --- |
| Create two automatic Figures | Numbers are `1` and `2` |
| Close Figure 1, then create an automatic Figure | Number `1` is reused |
| Call `figure(7)` twice | The second call selects the same Figure 7 |
| Create an automatic Figure while only Figure 7 exists | Number `1` is selected |
| Call `gcf` with no Figure | Figure 1 is created and returned |
| Call `gca` with no Figure/Axes | A Figure and an Axes are created |
| Call `ishold` with no Figure | Returns false, creates Figure 1, and does not create Axes |
| Execute `hold on` with no Figure | Creates a Figure and Axes and enables hold |
| Select an existing Axes | Its owning Figure also becomes current |
| Close the current Figure while another exists | The surviving Figure becomes current |

Measured public classes are:

- Figure: `matlab.ui.Figure`;
- Axes: `matlab.graphics.axis.Axes`;
- Line: `matlab.graphics.chart.primitive.Line`;
- Scatter: `matlab.graphics.chart.primitive.Scatter`;
- title/x-label/y-label: `matlab.graphics.primitive.Text`;
- Legend: `matlab.graphics.illustration.Legend`.

OpenMat may use different internal Rust enum names, but language-level `class`
and `isa` compatibility must eventually expose the measured public names.

## Basic `plot` decomposition

For `Y = [1 2; 3 4; 5 6]`, `plot(Y)` returns two Line handles. Both lines use
`XData = [1 2 3]`; their `YData` values are `[1 3 5]` and `[2 4 6]` in column
order. The returned handle array has size `2 x 1`, not `1 x 2`.

For `X = [10;20;30]` and the same three-by-two `Y`, `plot(X,Y)` returns two
Lines and both use `XData = [10 20 30]`.

When `X` and `Y` are both three-by-two matrices, columns are paired: two Lines
are created and their X data comes from the corresponding X column. A row X
vector of length three paired with a three-by-three Y matrix creates three
Lines that share that X vector.

A row Y vector creates one Line with X values `1:numel(Y)`. `plot(1)` creates
one point with X and Y both equal to one. `plot([])` returns a `0 x 1` handle
array and does not add an Axes child.

NaN input data remains NaN in the Line data property. When accepted X and Y
inputs are `single`, both public data properties remain `single`; plotting does
not widen them merely for rendering.

## Hold and object lifetime

New Axes initially report hold disabled. After `hold on`, another plot call
preserves the previous Line and adds another Axes child. After `hold off`, the
next plot call replaces both previous Lines: the old handles become invalid and
the Axes contains only the new child. The measured post-replacement Axes
`NextPlot` value is `replace`.

The measured default Figure `NextPlot` is `add` and remains `add` after a basic
plot. Axes hold state is independently represented by the Axes `NextPlot`
transition between `replace` and `add`; this finite observation does not cover
the complete `newplot` reset matrix.

`cla` removes Axes children, preserves the Axes handle, and preserves an active
hold state. `clf` removes the Axes and invalidates its old handle while
preserving the Figure. `close` removes the Figure.

R2022b's no-argument `ishold()` observation is numeric double `0` or `1`, while
`ishold(axesHandle)` is logical. This overload-level class distinction is part
of the authored conformance corpus rather than being normalized away.

These are identity and lifetime observations, not display-only behavior. An
OpenMat renderer must not retain deleted objects as live picking or legend
targets after the graphics transaction commits.

## Limits and labels

Basic `xlim` and `ylim` setters store the requested finite pairs and switch the
corresponding limit mode to `manual`. Title, x-label, and y-label creation each
returns a Text graphics handle. Legend creation returns a Legend graphics
handle.

Measured basic automatic limits are:

| X/Y data | `[xlim ylim]` |
| --- | --- |
| `X=1:3`, constant `Y=5` | `[1,3,4,6]` |
| single point `(1,1)` | `[-0,2,-0,2]` |
| `X=1e12+[0,1,2]`, `Y=0:2` | `[1e12,1e12+2,-0,2]` |
| `X=1:3`, `Y=[NaN,2,Inf]` | `[1,3,1,3]` |
| empty plot | `[0,1,0,1]` |
| `X=-3:-1`, `Y=[-10,0,10]` | `[-3,-1,-10,10]` |
| three zero points | `[-1,1,-1,1]` |

The signed zero spellings above reflect the observed floating-point values.
Automatic limits depend on the current surviving series and are tested per
authored case. Renderer-local pan or zoom must not silently change the kernel's
semantic limits.

## Compatibility use

These finite probes are implementation targets, not a claim that all MATLAB
graphics overloads are understood. Unsupported complex, datetime,
categorical, style, callback, and 3D forms remain structured boundaries until
separate black-box cases are accepted.
