"""Small, dependency-free NKI v1 client."""

from __future__ import annotations

import base64
import json
import os
import socket
import struct
import time
import uuid
from dataclasses import dataclass
from typing import Any

NKI_VERSION = 2
MAX_FRAME_BYTES = 16 * 1024 * 1024
CONTROL_ASSET_KINDS = frozenset(
    {"project", "model", "graph", "dataset", "experiment", "artifact"}
)


class NkiError(RuntimeError):
    """A structured error returned by the kernel."""

    def __init__(
        self,
        code: str,
        message: str,
        retryable: bool = False,
        *,
        failed_phase: str = "",
        cause: str = "",
        recommended_delay_ms: int = 0,
    ) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.retryable = retryable
        self.failed_phase = failed_phase
        self.cause = cause
        self.recommended_delay_ms = recommended_delay_ms


@dataclass(frozen=True)
class NkiClient:
    """Connect to an APEIR daemon over the length-prefixed NKI TCP transport."""

    address: tuple[str, int] = ("127.0.0.1", 8771)
    timeout_seconds: float = 30.0

    def request(self, method: str, payload: dict[str, Any] | None = None) -> Any:
        request_id = str(uuid.uuid4())
        body = json.dumps(
            {
                "request_id": request_id,
                "idempotency_key": str(uuid.uuid4()),
                "nki_version": NKI_VERSION,
                "principal_id": "python-sdk",
                "session_token": os.environ.get("NOUS_NKI_TOKEN", ""),
                "namespace": "default",
                "deadline_us": int((time.time() + self.timeout_seconds) * 1_000_000),
                "traceparent": "",
                "tracestate": "",
                "feature_flags": [],
                "method": method,
                "payload": base64.b64encode(
                    json.dumps(payload or {}, separators=(",", ":")).encode("utf-8")
                ).decode("ascii"),
            },
            separators=(",", ":"),
        ).encode("utf-8")
        if len(body) > MAX_FRAME_BYTES:
            raise ValueError("NKI request exceeds 16 MiB")

        with socket.create_connection(self.address, self.timeout_seconds) as connection:
            connection.sendall(struct.pack(">I", len(body)) + body)
            size = struct.unpack(">I", self._read_exact(connection, 4))[0]
            if size > MAX_FRAME_BYTES:
                raise ValueError("NKI response exceeds 16 MiB")
            response = json.loads(self._read_exact(connection, size))

        return self._parse_response(response, request_id)

    def health(self) -> dict[str, Any]:
        """Return the current kernel health payload."""

        return self.request("HealthCheck", {"deep": False})

    def submit(self, operation: dict[str, Any]) -> dict[str, Any]:
        """Submit a fully specified NEC operation through the kernel."""

        return self.request("SubmitWorkload", operation)

    def cancel(self, workload_id: str) -> dict[str, Any]:
        """Request cancellation of an active workload."""

        return self.request("CancelWorkload", {"workload_id": workload_id})

    def get_workload(self, workload_id: str) -> dict[str, Any]:
        """Return durable evidence for a completed workload."""

        return self.request("GetWorkload", {"workload_id": workload_id})

    def list_workloads(self, limit: int = 100) -> list[dict[str, Any]]:
        """List recent durable workload evidence."""

        if not 1 <= limit <= 1000:
            raise ValueError("limit must be between 1 and 1000")
        payload = self.request("ListWorkloads", {"limit": limit})
        return list(payload.get("workloads", []))

    def put_asset(
        self,
        kind: str,
        document: dict[str, Any],
        *,
        expected_generation: int | None = None,
    ) -> dict[str, Any]:
        """Create or generation-conditionally update a Runtime asset."""

        payload: dict[str, Any] = {
            "schema_version": 1,
            "kind": self._asset_kind(kind),
            "document": document,
        }
        if expected_generation is not None:
            if expected_generation < 1:
                raise ValueError("expected_generation must be positive")
            payload["expected_generation"] = expected_generation
        return self.request("PutControlAsset", payload)

    def get_asset(self, kind: str, asset_id: str) -> dict[str, Any]:
        """Get one Runtime asset by kind and ID."""

        return self.request(
            "GetControlAsset",
            {"schema_version": 1, "kind": self._asset_kind(kind), "id": asset_id},
        )

    def list_assets(self, kind: str) -> list[dict[str, Any]]:
        """List Runtime assets of one kind."""

        payload = self.request(
            "ListControlAssets",
            {"schema_version": 1, "kind": self._asset_kind(kind)},
        )
        return list(payload.get("assets", []))

    def delete_asset(self, kind: str, asset_id: str, expected_generation: int) -> None:
        """Delete a Runtime asset using generation compare-and-swap."""

        if expected_generation < 1:
            raise ValueError("expected_generation must be positive")
        self.request(
            "DeleteControlAsset",
            {
                "selector": {
                    "schema_version": 1,
                    "kind": self._asset_kind(kind),
                    "id": asset_id,
                },
                "expected_generation": expected_generation,
            },
        )

    def metrics(self) -> dict[str, Any]:
        """Return journal integrity and active-operation metrics."""

        return self.request("GetMetrics", {})

    @staticmethod
    def _asset_kind(kind: str) -> str:
        normalized = kind.strip().lower()
        if normalized not in CONTROL_ASSET_KINDS:
            raise ValueError(
                "asset kind must be project, model, graph, dataset, experiment, or artifact"
            )
        return normalized

    @staticmethod
    def _parse_response(response: Any, request_id: str) -> Any:
        if not isinstance(response, dict):
            raise TypeError("NKI response must be an object")
        if response.get("request_id") != request_id:
            raise ValueError("NKI response request_id does not match the request")
        if response.get("nki_version") != NKI_VERSION:
            raise ValueError("NKI response version is unsupported")
        status = response.get("status")
        if status == "error":
            error = response.get("error")
            if not isinstance(error, dict):
                raise ValueError("NKI error response has no structured error body")
            raise NkiError(
                str(error.get("code", "UNKNOWN")),
                str(error.get("message", "kernel request failed")),
                bool(error.get("retryable", False)),
                failed_phase=str(error.get("failed_phase", "")),
                cause=str(error.get("cause", "")),
                recommended_delay_ms=int(error.get("recommended_delay_ms", 0)),
            )
        if status != "success":
            raise ValueError("NKI response status is unsupported")
        return response.get("payload")

    @staticmethod
    def _read_exact(connection: socket.socket, size: int) -> bytes:
        chunks = bytearray()
        while len(chunks) < size:
            chunk = connection.recv(size - len(chunks))
            if not chunk:
                raise ConnectionError("NKI connection closed before the frame completed")
            chunks.extend(chunk)
        return bytes(chunks)
