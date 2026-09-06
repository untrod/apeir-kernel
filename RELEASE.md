# Release process

Every release is built from a reviewed commit with locked dependencies. Source
and binary artifacts are separate deliverables and have independent evidence.

## Source release

1. Confirm the version, compatibility policy, support matrix, changelog, and
   third-party notices.
2. Run formatting, static analysis, the full workspace test suite, process E2E,
   SDK tests, architecture checks, and documentation checks.
3. Scan the working tree and Git history for credentials, private paths, and
   unexpectedly large files. Review dependency license metadata and the SBOM.
4. Generate the source candidate outside the repository:

   ```text
   python scripts/prepare_source_candidate.py --output <new-output-directory>
   ```

5. Review `release-file-manifest.json`, extract the archive into a new
   directory, and repeat the contract and build checks against the extracted
   source.
6. Create the release tag only after the candidate commit and hosted CI results
   have been approved.

The source exporter includes tracked and unignored working-tree files. It
rejects symlinks, reparse points, generated binaries, private directories, and
an output path inside the repository. Each ZIP member is compared with a fresh
read of the source file. A dirty candidate is identified by its archive digest,
not by the HEAD revision alone.

## Binary release

`scripts/package.ps1` and `scripts/package.sh` build a platform bundle under
`target/distribution` and create an archive under `target/packages`. The bundle
contains the daemon, provider worker, kernel CLI, manifest schema, checksums,
license notices, and quick-start documentation.

Before publishing a binary archive:

1. build from the approved source tag on the declared target;
2. run native process, SDK, and conformance tests against those binaries;
3. validate `nous-kernel-release.json` against its schema;
4. verify every file size and SHA-256 digest in the manifest;
5. generate an SBOM and authenticated build provenance;
6. apply the platform signing policy and verify the resulting signatures;
7. record compiler, target, source revision, and known reproducibility limits.

A checksum detects unintended byte changes only when the expected checksum is
obtained through a trusted channel. It does not authenticate the publisher.

## Release criteria

A release is blocked by a failing required test, an unexplained source file, an
unknown license, an unresolved credential finding, a support claim without
matching evidence, or an unreviewed protocol/security change. Native Micro host
tests do not constitute MCU hardware certification, and CI configuration does
not constitute a completed platform run.

Tag creation, public uploads, license changes, contribution attribution
changes, and history rewrites require explicit maintainer approval.
