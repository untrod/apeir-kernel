# Changelog

All notable APEIR Kernel changes are recorded here. Historical `Nous` names are
retained where they identify released protocols, commands, or artifacts.

## Unreleased

### Added

- Canonical `apeird` and `apeir-kernelctl` commands with legacy aliases.
- Machine-readable APEIR brand-transition and Kernel distribution contracts.
- Independent Kernel/Distribution ownership boundaries.
- CI coverage for Windows and Linux Rust tests, formatting, Clippy, contracts,
  SDK tests, security scanning, dependency licenses, and SBOM generation.

### Compatibility

- Existing `nous.*.v1` protocols, persistent identifiers, receipts, and signed
  records remain unchanged.
- A future `apeir.*.v2` protocol requires explicit negotiation and migration.
