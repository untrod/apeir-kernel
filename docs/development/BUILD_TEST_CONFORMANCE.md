# Build, test, and conformance

Use the repository scripts as the stable developer entry points:

| Script | Purpose |
| --- | --- |
| `bootstrap` | Verify the required toolchain |
| `build` | Build the workspace with the locked dependency graph |
| `test` | Run Rust, process E2E, Python, C SDK, and Micro tests |
| `audit` | Check formatting, deny Clippy warnings, and validate diffs |
| `conformance` | Run standardized fault windows and bounded soak |
| `package` | Produce portable binaries and SHA-256 metadata |

PowerShell and POSIX shell wrappers execute the same underlying checks. The
PowerShell helper discovers a complete MSVC toolset because a partially
installed newer toolset must not shadow a valid compiler.

The default conformance run is intentionally short. Use `-RandomFaults 100` for
the deterministic-seed randomized crash campaign. Qualification runs must state
their explicit fault count and soak duration; a short run must not be reported
as a multi-hour soak.

Live provider validation uses `tests/e2e/real_model.py`. Pass only an
environment-variable name through `--credential-env`; never pass a credential
value on the command line. Stability qualification uses
`tests/e2e/soak.py --seconds <duration> --output .local/validation/<name>.json`.
