import socket
import unittest

from nous_kernel.client import NkiClient, NkiError


class ClientTests(unittest.TestCase):
    def test_structured_error_fields(self) -> None:
        error = NkiError("DENIED", "not allowed", retryable=False)
        self.assertEqual(error.code, "DENIED")
        self.assertFalse(error.retryable)
        self.assertEqual(str(error), "DENIED: not allowed")

    def test_read_exact_rejects_closed_connection(self) -> None:
        sender, receiver = socket.socketpair()
        sender.close()
        with receiver, self.assertRaises(ConnectionError):
            NkiClient._read_exact(receiver, 1)

    def test_response_identity_and_version_are_enforced(self) -> None:
        with self.assertRaisesRegex(ValueError, "request_id"):
            NkiClient._parse_response(
                {"request_id": "other", "nki_version": 2, "status": "success"},
                "expected",
            )
        with self.assertRaisesRegex(ValueError, "version"):
            NkiClient._parse_response(
                {"request_id": "expected", "nki_version": 99, "status": "success"},
                "expected",
            )

    def test_structured_response_error_metadata(self) -> None:
        response = {
            "request_id": "request-1",
            "nki_version": 2,
            "status": "error",
            "error": {
                "code": "PROVIDER_ERROR",
                "message": "provider failed",
                "retryable": True,
                "failed_phase": "provider",
                "cause": "provider-runtime",
                "recommended_delay_ms": 250,
            },
        }
        with self.assertRaises(NkiError) as caught:
            NkiClient._parse_response(response, "request-1")
        self.assertEqual(caught.exception.failed_phase, "provider")
        self.assertEqual(caught.exception.cause, "provider-runtime")
        self.assertEqual(caught.exception.recommended_delay_ms, 250)

    def test_control_asset_kind_and_generation_validation(self) -> None:
        client = NkiClient()
        self.assertEqual(client._asset_kind(" Project "), "project")
        with self.assertRaises(ValueError):
            client._asset_kind("unknown")
        with self.assertRaises(ValueError):
            client.list_workloads(0)
        with self.assertRaises(ValueError):
            client.delete_asset("project", "p", 0)


if __name__ == "__main__":
    unittest.main()
