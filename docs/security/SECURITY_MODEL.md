# Security model

The current trusted computing base comprises NKI envelope validation, journal
integrity, execution control, safety decisions, fenced leases, credential
reference enforcement, provider isolation, and result verification.

The NKI TCP listener is loopback-only. Provider children receive a cleared
environment. A credential value can cross the provider boundary only when its
environment-variable name is explicitly allow-listed; the name may be stored,
but the value may not be logged, journaled, or written to a manifest.

Models, providers, prompts, tools, knowledge, and workload inputs are untrusted.
The current candidate is not a hostile multi-tenant sandbox. Authenticated
remote transport, complete production capability enforcement, OS containment,
and secure-store integration remain release gates.
