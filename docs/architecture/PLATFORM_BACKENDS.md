# Platform backends

## Decision

APEIR Kernel has one portable authority core and multiple operating-system
execution backends. Linux-first means Linux is the first feature-complete
backend for cgroup v2, namespace, seccomp, and Landlock integration. It does
not mean Linux-only, and it does not remove the verified Windows 10 x64 path.

```text
SDK / CLI / applications
          |
          v
   NKI + portable core
   identity, admission, policy, state, leases,
   scheduling, commit, journal, recovery
          |
          v
      Executor ABI
       /       \
Linux backend  Windows backend
cgroup v2      Job Objects
namespaces     restricted tokens
seccomp        filesystem/process ACLs
Landlock       firewall policy
```

The public workload, capability, journal, receipt, and NKI contracts must not
change merely because an operating-system backend changes. A backend reports
what it can actually enforce; admission rejects requirements that it cannot
enforce. It must never silently translate an unavailable security control into
success.

## Current evidence

- The portable core, process-isolated Provider boundary, durable execution,
  recovery, and SDKs are validated on Windows 10 x64 and Windows ARM64.
- POSIX build and package scripts exist, but Linux x64 and Linux ARM64 do not
  yet have native evidence for this candidate.
- `LinuxIsolation` and `WindowsIsolation` are declarative profiles. Their
  presence does not prove enforcement by cgroups, namespaces, Landlock,
  AppContainer, or Job Objects.

## Implementation order

1. Freeze a small Executor ABI with probe, prepare, start, inspect, cancel, and
   cleanup lifecycle operations.
2. Make the existing native process path the reference driver and add bounded
   output, deadline, cancellation, and conformance tests.
3. Implement and verify Linux cgroup v2 process placement before adding
   container, WASM, device, or remote drivers.
4. Add Windows Job Object enforcement while retaining Windows 10 compatibility.
5. Keep container, WASM, microVM, device, and remote-node implementations out
   of the trusted core and require versioned manifests plus conformance proof.

## Non-goals

APEIR does not replace the Linux or Windows kernel, does not implement its own
CPU scheduler or network stack, and does not treat a declared capability as an
enforced capability. Desktop, document, model, and agent features remain
separate distributions above NKI.
