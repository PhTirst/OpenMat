# RFC 0006: App Designer first implementation

Status: Implemented vertical slice

## Decision

App Designer shares the web IDE's neutral gray, white and blue theme. React
renders both design and live controls. Numerical code and component lifecycle
methods run in the native Rust VM; the browser does not evaluate MATLAB code.
PlotView embeds the existing Rust/WebGPU surface and graphics protocol.

The shared theme tokens in `apps/web/src/styles.css` are the palette contract
for future workbench and designer controls:

| Role | Light | Dark |
| --- | --- | --- |
| Content surface | `#ffffff` | `#20252b` |
| Editor chrome | `#f5f7fa` | `#272d35` |
| Canvas surround | `#e9edf2` | `#171c22` |
| Primary text | `#283342` | `#dce4ee` |
| Secondary text | `#667488` | `#a3afbe` |
| Border | `#dce2e9` | `#39424d` |
| Accent | `#176bba` | `#70b9ff` |
| Selection surface | `#e7f1fc` | `#253d54` |

All 23 built-in types have distinct SVG geometry, reused by the palette, object
tree and inspector. New built-ins must supply an icon in the same registry.

Each window is a versioned UTF-8 `.omui` XML file. Editable callback functions
and reusable component classes remain `.m` files. There is no generated-code
region inside user callbacks. A project manifest is unnecessary for this
single-window slice; multi-window entry points and packaging remain future work.

## Design model

`UiDocument` owns a rooted component tree. Each node has an immutable ID, an
editable MATLAB identifier, a registered type, instance property overrides,
layout constraints, event function names and ordered children. IDs survive
save/reload and moves; duplicates get new IDs and names. The history contains
at most 100 transactions. Preview values and source-created internal children
never enter that history or the XML source.

One document has exactly one root Window. TabGroup only owns Tab children;
SplitPane owns at most two children. Containers support absolute placement,
rows, columns and grids. Absolute drops snap to eight logical pixels. Grid
indices are one-based. Parent layout controls child placement; a node's layout
mode controls its own children. The inspector exposes both relationships.

XML accepts no DTD or entity definitions, rejects unknown elements and
attributes, and validates nesting, IDs, names and property types before applying
changes. Serialization is canonical (including sorted property names); XML
comments and original whitespace are not source-format-preserved. Unknown
component types and their property overrides survive loading as placeholders.
See `spec/protocol/ui-v1.md` for the wire presentation format.

## Class extension

`matlab.ui.componentcontainer.ComponentContainer` is an OpenMat-authored
embedded base class, resolved after workspace/search-path candidates. Classes
inherit normal handle semantics. `initialize` invokes protected `setup` and
`update`; `refresh` invokes `update`. `openmat.ui.Control(type)` supplies the
standard browser control surface and `add(child)` establishes its child list.

Registration creates the requested class in a separate native kernel and
describes its inherited public stored properties. SetAccess determines inspector
editability. Private fields stay internal. Scalar text, double and logical
properties, cell lists/tables and bounded double matrices are represented by
the inspector. Native type information controls source-literal encoding and
is not a second hand-maintained property schema. Unsupported values do not
become editable properties. Dependent-property getters are not invoked for
reflection. MATLAB typed-property validation and enum editors are not supplied
by this slice.

The visual designer runs setup/update in a disposable native preview session
after design changes when custom classes are present. Only presentation is
retained. Its internally-created controls cannot be moved into the XML tree.
Loading an XML document automatically inspects previously unknown class names;
the Register action can explicitly refresh an already loaded class.

## Execution and ownership

Run creates a new kernel-v3 WebSocket session and `app`, a struct of stable
component-name fields containing live handle objects. Public instance overrides
are applied before initialize. Children declared in XML are attached after
component setup. Startup then calls the root's Startup binding or the application
callback. Subsequent events invoke a named `.m` function as `handler(app,event)`.
Event fields include Source and EventName, optional Value, and one-based Row and
Column for table events. Standard runtime controls also support function-handle
ButtonPushedFcn, ValueChangedFcn, CellEditCallback and SelectionChangedFcn. Source
components can use these callbacks to update private internals.

The existing sequential kernel queue remains authoritative. Browser input events
are serialized; pending ValueChanged events for the same control are coalesced.
The queue is bounded at 64 events. Interrupt uses the kernel's independent control
path. Stop disconnects that session, clears pending work and releases graphics
attachments. Main IDE workspace variables are separate. Current Folder/search
path services are still host-owned shared services, and app code retains normal
filesystem access. This is session isolation, not a security sandbox.

`openmat_ui('snapshot', component)` publishes a bounded full presentation after
execution. `openmat_ui('describe', component)` publishes public property metadata.
Output delivery follows the existing execute boundary; this slice does not stream
per-assignment UI changes during a long callback. PlotView's FigureIndex selects
the Nth distinct figure discovered in the preview session, not a MATLAB figure
handle number. Graphics attachments use that same kernel session ID and its
existing capability token.

## Persistence and failure handling

The existing workspace service performs revision-checked writes. A newly-created
file's revision is retained even if its first write fails, allowing a safe retry.
The callback file and XML are separate writes; failure of the second does not
roll back the first. The editor reports conflicts rather than overwriting newer
external contents. Browser-local, versioned recovery drafts supplement explicit
workspace saves. Sidebar sizes and the IDE theme are persisted independently.

The event inspector displays the effective callback function and its `.m` path,
including fallback to the application callback. Edit Code opens the source and
positions the editor at the component dispatch branch, or the function header.
Application and independent callback buffers retain separate revisions and
recovery drafts. Missing independent function files start as editable templates;
explicit Save creates them. Other read failures never produce replacement files.
The file selector switches buffers without discarding edits. Function paths are
resolved relative to the UI file, using `+package` folders for qualified names.

## Compatibility measurements and validation

Two locally authored MATLAB R2022b probes verify protected virtual dispatch from
a base-class method and lexical method visibility of an anonymous callback.
They produce normalized numeric results only; no MATLAB implementation,
documentation, tests or messages are copied. The matching parser/object/kernel
regressions live beside their implementations.

The frontend has model, XML, icon, inspector, save/reload and conflict tests.
The native kernel tests cover embedded classes, inherited metadata, setup/update,
captured callback objects and separate workspace state. Browser verification
covers the signal-analysis app against the native server and Rust/WebGPU renderer.

## Deliberate first-version boundaries

This implements a single-window designer, not a claim of complete MATLAB App
Designer compatibility. Advanced table features, automatic multi-selection
alignment, dockable editor panes, application packaging, dialog services,
dependent/enum/validator property editors and streaming long-running UI updates
are outside this slice. Image accepts HTTP(S) and bounded raster data URLs;
workspace asset picking is not yet implemented. `.omui` is an OpenMat format,
not a reader/writer for MATLAB `.mlapp` files.
Source-created internal controls currently use a vertical layout; their native
Position property is retained as metadata but does not drive browser placement.
Designer-authored nodes use the XML layout constraints.
