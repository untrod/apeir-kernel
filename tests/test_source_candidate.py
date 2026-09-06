"""Offline tests for exact, private-file-safe candidate exports."""

import hashlib
import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "candidate", Path(__file__).resolve().parents[1] / "scripts/prepare_source_candidate.py"
)
candidate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(candidate)


class SourceCandidateTests(unittest.TestCase):
    def test_exact_snapshot_and_deleted_file_exclusion(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repo"
            root.mkdir()
            (root / "README.md").write_bytes(b"# Candidate\r\n")
            (root / "Cargo.lock").write_bytes(b"version = 4\n")
            (root / "Cargo.toml").write_bytes(b"[workspace]\nmembers = []\n")

            def fake_git(_root, *args):
                if args[0] == "ls-files":
                    return b"README.md\0Cargo.lock\0Cargo.toml\0deleted.md\0"
                return b"example-revision\n" if args[0] == "rev-parse" else b" M README.md\n"

            output = Path(temporary) / "candidate"
            with patch.object(candidate, "git", side_effect=fake_git), patch.object(
                candidate, "REQUIRED_INPUTS", {"README.md", "Cargo.lock", "Cargo.toml"}
            ):
                result = candidate.prepare(root, output)
                with self.assertRaises(FileExistsError):
                    candidate.prepare(root, output)
            self.assertTrue(result["source_dirty"])
            self.assertEqual(result["file_count"], 3)
            manifest = json.loads((output / "release-file-manifest.json").read_text())
            with zipfile.ZipFile(result["archive"]) as bundle:
                self.assertEqual(bundle.namelist(), ["Cargo.lock", "Cargo.toml", "README.md"])
                for record in manifest["files"]:
                    payload = bundle.read(record["path"])
                    self.assertEqual(payload, (root / record["path"]).read_bytes())
                    self.assertEqual(hashlib.sha256(payload).hexdigest().upper(), record["sha256"])

    def test_private_and_binary_files_fail_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in ("target/private.txt", "app.exe", ".local/session.json"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"not exportable")
                with patch.object(candidate, "git", return_value=name.encode() + b"\0"):
                    with self.assertRaises(ValueError):
                        candidate.source_files(root)

    def test_output_inside_repository_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                candidate.prepare(root, root / "output")

    def test_missing_workspace_member_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/missing"]\n', encoding="utf-8"
            )
            with self.assertRaisesRegex(RuntimeError, "crates/missing/Cargo.toml"):
                candidate.validate_required_inputs(root, [root / "Cargo.toml"])


if __name__ == "__main__":
    unittest.main()
