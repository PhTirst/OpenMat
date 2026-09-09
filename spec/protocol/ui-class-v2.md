# OpenMat class UI definition v2

This extends `ui-v1.md` without changing the kernel transport or UI display
MIME version. Design documents and presentation payloads have independent versions.

## XML design document

The root is `OpenMatUI version="2" class="SignalApp"` with one root Window
Component. `class` is a required qualified M class name. The v1 `controller`
attribute is not accepted on v2. Component, Layout and typed Property formats
and traversal bounds remain unchanged.

```xml
<Event name="Clicked" handler="app.onRun" />
<Event name="Incremented" handler="StatusPanel.updateCount" />
```

A handler is exactly `receiver.method`, both valid identifiers. A receiver is
`app`, `self`, or an existing named designer node. `app` resolves to the root
instance, `self` to the emitting node, and node names through app properties.
Dangling receivers and bare function names are invalid. No Event element means
no designer connection; there is no application-wide fallback.

The class source provides `bindDesignerComponents(app, components)`. The builder
supplies a temporary struct mapping names to native objects. This method assigns
properties and installs listeners in the app's class access context. The struct
is construction input, not the application. Bodies and listeners are not embedded
in XML. Keep `.m` and `.omui` files together.

Version 1 remains accepted, serialized and executed under its original function
rules. Saving a v1 document does not migrate it automatically.

## Additive reflection metadata

`application/vnd.openmat.ui+json` remains `version: 1`. A described component
may include optional arrays alongside className/properties/schema/children:

```json
{
  "events": [{"name": "Incremented", "declaringClass": "CounterButton"}],
  "methods": [{"name": "onRun", "access": "private", "declaringClass": "SignalApp"}]
}
```

Events are visible and publicly listenable. Methods are implemented instance
methods with public/protected/private access and declaring class. Snapshots may
carry empty arrays. Frontends tolerate omitted arrays from older runtimes.
Existing payload bounds apply. Metadata never bypasses native access checks.

For subclass presentation, className identifies the actual class while inherited
Type selects a primitive renderer and icon. Public editable properties and event
metadata drive the inspector. Private fields and object/function values remain
hidden; no native object identity crosses into JavaScript.
