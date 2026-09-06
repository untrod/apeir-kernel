"""Export a reviewed working-tree source candidate, never a release approval.

The explicit output directory must be new and outside the repository. Git's
tracked and unignored files define the input set, including current changes.
No commit, tag, staging operation or remote operation is performed.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import stat
import subprocess
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN_PARTS = {".git", ".audit", ".local", ".nous", ".apeir", ".venv",
                   "target", "node_modules", "__pycache__", ".ruff_cache"}
FORBIDDEN_SUFFIXES = {".exe", ".dll", ".obj", ".pdb", ".db", ".sqlite", ".log", ".pyc"}
REQUIRED_INPUTS = {
    ".github/workflows/ci.yml",
    "Cargo.lock",
    "Cargo.toml",
    "LICENSE",
    "README.md",
    "README.zh-CN.md",
    "daemon/nousd/Cargo.toml",
    "micro/include/nous_micro.h",
    "providers/worker/Cargo.toml",
    "scripts/test.ps1",
    "scripts/test.sh",
    "sdk/c/include/nous_kernel.h",
    "sdk/python/pyproject.toml",
    "spec/contracts/v1/README.md",
    "spec/open-runtime/v1/README.md",
    "tests/e2e/kernel_e2e.py",
    "tools/nous/Cargo.toml",
}


def git(root: Path, *args: str) -> bytes:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True).stdout


def source_files(root: Path) -> list[Path]:
    raw = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
    files = []
    for name in sorted({value.decode("utf-8") for value in raw.split(b"\0") if value}):
        relative = Path(name)
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("source path escapes repository")
        path = root / relative
        if not path.exists() and not path.is_symlink():
            continue  # Intentional tracked deletion; not an export input.
        if any(part.casefold() in FORBIDDEN_PARTS for part in relative.parts):
            raise ValueError(f"private/generated directory in source set: {name}")
        if path.suffix.lower() in FORBIDDEN_SUFFIXES:
            raise ValueError(f"generated/binary file in source set: {name}")
        for ancestor in (path, *path.parents):
            if ancestor == root:
                break
            info = ancestor.lstat()
            if ancestor.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError(f"linked/reparse source is unsupported: {name}")
        path.resolve().relative_to(root)
        if not path.is_file():
            raise ValueError(f"non-file source input: {name}")
        files.append(path)
    return files


def validate_required_inputs(root: Path, files: list[Path]) -> None:
    names = {path.relative_to(root).as_posix() for path in files}
    required = set(REQUIRED_INPUTS)
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    for member in cargo.get("workspace", {}).get("members", []):
        required.add(f"{member.rstrip('/')}/Cargo.toml")
    missing = sorted(required - names)
    if missing:
        raise RuntimeError("source candidate is incomplete: " + ", ".join(missing))


def prepare(root: Path, output: Path) -> dict:
    root, output = root.resolve(), output.resolve()
    if output == root or root in output.parents:
        raise ValueError("output must be outside repository")
    files = source_files(root)
    if not files:
        raise ValueError("empty source candidate")
    validate_required_inputs(root, files)
    revision = git(root, "rev-parse", "HEAD").decode().strip()
    dirty = bool(git(root, "status", "--porcelain"))
    output.mkdir(parents=True, exist_ok=False)
    archive = output / "apeir-kernel-0.1.0-source-candidate.zip"
    records = []
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as bundle:
        for path in files:
            name = path.relative_to(root).as_posix()
            payload = path.read_bytes()
            digest = hashlib.sha256(payload).hexdigest().upper()
            # Fixed ZIP metadata; source bytes themselves are not normalized.
            entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            mode = 0o755 if path.suffix == ".sh" else 0o644
            entry.external_attr = (stat.S_IFREG | mode) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(entry, payload)
            category = "archive" if name.startswith("docs/archive/") else "source"
            if name == "Cargo.lock":
                category = "dependency-lock"
            records.append({"path": name, "size_bytes": len(payload), "sha256": digest,
                            "category": category})
    if source_files(root) != files:
        raise RuntimeError("source set changed during export")
    with zipfile.ZipFile(archive) as bundle:
        if bundle.namelist() != [item["path"] for item in records]:
            raise RuntimeError("archive file set mismatch")
        for item, path in zip(records, files):
            if hashlib.sha256(path.read_bytes()).hexdigest().upper() != item["sha256"]:
                raise RuntimeError(f"source changed during export: {item['path']}")
            if hashlib.sha256(bundle.read(item["path"])).hexdigest().upper() != item["sha256"]:
                raise RuntimeError(f"archive digest mismatch: {item['path']}")
    manifest = {
        "schema_version": 1, "product": "apeir-kernel", "kind": "source-release-candidate",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "source": {"revision": revision, "dirty": dirty},
        "file_count": len(records), "files": records,
    }
    (output / "release-file-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (output / "SHA256SUMS").write_text(
        "".join(f"{item['sha256']}  {item['path']}\n" for item in records), encoding="utf-8")
    archive_hash = hashlib.sha256(archive.read_bytes()).hexdigest().upper()
    (output / (archive.name + ".sha256")).write_text(
        f"{archive_hash}  {archive.name}\n", encoding="ascii")
    return {"file_count": len(records), "source_bytes": sum(r["size_bytes"] for r in records),
            "source_dirty": dirty, "archive": str(archive), "sha256": archive_hash,
            "verified": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(ROOT, args.output), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
