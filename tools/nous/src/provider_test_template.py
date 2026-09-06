import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1] / "src"))

from provider import respond  # noqa: E402


def test_health() -> None:
    assert respond({"type": "health"})["ok"] is True


def test_execute() -> None:
    result = respond({"type": "execute", "input": "hello"})
    assert result["ok"] is True
    assert result["output"] == "hello"
