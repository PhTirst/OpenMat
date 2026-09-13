# RFC 0019: discrete conditional execution domains

Status: implementation contract for the user-approved conditional subsystem milestone.

Scope: `simulation/`, `apps/web/src/simulation/`, simulation server routes/tests,
and milestone documentation. Existing language, accepted protocol specifications,
root workspace manifests and numerical C ABI remain unchanged.

Model/authoring schema 8 and `/simulation/v8` add an optional `execution` object
to a native `subsystem` kind. Enabled execution has `type: "enabled"`, positive
`period`, `statesWhenEnabling: "held" | "reset"`, and per-output policies.
Triggered execution has `type: "triggered"`, a positive control sampling
`period`, and `edge: "rising" | "falling" | "either"`. Each output policy has
finite `initial` values (scalar broadcast or signal width) and `whenDisabled:
"held" | "reset"`; triggered outputs require held. Policies are numbered in
the same order as the subsystem's Outports. Control handles are `enable` or
`trigger`, distinct from the numbered data inputs.

Version 1 accepts fixed-width real/logical signals and synchronous sampled
control, no continuous states inside a conditional domain. Enabled domains have
one periodic clock. Triggered children inherit invocation, with Unit Delay and
explicit discrete m state callbacks; periodic integration and rate-transition
operators inside triggered domains are rejected. Virtual nesting is allowed;
nested conditional domains are diagnosed until their ordering is specified.
Continuous plants remain outside and use the existing RK4/CVODE solvers.

Component library file schema 2 admits pure discrete components declaring
`sampleTime: -1`. Schema 1 keeps its positive-period requirement. Serialization
uses schema 2 only for inherited-period definitions. Model schemas 5 and newer
can attach these components; triggered domains interpret inheritance as invocation.

Compilation preserves domain membership, independently owned states, and output
boundaries. Conditional work has actual guards in the reference and LLVM numeric
programs. Inactive callbacks are not evaluated. State initialization/reset is
independent of held/reset output policy. Control is sampled only at its clock;
discrete states update once per active invocation, never on solver trials.
Repeated numerical evaluation during an event transaction cannot commit twice.
Candidate caches, control memories and states publish atomically after validation.

SLX profile `conditional-v1` extends `hybrid-v1` with supported EnablePort and
TriggerPort configurations. The importer retains execution boundaries and
rejects unsupported scheduling/initialization settings with source locations.
Older profiles and schemas retain their contracts; old routes reject schema 8.
No SLX writeback, arbitrary model callbacks or ordinary m JIT are introduced.

Validation uses independently authored R2022b models for startup, zero plateaus,
edge directions, disable/reenable, separate output and state policy combinations,
and coincident sample hits. Portable tests cover guarded execution, independent
instances, feedback, failure atomicity, version gates, persistence, editor flows
and unsupported cases. Original runnable examples exercise an enabled discrete
controller with a continuous plant and a triggered counter. Generated MATLAB
packages, outputs and native runtimes stay in ignored local directories.
