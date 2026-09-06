# APEIR Kernel binary bundle

The platform bundle runs the kernel without a source checkout, Python runtime,
desktop application, or Node.js installation. It contains the daemon, the
isolated reference provider worker, the kernel CLI, release metadata, and
license notices.

## Windows 10 x64

Open PowerShell in the extracted directory and create a local data directory:

```powershell
New-Item -ItemType Directory -Force .\data | Out-Null
$env:NOUS_NKI_TOKEN = '<random value of at least 32 characters>'
.\apeird.exe serve .\data\kernel.db .\nous-provider-worker.exe
```

Open a second PowerShell window in the same directory and set the identical
token:

```powershell
$env:NOUS_NKI_TOKEN = '<the same random value>'
.\apeir-kernelctl.exe doctor
.\apeir-kernelctl.exe run "hello" --backend reference
.\apeir-kernelctl.exe inspect
```

The reference backend is deterministic and uses no model account or network
service. Stop the daemon with Ctrl+C. The journal remains under `data` and is
used to recover committed state at the next start.

`nousd.exe` and `nous.exe` are compatibility commands in 0.1 bundles. New
automation should use `apeird.exe` and `apeir-kernelctl.exe`.

## Security boundary

NKI accepts loopback connections only. The session token authenticates a local
application-managed session; it is not a remote or multi-tenant identity
system. Protect the data directory with operating-system permissions. Do not
store provider credentials in command arguments, manifests, receipts, or the
journal.

Provider process separation supplies crash and termination containment. The
bundle is not an operating-system sandbox and does not replace filesystem,
network, account, or endpoint security controls.

## Artifact verification

1. Obtain the archive checksum through the release channel and verify the
   adjacent `.sha256` file before extraction.
2. Validate `nous-kernel-release.json` against
   `kernel-release-manifest-v1.schema.json`.
3. Confirm that the declared target matches the host and that the NKI version
   ranges overlap.
4. Recompute each artifact size and SHA-256 digest listed in the manifest.
5. Verify the release signature and provenance when supplied by the release
   channel.

Do not run an archive whose target, manifest, digest, signature policy, or
origin cannot be established.
