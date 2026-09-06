"""Process-level soak runner with failure injection and resource-growth evidence."""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import statistics
import subprocess
import threading
import time
from pathlib import Path

from kernel_e2e import KernelProcessTests, operation

ES_CONTINUOUS = 0x80000000
ES_SYSTEM_REQUIRED = 0x00000001


def set_system_awake(required: bool) -> None:
    if os.name != "nt":
        return
    flags = ES_CONTINUOUS | (ES_SYSTEM_REQUIRED if required else 0)
    if ctypes.windll.kernel32.SetThreadExecutionState(flags) == 0:
        raise OSError("SetThreadExecutionState failed")


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def process_sample(pid: int) -> dict[str, int]:
    if os.name == "nt":
        command = (
            f"$p=Get-Process -Id {pid} -ErrorAction Stop; "
            "[ordered]@{rss_bytes=[int64]$p.WorkingSet64;"
            "handles=[int64]$p.HandleCount;threads=[int64]$p.Threads.Count;"
            "cpu_100ns=[int64]$p.TotalProcessorTime.Ticks}|ConvertTo-Json -Compress"
        )
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", command],
            capture_output=True,
            text=True,
            timeout=15,
            check=True,
        )
        return json.loads(completed.stdout)

    status: dict[str, str] = {}
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
        if ":" in line:
            key, value = line.split(":", 1)
            status[key] = value.strip()
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8").split()
    clock_ticks = os.sysconf("SC_CLK_TCK")
    cpu_100ns = int((int(stat[13]) + int(stat[14])) * 10_000_000 / clock_ticks)
    return {
        "rss_bytes": int(status.get("VmRSS", "0 kB").split()[0]) * 1024,
        "handles": len(list(Path(f"/proc/{pid}/fd").iterdir())),
        "threads": int(status.get("Threads", "0")),
        "cpu_100ns": cpu_100ns,
    }


def cancel_workload() -> str:
    request = operation("reference-delay", model="5000", input_text="soak-cancel")
    response: dict = {}

    def submit() -> None:
        try:
            response.update(KernelProcessTests.request("SubmitWorkload", request))
        except (ConnectionError, OSError, TimeoutError) as error:
            response["transport_error"] = type(error).__name__

    worker = threading.Thread(target=submit, daemon=True)
    worker.start()
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
        if metrics["active_operations"] == 1:
            break
        time.sleep(0.02)
    else:
        return "cancellation:workload-never-active"
    cancellation = KernelProcessTests.request(
        "CancelWorkload", {"workload_id": request["workload_id"]}
    )
    worker.join(timeout=10)
    if worker.is_alive():
        return "cancellation:submit-still-running"
    if cancellation.get("status") != "success":
        return f"cancellation:request:{cancellation.get('error', {}).get('code', 'unknown')}"
    if "transport_error" in response:
        return f"cancellation:transport:{response['transport_error']}"
    if response.get("status") != "error":
        return f"cancellation:submit-status:{response.get('status', 'unknown')}"
    code = response.get("error", {}).get("code", "unknown")
    if code != "OPERATION_CANCELLED":
        return f"cancellation:submit-code:{code}"
    return "ok"


def journal_size() -> int:
    return sum(
        path.stat().st_size
        for path in KernelProcessTests.journal.parent.glob(
            f"{KernelProcessTests.journal.name}*"
        )
    )


def maximum_lifecycle_growth(samples: list[dict[str, int | float]], key: str) -> int:
    by_pid: dict[int, list[dict[str, int | float]]] = {}
    for sample in samples:
        by_pid.setdefault(int(sample["pid"]), []).append(sample)
    growth = [
        int(group[-1][key]) - int(group[0][key])
        for group in by_pid.values()
        if len(group) > 1
    ]
    return max(growth, default=0)


def write_json_atomic(path: Path, payload: dict) -> None:
    """Persist soak evidence without exposing a partially written JSON document."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def checkpoint_result(
    started_at: float,
    latencies: list[float],
    samples: list[dict[str, int | float]],
    counters: dict[str, int],
    metrics: dict,
) -> dict:
    return {
        "status": "running",
        "duration_seconds": round(time.monotonic() - started_at, 3),
        "operations": len(latencies),
        **counters,
        "latency_ms": {
            "mean": round(statistics.fmean(latencies), 3),
            "p50": round(percentile(latencies, 0.50), 3),
            "p95": round(percentile(latencies, 0.95), 3),
            "max": round(max(latencies), 3),
        },
        "samples": samples,
        "journal_sequence": metrics.get("journal_sequence"),
        "journal_integrity": (
            "pass" if not metrics.get("corrupted_sequences") else "fail"
        ),
        "active_operations": metrics.get("active_operations"),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--sample-seconds", type=int, default=30)
    parser.add_argument("--restart-every", type=int, default=250)
    parser.add_argument("--cancel-every", type=int, default=75)
    parser.add_argument("--deadline-every", type=int, default=50)
    parser.add_argument("--provider-failure-every", type=int, default=40)
    parser.add_argument("--stop-on-first-failure", action="store_true")
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    if arguments.seconds < 1 or arguments.sample_seconds < 1:
        raise SystemExit("duration and sample interval must be positive")

    latencies: list[float] = []
    samples: list[dict[str, int | float]] = []
    counters = {
        "unexpected_failures": 0,
        "provider_failures_injected": 0,
        "runtime_restarts": 0,
        "recovery_failures": 0,
        "cancellations": 0,
        "deadlines": 0,
        "duplicate_receipts": 0,
    }
    failure_reasons: dict[str, int] = {}
    first_failure: dict | None = None

    def record_failure(reason: str, iteration: int) -> None:
        nonlocal first_failure
        counters["unexpected_failures"] += 1
        failure_reasons[reason] = failure_reasons.get(reason, 0) + 1
        if first_failure is None:
            first_failure = {
                "reason": reason,
                "iteration": iteration,
                "elapsed_seconds": round(time.monotonic() - started_at, 3),
            }
    set_system_awake(True)
    KernelProcessTests.setUpClass()
    startup_seconds = [KernelProcessTests.last_startup_seconds]
    started_at = time.monotonic()
    next_sample = started_at
    metrics: dict = {}
    try:
        deadline = started_at + arguments.seconds
        iteration = 0
        while time.monotonic() < deadline:
            if arguments.restart_every > 0 and iteration > 0 and iteration % arguments.restart_every == 0:
                KernelProcessTests.restart_process(graceful=False)
                startup_seconds.append(KernelProcessTests.last_startup_seconds)
                counters["runtime_restarts"] += 1
                health = KernelProcessTests.request("HealthCheck", {})
                if health.get("status") != "success" or health["payload"]["state"] != "READY":
                    counters["recovery_failures"] += 1

            call_started = time.perf_counter()
            if (
                arguments.cancel_every > 0
                and iteration % arguments.cancel_every == arguments.cancel_every - 1
            ):
                cancellation_result = cancel_workload()
                if cancellation_result == "ok":
                    counters["cancellations"] += 1
                else:
                    record_failure(cancellation_result, iteration)
            elif (
                arguments.deadline_every > 0
                and iteration % arguments.deadline_every == arguments.deadline_every - 1
            ):
                delayed = operation("reference-delay", model="5000", input_text="soak-deadline")
                response = KernelProcessTests.request(
                    "SubmitWorkload",
                    delayed,
                    deadline_us=int((time.time() + 0.05) * 1_000_000),
                )
                if response.get("error", {}).get("code") == "DEADLINE_EXCEEDED":
                    counters["deadlines"] += 1
                else:
                    record_failure(
                        f"deadline:{response.get('error', {}).get('code', response.get('status', 'unknown'))}",
                        iteration,
                    )
            elif (
                arguments.provider_failure_every > 0
                and iteration % arguments.provider_failure_every
                == arguments.provider_failure_every - 1
            ):
                response = KernelProcessTests.request(
                    "SubmitWorkload", operation("reference-crash", input_text="soak-crash")
                )
                if response.get("error", {}).get("code") == "PROVIDER_ERROR":
                    counters["provider_failures_injected"] += 1
                else:
                    record_failure(
                        f"provider-crash:{response.get('error', {}).get('code', response.get('status', 'unknown'))}",
                        iteration,
                    )
            else:
                request = operation("reference", input_text=f"soak-{iteration}")
                response = KernelProcessTests.request("SubmitWorkload", request)
                if response.get("status") != "success":
                    record_failure(
                        f"normal:{response.get('error', {}).get('code', response.get('status', 'unknown'))}",
                        iteration,
                    )
                elif iteration % 25 == 0:
                    replay = KernelProcessTests.request("SubmitWorkload", request)
                    if (
                        replay.get("status") != "success"
                        or replay["payload"]["completed_at_us"]
                        != response["payload"]["completed_at_us"]
                    ):
                        counters["duplicate_receipts"] += 1
                        record_failure("normal:replay-mismatch", iteration)
            latencies.append((time.perf_counter() - call_started) * 1000)
            if arguments.stop_on_first_failure and first_failure is not None:
                break

            now = time.monotonic()
            if now >= next_sample:
                metrics = KernelProcessTests.request("GetMetrics", {})["payload"]
                if metrics["corrupted_sequences"] or metrics["active_operations"] != 0:
                    record_failure("sample:integrity-or-active", iteration)
                sample = process_sample(KernelProcessTests.process.pid)
                sample.update(
                    {
                        "pid": KernelProcessTests.process.pid,
                        "elapsed_seconds": round(now - started_at, 3),
                        "journal_bytes": journal_size(),
                    }
                )
                samples.append(sample)
                if arguments.output:
                    write_json_atomic(
                        arguments.output,
                        checkpoint_result(
                            started_at, latencies, samples, counters, metrics
                        )
                        | {
                            "failure_reasons": failure_reasons,
                            "first_failure": first_failure,
                        },
                    )
                next_sample = now + arguments.sample_seconds
            iteration += 1

        metrics = KernelProcessTests.request(
            "GetMetrics", {"verify_integrity": True}
        )["payload"]
        final_provider_children = KernelProcessTests.provider_child_count()
    finally:
        KernelProcessTests.tearDownClass()
        set_system_awake(False)

    result = {
        "status": "completed",
        "duration_seconds": round(time.monotonic() - started_at, 3),
        "operations": len(latencies),
        **counters,
        "failure_reasons": failure_reasons,
        "first_failure": first_failure,
        "latency_ms": {
            "mean": round(statistics.fmean(latencies), 3),
            "p50": round(percentile(latencies, 0.50), 3),
            "p95": round(percentile(latencies, 0.95), 3),
            "max": round(max(latencies), 3),
        },
        "growth": {
            "scope": "maximum within one daemon lifecycle",
            "rss_bytes": maximum_lifecycle_growth(samples, "rss_bytes"),
            "handles": maximum_lifecycle_growth(samples, "handles"),
            "threads": maximum_lifecycle_growth(samples, "threads"),
            "journal_bytes": int(samples[-1]["journal_bytes"])
            - int(samples[0]["journal_bytes"]),
        },
        "startup_seconds": {
            "samples": [round(value, 3) for value in startup_seconds],
            "max": round(max(startup_seconds), 3),
        },
        "samples": samples,
        "journal_sequence": metrics["journal_sequence"],
        "journal_integrity": "pass" if not metrics["corrupted_sequences"] else "fail",
        "active_operations": metrics["active_operations"],
        "orphan_provider_processes": final_provider_children,
    }
    encoded = json.dumps(result, indent=2)
    print(encoded)
    if arguments.output:
        write_json_atomic(arguments.output, result)
    if (
        counters["unexpected_failures"]
        or counters["recovery_failures"]
        or counters["duplicate_receipts"]
        or result["journal_integrity"] != "pass"
        or result["active_operations"] != 0
        or final_provider_children != 0
    ):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
