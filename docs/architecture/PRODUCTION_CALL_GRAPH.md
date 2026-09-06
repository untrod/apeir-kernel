# Production call graph

| Operation | Reachable path | State mutation | Status |
| --- | --- | --- | --- |
| Boot | `main -> BootCore::open -> verify_integrity -> KernelRuntime::recover -> listen` | orphan lease revocation and safe pending recovery | Verified |
| Submit | `NKI -> dispatch_request -> KernelRuntime::execute` | scheduling, lease, intent, receipt, commit, terminal state, release | Verified |
| Continuity plan | `NKI -> execute_continuously -> compatibility gate -> execute` | abort/rebind plus ordinary step records | Verified with reference processes |
| Provider probe | `NKI -> KernelRuntime -> ProcessProvider -> worker probe` | none; report returned to caller | Verified for reference and one remote service |
| Provider call | `DurableExecutor -> ProcessProvider -> worker --stdio-once` | normalized receipt is journaled by kernel | Verified |
| Transactional provider effect | `intent -> provider -> receipt -> StepCommit` | append-only journal | Verified |
| Recovery | `recover -> revoke_orphaned_leases -> pending_operations -> execute` | revoke plus safe replay | Verified |
| Cancel | `NKI -> KernelRuntime::cancel -> CancellationToken -> ProcessProvider` | request, terminal state, lease release | Verified |
| Shutdown | `ctrl-c -> KernelRuntime::shutdown -> cancel active -> wait empty` | normal cancellation records for active work | Verified |
| Checkpoint | NKI method name is reserved; daemon has no handler | none | Planned |

`inspect` and `explain` read journal-backed metrics and the latest recorded
decision trace. They do not mutate runtime state.
