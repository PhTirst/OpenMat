# Graphical model editor

OpenMat now includes a block diagram editor in the existing React workbench.
It uses the native Rust simulation engine with optional LLVM and CVODE. Browser and desktop share
the editor and theme; native file dialogs stay behind the desktop platform
adapter. No computation runs in React or in a browser m-language VM.

The latest milestone adds [Enabled and Triggered subsystems](../../simulation/docs/conditional-subsystems.md),
with conditional control handles, editable execution policies, event-marked Scope
records and two runnable examples. It uses model schema 8 and `/simulation/v8`.

## Open and run

Start the current source checkout using the normal development launcher:

```powershell
pwsh -NoProfile -File tools/openmat-dev/Invoke-OpenMatDev.ps1
```

Click **模型编辑器** in the workbench toolbar. Select **一阶反馈系统** from
the example menu and press **F5** or **运行**. Scope displays accepted native
simulation samples. **两个时间常数** demonstrates vector signals and
**离散计数器** demonstrates UnitDelay. The 300-block example is an interaction
fixture, not a solver benchmark. These changes require a build from current
source; previously published installers do not acquire them automatically.

The layout has a searchable block library and model tree on the left, the graph
in the center, an inspector on the right, and Scope/diagnostics below the graph.
Drag the dividers to resize these areas. The library/tree divider also supports
arrow keys when focused. Light and dark colors follow the workbench setting.

## Build a model

Drag a block from the library to the graph, or click a library item.
Connect a named output handle to a named input handle. An output can branch to
multiple consumers; an input accepts one connection. Select a block to edit its
name and numerical parameters. Double-clicking a block focuses its parameter;
double-clicking Scope opens the result pane.

| Block | Parameter and behavior |
| --- | --- |
| Constant | Finite scalar or column vector, for example `1` or `[1, 2]` |
| Sum | One or more `+`/`-` signs; all input widths must match |
| Gain | One scalar or a coefficient vector; elementwise multiplication |
| Integrator | Initial continuous state; input is its derivative |
| Unit Delay | Initial discrete state; updates on the model's shared sample period |
| Scope | Observes one scalar/vector signal |
| Step | Finite transition time and before/after vectors; continuous stage-time evaluation |
| M Function | Pure m function with declared inputs, parameters and output width |
| Component | Reusable m component with named ports, public parameters and private continuous/discrete states |

Every block has its own icon. Parameters accept numerical literals; they do not
evaluate m-language expressions, workspace variables or callbacks. The inspector
also edits start time, stop time, maximum step and the shared discrete sample
period. RK4 is available without extra dependencies. Configured hosts also offer
LLVM and CVODE Adams/BDF. The **非线性摆 · m 函数** example, source editor,
solver setup and source-file workflow are covered in the
[m functions and CVODE guide](../../simulation/docs/m-functions-cvode.md).

The library also provides custom Unit Delay, mass-spring-damper and discrete PI
templates. **新建自定义组件…** declares ports, parameter metadata and states;
the inspector opens the corresponding initialization/output/derivative/update
m functions. **保存到项目组件库** creates a discoverable `.omblock.json`
description with relative source references. See the
[stateful component guide](../../simulation/docs/stateful-components.md) for
creation, sampling semantics, source reuse and complete runnable examples.

Use Shift to select multiple nodes or draw a selection box. Right-click the graph,
a node, a connection or the object tree for the available operations. Selected
connections expose a draggable bend handle; their context menu can reset routing.
Moving nodes or changing parameters is undoable. Keyboard movement of a focused
selected node is saved as an authoring change.

Native models can now group selected blocks with **创建子系统**, navigate into
their children, and reconnect them with **展开子系统**. The library includes
Subsystem/Inport/Outport and native Product/Mux/Demux/State-Space/Transfer Fcn.
The **实验数据 · 子系统 PI 控制** example demonstrates embedded CSV input, native
hierarchy and **写入 m 工作区** for plotting results in ordinary m code. See
[native authoring](../../simulation/docs/native-authoring.md) for the complete
walkthrough, matrix controls, data interpolation and current limits.

| Shortcut | Action |
| --- | --- |
| F5 | Run the visible model; does not reload the page |
| Ctrl+S / Ctrl+Shift+S | Save / Save As |
| Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y | Undo / Redo |
| Ctrl+C / Ctrl+V | Copy/paste selected blocks and their internal connections |
| Delete / Backspace | Delete the selection and incident connections |
| F2 | Rename a selected block |
| Space+drag, middle-drag | Pan the graph |

Ordinary input text editing retains its own shortcuts. Run and save finish an
active inspector edit first. **检查模型** uses native validation; diagnostics
identify the block/port when available. Direct-feedthrough algebraic loops,
missing connections and mismatched widths are errors.

## Files and recovery

`.omsim` is a versioned UTF-8 JSON authoring document. It contains the RFC 0010
numeric model and editor metadata: labels, routing and viewport. It contains
neither React Flow runtime state nor Scope samples. Incomplete graphs can be
saved and completed later. Old raw `.omsim.json` models can be opened and saved
as `.omsim`. Unknown versions or unsupported fields fail explicitly.

Open model files from Current Folder or the editor's **打开** action. In Web,
**导入文件** reads a browser-selected file; saves write into the server's current
workspace using file revisions. If another program changes an opened file,
OpenMat reports a conflict instead of overwriting that change. Desktop uses
native file selection and Save As dialogs. For repeated revision-checked saves,
keep the model in Current Folder. Saving outside that folder is supported through
the native dialog; subsequent saves ask for a location again in v0.

A local draft is cached per workspace. Closing or replacing a dirty model offers
save/discard/cancel. Switching Current Folder through the workbench uses the same
guard; it asks you to stop an active simulation first. A cache is recovery help,
not a replacement for a saved file. Scope data is not included in the cache.

## SLX compatibility

Open or import an `.slx` file to select the `control-v1` OPC/SLX profile.
The original-structure view has an explicit m parameter area, a system tree,
subsystem navigation and an inspector. Double-click a virtual subsystem to enter
it; use the breadcrumb/system tree or **返回上层** to navigate. Missing variables
can be supplied in the parameter area or read from an assignment-only .m file.
Press **应用参数并检查** before running. Pending parameters or compatibility
errors disable run/check; unsupported blocks never become dummy executable blocks.

The profile adds ordinary nested virtual subsystems, routing, elementwise math,
continuous Sine/Step, State-Space and proper Transfer Fcn. It still rejects
inherited/multiple rates, executable callbacks, masks, library links, MATLAB
S-functions, model references, Stateflow and nonvirtual/conditional subsystems.
See [the SLX guide](../../simulation/docs/slx-import.md) for precise limits.

**查看 / 编辑数值模型** opens the flattened numerical snapshot for independent
edits. The original source hierarchy remains separate, and a visible notice warns
that reapplying source parameters replaces those edits. Saving creates a schema-4
.omsim containing the original package, hierarchy, parameter text and immutable
generated .m sources. It reopens and runs without a temporary source directory.
Generated callbacks are readable in the source panel, but changes use SLX
parameters; separate component-library export is unavailable for these generated
components. Original SLX is not overwritten or exported. Interactive package
uploads are limited to 2 MiB and parameter text to 64 KiB.

## Runs and results

`/simulation/v8` (with legacy `/simulation/v1` through `/simulation/v7`) shares the native server's existing configured port. A separate
WebSocket carries catalog/check/run/cancel/import requests; no extra server
listener is started. Each connection owns one job. Disconnect cancels its job;
a different connection cannot cancel it. The native worker evaluates an immutable
model snapshot and sends bounded, sequenced batches of accepted samples.

Scope draws at a bounded refresh rate and reduces traces to pixel envelopes for
display. CSV export retains every collected sample. Up to eight components are
visible at once; use the channel selector for wider signals. Editing numerical
parameters or wiring after a run marks the result as belonging to an earlier
model. Moving or renaming a block does not change the numerical result.

The interactive run limit is 100,000 frames and 2,000,000 scalar values. Exceeding
it produces an explicit error; shorten the run or increase the step. Cancellation
and failures preserve already received samples. This service is local-first and
does not add multi-user authentication or resource quotas.

## Current boundary

The editor and CLI support pure m function blocks, stateful m components with
fixed-size matrices and static loops, optional LLVM numerical execution, and
optional CVODE Adams/BDF integration. This does not
add JIT to ordinary m-language execution. Zero crossings, DAE/algebraic solving,
multiple sample rates, general signal buses, executable subsystems, full
MATLAB S-function compatibility and C generation remain future work. OpenMat's
own initialize/outputs/derivatives/update interface is implemented for components.

Implementation details and verification requirements are in
[RFC 0012](../rfcs/0012-simulation-editor-v0.md) and the new
[m function/CVODE contract](../rfcs/0013-m-function-cvode-v1.md), extended by
[the stateful component contract](../rfcs/0014-stateful-simulation-components-v1.md).
