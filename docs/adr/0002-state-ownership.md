# ADR 0002: Durable state ownership

Status: Accepted

The append-only journal is the sole durable authority. Each state class has one
mutation owner. Runtime maps and indexes are derived and rebuildable.

Using independent registries as durable truth was rejected because it creates
ambiguous recovery order and cross-module mutation races.
