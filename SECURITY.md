# Security policy

## Supported versions

APEIR Kernel has no production-supported release. Version 0.1 is a development
release for trusted local-host evaluation. Security fixes are applied to the
default branch until a supported release line is declared.

## Reporting a vulnerability

Use the repository's private GitHub Security Advisory form when it is
available, or contact a maintainer through a private project channel. Do not
open a public issue containing exploit details, credentials, private workload
data, device identities, local paths, journal contents, or private endpoints.

A useful report includes the affected revision and component, prerequisites,
minimal reproduction, security impact, and suggested mitigation. Replace all
secrets and private data with inert placeholders.

## Trust boundary

The trusted kernel components validate NKI requests, make admission and safety
decisions, schedule eligible work, grant fenced resource leases, interpret the
journal, control execution state, verify effects, and perform recovery.
Models, providers, extensions, tools, device adapters, knowledge sources, user
workloads, and user interfaces are untrusted.

Provider workers execute in separate processes. Their environment is cleared;
only required process variables and explicitly allowlisted credential
references may be passed. `NOUS_ALLOWED_CREDENTIALS` contains variable names,
not secret values. Manifests, receipts, logs, and journal records must never
contain credentials.

## Security limitations

- NKI is restricted to loopback and assumes one trusted local controller.
- The session token is not a complete multi-tenant identity or authorization
  system.
- Provider process separation is not an operating-system sandbox.
- Secure secret storage, filesystem permissions, network policy, endpoint
  protection, and account isolation are deployment responsibilities.
- Remote NKI, distributed authority, and hostile multi-tenant operation are not
  supported.
- Execution-proof data structures do not by themselves establish cryptographic
  attestation of the complete workload.

## Coordinated disclosure

Maintainers acknowledge reports, assess severity, develop and verify a fix,
and coordinate disclosure timing with the reporter. Do not publish unresolved
vulnerability details before coordinated disclosure or explicit approval.
