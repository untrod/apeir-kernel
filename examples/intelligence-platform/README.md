# Intelligence Platform reference project

This project demonstrates the developer-facing schemas without introducing a
second execution engine. The graph references the existing deterministic
mathematics backend and can be inspected with the APEIR CLI.

```powershell
apeir-kernelctl project validate examples/intelligence-platform
apeir-kernelctl graph validate examples/intelligence-platform/workflows/math.graph.yaml
apeir-kernelctl dataset validate examples/intelligence-platform/datasets/reference.dataset.yaml
apeir-kernelctl experiment validate examples/intelligence-platform/experiments/baseline.experiment.yaml
apeir-kernelctl model validate-profile examples/intelligence-platform/models/linear.profile.yaml
```

`nous.yaml` remains as a PackManifest-compatible entry point for existing
tooling. `project.yaml` carries the richer Project Format v1 metadata.
