# State ownership

The append-only Journal is the only durable runtime authority. In-memory maps,
queues, provider health, metrics, registries, and projections are derived views.

| State | Owner | Durable record | Mutation boundary |
| --- | --- | --- | --- |
| Workload and operation lifecycle | Execution core | `Workload` and `Operation` entries | `KernelRuntime` |
| Provider result | Execution core | `OperationReceipt` and `StepCommit` | `DurableExecutor` |
| Remote execution candidate | Node, then Kernel admission | signed `RemoteExecutionReceipt` embedded in accepted `OperationReceipt` | Node signs; `DurableExecutor` validates and journals |
| Governed reality target | Administrator configuration, bound per operation | `TargetBinding` fact and canonical digest | Registry resolves; `DurableExecutor` admits |
| Reality observation | Execution core | `ObservedEffect` with Evidence references | authorized `RealityObserver` through `DurableExecutor` |
| Reality verification | Execution core | `EffectVerification` bound to contract and observation | authorized `RealityVerifier` through `DurableExecutor` |
| Reality evidence bytes | Distribution Artifact Runtime | immutable SHA-256 Artifact and metadata | Artifact bridge PUT/GET only |
| Scheduling decision | Scheduler core | `SchedulerDecision` | `SchedulerCore` |
| Resource lease | Resource core | `ResourceLease` | `LeaseManager` through `KernelRuntime` |
| Semantic revision | State core | revision entry and object storage | compare-and-swap write |
| Execution profile | State core | `ExecutionProfile` observation | `ExecutionProfileStore` |

## Rules

1. Cross-component mutation uses an owner method, never a shared mutable map.
2. A persisted transition is immutable and checksummed.
3. Recovery revokes orphaned leases before granting a new fenced lease.
4. Provider success is not reality success. Only bound
   `EffectVerification(MATCH)` authorizes commit.
5. Large evidence remains in Artifact Runtime. Journal owns only immutable
   references, digests, identities, revisions, and verification facts.
6. `RealityAdapterRegistry` resolves mechanism and TargetBinding; neither is
   permission or commit authority.
7. `TransactionalEffectEngine` is an in-memory domain/test reference, not a
   production state owner. Production recovery and commit derive from Journal
   facts owned by `DurableExecutor`.
