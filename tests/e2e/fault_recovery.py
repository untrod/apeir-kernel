"""Crash-window recovery checks for the durable production executor."""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUFFIX = ".exe" if os.name == "nt" else ""
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
PROFILE = os.environ.get("NOUS_TEST_PROFILE", "debug")
NOUSD = TARGET / PROFILE / f"nousd{SUFFIX}"
WORKER = TARGET / PROFILE / f"nous-provider-worker{SUFFIX}"


class FaultRecoveryTests(unittest.TestCase):
    def run_window(self, fault_point: str) -> None:
        with tempfile.TemporaryDirectory(prefix="nous-fault-") as directory:
            journal = Path(directory) / "journal.db"
            input_text = f"recover {fault_point}"
            operation_id = f"op-{hashlib.sha256(input_text.encode()).hexdigest()}"
            command = [str(NOUSD), "run-once", str(journal), str(WORKER), input_text]
            environment = os.environ.copy()
            environment["NOUS_FAULT_POINT"] = fault_point
            environment["NOUS_FAULT_OPERATION"] = operation_id
            crashed = subprocess.run(
                command,
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                timeout=20,
                check=False,
            )
            self.assertNotEqual(crashed.returncode, 0, "fault point did not terminate nousd")

            recovered = subprocess.run(
                command,
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=20,
                check=True,
            )
            payload = json.loads(recovered.stdout)
            self.assertEqual(payload["operation_id"], operation_id)
            self.assertEqual(payload["result"], hashlib.sha256(input_text.encode()).hexdigest())

            replayed = subprocess.run(
                command,
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=20,
                check=True,
            )
            self.assertEqual(
                json.loads(replayed.stdout)["completed_at_us"], payload["completed_at_us"]
            )
            inspected = subprocess.run(
                [str(NOUSD), "inspect", str(journal)],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=20,
                check=True,
            )
            self.assertEqual(json.loads(inspected.stdout)["corrupted_sequences"], [])

            connection = sqlite3.connect(journal)
            try:
                phases = connection.execute(
                    "SELECT new_phase FROM journal_entries WHERE object_type = 'ResourceLease'"
                ).fetchall()
                receipts = connection.execute(
                    "SELECT COUNT(*) FROM journal_entries WHERE object_type = 'OperationReceipt'"
                ).fetchone()[0]
            finally:
                connection.close()
            active = sum(phase == "ACTIVE" for (phase,) in phases)
            terminal = sum(phase in {"RELEASED", "REVOKED", "EXPIRED"} for (phase,) in phases)
            self.assertEqual(active, terminal)
            self.assertEqual(receipts, 1)

    def test_after_intent(self) -> None:
        for _ in range(int(os.environ.get("NOUS_FAULT_ITERATIONS", "1"))):
            self.run_window("effect.after_intent")

    def test_after_provider_before_receipt(self) -> None:
        for _ in range(int(os.environ.get("NOUS_FAULT_ITERATIONS", "1"))):
            self.run_window("effect.after_execute")

    def test_after_receipt(self) -> None:
        for _ in range(int(os.environ.get("NOUS_FAULT_ITERATIONS", "1"))):
            self.run_window("effect.after_receipt")

    def test_after_commit(self) -> None:
        for _ in range(int(os.environ.get("NOUS_FAULT_ITERATIONS", "1"))):
            self.run_window("effect.after_commit")

    def test_after_in_memory_lease_release(self) -> None:
        for _ in range(int(os.environ.get("NOUS_FAULT_ITERATIONS", "1"))):
            self.run_window("resource.after_release")


if __name__ == "__main__":
    if not NOUSD.is_file() or not WORKER.is_file():
        raise SystemExit("build nousd with fault-injection and build the provider worker first")
    unittest.main(verbosity=2)
