"""Lifecycle and fault validation for a real local Ollama model."""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import threading
import time
from pathlib import Path
from urllib.request import urlopen

from kernel_e2e import KernelProcessTests, free_address, operation


def start_ollama(executable: Path, models: Path, port: int) -> subprocess.Popen[bytes]:
    environment = os.environ.copy()
    environment["OLLAMA_MODELS"] = str(models)
    environment["OLLAMA_HOST"] = f"127.0.0.1:{port}"
    flags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
    process = subprocess.Popen(
        [str(executable), "serve"],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=flags,
    )
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Ollama exited during startup")
        try:
            with urlopen(f"http://127.0.0.1:{port}/api/tags", timeout=1) as response:
                if response.status == 200:
                    return process
        except OSError:
            time.sleep(0.1)
    raise RuntimeError("Ollama did not become ready")


def stop_process(process: subprocess.Popen[bytes], *, force: bool) -> None:
    if process.poll() is not None:
        return
    if force:
        process.kill()
    elif os.name == "nt":
        process.send_signal(signal.CTRL_BREAK_EVENT)
    else:
        process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ollama-exe", type=Path, required=True)
    parser.add_argument("--models-dir", type=Path, required=True)
    parser.add_argument("--model", default="qwen3:0.6b")
    args = parser.parse_args()
    if not args.ollama_exe.is_file() or not args.models_dir.is_dir():
        raise SystemExit("Ollama executable or model directory is unavailable")

    _, port = free_address()
    endpoint = f"http://127.0.0.1:{port}"
    server = start_ollama(args.ollama_exe, args.models_dir, port)
    restarted: subprocess.Popen[bytes] | None = None
    KernelProcessTests.setUpClass()
    try:
        probe_payload = {
            "provider": "local-ollama",
            "backend": "ollama",
            "execution_domain": "local",
            "model": args.model,
            "endpoint": endpoint,
            "credential_env": "",
            "timeout_ms": 10_000,
        }
        probe = KernelProcessTests.request("ProbeEngine", probe_payload)
        if probe.get("status") != "success" or args.model not in probe["payload"]["available_models"]:
            raise AssertionError("Ollama model discovery failed")

        normal = operation("ollama", model=args.model, input_text="Return exactly: LOCAL_OK")
        normal.update({"endpoint": endpoint, "timeout_ms": 120_000})
        executed = KernelProcessTests.request(
            "SubmitWorkload",
            normal,
            deadline_us=int((time.time() + 150) * 1_000_000),
            socket_timeout=150,
        )
        if executed.get("status") != "success":
            raise AssertionError("Ollama execution failed")

        timeout_request = operation("ollama", model=args.model, input_text="Explain runtime durability")
        timeout_request.update({"endpoint": endpoint, "timeout_ms": 1})
        timed_out = KernelProcessTests.request("SubmitWorkload", timeout_request)
        if timed_out.get("error", {}).get("code") != "DEADLINE_EXCEEDED":
            raise AssertionError("Ollama timeout was not standardized")

        cancel_request = operation(
            "ollama",
            model=args.model,
            input_text="Write a detailed technical essay about durable execution and recovery.",
        )
        cancel_request.update({"endpoint": endpoint, "timeout_ms": 120_000})
        result: dict = {}

        def submit() -> None:
            result.update(
                KernelProcessTests.request(
                    "SubmitWorkload",
                    cancel_request,
                    deadline_us=int((time.time() + 150) * 1_000_000),
                    socket_timeout=150,
                )
            )

        thread = threading.Thread(target=submit, daemon=True)
        thread.start()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
            if metrics["active_operations"] == 1:
                break
            time.sleep(0.02)
        else:
            raise AssertionError("Ollama cancellation workload never became active")
        cancelled = KernelProcessTests.request(
            "CancelWorkload", {"workload_id": cancel_request["workload_id"]}
        )
        thread.join(timeout=10)
        if cancelled.get("status") != "success" or result.get("error", {}).get("code") != "OPERATION_CANCELLED":
            raise AssertionError("Ollama cancellation did not propagate")

        stop_process(server, force=True)
        unavailable = KernelProcessTests.request("ProbeEngine", probe_payload)
        if unavailable.get("status") != "error":
            raise AssertionError("terminated Ollama was not reported unavailable")
        restarted = start_ollama(args.ollama_exe, args.models_dir, port)
        reprobe = KernelProcessTests.request("ProbeEngine", probe_payload)
        if reprobe.get("status") != "success":
            raise AssertionError("restarted Ollama was not rediscovered")

        metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
        print(
            json.dumps(
                {
                    "provider": "ollama",
                    "model": args.model,
                    "model_count": len(probe["payload"]["available_models"]),
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
        stop_process(server, force=True)
        if restarted is not None:
            stop_process(restarted, force=False)
        KernelProcessTests.tearDownClass()


if __name__ == "__main__":
    main()
