# Provider SDK v1

Create a provider starter with:

```text
apeir-kernelctl new provider <name>
apeir-kernelctl provider validate <path>/provider.yaml
apeir-kernelctl provider install <path>/provider.yaml
```

The manifest declares identity, backend, process entrypoint, capabilities,
lifecycle operations, and credential references. A credential field contains
an environment-variable name only.

Provider processes accept one JSON request on stdin and return one JSON object
on stdout. Operations are `probe`, `metadata`, `capabilities`, `health`,
`execute`, `cancel`, and `shutdown`. Production execution hosts an external
provider as the directly managed isolated child, enforces timeout/cancellation,
and normalizes successful output into a durable receipt.

`provider install` resolves a relative entrypoint from the manifest directory
and writes the existing operational configuration. `provider init` remains the
backward-compatible manual configuration path.

Run dynamic conformance against a configured provider with:

```text
nous conformance provider <configured-name>
```

Conformance does not certify security or model quality. It verifies the runtime
contract and recorded execution behavior.
