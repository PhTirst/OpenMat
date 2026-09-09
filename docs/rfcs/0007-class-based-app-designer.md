# RFC 0007: Class-based App Designer

Status: Implemented; extends RFC 0006 for new documents

## Application and component identity

New applications use `classdef SignalApp < openmat.ui.AppBase`. The runtime
variable `app` is a native handle object. Named designer controls become its
properties, including the root alias (`app.MainWindow == app`). Version-1
function/struct applications keep their execution path.

The 23 primitives have embedded `openmat.ui` M classes. `AppBase` inherits
`Window`; concrete controls inherit ComponentContainer and declare relevant
properties and events. Users can derive `CounterButton < openmat.ui.Button`,
add public properties, override protected lifecycle methods and declare events.
The inherited `Type` chooses the renderer and icon; the actual class name must
never overwrite it. This API is an OpenMat design, not a claim that MATLAB's
built-in controls are subclassable in the same way.

## Connections and source ownership

Connections name a receiver and instance method: `app.onRun`, `self.reset`,
or `OtherControl.reset`. Root methods can be private because listener closures
are created inside the application class. Other objects must expose accessible
methods; the picker offers public instance methods. Custom events use native
`addlistener` and `notify`, including privately emitted signals.

The inspector opens the declaring `.m` class at the method declaration.
Creating an application handler inserts a private method; creating a handler
in another component inserts a public method into its existing source file.
User bodies and local functions are preserved. Source scanning is lexical and
conservative: incomplete classes fail without replacing the source. This is
not a general language-server refactoring implementation.

Saving v2 updates one `% <OpenMat:components>` region in the app class. It owns
generated control properties and the reserved `bindDesignerComponents` method,
which assigns handles and installs listeners. Manually declared properties are
reused. Users own everything outside that region. XML owns connections and
layout; direct edits to generated listeners are replaced on the next save.

Class and XML files use revision-checked writes, separately rather than as an
atomic transaction. Missing or conflicted custom class files are errors, never
replaced with empty classes. Qualified classes use UI-relative `+package` paths.

## Lifecycle and presentation

Run constructs the app and children, applies design overrides, attaches
children, binds properties/listeners, initializes children, initializes the app
and emits Startup. Code needing designer controls belongs in setup/onStartup,
after binding, rather than the constructor. New apps bind app.onStartup.

Browser input assigns an editable value, invokes any legacy callback property,
then emits the native primitive event. `source` is the actual control; the
event structure contains Source (name), EventName, Value and optionally
Row/Column. Custom notify calls supply their own event data. Refresh and a
bounded snapshot follow callbacks. Handles and listeners stay in the kernel.

Registration constructs and reflects a class without initializing the app.
It reports public stored properties, listenable events and method ownership.
Component previews execute setup/update in disposable sessions. Their properties
and internal children never overwrite the design definition. App saves refresh
its inspector schema; externally edited custom components require registration
again to refresh metadata.

## Validation and boundaries

Frontend regressions cover XML versions, source preservation, method insertion,
instance wiring, reflection and editing. Native tests cover inheritance, parent
constructor order, packaged explicit constructors, private callbacks and custom
signals. Browser checks exercise persistent class state, new inspector
properties and a derived button's signal-to-member connection.

Three locally authored MATLAB R2022b probes agree with the native constructor
regression: implicit, local and packaged explicit construction yield B, BL and
EX respectively. Only normalized results were retained. The Windows kernel
tests require `cargo build -p openmat-oex` to provide the C bridge DLL fixture.

Property editors support common scalars, lists and tables. Dependent properties,
arbitrary object graphs, validators, enums, multi-window packaging, general
rename refactoring, full `.mlapp` compatibility and streaming long callbacks
remain outside this slice.
