# Web Monaco to native OpenMat LSP

## Boundary and transport version

The browser does not start or embed a language-server process. A loopback-only
`openmat-server --listen 127.0.0.1:<port>` listener exposes three independent
WebSocket paths:

- `/kernel` for the negotiated kernel protocol;
- `/workspace/v1` when a workspace root is configured;
- `/lsp` for `openmat-lsp-websocket-v1`.

`openmat-lsp-websocket-v1` carries one standard JSON-RPC 2.0/LSP message in each
WebSocket text message. It does not use stdio `Content-Length` framing and adds
no proprietary JSON envelope. Each accepted `/lsp` connection owns a new
`openmat_lsp::Server`, document store, symbol index, request ID space, and LSP
lifecycle. Closing one connection cannot retain or expose documents to the next
connection.

The existing listener guard remains authoritative: `--listen` rejects every
non-loopback bind address, the handshake accepts only the exact endpoint path
without query parameters, and browser Origins must be `localhost` or
`127.0.0.1`. Kernel and workspace routing and sessions are otherwise unchanged.

Incoming and outgoing LSP JSON messages are limited to 1 MiB. Binary frames,
invalid UTF-8, protocol violations, and oversized messages receive WebSocket
close codes appropriate to the failure. Malformed JSON receives the normal
JSON-RPC parse error. `initialize`/`initialized`, correlated requests,
`shutdown`/`exit`, and LSP document lifecycle are handled by the reused
transport-neutral server state machine.

## Monaco integration

`CodeEditor` keeps lexical Monarch highlighting available independently of the
language connection. App owns one `SharedEditorSession` for the workbench and
Designer. It supplies all open documents to one `OpenMatMonacoLspWorkspace`.
Switching tabs or editor views retains the connection, undo history, document
versions, models, and provider registrations. Providers are backed by
JSON-RPC requests to `openmat-lsp`:

- completion and completion resolve;
- diagnostics markers;
- hover;
- definition and references;
- prepare rename and rename workspace edits;
- whole-document formatting;
- full semantic tokens;
- quick-fix code actions;
- hierarchical document symbols.

There is no static completion fallback. On connection loss the editor model and
typing remain usable, stale OpenMat markers are cleared, language requests
degrade to empty results, and the bridge retries with bounded backoff. A
reconnected LSP session reopens every retained document with its latest complete
text. Closing a tab sends only that document's `textDocument/didClose`; ending
the editor session closes all documents and performs `shutdown` followed by
`exit` when the connection is still available.

The workbench retains inactive Monaco models so references and multi-file
rename can operate on them. Model changes are routed by document ID, including
inactive edits, and mark the correct tabs dirty without writing files. Ordinary
Save and revision-conflict handling remain authoritative. Model ownership is
reference-counted across editor mounts. URI identity includes the workspace root
path, so recovered files from different roots cannot share a model even when a
restarted server reuses a root generation. Monaco line-ending normalization on
load does not mark an unchanged file dirty when switching tabs.

Monaco's definition, reference, rename, hover, and context-menu contributions
are loaded with the editor. An editor opener routes cross-file targets back to
the existing Open Editors tab and selects the requested UTF-16 range, preserving
unsaved text and original root identity. Unknown document URIs are not opened
implicitly unless they identify a readable `.m` file in the current workspace
root and generation. Missing targets are read in a bounded batch, registered as
ordinary clean drafts and Monaco models, and synchronized with `didOpen` before
navigation or another language request. A failed read or a root change abandons
the batch. Existing unsaved drafts always take precedence over disk.

The Designer shares this same session and source registry, including independent
callback files. Editing, undo, navigation, diagnostics and saving address the
same document in both views. Directly opening a design reads its actual controller
before registering a fallback template; hidden Designer state cannot resurrect
a discarded draft. A successful save advances the saved revision without
discarding edits made while the write was in flight.

Asynchronous results are rejected if the connection or synchronized documents
change while a request is in flight. Diagnostics are matched by URI and version;
workspace edits validate the LSP target versions and carry the corresponding
Monaco model versions. A late rename cannot overwrite a more recent edit in an
inactive tab.

The integration-facing `CodeEditor` props are:

- `lspUrl?: string | null`: explicit `/lsp` endpoint; `null` disables LSP;
- `documentPath`, `documentUri`, and `documentVersion`: active document identity;
- `workspaceDocuments`: all open document IDs, URIs, contents, and versions;
- `onWorkspaceDocumentChange`: routes model changes to the owning document;
- `onOpenDocument` and `reveal`: load/navigate a tab to a source range;
- `editorSession`: attach to App's shared model and language-session owner.

Without `workspaceDocuments`, the editor manages just its active source. App
passes its configured kernel/LSP endpoint explicitly, including when the endpoint
is supplied through props rather than build-time configuration.

## Native workspace index

The host supplies `WorkspaceSnapshot` values to
`Server::replace_workspace_documents`. A snapshot includes the canonical root
URI, current directory, ordered root-contained search paths, source texts and a
completeness flag. The document store keeps disk documents separate from open
overlays: unsaved text wins, closing a tab falls back to the latest disk version,
and deleted disk files disappear after their last overlay closes. Unchanged
texts reuse their syntax/index entries.

The host scans `.m` files through the workspace path boundary and refreshes on
filesystem events and root/search-path changes. Events are coalesced for 150 ms;
context is checked every 200 ms and a two-second fallback scan handles missed
watcher events. Rename forces a refresh. Scans are bounded to 2,048 documents,
16 MiB of source, 20,000 directory entries and depth 32, in addition to the
existing UTF-8, per-file size and reparse-point restrictions. An incomplete scan
is reported through LSP logging and disables workspace rename.

Function resolution uses source filenames, caller-local functions, caller
directories, `private`, current directory, ordered search paths and `+package`
names. Search paths outside the selected root are not indexed. Root changes
replace the disk index; retained drafts from old roots remain isolated.

## Cross-file refactoring

Definitions and references prepare missing Monaco models before returning
locations, so both F12 and the references preview work for unopened sources.
Rename first discovers affected files, then opens and synchronizes them and
requests the edit again against their latest texts and versions. Request
retries and target counts are bounded. Connection changes, cancellation,
unreadable targets and intervening source edits reject the result. Disk-only
edits have LSP `version: null`; open overlays have exact document versions.

Renaming an exported function also emits the standard LSP `RenameFile` operation,
since its `.m` filename must match the function name. The frontend stages the
filename change only when Monaco actually applies the text edit; undo/redo
before saving also updates that pending change. The normal Save action checks
for a destination collision, writes with the original revision and then renames
within the original root generation. The tab displays its pending destination.
Reload from Disk and Save a Copy discard the pending rename. Files are not
written merely by invoking F2.

Saves across multiple affected files are not one atomic transaction. A successful
text write followed by a failed rename retains the pending rename for retry.
Exported class and case-only exported-function renames are deliberately rejected;
class-associated design/resources and portable case-only moves need a separate
refactor operation. Dynamic object member resolution and nested-capture rename
remain conservative.

## URL configuration

The web client accepts `VITE_OPENMAT_LSP_URL`. If it is absent, it derives the
LSP URL only from a clean `ws:`/`wss:` kernel URL whose path is exactly
`/kernel`, replacing that path with `/lsp`. URLs containing credentials, query
strings, fragments, a non-WebSocket scheme, or an unexpected path are rejected
instead of being rewritten.

`tools/openmat-dev` validates the announced
`ws://127.0.0.1:<ephemeral-port>/kernel` URL, derives the sibling `/lsp` URL,
and passes both `VITE_OPENMAT_WS_URL` and `VITE_OPENMAT_LSP_URL` to Vite. Its
smoke lifecycle initializes LSP, opens a document, verifies completion from an
open-document function, verifies diagnostics change and clear, closes the
document, and completes `shutdown`/`exit` with close code 1000.

## Verification coverage and current limits

Core function completion reads the actual `openmat-builtins::minimal_registry`
through the registry's read-only name iterator. The names are cached once per
process without invoking built-in functions. Both stdio and WebSocket LSP
sessions therefore suggest implemented functions such as `fft`, `fft2`, and
`fftn`, in addition to keywords and current-document declarations. Local
declarations visible in the current lexical scope override same-name built-ins
without duplicate suggestions. Function parameters and local variables no
longer leak into sibling functions or methods. Nested function completion
includes enclosing bindings; script locals do not become local-function
bindings. Hover and completion resolve use the same scope calculation; resolve
uses the original item's text-edit range and document version.
Completion resolve and hover identify core built-ins; complete signatures and
function help and dynamically loaded plugin names are not supplied by this
catalog. Workspace search-path functions come from the separate source index.
Registering a new core function
automatically makes it available after rebuilding and restarting the host.

Rust socket E2E coverage exercises the actual `/lsp` handshake and
`openmat_lsp::Server`, including correlated completion, versioned diagnostic
changes, document-close cleanup, and orderly shutdown. Web tests cover URL
guards, JSON-RPC correlation and connection loss, the 1 MiB client limit,
Monaco completion conversion, diagnostic markers, change versions, multi-model
reconnect and close cleanup, stale-request rejection, and `CodeEditor` identity
wiring. Workbench tests cover tab navigation, inactive/batched edits, saving to
the original root, Unicode URI identity, and opening a file without unmounting
an existing editor.

Additional coverage exercises disk/overlay precedence, unopened-source lookup,
filename/search-path visibility, disk changes and deletion, root replacement,
incomplete snapshots, public-function resource operations, reference previews,
draft preparation/revalidation, and shared Designer/workbench edits and saves.
Document synchronization remains full-text. Full signature help, type-based
member completion, and complete nested-capture navigation remain later work.

The integrated browser check uses a disposable workspace with a function,
two callers and an App controller. Starting with one open source, F12 loads the
definition, Shift+F12 opens a references preview, and F2 changes all four drafts.
Before Save the disk is unchanged; afterward the public function's filename and
all callers agree. Directly opening the `.omui` loads its real controller,
Designer/workbench share undo, and Save and Run executes the renamed function
through a real M-language button callback (the fixture displays `3`). This
workflow also verifies that the standalone Monaco references contribution is
registered, which provider-only unit tests cannot establish.
