#!/usr/bin/env python3
"""Fail when a workspace crate crosses a frozen architecture boundary."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEPENDENCY = re.compile(r"^(nous-[a-z0-9-]+)\s*(?:=|\.)")

ALLOWED: dict[str, set[str]] = {
    "nous-types": set(),
    "nous-state": {"nous-types"},
    "nous-resource": {"nous-types", "nous-state"},
    "nous-security": {"nous-types"},
    "nous-nki": {"nous-types"},
    "nous-scheduler": {"nous-types"},
    "nous-execution-proof": {"nous-types"},
    "nous-intelligence": {"nous-types"},
    "nous-control-plane": {"nous-types", "nous-intelligence"},
    "nous-kernel-core": {
        "nous-types",
        "nous-state",
        "nous-resource",
        "nous-scheduler",
    },
    "nous-runtime-client": {"nous-types", "nous-nki"},
    "nous-cli": {
        "nous-types",
        "nous-nki",
        "nous-runtime-client",
        "nous-intelligence",
    },
}


def package_and_dependencies(path: Path) -> tuple[str, set[str]]:
    package = ""
    dependencies: set[str] = set()
    section = ""
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if section == "package" and line.startswith("name"):
            package = line.split("=", 1)[1].strip().strip('"')
        if section == "dependencies":
            match = DEPENDENCY.match(line)
            if match:
                dependencies.add(match.group(1))
    return package, dependencies


def main() -> int:
    manifests = sorted(ROOT.glob("crates/*/Cargo.toml")) + [
        ROOT / "tools" / "nous" / "Cargo.toml"
    ]
    violations: list[dict[str, str]] = []
    checked: dict[str, list[str]] = {}
    for manifest in manifests:
        package, dependencies = package_and_dependencies(manifest)
        if package not in ALLOWED:
            continue
        checked[package] = sorted(dependencies)
        for dependency in sorted(dependencies - ALLOWED[package]):
            violations.append(
                {
                    "package": package,
                    "dependency": dependency,
                    "manifest": str(manifest.relative_to(ROOT)),
                }
            )
    report = {
        "schema_version": 1,
        "status": "pass" if not violations else "fail",
        "checked": checked,
        "violations": violations,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if violations else 0


if __name__ == "__main__":
    raise SystemExit(main())
