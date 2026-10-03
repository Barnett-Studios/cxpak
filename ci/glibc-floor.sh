#!/usr/bin/env bash
# .github#20 review: the GLIBC-floor regression gate, extracted to a script so it
# is testable in the regular CI workflow against a canned fixture — the gate had
# never actually been shown to fail. Also fixes the gate itself: `objdump -T` on
# the release runner's native (x86_64) objdump cannot read a foreign-architecture
# ELF (an aarch64 binary) and reports "file format not recognized" — the aarch64
# row would fail every release, every time, release.yml's job never publishing.
# `readelf` has no such restriction; it parses the ELF structures directly rather
# than dispatching to a per-architecture BFD backend the way objdump's -T does.
#
# Usage: ci/glibc-floor.sh <path-to-ELF-binary-or-a-saved-readelf-dump> <floor e.g. 2.28>
#
# The max required GLIBC_2.N is taken over BOTH halves `readelf -V --wide` prints:
# the per-symbol version table (.gnu.version) AND the version-needs section
# (.gnu.version_r) — the latter is where the actual Name: GLIBC_2.NN requirements
# live, but the grep below doesn't care which half a match came from, so a future
# readelf output-format change can't quietly make this gate blind to one half.
set -euo pipefail

input="${1:?usage: glibc-floor.sh <binary-or-readelf-dump> <floor>}"
floor="${2:?usage: glibc-floor.sh <binary-or-readelf-dump> <floor>}"
floor_minor="${floor#2.}"

# If `input` is a real ELF file, run readelf on it ourselves; otherwise treat it
# as an already-captured `readelf -V --wide` dump (what the CI self-test below
# feeds this script, with no binary and no readelf invocation needed at all).
if readelf -h "$input" >/dev/null 2>&1; then
  dump=$(readelf -V --wide "$input" 2>/dev/null)
else
  dump=$(cat "$input")
fi

max=$(printf '%s\n' "$dump" | grep -oE 'GLIBC_2\.[0-9]+' | sed 's/GLIBC_2\.//' | sort -n | tail -1)

if [ -z "$max" ]; then
  echo "::error::no GLIBC_2.x symbol found at all in $input — investigate before trusting this gate" >&2
  exit 1
fi
if [ "$max" -gt "$floor_minor" ]; then
  echo "::error::$input requires GLIBC_2.$max, above the declared floor 2.$floor_minor — a dependency or std now needs a newer symbol; raise the pin deliberately (and the README) or avoid the dependency" >&2
  exit 1
fi
echo "GLIBC floor ok: $input requires at most GLIBC_2.$max (floor 2.$floor_minor)"
