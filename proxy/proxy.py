#!/usr/bin/env python3
"""
Transparent Anthropic API proxy that logs token usage.

Run this locally, then point VS Code Copilot's Anthropic base URL to
http://localhost:4001 instead of https://api.anthropic.com.

Logs each response's token usage to ~/.cctrack/proxy/YYYY-MM-DD.jsonl
so the cc-cost backend can track Copilot spend alongside Claude Code.
"""

import json
import os
import sys
import time
import uuid
from datetime import datetime, timezone
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError

UPSTREAM = os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com")
LISTEN_PORT = int(os.environ.get("PROXY_PORT", "4001"))
LOG_DIR = Path.home() / ".cctrack" / "proxy"

# Headers to forward from the client to Anthropic
FORWARD_HEADERS = {
    "anthropic-version",
    "content-type",
    "x-api-key",
    "anthropic-beta",
    "anthropic-dangerous-direct-browser-access",
}

# Headers NOT to forward back (hop-by-hop)
HOP_BY_HOP = {"transfer-encoding", "connection", "keep-alive"}


def ensure_log_dir():
    LOG_DIR.mkdir(parents=True, exist_ok=True)


def log_usage(model: str, usage: dict, request_id: str):
    """Append one JSONL line with token usage."""
    entry = {
        "request_id": request_id,
        "model": model or "unknown",
        "input_tokens": usage.get("input_tokens", 0),
        "output_tokens": usage.get("output_tokens", 0),
        "cache_creation_input_tokens": usage.get("cache_creation_input_tokens", 0),
        "cache_read_input_tokens": usage.get("cache_read_input_tokens", 0),
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "source": "copilot-proxy",
    }
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    log_path = LOG_DIR / f"{today}.jsonl"
    with open(log_path, "a") as f:
        f.write(json.dumps(entry) + "\n")


class ProxyHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length else b""

        # Check if this is a streaming request
        try:
            req_json = json.loads(body) if body else {}
        except json.JSONDecodeError:
            req_json = {}

        is_stream = req_json.get("stream", False)

        # Build upstream URL
        url = UPSTREAM.rstrip("/") + self.path

        # Build upstream request
        req = Request(url, data=body, method="POST")
        for header_name in FORWARD_HEADERS:
            value = self.headers.get(header_name)
            if value:
                req.add_header(header_name, value)

        try:
            resp = urlopen(req)
        except HTTPError as e:
            self.send_response(e.code)
            for key, val in e.headers.items():
                if key.lower() not in HOP_BY_HOP:
                    self.send_header(key, val)
            self.end_headers()
            self.wfile.write(e.read())
            return

        resp_body = resp.read()

        # Send response back to client
        self.send_response(resp.status)
        for key, val in resp.headers.items():
            if key.lower() not in HOP_BY_HOP:
                self.send_header(key, val)
        self.end_headers()
        self.wfile.write(resp_body)

        # Extract usage from response
        request_id = str(uuid.uuid4())

        if is_stream:
            # SSE stream: find the final message_delta or message_stop event
            self._log_stream_usage(resp_body, request_id)
        else:
            # Normal JSON response
            try:
                data = json.loads(resp_body)
                usage = data.get("usage", {})
                model = data.get("model", "")
                msg_id = data.get("id", request_id)
                if usage:
                    log_usage(model, usage, msg_id)
            except (json.JSONDecodeError, AttributeError):
                pass

    def _log_stream_usage(self, body: bytes, fallback_id: str):
        """Parse SSE stream body to extract final usage from message_delta."""
        text = body.decode("utf-8", errors="replace")
        model = ""
        final_usage = {}
        msg_id = fallback_id

        for line in text.split("\n"):
            line = line.strip()
            if not line.startswith("data: "):
                continue
            payload = line[6:]
            if payload == "[DONE]":
                break
            try:
                event = json.loads(payload)
            except json.JSONDecodeError:
                continue

            event_type = event.get("type", "")

            if event_type == "message_start":
                message = event.get("message", {})
                model = message.get("model", model)
                msg_id = message.get("id", msg_id)
                usage = message.get("usage", {})
                if usage:
                    final_usage = usage

            elif event_type == "message_delta":
                delta_usage = event.get("usage", {})
                if delta_usage:
                    # message_delta usage has output_tokens as cumulative
                    for k, v in delta_usage.items():
                        if isinstance(v, (int, float)):
                            final_usage[k] = final_usage.get(k, 0) + v

        if final_usage:
            log_usage(model, final_usage, msg_id)

    def do_GET(self):
        """Forward GET requests (e.g., /v1/models)."""
        url = UPSTREAM.rstrip("/") + self.path
        req = Request(url, method="GET")
        for header_name in FORWARD_HEADERS:
            value = self.headers.get(header_name)
            if value:
                req.add_header(header_name, value)

        try:
            resp = urlopen(req)
        except HTTPError as e:
            self.send_response(e.code)
            for key, val in e.headers.items():
                if key.lower() not in HOP_BY_HOP:
                    self.send_header(key, val)
            self.end_headers()
            self.wfile.write(e.read())
            return

        resp_body = resp.read()
        self.send_response(resp.status)
        for key, val in resp.headers.items():
            if key.lower() not in HOP_BY_HOP:
                self.send_header(key, val)
        self.end_headers()
        self.wfile.write(resp_body)

    def log_message(self, fmt, *args):
        """Quieter logging — only errors."""
        if args and str(args[0]).startswith("4") or str(args[0]).startswith("5"):
            super().log_message(fmt, *args)


def main():
    ensure_log_dir()
    server = HTTPServer(("127.0.0.1", LISTEN_PORT), ProxyHandler)
    print(f"Anthropic proxy listening on http://127.0.0.1:{LISTEN_PORT}")
    print(f"Forwarding to {UPSTREAM}")
    print(f"Logging usage to {LOG_DIR}/")
    print()
    print("Configure VS Code Copilot:")
    print(f'  "github.copilot.chat.models.anthropic.baseUrl": "http://127.0.0.1:{LISTEN_PORT}"')
    print()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopping proxy.")
        server.server_close()


if __name__ == "__main__":
    main()
