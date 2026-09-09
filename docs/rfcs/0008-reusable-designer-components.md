# RFC 0008: Reusable designer components and native UI lifecycle

Status: Implemented; additive extension of RFC 0007

## Reusable definition

The designer's **新建组件** creates a Panel-root XML v3 document paired with an
M class. A component is a real handle instance, including all its internal
controls. The host document stores that component's class and instance property
overrides. It does not copy its internal tree. Public stored properties drive the
inspector under the existing exposure rules.

Saving regenerates the marked region only: named child references, a constructor
when absent, `applyDesignerDefaults`, `buildDesignerComponents`, and listener
connections. The constructor applies root defaults before host overrides; tree
construction happens during initialize. A custom constructor must explicitly
call `obj.applyDesignerDefaults()`. User setup, update, event handlers, properties
and events stay outside the generated region. Generated child references are
publicly readable with private setters; mutating a referenced handle does not
replace that reference.

Root layout defines internal arrangement and initial size. The host may override
placement and size, but not the component's internal mode, columns, gap or padding.
The inspector links **编辑组件定义** to the paired class-name `.omui` path.
Keep component class and definition basenames together, including package paths.
Saving the definition rebuilds the class; existing applications use that version
on their next preview or run. Live class replacement is not implemented.

## Tree ownership and lifecycle

`initialize` runs buildDesignerComponents, setup, child initialization and refresh.
Repeated successful initialization does not duplicate children or listeners.
`refresh` invokes update and refreshes the current child tree, with re-entry guards.
Initialization errors are reported; setup is not transactional. Restart a failed
instance before retrying a setup that has already created external resources.

Use `parent.add(child)` to attach or reparent. Cycles are rejected; adding to an
initialized parent initializes the new child. `remove` detaches without deleting.
`delete` detaches, deletes owned listeners and children, calls teardown and
invalidates the handle. `owner.listen(source, signal, callback)` wraps native
addlistener and ties listener lifetime to the owner. Direct Children assignment
remains available for compatibility but bypasses ownership maintenance.

The backend publishes the current native Layout, including changes made by M
callbacks. Layout supports absolute, grid, row and column arrangements. In an
absolute parent, nonempty Position uses bottom-left coordinates; grid/flex parents
use Layout. Layout.X/Y retain the design document's top-left coordinate convention.
This is OpenMat's API, not a claim of complete MATLAB layout compatibility.

## Event identity

Each UI object has a read-only session-local RuntimeId. Snapshots expose this
presentation key, never an object handle or arbitrary object graph. Events find
the target by traversing the live root with findRuntime. Removing a sibling cannot
redirect an event; deleted or replaced targets are ignored. Keys are not stored
in XML and cannot be reused across sessions. The legacy snapshot path remains
readable when this additive field is absent.

## Acceptance example and boundaries

ParameterEditor combines a label, numeric input and slider, with Caption, Value,
Min, Max and ValueChanged. ParameterApp creates two independently configured
instances, adds and deletes a third from M, and changes grid placement when its
window width changes. This exercises definition reuse, instance overrides, private
handlers, signals, child-handle mutation, lifecycle and native layout snapshots.

The editor still shows the design tree; dynamically created children appear in
the running canvas, not as persistent designer nodes. No automatic property
observer, general data binding, breakpoint designer, live hot reload, multiselect,
arbitrary .mlapp import or code-to-XML reverse engineering is introduced here.
