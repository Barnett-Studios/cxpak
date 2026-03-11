#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    ENSURE_CXPAK="${SCRIPT_DIR}/../lib/ensure-cxpak"
    TEST_TMP="$(mktemp -d)"

    # Create a minimal git repo with a Rust file
    cd "${TEST_TMP}"
    git init -q
    git config user.email "test@test.com"
    git config user.name "Test"
    mkdir -p src
    cat > src/main.rs << 'RUST'
fn main() {
    let result = compute(21);
    println!("{}", result);
}

fn compute(x: i32) -> i32 {
    x * 2
}
RUST
    cat > Cargo.toml << 'TOML'
[package]
name = "test-project"
version = "0.1.0"
TOML
    git add -A
    git commit -q -m "initial"
}

teardown() {
    cd /
    rm -rf "${TEST_TMP}"
}

@test "ensure-cxpak resolves a binary" {
    run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [ -n "$output" ]
    # The output should be a path to an executable
    [ -x "$(echo "$output" | tail -1)" ]
}

@test "cxpak overview produces output via ensure-cxpak" {
    CXPAK="$("${ENSURE_CXPAK}")"
    cd "${TEST_TMP}"
    run "$CXPAK" overview --tokens 10k --format markdown .
    [ "$status" -eq 0 ]
    [[ "$output" == *"test-project"* ]] || [[ "$output" == *"main.rs"* ]]
}

@test "cxpak trace finds a symbol via ensure-cxpak" {
    CXPAK="$("${ENSURE_CXPAK}")"
    cd "${TEST_TMP}"
    run "$CXPAK" trace --tokens 10k compute .
    [ "$status" -eq 0 ]
    [[ "$output" == *"compute"* ]]
}

@test "cxpak diff shows no changes on clean repo" {
    CXPAK="$("${ENSURE_CXPAK}")"
    cd "${TEST_TMP}"
    run "$CXPAK" diff --tokens 10k .
    [ "$status" -eq 0 ]
    [[ "$output" == *"No changes"* ]]
}

@test "cxpak diff shows changes after modification" {
    CXPAK="$("${ENSURE_CXPAK}")"
    cd "${TEST_TMP}"
    echo "// new comment" >> src/main.rs
    run "$CXPAK" diff --tokens 10k .
    [ "$status" -eq 0 ]
    [[ "$output" == *"main.rs"* ]]
}

@test "cxpak clean removes .cxpak directory" {
    CXPAK="$("${ENSURE_CXPAK}")"
    cd "${TEST_TMP}"
    # First run overview to create .cxpak/
    "$CXPAK" overview --tokens 10k --format markdown . > /dev/null 2>&1
    [ -d ".cxpak" ]
    # Now clean
    run "$CXPAK" clean .
    [ "$status" -eq 0 ]
    [ ! -d ".cxpak" ]
}
