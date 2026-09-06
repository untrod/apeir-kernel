# Scheduler architecture

`SchedulerCore` is the single scheduling authority. Deterministic, state-aware,
and adaptive modes are policy sets, not parallel scheduler implementations.
Adaptive policy is forced back to deterministic unless learning is explicitly
authorized.

The current production path verifies hard feasibility and deterministic local
placement. Every dispatch writes a `DecisionTrace` containing policy,
candidates, hard rejections, selected node, reason, and resource snapshot.

The migrated program, workflow, deadline, locality, and experimental adaptive
algorithms are library capabilities. Durable queues, gang reservation,
cross-node fairness, and safe preemption are not yet production-connected and
remain `Experimental` or `Planned`.

This distinction is intentional: only behavior reachable through NKI and
covered by process-level evidence is described as verified.
