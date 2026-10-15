#!/usr/bin/env python3
"""Time from `cxpak serve --mcp <repo>` spawn until a tool call returns a ready answer.

Usage: mcp_ready.py <cxpak-binary> <repo> [timeout_s]
Prints: <repo> ready_s=<float> peak_rss_mb=<int>
"""
import json
import subprocess
import sys
import threading
import time

binary, repo = sys.argv[1], sys.argv[2]
timeout = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0

proc = subprocess.Popen(
    [binary, "serve", "--mcp", repo],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
)
peak = [0]
stop = threading.Event()


def sample_rss():
    while not stop.is_set():
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(proc.pid)], capture_output=True, text=True).stdout.strip()
        if out:
            peak[0] = max(peak[0], int(out))
        time.sleep(0.1)


def send(msg):
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def recv(want_id):
    while True:
        line = proc.stdout.readline()
        if not line:
            raise SystemExit("server exited")
        msg = json.loads(line)
        if msg.get("id") == want_id:
            return msg


index_ready = [None]


def watch_stderr():
    for line in proc.stderr:
        if "MCP index ready" in line and index_ready[0] is None:
            index_ready[0] = time.monotonic() - start


start = time.monotonic()
threading.Thread(target=sample_rss, daemon=True).start()
threading.Thread(target=watch_stderr, daemon=True).start()
try:
    send({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "mcp_ready", "version": "0"}}})
    recv(0)
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    i = 1
    while time.monotonic() - start < timeout:
        send({"jsonrpc": "2.0", "id": i, "method": "tools/call",
              "params": {"name": "cxpak_context", "arguments": {"op": "stats"}}})
        reply = json.dumps(recv(i))
        if '"indexing"' not in reply:
            first_answer = time.monotonic() - start
            while index_ready[0] is None and time.monotonic() - start < timeout:
                time.sleep(0.01)
            print(f"{repo} index_ready_s={index_ready[0]:.2f} first_answer_s={first_answer:.2f} peak_rss_mb={peak[0] // 1024}")
            break
        i += 1
        time.sleep(0.1)
    else:
        print(f"{repo} TIMEOUT after {timeout}s")
finally:
    stop.set()
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
