# Developer quick start

Build and verify the repository before changing contracts or execution code:

```powershell
./scripts/bootstrap.ps1
./scripts/build.ps1
./scripts/test.ps1
target/debug/apeir-kernelctl doctor
```

Run a deterministic workload without credentials or network access:

```powershell
target/debug/apeir-kernelctl run "hello" --backend reference
target/debug/apeir-kernelctl inspect
```

Create and validate extension declarations:

```powershell
target/debug/apeir-kernelctl new project sample-project
target/debug/apeir-kernelctl new provider sample-provider
target/debug/apeir-kernelctl provider validate sample-provider/provider.yaml
target/debug/apeir-kernelctl project validate sample-project
```

Use environment-variable names for credential references. Do not place secret
values in a manifest, command argument, test fixture, log, or journal record.

Continue with the [declarative contract guide](intelligence-platform.md),
[provider SDK](../providers/provider-sdk.md), and
[build and conformance guide](../development/BUILD_TEST_CONFORMANCE.md).
