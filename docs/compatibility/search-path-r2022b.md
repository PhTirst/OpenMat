# Dynamic MATLAB search-path compatibility with R2022b

OpenMat owns one mutable search path per runtime session. The kernel source
resolver, language filesystem service, server workspace API, and Web IDE all
share that state; the browser does not maintain a second authoritative list.

## Language surface

- `path` reads or replaces the path and returns the previous value when an
  output is requested.
- `addpath` accepts one or more directories or a `pathsep`-delimited `genpath`
  result. `-begin` is the default and `-end` appends while preserving argument
  order.
- `rmpath` removes one or more directories and returns the previous path when
  requested.
- `genpath` returns the root and ordinary descendants with a trailing
  `pathsep`. As measured in the local R2022b oracle, it excludes `private`,
  `@class`, and `+package` directories but includes dot-prefixed directories.
- Direct additions of `private`, `@class`, and `+package` directories are
  omitted from effective path state. Package and class-folder lookup use the
  directory above the special folder.
- Relative directory arguments use the session current folder, never the
  process-global working directory.

Ordinary calls, `feval('name', ...)`, named function handles, and `eval` all use
the same lazy `.m` function discovery. Caller-directory lookup precedes the
current folder, followed by search-path entries in order. `which` and `exist`
consult the same file discovery path. An eligible caller's `private` directory
precedes its ordinary sibling directory; a parent's presence as the current
folder or on the search path does not expose that private directory globally.

Search-path mutations increment a shared generation. A later resolution always
observes the current path order, and dynamically compiled functions are cached
by canonical file path and source-content hash.

## Web IDE integration

Workspace protocol v2 exposes `searchPath`, `addSearchPath`, and
`removeSearchPath` requests plus a `searchPathChanged` event. Current Folder
shows a `PATH` badge for included directories and provides both direct and
recursive add/remove actions. Recursive UI actions are equivalent to
`addpath(genpath(folder))`; they do not add a non-MATLAB `recursive` argument to
the language `addpath` function.

## Deferred boundary

The path is session-local. Persistent `savepath`/`pathdef` configuration,
`restoredefaultpath`, and `rehash` remain deferred. `@Class/Class.m` classdef
files and their declared sibling method files are resolved without adding the
class folder itself to the path. Pre-classdef objects created with the legacy
`class(structValue, 'ClassName')` form remain a separate object-model boundary.
Dynamic scripts, lexical imports, and `+package` functions/classes share the
live session path.
