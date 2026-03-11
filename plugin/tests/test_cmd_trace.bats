#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    CMD_FILE="${SCRIPT_DIR}/../commands/trace.md"
}

@test "command file exists" {
    [ -f "$CMD_FILE" ]
}

@test "has YAML frontmatter with description" {
    head -5 "$CMD_FILE" | grep -q "^description:"
}

@test "instructions reference ensure-cxpak" {
    grep -q "ensure-cxpak" "$CMD_FILE"
}

@test "instructions reference cxpak trace" {
    grep -q "cxpak trace" "$CMD_FILE"
}

@test "mentions symbol argument" {
    grep -qi "symbol" "$CMD_FILE"
}

@test "mentions --all flag" {
    grep -q "\-\-all" "$CMD_FILE"
}

@test "mentions default 50k budget" {
    grep -q "50k" "$CMD_FILE"
}
