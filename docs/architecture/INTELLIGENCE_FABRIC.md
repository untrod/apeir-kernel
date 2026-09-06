# Declarative intelligence contracts

APEIR Kernel includes provider-neutral declarations for systems composed of
models, algorithms, datasets, tools, and human or device steps. These contracts
support validation and planning; they do not create a second execution engine.

```text
Project / ModelProfile / IntelligenceGraph / Dataset / Experiment
                              |
                              v
          Open Runtime capability and resource contracts
                              |
                              v
             NKI admission and kernel execution path
```

## Ownership

Declarative types describe identity, schemas, capabilities, resources,
artifacts, provenance, and backend requirements. The kernel remains the only
authority for workload state, scheduling, leases, effects, receipts, and
recovery. A validated graph is data; it must be lowered to workload contracts
and submitted through NKI before it can execute.

Frameworks such as PyTorch, ONNX Runtime, TensorRT, scientific libraries,
robotics middleware, and vendor SDKs are provider implementations outside the
trusted core.

## Contracts

- `ModelProfile` supplies type-specific LLM, vision, mathematical, control, or
  custom metadata while reusing common ModelSpec identity and resource fields.
- `IntelligenceGraph` is an acyclic, portable graph. Validation checks unique
  nodes and edges, endpoints, compatible schema types, safe backend references,
  and cycles.
- `ProjectManifest` describes a portable project. `nous.yaml` is the v1
  PackManifest entry point; `project.yaml` contains project metadata.
- `DatasetManifest` records versioned artifact splits, schema, license,
  provenance, transforms, and quality metadata.
- `ExperimentManifest` records reproducibility inputs, resource requirements,
  metrics, artifacts, and lifecycle state.
- `MergeRequest` is a compatibility and license preflight. It never edits model
  weights or grants redistribution rights.

Backend identifiers such as `remote-api`, `python`, `native`, `onnx`,
`pytorch`, `container`, `wasm`, `device`, `human`, and `custom` select required
provider capabilities. They do not link those frameworks into the kernel.

## Implemented scope

The repository validates model profiles, graphs, projects, datasets,
experiments, diffs, merge prerequisites, and graph-to-workflow declarations.
Training, tensor merging, graph execution, and visual authoring require
external providers or distribution components.
