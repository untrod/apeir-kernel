"""Provider SDK v1 conformance fixture."""

from __future__ import annotations

import json
import sys
import time


def main() -> None:
    request = json.load(sys.stdin)
    operation = request.get("type")
    if operation == "execute":
        value = str(request.get("input", ""))
        if value == "__sleep__":
            time.sleep(10)
        if value == "__malformed__":
            sys.stdout.write("not-json")
            return
        response = {"ok": True, "output": value}
    elif operation in {"probe", "metadata", "capabilities", "health"}:
        response = {"ok": True, "capabilities": ["example.echo"]}
    elif operation in {"cancel", "shutdown"}:
        response = {"ok": True}
    else:
        response = {
            "ok": False,
            "error": {"code": "UNSUPPORTED_OPERATION", "message": "unsupported operation"},
        }
    json.dump(response, sys.stdout, separators=(",", ":"))


if __name__ == "__main__":
    main()
