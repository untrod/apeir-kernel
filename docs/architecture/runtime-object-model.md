# Runtime object model

APEIR exposes twelve object concepts without giving each one an independent
state store.

| Object | Purpose | State owner |
| --- | --- | --- |
| Runtime | process lifecycle and recovery | `BootCore` and `KernelRuntime` |
| Task | requested and observed execution | journal |
| Capability | portable behavior requirement | contract; evaluated by scheduler |
| Provider | capability implementation | isolated provider process |
| Resource | capacity and fenced allocation | resource core and journal |
| Context | workspace, trace, and semantic snapshot | operation submission |
| Artifact | immutable output reference and digest | committed receipt |
| Event | ordered observation | journal |
| Policy | scheduling and governance input | SchedulerCore |
| Identity | authenticated principal and grants | security boundary |
| Node | compute placement target | node state owner |
| Workflow | dependency graph and continuity | runtime coordinator |

Declarative `*Contract` structures under `nous-types::open_runtime` describe
these objects at extension boundaries. They do not introduce mutable ownership.
State changes still pass through NKI, execution admission, and the journal.
