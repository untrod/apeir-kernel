# ADR 0003: Provider process isolation

Status: Accepted

Providers run outside `nousd` and communicate through a versioned serialized
command and normalized receipt. The worker receives a cleared environment plus
only explicitly authorized credential references.

A one-shot process currently favors fault containment and deterministic cleanup
over throughput. A future pool must preserve the same isolation contract.
