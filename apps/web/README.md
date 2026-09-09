# OpenMat Web

The UI speaks the versioned Rust kernel wire schema over JSON text frames.

The Web workbench follows MATLAB interaction semantics: an editor, a native
REPL-backed Command Window, Workspace summaries, an editable Variable Editor,
Command History, and a Current Folder area. Select a Workspace row and use
**Open**, press Enter, or double-click the row to open its matrix in a managed
Variable Editor window. Each variable can remain open independently. Plain
display events are routed to the Command Window; every OpenMat plot
representation opens its own Figure window.

Figure, Variable Editor, and dynamically registered tools share the OpenMat
Window Manager. Windows overlap with deterministic focus/stacking, cascade on
creation, and support pointer move/resize, minimize, maximize, restore, and
application-controlled asynchronous close. The active title bar also supports
Alt+Arrow to move and Ctrl+Alt+Arrow to resize; Shift selects the larger step.
The same provider owns a separate modal-dialog queue above application windows.
File rename, directory-tree move, delete preflight, unsaved-close,
save-conflict, and clear-workspace dialogs share its focus trap, Escape
handling, stacking, and focus restoration without becoming draggable or
minimizable app windows.

## Development

Requires Node.js 20.19+ and pnpm.

```powershell
pnpm install
pnpm dev
pnpm typecheck
pnpm test
pnpm build
```

Set `VITE_OPENMAT_WS_URL` to the native server endpoint, for example:

```powershell
$env:VITE_OPENMAT_WS_URL = "ws://127.0.0.1:8765/kernel"
pnpm dev
```

`App` also accepts a `wsUrl` or a fully constructed `KernelTransport` for an
embedding host. When no URL or transport is supplied, the UI deliberately uses
in-process kernel and workspace mocks and labels both as demo content. With a
native URL, Current Folder derives the independent workspace-v3 endpoint,
lists the server's explicitly configured root, and creates new files or folders
without overwrite. Regular files can be downloaded byte-for-byte through a
short-lived ticket and a streaming HTTP attachment response. File selection and
drag-and-drop uploads use a separate one-time ticket plus a raw HTTP stream, so
binary files are never base64-encoded or forced through the bounded text editor
request. The Current Folder context menu can copy root-relative paths and flips
inside the visible browser viewport. File operations are independent of the
kernel protocol.
Workspace listing, bounded inspection, editor execution, and Command Window
REPL execution are real kernel requests. The Web client requires and locks
`openmat-kernel-v3`; it does not downgrade to an older kernel. The Variable Editor requests
one-based row/column pages of at most 256 top-level elements, maps the response's
column-major value order into an accessible table, and exposes slice controls
for dimensions three and above. Cell and struct previews use a recursive read-only
view: ordered struct fields, shaped empty aggregates, exact decimal integers and
numbers, missing versus empty strings, and UTF-16 code units are retained.
Nested exact values are shown completely; the client does not invent a second
local truncation boundary below the kernel's whole-top-level-element prefix.

The editor keeps multiple workspace files open as independently dirty tabs in
a vertical Open Editors section above the Current Folder file explorer.
Switching tabs does not discard edits, and rename, move, and delete operations
update every affected open document. Tab order, active identity, draft text,
base revision, and editor view position are saved to a versioned IndexedDB
session. After a reload or browser crash, clean tabs are refreshed from disk and
dirty drafts are reapplied only after their base revision is checked. If the
disk revision changed, Save remains disabled until the user opens the shared
conflict dialog. Drafts from another Current Folder
remain available as save-blocked recovery tabs until that folder is selected
and the file is opened again.

Rename validates Windows-compatible names while typing, Move selects a target
from the workspace directory tree, and Delete recursively counts affected
files and folders before enabling its permanent action. The UI contains no
blocking browser `prompt` or `confirm` calls. Revision-conflicting saves offer
Reload from Disk, Save a Copy, and Keep Editing. The copy path is created
without overwrite and written atomically before the editor switches identity.

Double-clicking a `double`, `single`, `logical`, or fixed-width integer matrix
cell opens an inline editor. Integer text remains exact through `uint64` and
`int64` instead of passing through JavaScript floating point. Enter commits one
element through a v3 `setVariableElement` request;
Escape cancels. Every bounded inspect includes a monotonic workspace revision,
and the write is applied only when that revision is still current. A conflict
keeps the typed value visible and requires an explicit reload, so a stale editor
cannot silently overwrite newer execution results. Aggregate, character,
string, sparse, object, and table values remain read-only. Very large
aggregate selections remain bounded by the requested top-level page and the
negotiated semantic and encoded frame budgets; a nested value that cannot be
returned whole is reported by the kernel rather than partially displayed.

The browser never executes the VM. `WebSocketKernelTransport` accepts text JSON
frames only, enforces the native server's 1 MiB limit on actual UTF-8 bytes,
correlates responses by `replyTo`, validates v3 inspect and mutation payloads against the
originating request and negotiated limits, rejects malformed responses, and
ignores unknown event types for forward compatibility. Unexpected socket
closure is reported through connection-loss subscriptions, allowing the UI to
disable affected operations while it recovers. Kernel and workspace
connections are tracked and retried independently with capped backoff, so a
healthy channel is not restarted with the failed one. The shared status
indicator identifies the recovering channel; workspace recovery reloads
Current Folder and search-path state, while kernel recovery renegotiates the
protocol and refreshes variables. Unsaved editor text is kept. In-flight
execution and file mutations fail rather than being replayed automatically,
avoiding duplicate side effects.

The Settings gear selects Modern Light or Modern Dark. The fixed first-run
default is Modern Light; the selection is persisted in localStorage, and the
matching Monaco theme changes with the IDE shell.

## Dynamic React applications

The public frontend contract is exported from `src/windowing/index.ts`.
`App` accepts in-process `OpenMatDynamicAppModule` objects, while a normal Vite
launch can load ESM module URLs from the semicolon-separated
`VITE_OPENMAT_APP_MODULES` setting. A module exports `registerOpenMatApps(host)`
directly or through its default export. The v1 host provides the app registry,
window operations, and the exact React runtime used by OpenMat:

```js
export function registerOpenMatApps(host) {
  function Counter({ payload, window }) {
    const [count, setCount] = host.react.useState(payload.initialValue);
    return host.react.createElement(
      "button",
      {
        type: "button",
        onClick: () => {
          setCount((value) => value + 1);
          window.setTitle(`Counter ${count + 1}`);
        },
      },
      `Count: ${count}`,
    );
  }

  host.apps.register({
    id: "example.counter",
    displayName: "Counter",
    defaultSize: { width: 420, height: 280 },
    minimumSize: { width: 240, height: 160 },
    component: Counter,
  });
  host.windows.open({
    id: "example.counter.main",
    appId: "example.counter",
    payload: { initialValue: 0 },
  });
}
```

URL-loaded modules should use `host.react` instead of bundling another React
copy. A component may return any normal React tree and use Hooks. Reopening a
stable window id updates its payload while preserving user geometry. Module
unload automatically closes windows opened through its scoped host and removes
its registrations; an optional cleanup returned by `registerOpenMatApps` is for
other resources such as event subscriptions. This is a trusted in-process UI
extension boundary—there is deliberately no iframe or sandbox. Native/backend
extension ABI is outside this frontend v1 contract.
