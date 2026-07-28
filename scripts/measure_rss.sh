#!/usr/bin/env bash
# Prints a timestamped RSS (MB) sample once per second until the process exits.
# Dirty-vs-live split: vmmap --summary <pid>  or  MALLOC_NANOZONE=0 leaks <pid>
set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <pid>" >&2
    exit 1
fi

PID="$1"

while kill -0 "$PID" 2>/dev/null; do
    RSS_KB=$(ps -o rss= -p "$PID" 2>/dev/null || true)
    if [[ -z "$RSS_KB" ]]; then
        break
    fi
    RSS_MB=$(( RSS_KB / 1024 ))
    printf '%s  pid=%s  rss=%s MB\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" "$PID" "$RSS_MB"
    sleep 1
done
