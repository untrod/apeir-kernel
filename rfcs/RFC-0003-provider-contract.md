# RFC-0003: Provider contract

Status: Accepted

Providers execute outside the kernel through a one-request process protocol.
The v1 lifecycle covers probe, metadata, capabilities, health, execute, cancel,
and shutdown. The kernel owns timeout, cancellation, receipt normalization,
credential scoping, and durable commit.
