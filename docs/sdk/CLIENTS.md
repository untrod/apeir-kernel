# Client SDKs

Rust and Python SDKs are clients of NKI. They do not schedule workloads, own
workload state, open the journal, execute recovery, or construct providers.

Both clients enforce the current NKI version, response request identity,
16 MiB frame limit, and bounded request deadline. Kernel errors preserve code,
retryability, failed phase, and source component.

Both SDKs expose workload submit, get, list, and cancel operations. They also
expose generation-safe Runtime catalog operations for Project, Model, Graph,
Dataset, Experiment, and Artifact documents. Public request types are defined
in `nous-types`; neither SDK depends on the durable control-plane
implementation.

The C SDK exposes contract and runtime-profile discovery through a versioned,
caller-owned structure. It returns no allocated memory, so no allocator crosses
the ABI boundary. Callers set `struct_size` and `abi_version` before use.
