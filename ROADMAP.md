# Roadmap

APEIR Kernel development is organized by system guarantees rather than product
features. Items move into the supported set only with executable tests and
platform-specific evidence.

## 0.1 stabilization

- publish the standalone source repository and versioned contract set;
- run Windows x64 and Linux x64 CI against the release revision;
- stabilize NKI v1/v2 negotiation, provider ABI, and compatibility commands;
- keep authorization, recovery, cancellation, and lease invariants under
  process-level regression tests;
- define signed source and binary provenance for binary distributions.

## Execution backends

- implement and verify Windows Job Object containment without dropping
  Windows 10 support;
- implement and verify Linux process supervision and cgroup v2 accounting;
- define common capability reporting for native process, container, WASM,
  device, and remote-node executors;
- qualify Linux ARM64 and Windows ARM64 from reproducible release inputs.

## Distributed execution

- authenticated remote NKI transport with explicit principal identity;
- content-addressed artifact transfer with resumption and size limits;
- durable node ownership, resource leases, placement, cancellation, and
  disconnect recovery;
- desired-state reconciliation and service lifecycle management.

## Edge and device execution

- versioned device identity, capability, and lease contracts;
- verified firmware deployment, boot observation, and rollback receipts;
- native APEIR Micro validation on an RTOS target;
- external ROS 2, OPC UA, and vendor-device adapters built above the provider
  boundary.

Model pricing, account budgets, marketplace features, desktop workflows, and
domain applications belong to the APEIR distribution. Kernel work is limited
to provider-neutral resource accounting and enforceable execution policy.
