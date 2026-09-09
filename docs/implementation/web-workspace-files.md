# Web workspace files

OpenMat Web uses a native-server workspace API. The browser receives the
session's absolute Current Folder path for navigation and display, but never
performs filesystem access directly.

## Endpoints and compatibility

- `/workspace/v1` remains available with protocol `openmat-workspace-v1` and
  its original root-only `list {}` and `create { name, kind }` requests.
- `/workspace/v2` uses protocol `openmat-workspace-v2` and the strict envelope
  below. V2 requests and responses are accepted only on the V2 endpoint.
- `/workspace/v3` uses protocol `openmat-workspace-v3`, preserves all V2 file
  operations, and adds ticketed HTTP downloads. The Web client uses V3.
- All workspace endpoints are enabled only when `openmat-server --listen ...
  --workspace-root <directory>` is used. The listener remains loopback-only.

Every V2 or V3 request uses the protocol matching its endpoint. For example, a
V2 request is:

```json
{
  "protocol": "openmat-workspace-v2",
  "requestId": "workspace-0001",
  "request": {
    "type": "list",
    "params": { "path": "", "recursive": true }
  }
}
```

Every response repeats `protocol` and `requestId`, and contains exactly one of
`result` or `error`. A successful result repeats the request type:

```json
{
  "protocol": "openmat-workspace-v2",
  "requestId": "workspace-0001",
  "ok": true,
  "result": { "type": "list", "data": {} }
}
```

Known request IDs cannot be reused on one WebSocket connection. Request and
parameter objects reject missing, mistyped, and unexpected fields.

## V2 operations

Paths are UTF-8, workspace-relative, forward-slash-separated strings. Only the
root-list path may be empty.

| Type | Exact params | Result data |
| --- | --- | --- |
| `currentDirectory` | `{}` | `{ path, rootName, generation }` |
| `changeDirectory` | `{ path }` | `{ path, rootName, generation }` |
| `browseDirectories` | `{ path }` | `{ path, parentPath, roots, entries }` |
| `list` | `{ path, recursive }` | `{ rootName, rootPath, rootGeneration, path, recursive, entries }` |
| `read` | `{ path }` | `{ path, content, revision, size, rootPath, rootGeneration }` |
| `write` | `{ path, content, expectedRevision, rootGeneration? }` | `{ entry }` |
| `create` | `{ path, kind }` | `{ entry }` |
| `rename` | `{ path, newName }` | `{ previousPath, entry }` |
| `move` | `{ path, targetPath }` | `{ previousPath, entry }` |
| `delete` | `{ path, recursive, confirmPath }` | `{ path, kind, recursive }` |

An entry has `name`, `path`, `kind`, `size`, and `revision`. `kind` is `file`,
`directory`, or `other`. Directory/other sizes and revisions are `null`.
Listings do not hash file contents, so listed file revisions may also be
`null`; `read`, `create(file)`, `write`, rename, and move return a usable file
revision.

## V3 download extension

V3 supports every V2 operation above and adds:

| Type | Exact params | Result data |
| --- | --- | --- |
| `prepareDownload` | `{ path, rootGeneration }` | `{ ticket, name, size, expiresInSeconds }` |

`prepareDownload` validates a regular file against the requested Current Folder
generation and returns a cryptographically random, one-use ticket. Tickets expire
after 60 seconds and are never accepted as filesystem paths. The browser consumes
the ticket with an ordinary HTTP `GET /workspace/download/{ticket}` request on the
same loopback listener. A successful response streams the file's original bytes
with `Content-Type: application/octet-stream`, `Content-Disposition: attachment`,
`Content-Length`, `Cache-Control: no-store`, and `X-Content-Type-Options: nosniff`.
The HTTP response does not buffer the complete file in either JSON or a browser
Blob. Missing, malformed, expired, or already-consumed tickets return `404`.

Current Folder is session state shared by workspace-v2, workspace-v3, and the
Kernel. An absolute path or a path relative to the current directory can be selected;
`browseDirectories` enumerates directories on the server machine without
changing Current Folder. A successful `changeDirectory` does not restart the
Kernel or clear variables and Figures. V2 and V3 connections receive an
independent `currentDirectoryChanged` event, including changes made by
language-level `cd`.
They also receive a debounced `workspaceChanged { rootGeneration }` event when
the server observes a filesystem change below the current root. The event is a
refresh signal rather than a precise file-operation log; clients rescan the
root and already-expanded directories to obtain authoritative state.

`write` is optimistic: `expectedRevision` must be the revision returned by the
last read or save. `rootGeneration` keeps an already-open document attached to
the directory from which it was opened even after Current Folder changes. The
server hashes the current bytes again immediately before an atomic
same-directory replacement. A mismatch returns
`workspace.revisionConflict` with `details.currentRevision`; editor content is
not discarded or overwritten.

Directory deletion is deliberately narrow:

- a file requires `recursive: false` and `confirmPath: null`;
- a non-recursive directory delete succeeds only when the directory is empty;
- recursive directory deletion requires `confirmPath` to exactly equal `path`;
- the Web UI also asks the user to confirm the exact dangerous target.

## Dynamic root jail and text boundary

File-operation paths remain workspace-relative and reject absolute paths,
backslashes, `.`/`..`, empty segments, NUL, Windows-illegal characters and
suffixes, reserved device stems, and components longer than the Windows limit.
Before each operation the service snapshots and validates the current root.
Existing components are inspected one at a time; symbolic links and Windows
reparse points are never traversed. Recursive move/delete checks the complete
subtree before the mutation. Current Folder itself may move anywhere the local
server process can read, but the selected directory must be a real directory,
not a reparse point.

Only regular UTF-8 files up to 131072 bytes can be opened or saved. Binary,
invalid-UTF-8, directory, special-item, and oversized reads return structured
errors such as `workspace.invalidUtf8`, `workspace.notTextFile`,
`workspace.reparsePoint`, and `workspace.fileTooLarge`. Downloads have no text or
128 KiB restriction: they still require a regular file and reuse the same
workspace-relative path jail and reparse-point checks.

Atomic saves create a unique temporary file in the destination directory,
flush its bytes, copy the existing permissions, verify the expected revision a
second time, and replace the destination. Temporary files are removed on a
failed save.

## Web editor lifecycle

The editable Current Folder address bar provides Back, Forward, Up, direct path
entry, and an in-app server-directory browser. Double-clicking a directory only
expands or collapses it; its context-menu Open action changes Current Folder. The pane
loads the root non-recursively and loads each directory on first expansion, so
a large workspace does not have to fit in one WebSocket message. Loaded
directories are refreshed independently and rendered together as an ARIA tree.
Mouse, context-menu, `Enter`/Space, arrow navigation, `F2`, Delete, and
Shift+F10 operations are supported. The pane also provides an icon-only manual
refresh action, while server filesystem notifications automatically coalesce
and refresh changes made by Kernel functions or external tools.

Clicking a regular file reads it before replacing the current editor document.
Dirty switches preserve their tabs; closing a dirty tab opens a managed
Save/Don't Save/Cancel dialog. A newly created `.m` file is opened immediately.
Save and Ctrl/Cmd+S write the current content with its last revision and
original root generation. Revision conflicts open a Reload from Disk/Save a
Copy/Keep Editing dialog. Reload installs a fresh read, Save a Copy creates and
atomically writes a new sibling path before changing editor identity, and Keep
Editing preserves the dirty buffer without overwriting disk. Recovered-draft
conflicts share this flow. Run sends the current in-memory editor
text and its real path as the kernel `sourceName`; if Current Folder has changed
since the document was opened, that source name is absolute so same-directory
source resolution remains attached to the document. Saving first is not
required.

Multiple documents remain open with one active tab. Open Editors remains visible
even when empty. The editor starts empty; there is no fixed `alpha_demo.m` model.
Designer controllers and callback sources use this same document registry.
New sources have an empty revision until their first successful create/write;
even an empty new source remains unsaved. Recovery restores a missing new draft
only after a verified not-found response in its original root, and reports a
conflict if another file now occupies that path.

LSP function rename can stage a `pendingPath` on a document. This marks the
document dirty and is persisted with its recovery snapshot. Save checks the
destination, writes the source with its expected revision, and then performs the
pending filename change. `rename` accepts an optional `rootGeneration` in both
workspace v2 and v3. Explicit generations use a fixed root service even if they
currently match Current Folder, so an intervening directory change cannot
redirect an in-flight save or rename to another root. Omission retains the
existing current-root behavior. Reload from Disk and Save a Copy clear pending
renames. Multi-file refactoring is staged in drafts; saving all affected files
is not an atomic multi-file transaction.

## Monaco document identity contract

`CodeEditor` now accepts these stable integration props:

```ts
documentPath: string;    // workspace-relative path, for UI/diagnostics
documentUri: string;     // openmat-workspace://root-N/ URI, used as Monaco model path
documentVersion: number; // increments for content edits and identity remaps
```

`EditorPane` passes the same values through. The URI is path-segment encoded
and includes both the root path and generation, so equal relative paths in
different Current Folders cannot collide even after restarting the host.
URI parsing accepts Monaco's canonical encoding and rejects malformed or
out-of-root targets. The version does not reset during ordinary saves.

## Verification

Coverage includes workspace service unit tests, a real TCP WebSocket V2 file
lifecycle, WebSocket client correlation and error parsing, ARIA/keyboard tree
tests, and App DOM tests for open/save/run, managed modal focus and queueing,
rename validation, directory-tree move, delete preflight, dirty close,
save-conflict reload/copy/retention, and move identity. `openmat-dev -Smoke`
also exercises V2 recursive
list, create, read, atomic write, an external-change conflict, rename, move,
and exact delete against a real server and temporary workspace root.
