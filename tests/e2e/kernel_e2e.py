"""Process-level conformance tests for the production NKI execution path."""

from __future__ import annotations

import base64
import ctypes
import hashlib
import json
import os
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXE_SUFFIX = ".exe" if os.name == "nt" else ""
TARGET_DIR = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
PROFILE = os.environ.get("NOUS_TEST_PROFILE", "debug")
NOUSD = TARGET_DIR / PROFILE / f"nousd{EXE_SUFFIX}"
WORKER = TARGET_DIR / PROFILE / f"nous-provider-worker{EXE_SUFFIX}"


def free_address() -> tuple[str, int]:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()


def operation(
    backend: str,
    *,
    model: str = "",
    input_text: str = "probe",
    provider_entrypoint: str = "",
) -> dict:
    operation_id = f"op-{uuid.uuid4()}"
    execution_domain = {
        "openai-compatible": "remote",
        "ollama": "local",
        "edge-openai-compatible": "edge",
        "external-process": "local",
    }.get(backend, "reference")
    return {
        "operation_id": operation_id,
        "workload_id": f"workload-{operation_id}",
        "step_id": "step-1",
        "backend": backend,
        "execution_domain": execution_domain,
        "model": model,
        "endpoint": "",
        "credential_env": "",
        "provider_entrypoint": provider_entrypoint,
        "input": input_text,
        "delivery": "IDEMPOTENT",
        "snapshot": {
            "model_revision": model or "none",
            "provider_revision": f"{backend}-v1",
            "prompt_revision": "prompt-v1",
            "tool_revision": "none",
            "knowledge_revision": "none",
            "policy_revision": "deterministic-v1",
            "capability_revision": f"{backend}-v1",
            "context_revision": "context-v1",
        },
        "timeout_ms": 15_000,
    }


def capability_manifest(provider: str, backend: str) -> dict:
    return {
        "schema_version": 1,
        "provider": provider,
        "backend": backend,
        "model": "",
        "runtime_class": "reference",
        "chat": "SUPPORTED",
        "streaming": "UNSUPPORTED",
        "tools": "SUPPORTED",
        "structured_output": "SUPPORTED",
        "vision": "UNSUPPORTED",
        "embedding": "UNSUPPORTED",
        "reasoning": "SUPPORTED",
        "local_execution": "SUPPORTED",
        "remote_execution": "UNSUPPORTED",
        "context_length": 32768,
        "privacy_local": True,
        "privacy": "LOCAL_ONLY",
        "estimated_latency_ms": 1,
        "estimated_cost_microcents": 0,
        "probed_at_us": int(time.time() * 1_000_000),
    }


class KernelProcessTests(unittest.TestCase):
    process: subprocess.Popen[str]
    address: tuple[str, int]
    temporary: tempfile.TemporaryDirectory[str]
    journal: Path
    auth_token = "e2e-local-session-token-000000000001"

    @classmethod
    def setUpClass(cls) -> None:
        if not NOUSD.is_file() or not WORKER.is_file():
            raise RuntimeError("build nousd and nous-provider-worker before running E2E tests")
        cls.temporary = tempfile.TemporaryDirectory(prefix="nous-kernel-e2e-")
        cls.journal = Path(cls.temporary.name) / "journal.db"
        cls.address = free_address()
        cls.start_process()

    @classmethod
    def start_process(cls) -> None:
        started_at = time.monotonic()
        flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
        environment = os.environ.copy()
        environment["NOUS_NKI_TOKEN"] = cls.auth_token
        cls.process = subprocess.Popen(
            [
                str(NOUSD),
                "serve",
                str(cls.journal),
                str(WORKER),
                f"{cls.address[0]}:{cls.address[1]}",
            ],
            cwd=ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            creationflags=flags,
            env=environment,
        )
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if cls.process.poll() is not None:
                stderr = cls.process.stderr.read() if cls.process.stderr else ""
                raise RuntimeError(f"nousd exited during startup: {stderr}")
            try:
                with socket.create_connection(cls.address, timeout=0.2):
                    cls.last_startup_seconds = time.monotonic() - started_at
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError("nousd did not become ready")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.stop_process(graceful=True)
        for attempt in range(20):
            try:
                cls.temporary.cleanup()
                return
            except PermissionError:
                if attempt == 19:
                    raise
                time.sleep(0.1)

    @classmethod
    def stop_process(cls, *, graceful: bool) -> None:
        if cls.process.poll() is None:
            if graceful and os.name == "nt":
                cls.process.send_signal(signal.CTRL_BREAK_EVENT)
            elif graceful:
                cls.process.send_signal(signal.SIGINT)
            else:
                cls.process.kill()
            try:
                cls.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                cls.process.kill()
                cls.process.wait(timeout=5)
        if cls.process.stderr is not None:
            cls.process.stderr.close()

    @classmethod
    def restart_process(cls, *, graceful: bool = False) -> None:
        cls.stop_process(graceful=graceful)
        cls.start_process()

    @classmethod
    def request(
        cls,
        method: str,
        payload: dict,
        *,
        deadline_us: int | None = None,
        socket_timeout: float = 20,
        session_token: str | None = None,
        nki_version: int = 2,
        address: tuple[str, int] | None = None,
    ) -> dict:
        request_id = str(uuid.uuid4())
        envelope = {
            "request_id": request_id,
            "idempotency_key": str(uuid.uuid4()),
            "nki_version": nki_version,
            "principal_id": "e2e-conformance",
            "session_token": cls.auth_token if session_token is None else session_token,
            "namespace": "default",
            "deadline_us": deadline_us or int((time.time() + 10) * 1_000_000),
            "traceparent": "",
            "tracestate": "",
            "feature_flags": [],
            "method": method,
            "payload": base64.b64encode(json.dumps(payload).encode()).decode(),
        }
        body = json.dumps(envelope, separators=(",", ":")).encode()
        with socket.create_connection(address or cls.address, timeout=socket_timeout) as connection:
            connection.sendall(struct.pack(">I", len(body)) + body)
            size = struct.unpack(">I", cls.read_exact(connection, 4))[0]
            response = json.loads(cls.read_exact(connection, size))
        if response.get("request_id") != request_id:
            raise AssertionError("response request_id mismatch")
        return response

    @staticmethod
    def read_exact(connection: socket.socket, size: int) -> bytes:
        body = bytearray()
        while len(body) < size:
            chunk = connection.recv(size - len(body))
            if not chunk:
                raise ConnectionError("connection closed during NKI frame")
            body.extend(chunk)
        return bytes(body)

    @classmethod
    def provider_child_count(cls) -> int:
        if os.name == "nt":
            class ProcessEntry(ctypes.Structure):
                _fields_ = [
                    ("dwSize", ctypes.c_ulong),
                    ("cntUsage", ctypes.c_ulong),
                    ("th32ProcessID", ctypes.c_ulong),
                    ("th32DefaultHeapID", ctypes.c_size_t),
                    ("th32ModuleID", ctypes.c_ulong),
                    ("cntThreads", ctypes.c_ulong),
                    ("th32ParentProcessID", ctypes.c_ulong),
                    ("pcPriClassBase", ctypes.c_long),
                    ("dwFlags", ctypes.c_ulong),
                    ("szExeFile", ctypes.c_wchar * 260),
                ]

            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel32.CreateToolhelp32Snapshot.argtypes = [ctypes.c_ulong, ctypes.c_ulong]
            kernel32.CreateToolhelp32Snapshot.restype = ctypes.c_void_p
            kernel32.Process32FirstW.argtypes = [
                ctypes.c_void_p,
                ctypes.POINTER(ProcessEntry),
            ]
            kernel32.Process32FirstW.restype = ctypes.c_int
            kernel32.Process32NextW.argtypes = [
                ctypes.c_void_p,
                ctypes.POINTER(ProcessEntry),
            ]
            kernel32.Process32NextW.restype = ctypes.c_int
            kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
            kernel32.CloseHandle.restype = ctypes.c_int
            snapshot = kernel32.CreateToolhelp32Snapshot(0x00000002, 0)
            if snapshot == ctypes.c_void_p(-1).value:
                raise OSError(ctypes.get_last_error(), "process snapshot failed")
            entry = ProcessEntry()
            entry.dwSize = ctypes.sizeof(entry)
            count = 0
            try:
                available = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
                while available:
                    if entry.th32ParentProcessID == cls.process.pid and entry.szExeFile.lower() in {
                        "nous-provider-worker.exe",
                        "python.exe",
                    }:
                        count += 1
                    available = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
            finally:
                kernel32.CloseHandle(snapshot)
            return count
        children = Path(f"/proc/{cls.process.pid}/task/{cls.process.pid}/children")
        if not children.exists():
            return 0
        return len(children.read_text(encoding="utf-8").split())

    def test_01_health_and_deadline(self) -> None:
        denied = self.request("HealthCheck", {}, session_token="invalid")
        self.assertEqual(denied["status"], "error")
        self.assertEqual(denied["error"]["code"], "UNAUTHENTICATED")
        health = self.request("HealthCheck", {"deep": True})
        self.assertEqual(health["status"], "success")
        self.assertEqual(health["payload"]["state"], "READY")
        expired = self.request("HealthCheck", {}, deadline_us=1)
        self.assertEqual(expired["status"], "error")
        self.assertEqual(expired["error"]["code"], "DEADLINE_EXCEEDED")

    def test_02_reference_execution_is_durable_and_idempotent(self) -> None:
        request = operation("reference", input_text="durable reference operation")
        first = self.request("SubmitWorkload", request)
        self.assertEqual(first["status"], "success")
        payload = first["payload"]
        self.assertEqual(
            payload["result"], hashlib.sha256(request["input"].encode()).hexdigest()
        )
        self.assertEqual(payload["decision_trace"]["selected"], "local-reference")
        before = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        second = self.request("SubmitWorkload", request)
        after = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        self.assertEqual(first["payload"]["completed_at_us"], second["payload"]["completed_at_us"])
        self.assertEqual(before, after)

    def test_02_reality_contract_requires_v3_and_configured_verifier(self) -> None:
        request = operation("reference", input_text="must not execute")
        request["effect_contract"] = {
            "schema_version": 1,
            "effect_id": "effect-unsupported",
            "target": "service:test",
            "expectation": {
                "schema": "apeir.service-health/v1",
                "subject": "service:test",
                "expected_value": {"http_status": 200},
                "evidence_requirement": ["http-response"],
            },
            "verification": "INDEPENDENT",
        }
        old = self.request("SubmitWorkload", request, nki_version=2)
        self.assertEqual(old["status"], "error")
        self.assertEqual(old["error"]["code"], "NKI_VERSION_UNSUPPORTED")
        before = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        current = self.request("SubmitWorkload", request, nki_version=3)
        after = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        self.assertEqual(current["status"], "error")
        self.assertIn("observer and verifier", current["error"]["message"])
        self.assertEqual(before, after)

    def test_03_provider_crash_is_isolated(self) -> None:
        failed = self.request("SubmitWorkload", operation("reference-crash"))
        self.assertEqual(failed["status"], "error")
        self.assertEqual(failed["error"]["code"], "PROVIDER_ERROR")
        health = self.request("HealthCheck", {})
        self.assertEqual(health["payload"]["state"], "READY")
        recovered = self.request("SubmitWorkload", operation("reference"))
        self.assertEqual(recovered["status"], "success")

    def test_04_cancellation_reaches_provider_and_releases_resources(self) -> None:
        delayed = operation("reference-delay", model="10000", input_text="cancel")
        result: dict = {}

        def submit() -> None:
            result.update(self.request("SubmitWorkload", delayed))

        thread = threading.Thread(target=submit, daemon=True)
        thread.start()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            metrics = self.request("GetMetrics", {})["payload"]
            if metrics["active_operations"] == 1:
                break
            time.sleep(0.02)
        else:
            self.fail("delayed workload never became active")
        cancelled = self.request("CancelWorkload", {"workload_id": delayed["workload_id"]})
        self.assertEqual(cancelled["status"], "success")
        thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result["status"], "error")
        self.assertEqual(result["error"]["code"], "OPERATION_CANCELLED")
        metrics = self.request("GetMetrics", {})["payload"]
        self.assertEqual(metrics["active_operations"], 0)
        self.assertEqual(metrics["corrupted_sequences"], [])
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.provider_child_count() != 0:
            time.sleep(0.05)
        self.assertEqual(self.provider_child_count(), 0)

    def test_05_envelope_deadline_stops_active_provider(self) -> None:
        delayed = operation("reference-delay", model="10000", input_text="deadline")
        response = self.request(
            "SubmitWorkload",
            delayed,
            deadline_us=int((time.time() + 0.2) * 1_000_000),
        )
        self.assertEqual(response["status"], "error")
        self.assertEqual(response["error"]["code"], "DEADLINE_EXCEEDED")
        metrics = self.request("GetMetrics", {})["payload"]
        self.assertEqual(metrics["active_operations"], 0)
        self.assertEqual(metrics["corrupted_sequences"], [])
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.provider_child_count() != 0:
            time.sleep(0.05)
        self.assertEqual(self.provider_child_count(), 0)

    def test_06_reference_capability_probe_uses_provider_process(self) -> None:
        response = self.request(
            "ProbeEngine",
            {
                "provider": "reference",
                "backend": "reference",
                "model": "reference",
                "endpoint": "",
                "credential_env": "",
                "timeout_ms": 5_000,
            },
        )
        self.assertEqual(response["status"], "success")
        self.assertTrue(response["payload"]["reachable"])
        self.assertEqual(
            response["payload"]["capabilities"]["runtime_class"], "reference"
        )
        self.assertIn(
            "deterministic conformance backend; not a real model",
            response["payload"]["warnings"],
        )

    def test_07_process_failover_continues_same_workload(self) -> None:
        workload_id = f"continuity-{uuid.uuid4()}"
        step_1 = operation("reference", input_text="committed step")
        step_1.update(
            {
                "operation_id": f"{workload_id}-step-1",
                "workload_id": workload_id,
                "step_id": "step-1",
            }
        )
        primary = operation("reference-crash", input_text="remaining step")
        primary.update(
            {
                "operation_id": f"{workload_id}-step-2",
                "workload_id": workload_id,
                "step_id": "step-2",
            }
        )
        fallback = json.loads(json.dumps(primary))
        fallback["backend"] = "reference"
        fallback["snapshot"]["provider_revision"] = "reference-b-v1"
        fallback["snapshot"]["capability_revision"] = "reference-b-v1"
        plan = {
            "workload_id": workload_id,
            "process_identity": "e2e-agent-process",
            "steps": [
                {
                    "step_id": "step-1",
                    "requirements": {"chat": True, "tools": True, "local_only": True},
                    "allow_degraded": False,
                    "candidates": [
                        {
                            "request": step_1,
                            "capabilities": capability_manifest("reference-a", "reference"),
                        }
                    ],
                },
                {
                    "step_id": "step-2",
                    "requirements": {"chat": True, "tools": True, "local_only": True},
                    "allow_degraded": False,
                    "candidates": [
                        {
                            "request": primary,
                            "capabilities": capability_manifest(
                                "reference-a", "reference-crash"
                            ),
                        },
                        {
                            "request": fallback,
                            "capabilities": capability_manifest("reference-b", "reference"),
                        },
                    ],
                },
            ],
        }
        first = self.request("SubmitContinuityPlan", plan)
        self.assertEqual(first["status"], "success")
        self.assertEqual(first["payload"]["workload_id"], workload_id)
        self.assertEqual(first["payload"]["process_identity"], "e2e-agent-process")
        self.assertEqual(len(first["payload"]["completed_steps"]), 2)
        self.assertEqual(len(first["payload"]["failovers"]), 1)
        self.assertEqual(first["payload"]["failovers"][0]["from_provider"], "reference-a")
        self.assertEqual(first["payload"]["failovers"][0]["to_provider"], "reference-b")
        before = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        replay = self.request("SubmitContinuityPlan", plan)
        after = self.request("GetMetrics", {})["payload"]["journal_sequence"]
        self.assertEqual(replay["status"], "success")
        self.assertEqual(before, after)
        self.assertEqual(
            [step["completed_at_us"] for step in first["payload"]["completed_steps"]],
            [step["completed_at_us"] for step in replay["payload"]["completed_steps"]],
        )

    def test_08_mathematical_model_uses_production_execution(self) -> None:
        request = operation(
            "reference-math",
            input_text=json.dumps(
                {
                    "operation": "linear",
                    "coefficients": [2, 3],
                    "input": [4, 5],
                    "bias": 1,
                }
            ),
        )
        response = self.request("SubmitWorkload", request)
        self.assertEqual(response["status"], "success")
        self.assertEqual(json.loads(response["payload"]["result"])["result"], 24.0)
        self.assertEqual(
            response["payload"]["decision_trace"]["selected_provider"],
            "reference-math",
        )

    def test_09_external_provider_sdk_uses_isolated_worker(self) -> None:
        entrypoint = ROOT / "tests" / "e2e" / "fixtures" / "provider_echo.py"
        request = operation(
            "external-process",
            model="echo-v1",
            input_text="external provider result",
            provider_entrypoint=str(entrypoint),
        )
        response = self.request("SubmitWorkload", request)
        self.assertEqual(response["status"], "success")
        self.assertEqual(response["payload"]["result"], "external provider result")
        self.assertEqual(
            response["payload"]["decision_trace"]["selected_provider"],
            "external-process",
        )
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.provider_child_count() != 0:
            time.sleep(0.05)
        self.assertEqual(self.provider_child_count(), 0)

    def test_10_external_provider_timeout_kills_the_process(self) -> None:
        entrypoint = ROOT / "tests" / "e2e" / "fixtures" / "provider_echo.py"
        request = operation(
            "external-process",
            input_text="__sleep__",
            provider_entrypoint=str(entrypoint),
        )
        request["timeout_ms"] = 100
        response = self.request("SubmitWorkload", request)
        self.assertEqual(response["status"], "error")
        self.assertEqual(response["error"]["code"], "DEADLINE_EXCEEDED")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.provider_child_count() != 0:
            time.sleep(0.05)
        self.assertEqual(self.provider_child_count(), 0)

    def test_11_external_provider_malformed_output_is_standardized(self) -> None:
        entrypoint = ROOT / "tests" / "e2e" / "fixtures" / "provider_echo.py"
        request = operation(
            "external-process",
            input_text="__malformed__",
            provider_entrypoint=str(entrypoint),
        )
        response = self.request("SubmitWorkload", request)
        self.assertEqual(response["status"], "error")
        self.assertEqual(response["error"]["code"], "PROVIDER_ERROR")
        self.assertEqual(self.request("HealthCheck", {})["payload"]["state"], "READY")

    def test_12_external_provider_cancellation_kills_the_process(self) -> None:
        entrypoint = ROOT / "tests" / "e2e" / "fixtures" / "provider_echo.py"
        request = operation(
            "external-process",
            input_text="__sleep__",
            provider_entrypoint=str(entrypoint),
        )
        result: dict = {}

        def submit() -> None:
            result.update(self.request("SubmitWorkload", request))

        thread = threading.Thread(target=submit, daemon=True)
        thread.start()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if self.request("GetMetrics", {})["payload"]["active_operations"] == 1:
                break
            time.sleep(0.02)
        else:
            self.fail("external workload never became active")
        cancelled = self.request("CancelWorkload", {"workload_id": request["workload_id"]})
        self.assertEqual(cancelled["status"], "success")
        thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result["error"]["code"], "OPERATION_CANCELLED")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.provider_child_count() != 0:
            time.sleep(0.05)
        self.assertEqual(self.provider_child_count(), 0)

    def test_13_runtime_control_api_is_durable_and_generation_safe(self) -> None:
        project = {
            "schema_version": 1,
            "id": "e2e-project",
            "name": "E2E project",
            "version": "1.0.0",
            "description": "control plane",
            "license": "Apache-2.0",
            "models": [],
            "algorithms": [],
            "datasets": [],
            "providers": [],
            "workflows": [],
            "policies": [],
            "experiments": [],
            "deployments": [],
            "secret_refs": {},
            "metadata": {},
        }
        put = {
            "schema_version": 1,
            "kind": "project",
            "document": project,
        }
        created = self.request("PutControlAsset", put)
        self.assertEqual(created["status"], "success")
        self.assertEqual(created["payload"]["generation"], 1)
        replay = self.request("PutControlAsset", put)
        self.assertEqual(replay["payload"]["generation"], 1)

        changed = json.loads(json.dumps(project))
        changed["description"] = "updated"
        conflict = self.request(
            "PutControlAsset",
            {"schema_version": 1, "kind": "project", "document": changed},
        )
        self.assertEqual(conflict["error"]["code"], "CONTROL_CONFLICT")
        updated = self.request(
            "PutControlAsset",
            {
                "schema_version": 1,
                "kind": "project",
                "document": changed,
                "expected_generation": 1,
            },
        )
        self.assertEqual(updated["payload"]["generation"], 2)
        selector = {"schema_version": 1, "kind": "project", "id": "e2e-project"}
        listed = self.request(
            "ListControlAssets", {"schema_version": 1, "kind": "project"}
        )
        self.assertEqual(len(listed["payload"]["assets"]), 1)

        self.restart_process()
        restored = self.request("GetControlAsset", selector)
        self.assertEqual(restored["payload"]["generation"], 2)

        task = operation("reference", input_text="control-api-task")
        self.assertEqual(self.request("SubmitWorkload", task)["status"], "success")
        fetched = self.request("GetWorkload", {"workload_id": task["workload_id"]})
        self.assertEqual(fetched["payload"]["workload_id"], task["workload_id"])
        workloads = self.request("ListWorkloads", {"limit": 10})
        self.assertTrue(
            any(
                item["workload_id"] == task["workload_id"]
                for item in workloads["payload"]["workloads"]
            )
        )

        deleted = self.request(
            "DeleteControlAsset",
            {"selector": selector, "expected_generation": 2},
        )
        self.assertTrue(deleted["payload"]["deleted"])
        missing = self.request("GetControlAsset", selector)
        self.assertEqual(missing["error"]["code"], "CONTROL_NOT_FOUND")


    def test_14_extension_authorization_binds_original_admission(self) -> None:
        digest = "sha256:" + "a" * 64
        admission = {
            "schema_version": 1,
            "extension_id": "e2e-admission-binding",
            "version": "1.0.0",
            "content_digest": digest,
            "source_format": "native",
            "compatibility_level": 1,
            "kinds": ["tool"],
            "capability_requests": [{"capability": "workspace.read", "access": "read",
                                     "scope": ["workspace"], "required": True}],
            "executor": "isolated-worker",
            "metadata": {"authority": "none", "supply_chain_verified": "true",
                         "signature_status": "Unsigned",
                         **{key: digest for key in ("normalized_ir_digest", "sbom_digest",
                                                   "provenance_digest", "signature_digest")}},
        }
        admitted = self.request("AdmitExtension", admission)
        self.assertEqual(admitted["status"], "success", admitted)
        receipt = admitted["payload"]["receipt_id"]
        self.restart_process()
        authorization = {"admission": admission, "admission_receipt_id": receipt,
                         "approval_id": "e2e-original-approval",
                         "approved_capabilities": ["workspace.read"]}
        valid = self.request("AuthorizeExtension", authorization)
        self.assertEqual(valid["status"], "success", valid)
        self.assertEqual(valid["payload"]["granted_capabilities"], ["workspace.read"])
        for change in ("capability", "scope", "executor", "metadata"):
            with self.subTest(change=change):
                altered = json.loads(json.dumps(authorization))
                altered["approval_id"] = "e2e-altered-" + change
                if change == "capability":
                    altered["admission"]["capability_requests"][0]["capability"] = "process.execute"
                    altered["approved_capabilities"] = ["process.execute"]
                elif change == "scope":
                    altered["admission"]["capability_requests"][0]["scope"] = ["elsewhere"]
                elif change == "executor":
                    altered["admission"]["executor"] = "different-worker"
                else:
                    altered["admission"]["metadata"]["signature_status"] = "Verified"
                before = self.request("GetMetrics", {})["payload"]["journal_sequence"]
                denied = self.request("AuthorizeExtension", altered)
                self.assertEqual(denied["status"], "error", denied)
                after = self.request("GetMetrics", {})["payload"]["journal_sequence"]
                self.assertEqual(before, after, "rejected authorization must not mutate journal")

    def test_15_configured_reality_service_runs_through_nki(self) -> None:
        with tempfile.TemporaryDirectory(prefix="nous-reality-nki-") as directory:
            root = Path(directory)
            service_address = free_address()
            daemon_address = free_address()
            mode = root / "service-mode.json"
            mode.write_text(json.dumps({"status": 200, "version": "v2"}), encoding="utf-8")
            fixture = ROOT / "crates/nous-kernel-core/tests/fixtures/service_health.py"
            service = subprocess.Popen(
                ["python", str(fixture), str(service_address[1]), str(mode)],
                cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            daemon = None
            try:
                for _ in range(100):
                    if service.poll() is not None:
                        self.fail("reality service fixture exited")
                    try:
                        with socket.create_connection(service_address, timeout=0.2):
                            break
                    except OSError:
                        time.sleep(0.02)
                else:
                    self.fail("reality service fixture did not start")

                config = root / "reality-config.json"
                config.write_text(json.dumps({
                    "schema_version": 1,
                    "target": "service:nki-test",
                    "subject": "service:nki-test",
                    "service_address": f"{service_address[0]}:{service_address[1]}",
                }), encoding="utf-8")
                journal = root / "reality.db"
                environment = os.environ.copy()
                environment["NOUS_NKI_TOKEN"] = self.auth_token
                daemon = subprocess.Popen(
                    [str(NOUSD), "serve", str(journal), str(WORKER),
                     f"{daemon_address[0]}:{daemon_address[1]}", str(config)],
                    cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                    text=True,
                    creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
                    env=environment,
                )
                for _ in range(200):
                    if daemon.poll() is not None:
                        self.fail(f"configured nousd exited: {daemon.stderr.read()}")
                    try:
                        with socket.create_connection(daemon_address, timeout=0.2):
                            break
                    except OSError:
                        time.sleep(0.05)
                else:
                    self.fail("configured nousd did not start")

                def contracted_request() -> dict:
                    request = operation("reference", input_text="service effect")
                    request["delivery"] = "AT_MOST_ONCE"
                    request["effect_contract"] = {
                        "schema_version": 1,
                        "effect_id": "effect-" + request["operation_id"],
                        "target": "service:nki-test",
                        "expectation": {
                            "schema": "apeir.service-health/v1",
                            "subject": "service:nki-test",
                            "expected_value": {"http_status": 200, "version": "v2"},
                            "evidence_requirement": ["http-health", "http-version"],
                        },
                        "verification": "INDEPENDENT",
                    }
                    return request

                healthy = self.request(
                    "SubmitWorkload", contracted_request(), nki_version=3,
                    address=daemon_address,
                )
                self.assertEqual(healthy["status"], "success", healthy)
                self.assertEqual(healthy["payload"]["executor_identity"], "apeir.process-provider")
                evidence_dir = journal.with_suffix(".evidence")
                evidence_files = list(evidence_dir.iterdir())
                self.assertEqual(len(evidence_files), 2)
                for path in evidence_files:
                    self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), path.name)

                mode.write_text(json.dumps({"status": 502, "version": "v2"}), encoding="utf-8")
                unhealthy = self.request(
                    "SubmitWorkload", contracted_request(), nki_version=3,
                    address=daemon_address,
                )
                self.assertEqual(unhealthy["status"], "error", unhealthy)
                self.assertEqual(unhealthy["error"]["code"], "REALITY_VERIFICATION_FAILED")

                outside_scope = contracted_request()
                outside_scope["effect_contract"]["target"] = "service:other"
                before = self.request("GetMetrics", {}, address=daemon_address)["payload"]["journal_sequence"]
                denied = self.request(
                    "SubmitWorkload", outside_scope, nki_version=3,
                    address=daemon_address,
                )
                after = self.request("GetMetrics", {}, address=daemon_address)["payload"]["journal_sequence"]
                self.assertEqual(denied["status"], "error", denied)
                self.assertEqual(before, after)
            finally:
                if daemon is not None:
                    daemon.kill()
                    daemon.wait(timeout=10)
                    if daemon.stderr is not None:
                        daemon.stderr.close()
                service.kill()
                service.wait(timeout=10)


if __name__ == "__main__":
    unittest.main(verbosity=2)
