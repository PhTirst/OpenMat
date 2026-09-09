# OpenMat reusable UI component definition v3

This additive extension preserves `ui-class-v2.md` and `ui-v1.md`. XML definition
versions are independent from the UI display MIME version.

## XML

`<OpenMatUI version="3" kind="component" class="ParameterEditor">` contains one
root Panel Component. `kind` and a valid qualified class name are required;
`controller` is not accepted. Component, Layout, Property and Event formats,
identifier rules, bounds and `receiver.method` connections are unchanged from v2.
The root aliases the component instance. Internal design IDs remain local to the
definition and are not baked into runtime child IDs. v1/v2 retain Window roots.

The paired M class owns the generated buildDesignerComponents method. Each fresh
instance builds a fresh internal tree. Application documents reference the class
as one component and serialize only its host placement and property overrides.

## Additive snapshot fields

`application/vnd.openmat.ui+json` remains `version: 1`. Each component may carry
an optional `layout` object with the same lowercase keys as the frontend Layout:

```json
{
  "properties": {"RuntimeId": "ui-42", "Type": "Panel"},
  "layout": {"mode": "grid", "columns": 3, "gap": 12, "padding": 12,
             "width": 480, "height": 88, "row": 1, "column": 2}
}
```

The native scalar Layout struct maps Mode/X/Y/Width/Height/Row/Column/RowSpan/
ColumnSpan/Columns/Gap/Padding/Grow to lower-initial keys. Modes are absolute,
grid, row or column. Numbers must be finite in [0,10000], dimensions at least 1,
and row/column/count/span values positive integers. Unknown fields are rejected.
Missing snapshot layout fields retain the document/default values.

RuntimeId is a read-only presentation string unique within a native session.
The frontend may echo it to resolve an event by traversal from its current UI
root; it is not a serializable native handle or a cross-session capability.
Parent references, private state and object graphs stay inside the native VM.
Neither RuntimeId nor Layout is a general inspector property: layout has its own
editor. Existing payload limits and access enforcement continue to apply.
