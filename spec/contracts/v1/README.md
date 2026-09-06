# APEIR Kernel foundation contracts v1

This directory is the compatibility boundary for APEIR Kernel. Contract IDs
retain their `Nous` expansion and wire identity throughout v1.
The ten contracts below are frozen at version 1. Additive fields must remain
optional. Removing a field, changing its meaning, or weakening a safety rule
requires a new major contract version.

| ID | Contract | Authority | Existing representation |
|---|---|---|---|
| NEC | Nous Execution Contract | `KernelRuntime` | operation request, workload identity, delivery semantics, and lifecycle |
| NSO | Nous State Ownership | `nous-state` journal | immutable entries, integrity, replay, generation, and fencing |
| NTE | Nous Transactional Effects | `DurableExecutor` | intent, provider receipt, commit, and safe recovery |
| NRP | Nous Resource Protocol | resource and scheduler cores | admission, placement, fenced lease, and release |
| NPA | Nous Provider ABI | provider runtime | serialized command, isolated process, and normalized receipt |
| NKI | Nous Kernel Interface | NKI server | `nous-nki` envelope, methods, and framing |
| NMP | Nous Micro Protocol | Nous Micro | fixed-size `nous_nmp_frame_v1` |
| NCT | Nous Credential Contract | credential broker | reference and scoped lease only |
| NLC | Nous Learning Contract | learning governance | policy lifecycle; not production-connected |
| NSE | Nous Safety Envelope | safety gate | immutable hard constraints and safe fallback |

## Required order

Every production execution follows this order:

`NKI -> NEC validation -> NSE -> NRP admission and scheduling -> NSO intent -> execution -> NTE -> NSO commit -> result`

Learning may propose scheduler or routing policy changes through NLC. It cannot
modify NKI, state ownership, credentials, approval rules, or the safety envelope.
An NSE rejection always overrides an NLC approval.

## Runtime profiles

- Core targets all contracts and durable operation. Its current maturity is
  recorded per capability rather than implied by this profile.
- Edge must retain the same wire contracts with local scheduling and durable
  state; no complete Edge profile is currently validated.
- Micro targets NEC, NTE, NMP, NCT, and NSE with fixed memory and no dynamic
  allocation; only the portable host implementation is currently validated.

These are profile requirements, not certification that every target or backend
implements the complete profile. Platform and capability evidence is recorded
in the [support matrix](../../../docs/reference/support-matrix.md).

The Rust registry in `nous-types::FOUNDATION_CONTRACTS` and the C header
`sdk/c/include/nous_kernel.h` are machine-readable mirrors of this document.
