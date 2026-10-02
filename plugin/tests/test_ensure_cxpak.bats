#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    ENSURE_CXPAK="${SCRIPT_DIR}/../lib/ensure-cxpak"
    TEST_TMP="$(mktemp -d)"
    export CXPAK_INSTALL_DIR="${TEST_TMP}/install"
}

teardown() {
    rm -rf "${TEST_TMP}"
}

@test "returns path when cxpak is on PATH" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 0.4.0"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:${PATH}" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/cxpak" ]]
}

@test "returns cached binary if already downloaded" {
    mkdir -p "${CXPAK_INSTALL_DIR}"
    cat > "${CXPAK_INSTALL_DIR}/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 0.4.0"
SH
    chmod +x "${CXPAK_INSTALL_DIR}/cxpak"

    PATH="/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${CXPAK_INSTALL_DIR}/cxpak"* ]]
}

@test "detects Darwin arm64 platform correctly" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/uname" << 'SH'
#!/bin/sh
case "$1" in
    -s) echo "Darwin" ;;
    -m) echo "arm64" ;;
    *) echo "Darwin" ;;
esac
SH
    chmod +x "${TEST_TMP}/bin/uname"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"aarch64-apple-darwin"* ]]
}

@test "detects Linux x86_64 platform correctly" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/uname" << 'SH'
#!/bin/sh
case "$1" in
    -s) echo "Linux" ;;
    -m) echo "x86_64" ;;
    *) echo "Linux" ;;
esac
SH
    chmod +x "${TEST_TMP}/bin/uname"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"x86_64-unknown-linux-gnu"* ]]
}

@test "detects Darwin x86_64 platform correctly" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/uname" << 'SH'
#!/bin/sh
case "$1" in
    -s) echo "Darwin" ;;
    -m) echo "x86_64" ;;
    *) echo "Darwin" ;;
esac
SH
    chmod +x "${TEST_TMP}/bin/uname"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"x86_64-apple-darwin"* ]]
}

@test "detects Linux aarch64 platform correctly" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/uname" << 'SH'
#!/bin/sh
case "$1" in
    -s) echo "Linux" ;;
    -m) echo "aarch64" ;;
    *) echo "Linux" ;;
esac
SH
    chmod +x "${TEST_TMP}/bin/uname"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}" --dry-run
    [ "$status" -eq 0 ]
    [[ "$output" == *"aarch64-unknown-linux-gnu"* ]]
}

@test "fails on unsupported OS" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/uname" << 'SH'
#!/bin/sh
case "$1" in
    -s) echo "MINGW64_NT" ;;
    -m) echo "x86_64" ;;
    *) echo "MINGW64_NT" ;;
esac
SH
    chmod +x "${TEST_TMP}/bin/uname"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}" --dry-run
    [ "$status" -ne 0 ]
    [[ "$output" == *"Unsupported"* ]]
}

@test "accepts a newer patch version within the same major" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.2.2"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    # No brew on PATH: isolates the PATH-resolution comparison from the
    # auto-install fallback, per the #119 repro.
    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${TEST_TMP}/bin/cxpak"* ]]
}

@test "accepts a newer minor version within the same major" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.3.0"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${TEST_TMP}/bin/cxpak"* ]]
}

@test "rejects an older patch version within the same major" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.2.0"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a newer major version" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 4.0.0"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects an older major version" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 2.9.9"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a missing binary exactly as before" {
    PATH="/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a pre-release that is not an exact match" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.2.1-rc.1"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    # REQUIRED_VERSION is a plain 3.2.1 release, not the "3.2.1-rc.1"
    # pre-release string, so caret-range matching must not apply here —
    # only an exact string match would qualify, per Cargo's own
    # pre-release rule.
    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a pre-release of a different minor, caret range notwithstanding" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.3.0-rc.1"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a version string missing its patch component" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 3.3"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "prefers PATH binary over cached" {
    mkdir -p "${TEST_TMP}/bin"
    cat > "${TEST_TMP}/bin/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 0.4.0"
SH
    chmod +x "${TEST_TMP}/bin/cxpak"

    mkdir -p "${CXPAK_INSTALL_DIR}"
    cat > "${CXPAK_INSTALL_DIR}/cxpak" << 'SH'
#!/bin/sh
echo "cxpak 0.3.0"
SH
    chmod +x "${CXPAK_INSTALL_DIR}/cxpak"

    PATH="${TEST_TMP}/bin:${PATH}" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${TEST_TMP}/bin/cxpak"* ]]
}
