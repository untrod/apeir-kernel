# ADR 0004: Transactional effect semantics

Status: Accepted

An external operation records intent before execution, receipt after execution,
and commit only after receipt durability. Recovery re-enters only operations
declared idempotent or reconcilable. A durable receipt is committed without
re-executing the provider.

Automatic retries at the SDK, provider, and execution layers were rejected.
Each layer owns a distinct failure class and never multiplies operation retries.
