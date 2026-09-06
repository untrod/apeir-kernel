# ADR 0005: One scheduler core

Status: Accepted

Admission, placement, program, workflow, and adaptive behavior are scopes or
policies under `SchedulerCore`; they are not independent schedulers. The first
production policy is deterministic, hard-feasible, and explainable.

Advanced policies remain experimental until they share durable queue,
reservation, dispatch, and recovery semantics with the production core.
