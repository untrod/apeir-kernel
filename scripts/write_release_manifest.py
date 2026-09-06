"""Write the portable APEIR Kernel release manifest.

The manifest is the hand-off boundary between the independent Kernel project
and products that consume its binaries. It deliberately contains no secrets
and does not require a consumer to have the Kernel source checkout.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NKI_SOURCE = ROOT / "crates" / "nous-nki" / "src" / "lib.rs"


def _command(*arguments: str) -> str:
    return subprocess.run(
        arguments,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout


def _workspace_version() -> str:
    metadata = json.loads(
        _command("cargo", "metadata", "--format-version", "1", "--no-deps", "--locked")
    )
    versions = {
        package["version"]
        for package in metadata["packages"]
        if package["name"] in {"nousd", "nous-provider-worker", "nous-cli"}
    }
    if len(versions) != 1:
        raise RuntimeError("Kernel release binaries must share one workspace version")
    return versions.pop()


def _nki_versions() -> tuple[int, int]:
    source = NKI_SOURCE.read_text(encoding="utf-8")

    def value(name: str) -> int:
        match = re.search(rf"pub const {name}: u32 = (\d+);", source)
        if match is None:
            raise RuntimeError(f"Could not read {name} from {NKI_SOURCE}")
        return int(match.group(1))

    return value("NKI_VERSION"), value("MIN_NKI_VERSION")


def _source_identity() -> tuple[str, bool]:
    if not (ROOT / ".git").exists():
        return "SOURCE_ARCHIVE", False
    revision = _command("git", "rev-parse", "HEAD").strip()
    dirty = bool(_command("git", "status", "--porcelain").strip())
    return revision, dirty


def artifact_record(role: str, path: Path) -> dict[str, object]:
    resolved = path.resolve(strict=True)
    digest = hashlib.sha256()
    with resolved.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "role": role,
        "name": resolved.name,
        "sha256": digest.hexdigest().upper(),
        "size_bytes": resolved.stat().st_size,
    }


def build_manifest(
    *,
    target: str,
    version: str,
    nki_current: int,
    nki_minimum: int,
    revision: str,
    dirty: bool,
    artifacts: list[dict[str, object]],
    generated_at: str | None = None,
) -> dict[str, object]:
    return {
        "schema_version": 1,
        "product": "nous-kernel",
        "version": version,
        "target": target,
        "profile": "release",
        "generated_at": generated_at
        or dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "source": {"revision": revision, "dirty": dirty},
        "protocols": {
            "nki": {"minimum": nki_minimum, "current": nki_current},
            "provider_contract": {"major": 1},
        },
        "artifacts": sorted(artifacts, key=lambda item: (str(item["role"]), str(item["name"]))),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument(
        "--artifact",
        action="append",
        default=[],
        metavar="ROLE=PATH",
        help="Release artifact and its stable role; may be repeated",
    )
    arguments = parser.parse_args()
    if not arguments.artifact:
        parser.error("at least one --artifact is required")

    records = []
    roles = set()
    for value in arguments.artifact:
        role, separator, raw_path = value.partition("=")
        if not separator or not role or not raw_path:
            parser.error("--artifact must use ROLE=PATH")
        if role in roles:
            parser.error(f"duplicate artifact role: {role}")
        roles.add(role)
        records.append(artifact_record(role, Path(raw_path)))

    required_roles = {"daemon", "provider-worker", "cli"}
    if roles != required_roles:
        parser.error(
            "release artifacts must contain exactly these roles: "
            + ", ".join(sorted(required_roles))
        )

    version = _workspace_version()
    nki_current, nki_minimum = _nki_versions()
    revision, dirty = _source_identity()
    manifest = build_manifest(
        target=arguments.target,
        version=version,
        nki_current=nki_current,
        nki_minimum=nki_minimum,
        revision=revision,
        dirty=dirty,
        artifacts=records,
    )
    output = arguments.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    temporary.replace(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
