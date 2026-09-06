# Kernel boundary

## Trusted computing base

The trusted core is limited to execution control, identity and capability
enforcement, safety decisions, the journal, resource leases, transactional
effects, provider isolation, and verification. Models, agents, providers,
plugins, tools, knowledge sources, and user interfaces are untrusted inputs.

## Included authorities

- Execution owns workload admission through terminal state.
- State owns the append-only journal, integrity checks, replay, and revisions.
- Resource owns lease lifecycle and fencing authorization.
- Scheduler owns feasibility, placement, and the recorded decision trace.
- Effects own intent, authorization, receipt, commit, and recovery semantics.
- Provider runtime owns child-process isolation and protocol translation.
- NKI is the only external control boundary.

## Excluded surfaces

Desktop UI, chat products, agent frameworks, RAG products, OpenClaw, full ROS 2
or OPC UA systems, and model-serving engines are adapters or products above the
kernel. No kernel crate depends on those layers.

## Dependency direction

```text
applications -> SDK -> NKI -> kernel -> provider contract
```

Reverse dependencies are prohibited. The CLI and SDK may not open the journal,
construct providers, or mutate resource state.

The current TCP transport is restricted to loopback. Remote exposure requires a
future authenticated transport and is not enabled by configuration alone.
