"""Real provider validation without persisting or printing credential values."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import time

from kernel_e2e import KernelProcessTests, operation


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", required=True)
    parser.add_argument(
        "--backend",
        required=True,
        choices=("openai-compatible", "ollama", "edge-openai-compatible"),
    )
    parser.add_argument("--model", required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--credential-env", default="")
    parser.add_argument("--prompt", default="Reply with the single word READY.")
    arguments = parser.parse_args()
    if arguments.credential_env:
        if arguments.credential_env not in os.environ:
            raise SystemExit("credential environment reference is unavailable")
        os.environ["NOUS_ALLOWED_CREDENTIALS"] = arguments.credential_env

    KernelProcessTests.setUpClass()
    try:
        probe_started = time.perf_counter()
        probe = KernelProcessTests.request(
            "ProbeEngine",
            {
                "provider": arguments.provider,
                "backend": arguments.backend,
                "model": arguments.model,
                "endpoint": arguments.endpoint,
                "credential_env": arguments.credential_env,
                "timeout_ms": 30_000,
            },
        )
        probe_elapsed = (time.perf_counter() - probe_started) * 1000
        if probe.get("status") != "success" or not probe["payload"]["reachable"]:
            error = probe.get("error", {})
            raise RuntimeError(
                f"provider capability probe failed: {error.get('code', 'UNKNOWN')} "
                f"{error.get('message', '')}"
            )

        request = operation(
            arguments.backend, model=arguments.model, input_text=arguments.prompt
        )
        request["endpoint"] = arguments.endpoint
        request["credential_env"] = arguments.credential_env
        request["timeout_ms"] = 60_000
        execute_started = time.perf_counter()
        response = KernelProcessTests.request("SubmitWorkload", request)
        execute_elapsed = (time.perf_counter() - execute_started) * 1000
        if response.get("status") != "success" or not response["payload"].get("result"):
            error = response.get("error", {})
            raise RuntimeError(
                f"real model execution failed: {error.get('code', 'UNKNOWN')} "
                f"{error.get('message', '')}"
            )
        result = response["payload"]["result"]
        metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
        report = {
            "provider": arguments.provider,
            "backend": arguments.backend,
            "model": arguments.model,
            "credential_reference": arguments.credential_env or None,
            "credential_value_disclosed": False,
            "probe_reachable": True,
            "reported_model_count": len(probe["payload"]["available_models"]),
            "probe_latency_ms": round(probe_elapsed, 3),
            "execution_latency_ms": round(execute_elapsed, 3),
            "result_sha256": hashlib.sha256(result.encode()).hexdigest(),
            "result_length": len(result),
            "workload_id": response["payload"]["workload_id"],
            "journal_sequence": metrics["journal_sequence"],
            "journal_integrity": "pass"
            if not metrics["corrupted_sequences"]
            else "fail",
        }
        print(json.dumps(report, indent=2))
    finally:
        KernelProcessTests.tearDownClass()


if __name__ == "__main__":
    main()
