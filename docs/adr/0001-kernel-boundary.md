# ADR 0001: Kernel boundary

Status: Accepted

The kernel contains execution semantics, state ownership, journal and recovery,
effects, resource leases, scheduling, identity, safety, provider isolation, and
NKI. Product UI, agents, retrieval, model serving, robotics, and industrial
protocols remain adapters above NKI.

This keeps the trusted computing base small and prevents product dependencies
from becoming kernel compatibility obligations.
