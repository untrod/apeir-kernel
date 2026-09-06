# ModelSpec v1

ModelSpec describes a model, algorithm, optimization routine, controller, or
simulation without embedding it in the kernel. Required identity fields are
schema version, ID, name, kind, version, at least one namespaced capability,
and a backend kind.

The remaining sections describe input and output schemas, resources,
deployment, evaluation, artifacts, provenance, compatibility, and security.
Unknown fields are rejected. ModelSpec is a declaration; registration does not
download, trust, or execute an artifact.

```text
apeir-kernelctl model validate model.yaml
apeir-kernelctl model register model.yaml
apeir-kernelctl model list
apeir-kernelctl model show <id>
apeir-kernelctl model unregister <id>
```
