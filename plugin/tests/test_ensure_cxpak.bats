#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    ENSURE_CXPAK="${SCRIPT_DIR}/../lib/ensure-cxpak"
    TEST_TMP="$(mktemp -d)"
    export CXPAK_INSTALL_DIR="${TEST_TMP}/install"
    # Fixtures are derived from the script's own REQUIRED_VERSION so a release bump
    # can't silently turn "newer"/"older" fixtures into the wrong side of the pin.
    REQ="$(sed -n 's/^REQUIRED_VERSION="\(.*\)"$/\1/p' "${ENSURE_CXPAK}")"
    IFS=. read -r MAJ MIN PAT <<< "${REQ}"
    NEWER_PATCH="${MAJ}.${MIN}.$((PAT + 1))"
    NEWER_MINOR="${MAJ}.$((MIN + 1)).0"
    if [ "${PAT}" -gt 0 ]; then OLDER="${MAJ}.${MIN}.$((PAT - 1))"
    elif [ "${MIN}" -gt 0 ]; then OLDER="${MAJ}.$((MIN - 1)).0"
    else OLDER="$((MAJ - 1)).0.0"; fi
}

# Write a fake cxpak at $1 that reports version $2.
fake_cxpak() {
    mkdir -p "$(dirname "$1")"
    printf '#!/bin/sh\necho "cxpak %s"\n' "$2" > "$1"
    chmod +x "$1"
}

teardown() {
    rm -rf "${TEST_TMP}"
}

@test "returns path when cxpak is on PATH" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${REQ}"

    PATH="${TEST_TMP}/bin:${PATH}" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/cxpak" ]]
}

@test "returns cached binary if already downloaded" {
    fake_cxpak "${CXPAK_INSTALL_DIR}/cxpak" "${REQ}"

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
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${NEWER_PATCH}"

    # No brew on PATH: isolates the PATH-resolution comparison from the
    # auto-install fallback, per the #119 repro.
    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${TEST_TMP}/bin/cxpak"* ]]
}

@test "accepts a newer minor version within the same major" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${NEWER_MINOR}"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${TEST_TMP}/bin/cxpak"* ]]
}

@test "rejects an older patch version within the same major" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${OLDER}"

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
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${REQ}-rc.1"

    # REQUIRED_VERSION is a plain release, not its "-rc.1" pre-release
    # pre-release string, so caret-range matching must not apply here —
    # only an exact string match would qualify, per Cargo's own
    # pre-release rule.
    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a pre-release of a different minor, caret range notwithstanding" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${NEWER_MINOR}-rc.1"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "rejects a version string missing its patch component" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${MAJ}.$((MIN + 1))"

    PATH="${TEST_TMP}/bin:/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"not found"* ]]
}

@test "prefers PATH binary over cached" {
    fake_cxpak "${TEST_TMP}/bin/cxpak" "${REQ}"

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
