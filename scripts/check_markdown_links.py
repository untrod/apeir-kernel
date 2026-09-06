"""Fail when a Markdown file links to a missing local target.

Local links are resolved relative to the file that contains them. External
links (http/https/mailto/ftp) and in-document anchors (``#...``) are ignored.
The check walks every ``*.md`` file under the repository root and skips
generated or private directories that are not part of the source candidate.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
EXCLUDED_DIRS = {".git", ".audit", ".local", "target", ".ruff_cache", ".cache", "__pycache__"}

INLINE_LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def markdown_files(root: Path) -> list[Path]:
    files = []
    for current, directories, names in os.walk(root):
        directories[:] = sorted(name for name in directories if name not in EXCLUDED_DIRS)
        files.extend(Path(current) / name for name in names if name.endswith(".md"))
    return sorted(files)


def _strip_title(target: str) -> str:
    target = target.strip()
    if target.startswith("<") and ">" in target:
        target = target[1:target.index(">")]
    else:
        target = re.sub(r"\s+['\"].*['\"]\s*$", "", target).strip()
    return target


def local_targets(path: Path) -> list[str]:
    """Return the resolved local file targets a Markdown file links to."""
    results: list[str] = []
    text = path.read_text(encoding="utf-8", errors="replace")
    # Examples in fenced blocks are not rendered document links.
    text = re.sub(r"(?ms)^\s*(`{3,}|~{3,})[^\n]*\n.*?^\s*\1\s*$", "", text)
    for match in INLINE_LINK.finditer(text):
        target = _strip_title(match.group(1))
        if not target:
            continue
        parsed = urlsplit(target)
        if target.startswith("#") or parsed.scheme or parsed.netloc:
            continue
        path_part = unquote(parsed.path)
        if not path_part:
            continue
        # Preserve the spelling: Windows resolve() can correct case and hide
        # links that will fail on a case-sensitive Linux checkout.
        results.append(Path(os.path.abspath(path.parent / path_part)).as_posix())
    return results


def broken_links(root: Path) -> list[dict[str, str]]:
    root = root.resolve()
    findings: list[dict[str, str]] = []
    for path in markdown_files(root):
        for target in local_targets(path):
            resolved = Path(target)
            reason = ""
            try:
                resolved.resolve().relative_to(root)
                parts = resolved.relative_to(root).parts
            except ValueError:
                reason = "outside_repository"
                parts = ()
            if not reason and not resolved.exists():
                reason = "missing_target"
            if not reason:
                parent = root
                for part in parts:
                    if part not in {item.name for item in parent.iterdir()}:
                        reason = "path_case_mismatch"
                        break
                    parent /= part
            if reason:
                findings.append(
                    {
                        "path": path.relative_to(root).as_posix(),
                        "target": target,
                        "reason": reason,
                    }
                )
    return findings


def main() -> int:
    root = ROOT
    files = markdown_files(root)
    findings = broken_links(root)
    report = {
        "schema_version": 1,
        "status": "pass" if not findings else "fail",
        "files_checked": len(files),
        "broken_links": findings,
    }
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
