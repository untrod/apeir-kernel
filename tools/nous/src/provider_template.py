"""Minimal Provider SDK v1 process adapter."""

from __future__ import annotations

import hashlib
import json
import sys


def respond(request: dict[str, object]) -> dict[str, object]:
    operation = request.get("type")
    if operation in {"probe", "metadata", "capabilities", "health"}:
        return {
            "ok": True,
            "provider": "example",
            "capabilities": ["example.echo"],
        }
    if operation == "execute":
        value = str(request.get("input", ""))
        return {
            "ok": True,
            "output": value,
            "sha256": hashlib.sha256(value.encode()).hexdigest(),
        }
    if operation in {"cancel", "shutdown"}:
        return {"ok": True}
    return {"ok": False, "error": {"code": "UNSUPPORTED_OPERATION"}}


def main() -> None:
    request = json.load(sys.stdin)
    json.dump(respond(request), sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main()
