# Contributing

Start with an issue for behavior changes. Small fixes may go directly to a pull
request. Architecture-breaking changes require an RFC under `rfcs/` before
implementation.

Use focused commits and preserve compatibility unless an accepted RFC includes
a migration. New behavior needs tests. Run the platform script set before
requesting review:

```powershell
./scripts/build.ps1
./scripts/test.ps1
./scripts/audit.ps1
./scripts/conformance.ps1 -SoakSeconds 30
```

Do not commit secrets, private endpoints, logs, databases, generated binaries,
model weights, or datasets. Generated code must be identified and reproducible.
Comments should explain invariants, protocol behavior, security decisions, or
non-obvious concurrency; avoid conversational narration and decorative blocks.

Contributors certify that they have the right to submit their work under the
project license. Review roles and decisions are described in [GOVERNANCE.md](GOVERNANCE.md).
