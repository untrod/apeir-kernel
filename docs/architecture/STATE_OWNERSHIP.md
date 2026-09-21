# State ownership

The append-only journal is the only durable runtime authority. In-memory maps,
queues, provider health, metrics, and semantic representations are derived
views. A restart may rebuild them; they must not become a second durable truth.

| State | Owner | Durable record | Mutation boundary |
| --- | --- | --- | --- |
| Workload and operation lifecycle | Execution core | `Workload` and `Operation` entries | `KernelRuntime` |
| Provider result | Execution core | `OperationReceipt` and `StepCommit` | `DurableExecutor` |
| Reality observation | Execution core | `ObservedEffect` with evidence references and digest | authorized `RealityObserver` through `DurableExecutor` |
| Reality verification | Execution core | `EffectVerification` bound to contract and observation digests | authorized `RealityVerifier` through `DurableExecutor` |
| Scheduling decision | Scheduler core | `SchedulerDecision` | `SchedulerCore` |
| Resource lease | Resource core | `ResourceLease` | `LeaseManager` through `KernelRuntime` |
| Effect transaction | Effect core | intent, receipt, and commit | effect authority |
| Semantic revision | State core | revision entry and object storage | compare-and-swap write |
| Identity and capability | Security core | authorized grant record | security authority |
| Provider process health | Provider runtime | transition or metric | provider supervisor |
| Execution profile | State core | `ExecutionProfile` observation | `ExecutionProfileStore` |

## Rules

1. Cross-component mutation uses an owner method, never a shared mutable map.
2. A persisted transition is immutable and checksummed.
3. Derived state is fail-closed when its synchronization primitive is poisoned.
4. Recovery revokes orphaned leases before granting a new fenced lease.
5. The effect transaction table and idempotency index share one atomic lock.
6. Provider success is never promoted to reality success. Only a bound
   `EffectVerification(MATCH)` authorizes a reality-effect commit.
7. Large evidence remains in Artifact Runtime. The journal owns only immutable
   references, digests, identities, policy revisions, and verification facts.
