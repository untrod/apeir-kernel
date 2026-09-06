# Support matrix

This matrix describes the implementation and verification scope of APEIR
Kernel 0.1. A configured adapter is not considered verified merely because its
schema or command exists.

## Platforms

| Platform | Build status | Verification scope |
| --- | --- | --- |
| Windows 10 x64 | Verified locally | locked Rust workspace; daemon/worker process E2E; Python and C SDKs; Micro host tests |
| Windows ARM64 | Build support present | requires repeatable verification against the release revision |
| Linux x64 | Build scripts present | hosted or native release-revision verification required |
| Linux ARM64 | Not certified | native machine verification required |
| MCU / RTOS | Portable C implementation | host tests only; no hardware certification |

## Capabilities

| Capability | Status | Scope |
| --- | --- | --- |
| NKI loopback transport | Verified | length-prefixed requests, session token, deadlines, structured errors |
| Journal integrity and recovery | Verified | local SQLite journal, replay, orphan lease cleanup, safe delivery semantics |
| Deterministic reference execution | Verified | child-process provider path |
| Mathematical reference execution | Verified | deterministic local backend |
| Provider crash, timeout, and cancellation handling | Verified | process-level tests |
| Extension admission and authorization binding | Verified | receipt binding and altered-request rejection |
| OpenAI-compatible adapter | Implemented | requires operator credentials and endpoint validation; not part of offline conformance |
| Ollama and edge-compatible adapters | Implemented | service-specific release verification required |
| Cross-provider continuity | Tested with reference providers | external provider qualification required |
| C and Python clients | Verified on Windows x64 | protocol and error handling tests |
| Resource lease and deterministic local placement | Verified in unit/process scope | not a distributed scheduler |
| Remote NKI and multi-host execution | Unsupported | authenticated remote transport is not implemented |
| Hostile multi-tenant isolation | Unsupported | loopback trusted-controller boundary only |
| Containers, WASM, ROS 2, OPC UA, YOLO, physical devices | Contract or design level | supplied by external integrations and independently qualified |

“Verified” means the repository contains an executable test that has passed for
the stated scope. It does not imply penetration testing, formal verification,
hardware certification, service availability, or fitness for a particular
production deployment.
