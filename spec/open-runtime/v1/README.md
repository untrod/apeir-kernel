# Open Runtime schemas v1

Rust declarations in `nous-types::open_runtime` are the executable schema
authority for CapabilityContract, runtime object contracts, ModelSpec,
ProviderManifest, and PackManifest. Documents may be YAML or JSON.

`ExtensionAdmissionRequest` is the narrow Kernel projection of a distribution
extension. It carries only immutable identity, content digest, compatibility
level, requested capabilities, and governed executor class. Skill instructions,
templates, MCP configuration, and other vendor payloads remain outside the
trusted Kernel. Validation is not authorization.

Extension execution uses three additional public contracts:

- `ExtensionExecutionRequest` binds one operation, input digest, capability,
  executor and scope to an existing authorization receipt;
- `ExtensionExecutionPermit` binds the Kernel actor and policy decision to that
  exact request;
- `ExtensionRevocationRequest` invalidates the latest matching authorization.

All three reject unknown fields. A Runtime executor must not invoke an adapter
without a valid permit, and a revocation becomes the latest authority state in
the Kernel journal.

Version 1 rejects unknown fields. Additive changes require optional fields and
compatible defaults. Breaking changes require a new major schema version, an
RFC, compatibility notes, and migration guidance.
