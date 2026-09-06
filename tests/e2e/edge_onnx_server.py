"""Loopback-only OpenAI-compatible edge provider backed by ONNX Runtime.

The endpoint is intentionally small: it validates the production provider ABI
against a real, separately hosted ONNX inference session. It is a conformance
fixture, not an LLM server.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import numpy as np
import onnx
import onnxruntime as ort
from onnx import TensorProto, helper

MODEL_ID = "edge-linear-v1"


def create_model(path: Path) -> None:
    input_info = helper.make_tensor_value_info("input", TensorProto.FLOAT, [1, 2])
    output_info = helper.make_tensor_value_info("output", TensorProto.FLOAT, [1, 2])
    scale = helper.make_tensor("scale", TensorProto.FLOAT, [1, 2], [2.0, 3.0])
    bias = helper.make_tensor("bias", TensorProto.FLOAT, [1, 2], [1.0, -1.0])
    graph = helper.make_graph(
        [
            helper.make_node("Mul", ["input", "scale"], ["scaled"]),
            helper.make_node("Add", ["scaled", "bias"], ["output"]),
        ],
        "nous-edge-linear",
        [input_info],
        [output_info],
        [scale, bias],
    )
    model = helper.make_model(
        graph,
        producer_name="nous-edge-conformance",
        opset_imports=[helper.make_opsetid("", 17)],
    )
    model.ir_version = 10
    onnx.checker.check_model(model)
    onnx.save(model, path)


class EdgeRuntime:
    def __init__(self, model_path: Path) -> None:
        started = time.perf_counter()
        self.session = ort.InferenceSession(
            str(model_path), providers=["CPUExecutionProvider"]
        )
        self.load_ms = (time.perf_counter() - started) * 1_000
        self.inference_count = 0
        self.last_latency_ms = 0.0

    def infer(self, text: str) -> dict[str, Any]:
        if text.startswith("delay:"):
            delay_ms = min(int(text.split(":", 1)[1]), 30_000)
            time.sleep(delay_ms / 1_000)
        encoded = text.encode("utf-8")
        features = np.array(
            [[float(len(encoded)), float(sum(encoded) % 997)]], dtype=np.float32
        )
        started = time.perf_counter()
        output = self.session.run(["output"], {"input": features})[0]
        self.last_latency_ms = (time.perf_counter() - started) * 1_000
        self.inference_count += 1
        return {
            "model": MODEL_ID,
            "runtime": "onnxruntime",
            "providers": self.session.get_providers(),
            "result": [round(float(item), 4) for item in output[0]],
            "input_sha256": hashlib.sha256(encoded).hexdigest(),
        }


def handler_for(runtime: EdgeRuntime) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        server_version = "NousEdgeONNX/1"

        def send_json(self, status: int, payload: dict[str, Any]) -> None:
            body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            if self.path == "/models":
                self.send_json(200, {"data": [{"id": MODEL_ID, "object": "model"}]})
            elif self.path == "/health":
                self.send_json(
                    200,
                    {
                        "status": "ready",
                        "runtime": "onnxruntime",
                        "providers": runtime.session.get_providers(),
                        "model_load_ms": round(runtime.load_ms, 3),
                        "inference_count": runtime.inference_count,
                        "last_latency_ms": round(runtime.last_latency_ms, 3),
                        "thread_count": threading.active_count(),
                    },
                )
            else:
                self.send_json(404, {"error": "not found"})

        def do_POST(self) -> None:
            if self.path != "/chat/completions":
                self.send_json(404, {"error": "not found"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                request = json.loads(self.rfile.read(length))
                if request.get("model") != MODEL_ID:
                    self.send_json(404, {"error": "model unavailable"})
                    return
                messages = request.get("messages", [])
                text = str(messages[-1].get("content", "")) if messages else ""
                result = runtime.infer(text)
                self.send_json(
                    200,
                    {
                        "model": MODEL_ID,
                        "choices": [
                            {"index": 0, "message": {"role": "assistant", "content": json.dumps(result)}}
                        ],
                    },
                )
            except (ValueError, TypeError, json.JSONDecodeError) as error:
                self.send_json(400, {"error": str(error)})

        def log_message(self, format: str, *args: object) -> None:
            return

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18081)
    parser.add_argument("--model-file", type=Path)
    arguments = parser.parse_args()
    if arguments.host not in {"127.0.0.1", "::1", "localhost"}:
        raise SystemExit("the conformance provider must bind to loopback")

    with tempfile.TemporaryDirectory(prefix="nous-edge-onnx-") as directory:
        model_path = arguments.model_file or Path(directory) / "edge-linear.onnx"
        if not model_path.exists():
            create_model(model_path)
        runtime = EdgeRuntime(model_path)
        server = ThreadingHTTPServer((arguments.host, arguments.port), handler_for(runtime))
        print(
            json.dumps(
                {
                    "status": "ready",
                    "model": MODEL_ID,
                    "runtime": "onnxruntime",
                    "providers": runtime.session.get_providers(),
                    "model_load_ms": round(runtime.load_ms, 3),
                }
            ),
            flush=True,
        )
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
        finally:
            server.server_close()


if __name__ == "__main__":
    main()
