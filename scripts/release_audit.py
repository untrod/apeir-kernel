"""Generate reproducible security, dependency, and SBOM release evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SECRET_PATTERNS = {
    "openai_style_key": re.compile(r"\bsk-[A-Za-z0-9_-]{20,}"),
    "aws_access_key": re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    "private_key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "assigned_secret": re.compile(
        r"(?i)\b(?:api[_-]?key|token|password|secret)\s*[:=]\s*['\"][^'$%{][^'\"]{11,}['\"]"
    ),
    "personal_windows_path": re.compile(r"(?i)\b[A-Z]:\\Users\\[^\\\s]+"),
}
TEXT_SUFFIXES = {
    ".c",
    ".h",
    ".json",
    ".md",
    ".py",
    ".rs",
    ".toml",
    ".txt",
    ".yaml",
    ".yml",
}


def command(*arguments: str) -> str:
    return subprocess.run(
        arguments,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout


def candidate_files() -> list[Path]:
    if (ROOT / ".git").exists():
        output = command(
            "git",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
        )
        return [ROOT / line for line in output.splitlines() if line]
    excluded = {".git", ".local", "target", "__pycache__"}
    return [
        path
        for path in ROOT.rglob("*")
        if path.is_file() and not any(part in excluded for part in path.relative_to(ROOT).parts)
    ]


def scan_sources() -> list[dict[str, object]]:
    findings: list[dict[str, object]] = []
    for path in candidate_files():
        if path.suffix.lower() not in TEXT_SUFFIXES or not path.is_file():
            continue
        content = path.read_text(encoding="utf-8", errors="replace")
        for line_number, line in enumerate(content.splitlines(), 1):
            for category, pattern in SECRET_PATTERNS.items():
                if pattern.search(line):
                    findings.append(
                        {
                            "category": category,
                            "path": path.relative_to(ROOT).as_posix(),
                            "line": line_number,
                        }
                    )
    return findings


def dependency_evidence() -> tuple[list[dict[str, object]], dict[str, object]]:
    metadata = json.loads(command("cargo", "metadata", "--format-version", "1", "--locked"))
    components = []
    licenses = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package.get("source") is None:
            continue
        license_name = package.get("license") or "UNKNOWN"
        record = {
            "name": package["name"],
            "version": package["version"],
            "license": license_name,
            "source": package.get("source", ""),
        }
        licenses.append(record)
        components.append(
            {
                "type": "library",
                "name": package["name"],
                "version": package["version"],
                "purl": f"pkg:cargo/{package['name']}@{package['version']}",
                "licenses": [{"expression": license_name}],
            }
        )
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "timestamp": dt.datetime.now(dt.timezone.utc).isoformat(),
            "component": {"type": "application", "name": "nous-kernel", "version": "0.1.0"},
        },
        "components": components,
    }
    return licenses, sbom


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=ROOT / ".audit" / "release")
    arguments = parser.parse_args()
    output = arguments.output if arguments.output.is_absolute() else ROOT / arguments.output
    output.mkdir(parents=True, exist_ok=True)

    findings = scan_sources()
    licenses, sbom = dependency_evidence()
    unknown = [item for item in licenses if item["license"] == "UNKNOWN"]
    revision = (
        command("git", "rev-parse", "HEAD").strip()
        if (ROOT / ".git").exists()
        else "SOURCE_ARCHIVE"
    )
    security = {
        "revision": revision,
        "files_scanned": len(candidate_files()),
        "finding_count": len(findings),
        "findings": findings,
    }
    (output / "security-scan.json").write_text(
        json.dumps(security, indent=2) + "\n", encoding="utf-8"
    )
    (output / "dependency-licenses.json").write_text(
        json.dumps({"dependencies": licenses, "unknown": unknown}, indent=2) + "\n",
        encoding="utf-8",
    )
    (output / "sbom.cdx.json").write_text(
        json.dumps(sbom, indent=2) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "files_scanned": len(candidate_files()),
                "security_findings": len(findings),
                "dependencies": len(licenses),
                "unknown_licenses": len(unknown),
            }
        )
    )
    return 1 if findings or unknown else 0


if __name__ == "__main__":
    raise SystemExit(main())
