from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from write_release_manifest import artifact_record, build_manifest  # noqa: E402


class ReleaseManifestTests(unittest.TestCase):
    def test_manifest_is_complete_deterministic_and_secret_free(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = []
            for role, name in (
                ("daemon", "nousd.exe"),
                ("provider-worker", "nous-provider-worker.exe"),
                ("cli", "nous.exe"),
            ):
                path = root / name
                path.write_bytes(role.encode("utf-8"))
                artifacts.append(artifact_record(role, path))

            manifest = build_manifest(
                target="x86_64-pc-windows-msvc",
                version="0.1.0",
                nki_current=2,
                nki_minimum=1,
                revision="a" * 40,
                dirty=False,
                artifacts=list(reversed(artifacts)),
                generated_at="2026-09-04T00:00:00Z",
            )

        self.assertEqual(manifest["product"], "nous-kernel")
        self.assertEqual(manifest["protocols"]["nki"], {"minimum": 1, "current": 2})
        self.assertEqual(
            [item["role"] for item in manifest["artifacts"]],
            ["cli", "daemon", "provider-worker"],
        )
        self.assertNotIn("credential", str(manifest).lower())
        self.assertTrue(all(len(item["sha256"]) == 64 for item in manifest["artifacts"]))


if __name__ == "__main__":
    unittest.main()
