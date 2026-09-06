"""Fault validation for the real ONNX Runtime edge provider process."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.request import urlopen

from kernel_e2e import KernelProcessTests, free_address, operation

ROOT = Path(__file__).resolve().parents[2]
SERVER = Path(__file__).with_name("edge_onnx_server.py")


def start_server(port: int) -> subprocess.Popen[bytes]:
    flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
    process = subprocess.Popen(
        [sys.executable, str(SERVER), "--port", str(port)],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=flags,
    )
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("edge provider exited during startup")
        try:
            with urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
                if response.status == 200:
                    return process
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("edge provider did not become ready")


def stop_server(process: subprocess.Popen[bytes], *, force: bool) -> None:
    if process.poll() is not None:
        return
    if force:
        process.kill()
    elif os.name == "nt":
        process.send_signal(signal.CTRL_BREAK_EVENT)
    else:
        process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def main() -> None:
    _, port = free_address()
    endpoint = f"http://127.0.0.1:{port}"
    edge = start_server(port)
    restarted: subprocess.Popen[bytes] | None = None
    KernelProcessTests.setUpClass()
    try:
        probe_payload = {
            "provider": "edge-onnx",
            "backend": "edge-openai-compatible",
            "execution_domain": "edge",
            "model": "edge-linear-v1",
            "endpoint": endpoint,
            "credential_env": "",
            "timeout_ms": 5_000,
        }
        probe = KernelProcessTests.request("ProbeEngine", probe_payload)
        if probe.get("status") != "success" or not probe["payload"]["reachable"]:
            raise AssertionError("edge probe failed")

        normal = operation(
            "edge-openai-compatible", model="edge-linear-v1", input_text="edge-normal"
        )
        normal.update({"endpoint": endpoint, "timeout_ms": 5_000})
        executed = KernelProcessTests.request("SubmitWorkload", normal)
        if executed.get("status") != "success":
            raise AssertionError("edge normal execution failed")

        timeout_request = operation(
            "edge-openai-compatible", model="edge-linear-v1", input_text="delay:2000"
        )
        timeout_request.update({"endpoint": endpoint, "timeout_ms": 100})
        timed_out = KernelProcessTests.request("SubmitWorkload", timeout_request)
        if timed_out.get("error", {}).get("code") != "DEADLINE_EXCEEDED":
            raise AssertionError("edge timeout was not standardized")

        cancel_request = operation(
            "edge-openai-compatible", model="edge-linear-v1", input_text="delay:10000"
        )
        cancel_request.update({"endpoint": endpoint, "timeout_ms": 30_000})
        result: dict = {}

        def submit() -> None:
            result.update(
                KernelProcessTests.request(
                    "SubmitWorkload", cancel_request, socket_timeout=40
                )
            )

        thread = threading.Thread(target=submit, daemon=True)
        thread.start()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
            if metrics["active_operations"] == 1:
                break
            time.sleep(0.02)
        else:
            raise AssertionError("edge cancellation workload never became active")
        cancelled = KernelProcessTests.request(
            "CancelWorkload", {"workload_id": cancel_request["workload_id"]}
        )
        thread.join(timeout=5)
        if cancelled.get("status") != "success" or result.get("error", {}).get("code") != "OPERATION_CANCELLED":
            raise AssertionError("edge cancellation did not propagate")

        stop_server(edge, force=True)
        unavailable = KernelProcessTests.request("ProbeEngine", probe_payload)
        if unavailable.get("status") != "error" or unavailable.get("error", {}).get("code") != "PROVIDER_ERROR":
            raise AssertionError("terminated edge provider was not reported unavailable")
        restarted = start_server(port)
        reprobe = KernelProcessTests.request("ProbeEngine", probe_payload)
        if reprobe.get("status") != "success":
            raise AssertionError("restarted edge provider was not discovered")

        metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
        with urlopen(f"{endpoint}/health", timeout=3) as response:
            health = json.load(response)
        print(
            json.dumps(
                {
                    "runtime": health["runtime"],
                    "device_provider": health["providers"],
                    "model_load_ms": health["model_load_ms"],
                    "normal_execution": "pass",
                    "timeout": "pass",
                    "cancellation": "pass",
                    "forced_termination": "pass",
                    "restart_and_reprobe": "pass",
                    "active_operations": metrics["active_operations"],
                    "provider_children": KernelProcessTests.provider_child_count(),
                    "journal_integrity": "pass"
                    if not metrics["corrupted_sequences"]
                    else "fail",
                },
                indent=2,
            )
        )
    finally:
        stop_server(edge, force=True)
        if restarted is not None:
            stop_server(restarted, force=False)
        KernelProcessTests.tearDownClass()


if __name__ == "__main__":
    main()
