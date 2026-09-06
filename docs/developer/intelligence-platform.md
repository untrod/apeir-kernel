# Intelligence engineering quick start

Build the CLI, then validate the reference project:

```powershell
cargo build -p nous-cli
target/debug/apeir-kernelctl project validate examples/intelligence-platform
target/debug/apeir-kernelctl graph validate examples/intelligence-platform/workflows/math.graph.yaml
target/debug/apeir-kernelctl dataset validate examples/intelligence-platform/datasets/reference.dataset.yaml
target/debug/apeir-kernelctl experiment validate examples/intelligence-platform/experiments/baseline.experiment.yaml
```

Create a project:

```powershell
target/debug/apeir-kernelctl new project my-system
target/debug/apeir-kernelctl project validate my-system
```

The scaffold writes a compatibility `nous.yaml` PackManifest and a richer
`project.yaml`. Project secrets contain environment-variable names only. Never
place an API key in either file.

Analyze an existing local project without changing it:

```powershell
target/debug/apeir-kernelctl project analyze path/to/existing-project
target/debug/apeir-kernelctl project import path/to/existing-project --dry-run
```

The bounded scan skips generated directories and symlinks, never opens model
weight files, and reports languages, dependency manifests, entrypoints, model
assets, containers, tests, licenses, frameworks, and adapter-first migration
recommendations. Repository cloning and adapter generation remain explicit,
reviewable steps.

## Graphs

Graph nodes can represent models, algorithms, preprocessors, postprocessors,
routers, adapters, verifiers, solvers, tools, devices, or human steps. A graph
must be acyclic and its connected JSON Schema types must agree.

```powershell
target/debug/apeir-kernelctl graph diff graph-a.yaml graph-b.yaml
target/debug/apeir-kernelctl graph export-workflow graph-a.yaml
```

Validation does not execute a graph. `export-workflow` lowers topology into the
existing Runtime Foundation `WorkflowContract`. Production execution must then
compile and submit it through the existing Task Graph and NKI; tools must not
add a parallel execution path.

## Models and merge preflight

Registered ModelSpec commands remain compatible. `inspect`, `remove`, and
`export` are developer-friendly aliases or additions.

```powershell
target/debug/apeir-kernelctl model inspect reference-linear-v1
target/debug/apeir-kernelctl model validate-profile profile-a.yaml
target/debug/apeir-kernelctl model inspect-profile profile-a.yaml
target/debug/apeir-kernelctl model diff profile-a.yaml profile-b.yaml
target/debug/apeir-kernelctl model merge-check merge-request.yaml
```

`merge-check` validates metadata and declared license rights. It never edits
weights. Unknown derivative-work or redistribution permission blocks the
operation instead of guessing.

## Experiments

Experiments capture reproducibility inputs and produce stable diffs:

```powershell
target/debug/apeir-kernelctl experiment diff baseline.yaml candidate.yaml
```

Generated datasets, checkpoints, weights, logs, and evaluation reports should
be stored as artifacts. Large model files remain excluded from Git.
