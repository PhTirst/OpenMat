# Web workspace and themes

This implementation adds two UI facilities without changing any accepted
kernel v0, v1, or v2 request, response, event, capability, or preview shape.

## Workspace service boundary

The command openmat-server --listen 127.0.0.1:0 --workspace-root DIRECTORY
exposes two independent WebSocket endpoints on the announced loopback listener:

- /kernel continues to carry the negotiated openmat-kernel-v0, v1, or v2
  session.
- /workspace/v1 carries openmat-workspace-v1. It has no kernel session ID,
  execution request, workspace variable, or aggregate-preview concept.

The development launcher passes an explicit workspace root. The web client
derives /workspace/v1 from the announced kernel URL rather than adding file
operations to the kernel protocol.

Workspace requests use an envelope with protocol, requestId, and request
properties. The two v1 requests are:

- list with empty params, returning rootName and ordered entries;
- create with name and kind, where kind is file or directory.

Responses echo protocol and requestId. Success has ok: true and a typed result;
failure has ok: false and code, message, and optional field properties. Stable
failure codes distinguish malformed requests, unsupported versions, path
escape, Windows-invalid and reserved names, existing entries, permissions, root
changes, and other I/O failures.

Only one direct child name is accepted. Absolute paths, dot components, slash,
and backslash are rejected. The service intentionally enforces
Windows-compatible component rules on every host, including device names such
as CON, so a workspace remains portable. File creation uses create_new;
directory creation uses the platform's single-component create operation.
Neither overwrites an existing item. The configured root is canonicalized at
startup and rechecked before each list or create. A configured root that is
itself a symbolic link or Windows reparse point is rejected, and listed reparse
entries are marked other; this first version does not navigate into
directories.

The remaining race is replacement of a previously verified root between the
last root check and the operating-system create call. The API minimizes the
window with single-component, no-overwrite operations and never performs a
separate target existence check. A future nested workspace API should use
directory-handle-relative platform primitives before it permits traversal.

## Current Folder interaction

The Current Folder header uses local, dependency-free SVG components for New
File and New Folder. The buttons have accessible names, titles, native keyboard
operation, focus treatment, and disabled state. Creation uses an inline labeled
name form. After a successful server create, the client selects the returned
entry and performs a fresh list. Failures keep the form and exact name in place
and show both the server message and stable error code. There is no delete,
rename, upload, overwrite, or recursive navigation operation.

When no server URL is configured, the existing demo mode uses an explicitly
labeled in-memory workspace client. A configured native server always uses the
real workspace endpoint.

## Theme contract

The fixed first-run default is modern-dark; it never consults the host color
scheme. The selected modern-light or modern-dark value is stored under
openmat.ide.theme in localStorage. Missing, blocked, or unknown storage values
fall back to the same dark default.

Both themes define the complete token set used by the shell, pane headers,
editor loading surface, Command Window, Workspace, Current Folder, Variable
Editor, settings and creation forms, figures, tables, focus rings, hover states,
and disabled controls. Monaco defines matching openmat-modern-light and
openmat-modern-dark themes and receives the same React theme state as the
shell. The Settings gear is a pure icon button; its panel currently contains
only the labeled Theme select.

The theme work does not alter Command Window input semantics, kernel aggregate
inspection, or Variable Editor writeback.
