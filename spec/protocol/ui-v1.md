# OpenMat UI presentation v1

This additive display MIME uses the existing kernel-v3 transport:
`application/vnd.openmat.ui+json`. It introduces no Rust ABI or pointer boundary.

```json
{
  "version": 1,
  "kind": "snapshot",
  "component": {
    "className": "openmat.ui.Control",
    "properties": {"Id": "root", "Name": "MainWindow", "Type": "Window"},
    "schema": [],
    "children": []
  }
}
```

`kind` is `snapshot` or `describe`. Both contain the same recursive component
shape. `describe` adds `schema` entries for the described component's public
stored properties: `name`, `value`, `writable`, and native `className`. An
unsupported value is null and not writable. Child components carry presentation
without reflection entries. Parent references and function handles are never
serialized. Components must derive from the embedded ComponentContainer base.

Values are finite scalars (number, boolean, UTF-16 text represented in JSON),
string cell vectors or two-dimensional scalar arrays. Matrices are presented as
rows even though native storage is column-major. Nested arbitrary structs and
object graphs are excluded. The runtime bounds traversal at depth 32 and 16,384
value visits, cell/numeric arrays at 4,096 elements, text at fewer than 65,536
code units and the final UTF-8 payload at 750,000 bytes. The browser additionally
limits the component tree to 1,000 nodes and rejects unknown versions. The outer
kernel frame limit still applies.

Callbacks travel as ordinary serialized kernel execute requests assembled from
validated identifiers and escaped scalar/array literals. Only registered public
editable properties can be changed through UI events. Generated child paths use
validated one-based child indices; native object handles never enter JavaScript.
Multiple events use a single sequential execution stream, with an independent
interrupt request. Snapshots update only the live renderer; they are never
interpreted as edits to the XML design definition.

The XML source root is `OpenMatUI version="1" controller="FunctionName"` with
one Window `Component`. Components contain one optional Layout, typed Property
elements, Event name/handler references and child Components. Property type is
`string`, `number`, `boolean` or `json` (only bounded lists and scalar tables).
The canonical serializer writes all layout fields, only instance overrides and
explicit event bindings. Runtime-only children, property metadata, object
references, graphics tokens and variable values are not persisted.
