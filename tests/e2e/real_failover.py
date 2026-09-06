"""Validate durable continuity from a terminated real provider to another backend."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
import uuid
from pathlib import Path

from kernel_e2e import KernelProcessTests, operation


def manifest(provider: str, backend: str, model: str, domain: str) -> dict:
    local = domain in {"local", "edge"}
    return {
        "schema_version": 1,
        "provider": provider,
        "backend": backend,
        "model": model,
        "runtime_class": domain,
        "chat": "UNKNOWN",
        "streaming": "UNKNOWN",
        "tools": "UNKNOWN",
        "structured_output": "UNKNOWN",
        "vision": "UNKNOWN",
        "embedding": "UNKNOWN",
        "reasoning": "UNKNOWN",
        "local_execution": "SUPPORTED" if local else "UNSUPPORTED",
        "remote_execution": "UNSUPPORTED" if local else "SUPPORTED",
        "context_length": None,
        "privacy_local": local,
        "privacy": "LOCAL_ONLY" if local else "PROVIDER_MANAGED",
        "estimated_latency_ms": None,
        "estimated_cost_microcents": 0 if local else None,
        "probed_at_us": int(time.time() * 1_000_000),
    }


def candidate(
    workload_id: str,
    step_id: str,
    provider: str,
    backend: str,
    model: str,
    endpoint: str,
    domain: str,
    text: str,
) -> dict:
    request = operation(backend, model=model, input_text=text)
    request.update(
        {
            "operation_id": f"{workload_id}-{step_id}",
            "workload_id": workload_id,
            "step_id": step_id,
            "endpoint": endpoint,
            "execution_domain": domain,
            "timeout_ms": 30_000,
        }
    )
    request["snapshot"]["provider_revision"] = f"{provider}-v1"
    request["snapshot"]["model_revision"] = model
    request["snapshot"]["capability_revision"] = f"{provider}-probe-v1"
    return {"request": request, "capabilities": manifest(provider, backend, model, domain)}


def terminate_process(pid: int) -> None:
    if os.name == "nt":
        subprocess.run(
            ["taskkill.exe", "/PID", str(pid), "/T", "/F"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    else:
        os.kill(pid, 9)


def journal_entries(path: Path) -> list[dict]:
    import sqlite3

    database = sqlite3.connect(path)
    try:
        rows = database.execute(
            "SELECT sequence, entry_type, object_type, new_phase, payload "
            "FROM journal_entries ORDER BY sequence"
        ).fetchall()
    finally:
        database.close()
    entries = []
    for sequence, entry_type, object_type, phase, payload in rows:
        entries.append(
            {
                "sequence": sequence,
                "entry_type": entry_type,
                "object_type": object_type,
                "phase": phase,
                "payload": json.loads(payload) if payload else {},
            }
        )
    return entries


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--primary-pid", type=int, required=True)
    parser.add_argument("--primary-endpoint", default="http://127.0.0.1:11434")
    parser.add_argument("--primary-model", default="qwen3:0.6b")
    parser.add_argument("--fallback-endpoint", default="http://127.0.0.1:18081")
    parser.add_argument("--fallback-model", default="edge-linear-v1")
    args = parser.parse_args()

    workload_id = f"real-continuity-{uuid.uuid4()}"
    first = candidate(
        workload_id,
        "step-1",
        "local-ollama",
        "ollama",
        args.primary_model,
        args.primary_endpoint,
        "local",
        "Return exactly: FIRST_STEP_COMMITTED",
    )
    primary_second = candidate(
        workload_id,
        "step-2",
        "local-ollama",
        "ollama",
        args.primary_model,
        args.primary_endpoint,
        "local",
        "Continue after the durable first step.",
    )
    fallback_second = candidate(
        workload_id,
        "step-2",
        "edge-onnx",
        "edge-openai-compatible",
        args.fallback_model,
        args.fallback_endpoint,
        "edge",
        "Continue after the durable first step.",
    )
    initial_plan = {
        "workload_id": workload_id,
        "process_identity": "real-model-continuity",
        "steps": [
            {
                "step_id": "step-1",
                "requirements": {},
                "allow_degraded": True,
                "candidates": [first],
            }
        ],
    }
    resumed_plan = json.loads(json.dumps(initial_plan))
    resumed_plan["steps"].append(
        {
            "step_id": "step-2",
            "requirements": {},
            "allow_degraded": True,
            "candidates": [primary_second, fallback_second],
        }
    )

    KernelProcessTests.setUpClass()
    try:
        request_deadline = int((time.time() + 120) * 1_000_000)
        committed = KernelProcessTests.request(
            "SubmitContinuityPlan",
            initial_plan,
            deadline_us=request_deadline,
            socket_timeout=120,
        )
        if committed.get("status") != "success":
            raise RuntimeError(f"primary model step failed: {committed.get('error', {})}")
        terminate_process(args.primary_pid)
        time.sleep(1)
        resumed = KernelProcessTests.request(
            "SubmitContinuityPlan",
            resumed_plan,
            deadline_us=int((time.time() + 120) * 1_000_000),
            socket_timeout=120,
        )
        if resumed.get("status") != "success":
            observations = [
                entry
                for entry in journal_entries(KernelProcessTests.journal)
                if entry["object_type"] == "ProviderCandidate"
            ]
            raise RuntimeError(
                f"failover execution failed: {resumed.get('error', {})}; "
                f"candidate observations={observations}"
            )
        payload = resumed["payload"]
        if payload["workload_id"] != workload_id or len(payload["completed_steps"]) != 2:
            raise AssertionError("continuity identity or committed step history changed")
        if len(payload["failovers"]) != 1:
            raise AssertionError("expected exactly one recorded provider failover")
        failover = payload["failovers"][0]
        if failover["from_provider"] != "local-ollama" or failover["to_provider"] != "edge-onnx":
            raise AssertionError("unexpected failover route")

        before = KernelProcessTests.request("GetMetrics", {})["payload"]["journal_sequence"]
        replayed = KernelProcessTests.request(
            "SubmitContinuityPlan",
            resumed_plan,
            deadline_us=int((time.time() + 120) * 1_000_000),
            socket_timeout=120,
        )
        after = KernelProcessTests.request("GetMetrics", {})["payload"]["journal_sequence"]
        if replayed.get("status") != "success" or before != after:
            raise AssertionError("continuity replay appended duplicate journal effects")

        entries = journal_entries(KernelProcessTests.journal)
        step_commits = [
            entry
            for entry in entries
            if entry["object_type"] == "StepCommit" and entry["phase"] == "COMMITTED"
        ]
        rebinds = [entry for entry in entries if entry["object_type"] == "ProviderRebind"]
        report = {
            "workload_id": workload_id,
            "same_workload_id": True,
            "primary_backend": "ollama",
            "primary_process_terminated": True,
            "fallback_backend": "edge-openai-compatible",
            "completed_step_count": len(payload["completed_steps"]),
            "committed_operation_count": len(step_commits),
            "provider_rebind_count": len(rebinds),
            "failover": failover,
            "journal_sequence": after,
            "duplicate_entries_on_replay": after - before,
            "journal_integrity": "pass",
        }
        print(json.dumps(report, indent=2))
    finally:
        KernelProcessTests.tearDownClass()


if __name__ == "__main__":
    main()
