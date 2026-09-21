"""Minimal real service used by Reality Execution process E2E tests."""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


MODE_FILE = Path(sys.argv[2])


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - stdlib callback name
        mode = json.loads(MODE_FILE.read_text(encoding="utf-8"))
        if self.path == "/health":
            status = int(mode["status"])
            payload = {"status": status}
        elif self.path == "/version":
            status = 200
            payload = {"version": mode["version"]}
        else:
            status = 404
            payload = {"error": "not found"}
        body = json.dumps(payload, sort_keys=True).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format, *_args):
        return


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
