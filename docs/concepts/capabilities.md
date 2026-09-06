# Capabilities

A capability names behavior independently of a provider or model. Identifiers
are lowercase namespaces such as `reasoning.generate`, `vision.detect`, and
`mathematics.evaluate`. Each CapabilityContract has a version, input and output
schemas, constraints, requirements, and metadata.

Providers advertise capabilities. Workloads request them. Discovery narrows
candidates, then SchedulerCore applies resource, privacy, health, deadline,
cost, and safety constraints. Capability matching never authorizes an action by
itself.
