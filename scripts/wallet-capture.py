#!/usr/bin/env python3
"""Receive one local OpenID4VP direct_post and print its diagnostic report."""
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import secrets
import subprocess
import sys
import time

MAX_BODY = 4 * 1024 * 1024


class CaptureServer(HTTPServer):
    def __init__(self, port, timeout, inspector, request_args):
        self.deadline = time.monotonic() + timeout
        self.inspector = inspector
        self.request_args = request_args
        self.response_path = "/response/" + secrets.token_urlsafe(24)
        self.report = None
        super().__init__(("127.0.0.1", port), CaptureHandler)

    def get_request(self):
        sock, addr = super().get_request()
        sock.settimeout(max(0.01, min(5, self.deadline - time.monotonic())))
        return sock, addr

    def capture(self):
        host, port = self.server_address
        print(json.dumps({"response_uri": f"http://{host}:{port}{self.response_path}"}), flush=True)
        while self.report is None:
            self.timeout = self.deadline - time.monotonic()
            if self.timeout <= 0:
                raise TimeoutError("no decodable Wallet response received")
            self.handle_request()
        return self.report


class CaptureHandler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        # Request paths, bodies and headers can contain session data.
        pass

    def reply(self, status):
        payload = b"{}"
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        server = self.server
        if self.path != server.response_path:
            self.reply(404)
            return
        if self.headers.get_content_type() != "application/x-www-form-urlencoded":
            self.reply(415)
            return
        lengths = self.headers.get_all("Content-Length", [])
        if self.headers.get("Transfer-Encoding") is not None or len(lengths) != 1:
            self.reply(400)
            return
        length = lengths[0]
        if not length.isascii() or not length.isdecimal() or len(length) > 10:
            self.reply(400)
            return
        size = int(length)
        if not 0 < size <= MAX_BODY:
            self.reply(413)
            return
        try:
            body = self.rfile.read(size)
            if len(body) != size:
                self.reply(400)
                return
            args = {**server.request_args, "direct_post_body": body.decode("utf-8")}
        except UnicodeDecodeError:
            self.reply(400)
            return
        remaining = server.deadline - time.monotonic()
        if remaining <= 0:
            self.reply(408)
            return
        try:
            result = subprocess.run(
                [server.inspector], input=json.dumps(args), text=True,
                capture_output=True, timeout=min(10, remaining), check=False,
            )
        except subprocess.TimeoutExpired:
            self.reply(504)
            return
        if result.returncode:
            self.reply(400)
            return
        server.report = json.loads(result.stdout)
        # Acknowledges receipt; the local report carries validation diagnostics.
        self.reply(200)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path, help="JSON with format, dcql_query and optional expectations")
    parser.add_argument("--inspector", type=Path, default=Path("target/debug/examples/presentation_compare"))
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=60, metavar="SECONDS")
    args = parser.parse_args()
    if not (0 <= args.port <= 65535 and 1 <= args.timeout <= 600):
        parser.error("port must be 0–65535 and timeout 1–600 seconds")
    try:
        inspector = args.inspector.resolve(strict=True)
        request_args = json.loads(args.request.read_text(encoding="utf-8"))
        if (
            not isinstance(request_args, dict)
            or request_args.get("format") not in ("sd-jwt", "mdoc")
            or not isinstance(request_args.get("dcql_query"), dict)
            or {"token", "vp_token", "expected_paths", "direct_post_body"} & request_args.keys()
        ):
            parser.error("request must provide format and dcql_query, with no presentation input")
        with CaptureServer(args.port, args.timeout, inspector, request_args) as server:
            print(json.dumps(server.capture()), flush=True)
    except (OSError, ValueError, TimeoutError) as error:
        print(f"wallet capture: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
