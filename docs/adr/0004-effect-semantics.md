# ADR 0004: Transactional effect semantics

Status: Accepted

An external operation records intent before execution and an execution receipt
after provider execution. For legacy operations, receipt durability is the
commit boundary. Operations carrying an `EffectContract` additionally require
an independent observation and a bound `MATCH` verification before commit, as
defined by ADR 0006.

Recovery re-executes only operations declared idempotent. Reconcilable effects
resume observation/reconciliation instead of blindly repeating the provider.
A durable execution receipt is never executed again.

Automatic retries at the SDK, provider, and execution layers were rejected.
Each layer owns a distinct failure class and never multiplies operation retries.
