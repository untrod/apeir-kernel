# Runtime lifecycle

`BootCore` opens and verifies the journal before reaching `READY`. Admission
validates sensitive references, safety, capability, and resources. Execution
records intent before invoking a provider, then records a normalized receipt
and commit. Cancellation and deadlines terminate the active provider process
and release its fenced lease. Recovery replays committed receipts and refuses
unsafe non-idempotent re-execution.
