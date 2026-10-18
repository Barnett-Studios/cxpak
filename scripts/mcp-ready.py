#!/usr/bin/env python3
"""Time from `cxpak serve --mcp <repo>` spawn until a tool call returns a ready answer.

Usage: mcp_ready.py <cxpak-binary> <repo> [timeout_s] [--lifecycle SECONDS]
Prints (no --lifecycle): <repo> index_ready_s=<float> first_answer_s=<float> peak_rss_mb=<int>
Prints (--lifecycle N):  <repo> index_ready_s=<float> first_answer_s=<float>
    build_peak_mb=<int>@<float>s post_ready_peak_mb=<int>@<float>s peak_mb=<int>

`--lifecycle SECONDS` keeps the RSS sampler running for SECONDS after "MCP index ready" instead
of exiting at Ready. cxpak's background phase (conventions republish, store save, embeddings) runs
after Ready and after this script's original exit point, so without `--lifecycle` a second, later
RSS peak in that phase is invisible to the gate that reads this script's output.
"""
import json
import subprocess
import sys
import threading
import time

args = sys.argv[1:]
lifecycle = 0.0
if "--lifecycle" in args:
    i = args.index("--lifecycle")
    lifecycle = float(args[i + 1])
    del args[i : i + 2]

binary, repo = args[0], args[1]
timeout = float(args[2]) if len(args) > 2 else 300.0

proc = subprocess.Popen(
    [binary, "serve", "--mcp", repo],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
)
samples = []  # list of (t_s, rss_mb)
stop = threading.Event()


def sample_rss():
    while not stop.is_set():
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(proc.pid)], capture_output=True, text=True).stdout.strip()
        if out:
            samples.append((time.monotonic() - start, int(out) // 1024))
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
            if lifecycle > 0:
                deadline = index_ready[0] + lifecycle if index_ready[0] is not None else first_answer + lifecycle
                while time.monotonic() - start < deadline:
                    time.sleep(0.05)
                boundary = index_ready[0] if index_ready[0] is not None else first_answer
                build = [rss for t, rss in samples if t <= boundary]
                post = [(t, rss) for t, rss in samples if t > boundary]
                build_peak = max(build) if build else 0
                if post:
                    post_t, post_peak = max(post, key=lambda p: p[1])
                else:
                    post_t, post_peak = boundary, 0
                overall_peak = max(rss for _, rss in samples) if samples else 0
                build_t = max((t for t, rss in samples if rss == build_peak and t <= boundary), default=boundary)
                print(
                    f"{repo} index_ready_s={index_ready[0]:.2f} first_answer_s={first_answer:.2f} "
                    f"build_peak_mb={build_peak}@{build_t:.2f}s "
                    f"post_ready_peak_mb={post_peak}@{post_t:.2f}s peak_mb={overall_peak}"
                )
            else:
                print(f"{repo} index_ready_s={index_ready[0]:.2f} first_answer_s={first_answer:.2f} "
                      f"peak_rss_mb={max((rss for _, rss in samples), default=0)}")
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
