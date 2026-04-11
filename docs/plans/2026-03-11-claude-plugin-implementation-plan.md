# cxpak Claude Code Plugin — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Claude Code plugin that exposes cxpak's codebase indexing as auto-invoked skills and user-invoked commands.

**Architecture:** The plugin is a directory of markdown files (skills, commands) plus a bash script (`ensure-cxpak`) that auto-downloads the cxpak binary if not on PATH. Skills are SKILL.md files with YAML frontmatter that Claude auto-invokes. Commands are `.md` files that Claude reads when the user types `/cxpak:<name>`. All files call `ensure-cxpak` before running cxpak.

**Tech Stack:** Bash (ensure-cxpak script), Markdown + YAML frontmatter (skills/commands), BATS (bash testing framework for shell script tests)

---

## Reference: Plugin Anatomy

Claude Code plugins have this structure:
```
plugin/
├── skills/<name>/SKILL.md    # Auto-invoked: YAML frontmatter (name, description) + markdown instructions
├── commands/<name>.md         # User-invoked via /plugin:name: YAML frontmatter (description) + markdown instructions
├── lib/                       # Shared scripts
├── LICENSE
└── README.md
```

**Skill SKILL.md format:**
```yaml
---
name: skill-name
description: "When to auto-invoke this skill"
---

# Skill Title

Instructions for Claude when this skill is triggered...
```

**Command .md format:**
```yaml
---
description: "What this command does"
---

Instructions for Claude when user invokes /plugin:command...
```

---

### Task 1: ensure-cxpak Script

**Files:**
- Create: `plugin/lib/ensure-cxpak`
- Create: `plugin/tests/test_ensure_cxpak.bats`

**Context:** This bash script is the foundation — every skill and command depends on it. It resolves the path to the `cxpak` binary, downloading it if necessary. It must be tested thoroughly since a broken download means the entire plugin fails.

**Step 1: Install BATS testing framework**

Run:
```bash
brew install bats-core
```

Verify:
```bash
bats --version
```

Expected: `Bats 1.x.x`

**Step 2: Write the failing tests**

Create `plugin/tests/test_ensure_cxpak.bats`:

```bash
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
    # Create a fake cxpak on PATH
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

    # Remove cxpak from PATH so it falls through to cached check
    PATH="/usr/bin:/bin" run "${ENSURE_CXPAK}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"${CXPAK_INSTALL_DIR}/cxpak"* ]]
}

@test "detects Darwin arm64 platform correctly" {
    # Mock uname to return Darwin + arm64
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

    # Run with mocked uname, no cxpak on PATH, no cached binary
    # Use --dry-run flag to just print the URL without downloading
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

@test "prefers PATH binary over cached" {
    # Create both a PATH binary and a cached binary
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
```

**Step 3: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_ensure_cxpak.bats
```

Expected: All tests FAIL (script doesn't exist yet)

**Step 4: Write the ensure-cxpak script**

Create `plugin/lib/ensure-cxpak`:

```bash
#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${CXPAK_INSTALL_DIR:-${HOME}/.claude/plugins/cxpak/bin}"
GITHUB_REPO="lyubomir-bozhinov/cxpak"
DRY_RUN=false

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=true ;;
    esac
done

# 1. Check PATH
if command -v cxpak >/dev/null 2>&1; then
    CXPAK_PATH="$(command -v cxpak)"
    if [ "$DRY_RUN" = false ]; then
        echo "$CXPAK_PATH"
    else
        echo "found on PATH: $CXPAK_PATH"
    fi
    exit 0
fi

# 2. Check cached install
if [ -x "${INSTALL_DIR}/cxpak" ]; then
    if [ "$DRY_RUN" = false ]; then
        echo "${INSTALL_DIR}/cxpak"
    else
        echo "found cached: ${INSTALL_DIR}/cxpak"
    fi
    exit 0
fi

# 3. Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
    Darwin)
        case "${ARCH}" in
            arm64)  TARGET="aarch64-apple-darwin" ;;
            x86_64) TARGET="x86_64-apple-darwin" ;;
            *) echo "Unsupported architecture: ${ARCH}" >&2; exit 1 ;;
        esac
        ;;
    Linux)
        case "${ARCH}" in
            aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
            x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
            *) echo "Unsupported architecture: ${ARCH}" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "Unsupported OS: ${OS}" >&2
        exit 1
        ;;
esac

TARBALL="cxpak-${TARGET}.tar.gz"

# Dry run: just print what would be downloaded
if [ "$DRY_RUN" = true ]; then
    echo "would download: ${TARBALL} for ${TARGET}"
    exit 0
fi

# 4. Download latest release
echo "cxpak not found. Downloading..." >&2

LATEST_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/${TARBALL}"

mkdir -p "${INSTALL_DIR}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

curl -fsSL "${LATEST_URL}" -o "${TMP_DIR}/${TARBALL}" 2>&1 >&2
tar -xzf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}" 2>&1 >&2
cp "${TMP_DIR}/cxpak" "${INSTALL_DIR}/cxpak"
chmod +x "${INSTALL_DIR}/cxpak"

echo "Downloaded cxpak to ${INSTALL_DIR}/cxpak" >&2
echo "${INSTALL_DIR}/cxpak"
```

**Step 5: Make executable and run tests**

Run:
```bash
chmod +x plugin/lib/ensure-cxpak
bats plugin/tests/test_ensure_cxpak.bats
```

Expected: All 8 tests PASS

**Step 6: Commit**

```bash
git add plugin/lib/ensure-cxpak plugin/tests/test_ensure_cxpak.bats
git commit -m "feat: add ensure-cxpak auto-download script with tests"
```

---

### Task 2: codebase-context Skill

**Files:**
- Create: `plugin/skills/codebase-context/SKILL.md`
- Create: `plugin/tests/test_skill_codebase_context.bats`

**Context:** This skill auto-triggers when Claude detects the user is asking about codebase structure. It runs `cxpak overview` and injects the result. The SKILL.md file contains both the YAML frontmatter (for discovery) and the full instructions (for execution).

**Step 1: Write the test**

Create `plugin/tests/test_skill_codebase_context.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    SKILL_FILE="${SCRIPT_DIR}/../skills/codebase-context/SKILL.md"
}

@test "skill file exists" {
    [ -f "$SKILL_FILE" ]
}

@test "has valid YAML frontmatter with name" {
    head -10 "$SKILL_FILE" | grep -q "^name: codebase-context$"
}

@test "has valid YAML frontmatter with description" {
    head -10 "$SKILL_FILE" | grep -q "^description:"
}

@test "description mentions codebase/architecture/structure" {
    description=$(sed -n '/^---$/,/^---$/p' "$SKILL_FILE" | grep "^description:")
    [[ "$description" == *"codebase"* ]] || [[ "$description" == *"architecture"* ]] || [[ "$description" == *"structure"* ]]
}

@test "instructions reference ensure-cxpak" {
    grep -q "ensure-cxpak" "$SKILL_FILE"
}

@test "instructions reference cxpak overview command" {
    grep -q "cxpak overview" "$SKILL_FILE"
}

@test "instructions mention default 50k budget" {
    grep -q "50k" "$SKILL_FILE"
}

@test "instructions tell Claude to ask for budget" {
    grep -qi "ask.*budget\|budget.*ask\|ask.*token" "$SKILL_FILE"
}

@test "instructions specify markdown format" {
    grep -q "\-\-format markdown" "$SKILL_FILE"
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_skill_codebase_context.bats
```

Expected: All tests FAIL (skill file doesn't exist)

**Step 3: Write the skill**

Create `plugin/skills/codebase-context/SKILL.md`:

```markdown
---
name: codebase-context
description: "Use when the user asks about codebase structure, architecture, what a project does, how components relate, or needs project-wide context to answer a question."
---

# Codebase Context via cxpak

When this skill is triggered, gather structured codebase context using cxpak before answering the user's question.

## Steps

1. **Ask for token budget.** Ask the user: "How large a context budget should I use? (default: 50k tokens)" If they don't specify, use `50k`.

2. **Resolve the cxpak binary.** Run the ensure-cxpak script to get the binary path:
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

3. **Run cxpak overview.** Execute:
   ```bash
   "$CXPAK" overview --tokens <budget> --format markdown .
   ```
   Where `<budget>` is what the user specified (e.g., `50k`, `100k`, `20k`).

4. **Use the output.** The command outputs a structured codebase summary including:
   - Project metadata (file counts, languages, tokens)
   - Directory tree
   - Module/component map with public symbols
   - Dependency graph (import relationships)
   - Key files (README, configs, manifests)
   - Function/type signatures
   - Git context (recent commits, churn)

5. **Answer the user's question** using the cxpak output as your primary source of codebase understanding.

6. **Mention the source.** Tell the user that cxpak provided the structured codebase context, e.g., "Based on the cxpak overview of this repo..."

## Important

- Always use `--format markdown` — it's native to your context window.
- If cxpak fails (not a git repo, no files found), fall back to standard file reading.
- The overview may include `.cxpak/` detail file pointers if the repo exceeds the budget — you can read those for deeper context.
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_skill_codebase_context.bats
```

Expected: All 9 tests PASS

**Step 5: Commit**

```bash
git add plugin/skills/codebase-context/SKILL.md plugin/tests/test_skill_codebase_context.bats
git commit -m "feat: add codebase-context skill"
```

---

### Task 3: diff-context Skill

**Files:**
- Create: `plugin/skills/diff-context/SKILL.md`
- Create: `plugin/tests/test_skill_diff_context.bats`

**Context:** This skill auto-triggers when Claude detects the user wants to review changes. It runs `cxpak diff` to get both the diff and surrounding dependency context.

**Step 1: Write the test**

Create `plugin/tests/test_skill_diff_context.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    SKILL_FILE="${SCRIPT_DIR}/../skills/diff-context/SKILL.md"
}

@test "skill file exists" {
    [ -f "$SKILL_FILE" ]
}

@test "has valid YAML frontmatter with name" {
    head -10 "$SKILL_FILE" | grep -q "^name: diff-context$"
}

@test "has valid YAML frontmatter with description" {
    head -10 "$SKILL_FILE" | grep -q "^description:"
}

@test "description mentions changes/review/PR" {
    description=$(sed -n '/^---$/,/^---$/p' "$SKILL_FILE" | grep "^description:")
    [[ "$description" == *"change"* ]] || [[ "$description" == *"review"* ]] || [[ "$description" == *"PR"* ]]
}

@test "instructions reference ensure-cxpak" {
    grep -q "ensure-cxpak" "$SKILL_FILE"
}

@test "instructions reference cxpak diff command" {
    grep -q "cxpak diff" "$SKILL_FILE"
}

@test "instructions mention default 50k budget" {
    grep -q "50k" "$SKILL_FILE"
}

@test "instructions mention git ref option" {
    grep -q "\-\-git-ref" "$SKILL_FILE"
}

@test "instructions tell Claude to ask for budget" {
    grep -qi "ask.*budget\|budget.*ask\|ask.*token" "$SKILL_FILE"
}

@test "instructions specify markdown format" {
    grep -q "\-\-format markdown" "$SKILL_FILE"
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_skill_diff_context.bats
```

Expected: All tests FAIL

**Step 3: Write the skill**

Create `plugin/skills/diff-context/SKILL.md`:

```markdown
---
name: diff-context
description: "Use when the user asks to review changes, understand what changed, prepare a PR description, or needs context about recent modifications."
---

# Diff Context via cxpak

When this skill is triggered, gather structured diff context with surrounding dependency information using cxpak before answering.

## Steps

1. **Ask for token budget.** Ask the user: "How large a context budget should I use for the diff? (default: 50k tokens)" If they don't specify, use `50k`.

2. **Ask for git ref (optional).** Ask: "Diff against which ref? (default: HEAD — shows uncommitted changes)" Common choices:
   - HEAD (default) — working tree changes
   - `main` — changes vs main branch
   - A specific commit SHA or tag

3. **Resolve the cxpak binary.** Run:
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

4. **Run cxpak diff.** Execute:
   ```bash
   "$CXPAK" diff --tokens <budget> --format markdown [--git-ref <ref>] .
   ```
   Omit `--git-ref` if the user wants the default (HEAD / working tree).

5. **Use the output.** The command outputs:
   - **Changes section** — actual diff hunks for each changed file
   - **Context section** — related files pulled in by dependency graph (callers, types, imports used by changed code)

   The diff is included first; remaining budget is filled with dependency context ordered by proximity to changed files.

6. **Answer the user's question** using the diff output. For code reviews, focus on the changes and use the context to understand impact. For PR descriptions, summarize what changed and why it matters.

7. **Mention the source.** Tell the user that cxpak provided the diff context, e.g., "Based on the cxpak diff analysis..."

## Important

- Always use `--format markdown`.
- If there are no changes, cxpak will print "No changes" — relay this to the user.
- For large diffs exceeding the budget, cxpak truncates least-important hunks while preserving the most-changed files.
- The `--all` flag can be added for full BFS traversal of the dependency graph (vs default 1-hop).
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_skill_diff_context.bats
```

Expected: All 10 tests PASS

**Step 5: Commit**

```bash
git add plugin/skills/diff-context/SKILL.md plugin/tests/test_skill_diff_context.bats
git commit -m "feat: add diff-context skill"
```

---

### Task 4: overview Command

**Files:**
- Create: `plugin/commands/overview.md`
- Create: `plugin/tests/test_cmd_overview.bats`

**Context:** This command is invoked when the user types `/cxpak:overview`. It asks for budget, runs the overview, and injects the result.

**Step 1: Write the test**

Create `plugin/tests/test_cmd_overview.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    CMD_FILE="${SCRIPT_DIR}/../commands/overview.md"
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

@test "instructions reference cxpak overview" {
    grep -q "cxpak overview" "$CMD_FILE"
}

@test "mentions default 50k budget" {
    grep -q "50k" "$CMD_FILE"
}

@test "supports path argument" {
    grep -qi "path\|directory\|argument" "$CMD_FILE"
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_cmd_overview.bats
```

Expected: All tests FAIL

**Step 3: Write the command**

Create `plugin/commands/overview.md`:

```markdown
---
description: "Run cxpak overview to get structured codebase context within a token budget"
---

# /cxpak:overview

Run a cxpak overview on the current project (or a specified path).

## Steps

1. **Parse arguments.** The user may pass a path as an argument (e.g., `/cxpak:overview /path/to/repo`). Default to `.` (current working directory).

2. **Ask for token budget.** Ask: "Token budget for the overview? (default: 50k)" Use their answer or default to `50k`.

3. **Resolve cxpak binary:**
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

4. **Run the overview:**
   ```bash
   "$CXPAK" overview --tokens <budget> --format markdown <path>
   ```

5. **Present the output** to the user. The overview contains project metadata, directory tree, module map, dependency graph, key files, signatures, and git context.

6. **Offer to dive deeper.** If the overview mentions truncated sections with `.cxpak/` detail files, offer to read them for more detail.
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_cmd_overview.bats
```

Expected: All 6 tests PASS

**Step 5: Commit**

```bash
git add plugin/commands/overview.md plugin/tests/test_cmd_overview.bats
git commit -m "feat: add /cxpak:overview command"
```

---

### Task 5: trace Command

**Files:**
- Create: `plugin/commands/trace.md`
- Create: `plugin/tests/test_cmd_trace.bats`

**Context:** Invoked via `/cxpak:trace <symbol>`. Traces a symbol through the dependency graph.

**Step 1: Write the test**

Create `plugin/tests/test_cmd_trace.bats`:

```bash
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
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_cmd_trace.bats
```

Expected: All tests FAIL

**Step 3: Write the command**

Create `plugin/commands/trace.md`:

```markdown
---
description: "Run cxpak trace to find a symbol and pack relevant code paths within a token budget"
---

# /cxpak:trace

Trace a symbol through the codebase dependency graph.

## Steps

1. **Parse arguments.** The user must provide a symbol name (e.g., `/cxpak:trace handle_request`). If no symbol is provided, ask for one. The user may also provide a path (default `.`) and `--all` for full BFS traversal.

2. **Ask for token budget.** Ask: "Token budget for the trace? (default: 50k)" Use their answer or default to `50k`.

3. **Resolve cxpak binary:**
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

4. **Run the trace:**
   ```bash
   "$CXPAK" trace --tokens <budget> --format markdown [--all] <symbol> <path>
   ```
   Add `--all` only if the user requested full graph traversal.

5. **Present the output.** The trace contains:
   - The target symbol and where it's defined
   - Files that use/call the symbol (dependencies)
   - Full source of relevant files, prioritized by dependency distance

6. **Explain what was found.** Summarize the symbol's role and how it connects to the rest of the codebase.
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_cmd_trace.bats
```

Expected: All 7 tests PASS

**Step 5: Commit**

```bash
git add plugin/commands/trace.md plugin/tests/test_cmd_trace.bats
git commit -m "feat: add /cxpak:trace command"
```

---

### Task 6: diff Command

**Files:**
- Create: `plugin/commands/diff.md`
- Create: `plugin/tests/test_cmd_diff.bats`

**Context:** Invoked via `/cxpak:diff`. Shows changes with dependency context.

**Step 1: Write the test**

Create `plugin/tests/test_cmd_diff.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    CMD_FILE="${SCRIPT_DIR}/../commands/diff.md"
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

@test "instructions reference cxpak diff" {
    grep -q "cxpak diff" "$CMD_FILE"
}

@test "mentions --git-ref option" {
    grep -q "\-\-git-ref" "$CMD_FILE"
}

@test "mentions default 50k budget" {
    grep -q "50k" "$CMD_FILE"
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_cmd_diff.bats
```

Expected: All tests FAIL

**Step 3: Write the command**

Create `plugin/commands/diff.md`:

```markdown
---
description: "Run cxpak diff to show changes with dependency context within a token budget"
---

# /cxpak:diff

Show what changed in the repo with surrounding dependency context.

## Steps

1. **Parse arguments.** The user may pass a path (default `.`) and `--git-ref <ref>`.

2. **Ask for token budget.** Ask: "Token budget for the diff? (default: 50k)" Default to `50k`.

3. **Ask for git ref (optional).** Ask: "Diff against which ref? (default: HEAD — shows uncommitted changes)" Common options: HEAD, main, a branch name, a commit SHA.

4. **Resolve cxpak binary:**
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

5. **Run the diff:**
   ```bash
   "$CXPAK" diff --tokens <budget> --format markdown [--git-ref <ref>] <path>
   ```
   Omit `--git-ref` for the default (HEAD / working tree changes).

6. **Present the output.** The diff contains:
   - **Changes** — actual diff hunks per file
   - **Context** — related files pulled in by dependency graph

7. **Offer analysis.** Based on the diff, offer to review the changes, suggest improvements, or help write a PR description.
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_cmd_diff.bats
```

Expected: All 6 tests PASS

**Step 5: Commit**

```bash
git add plugin/commands/diff.md plugin/tests/test_cmd_diff.bats
git commit -m "feat: add /cxpak:diff command"
```

---

### Task 7: clean Command

**Files:**
- Create: `plugin/commands/clean.md`
- Create: `plugin/tests/test_cmd_clean.bats`

**Context:** Invoked via `/cxpak:clean`. Clears the `.cxpak/` cache directory. Simplest command — no questions asked.

**Step 1: Write the test**

Create `plugin/tests/test_cmd_clean.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    CMD_FILE="${SCRIPT_DIR}/../commands/clean.md"
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

@test "instructions reference cxpak clean" {
    grep -q "cxpak clean" "$CMD_FILE"
}

@test "no budget question needed" {
    ! grep -qi "budget\|token" "$CMD_FILE"
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_cmd_clean.bats
```

Expected: All tests FAIL

**Step 3: Write the command**

Create `plugin/commands/clean.md`:

```markdown
---
description: "Clear the cxpak cache and output files"
---

# /cxpak:clean

Remove the `.cxpak/` directory (cache and output files) from the current project.

## Steps

1. **Parse arguments.** The user may pass a path (default `.`).

2. **Resolve cxpak binary:**
   ```bash
   CXPAK="$("${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak")"
   ```

3. **Run clean:**
   ```bash
   "$CXPAK" clean <path>
   ```

4. **Confirm.** Tell the user: "Cleared `.cxpak/` cache and output files."
```

**Step 4: Run tests**

Run:
```bash
bats plugin/tests/test_cmd_clean.bats
```

Expected: All 5 tests PASS

**Step 5: Commit**

```bash
git add plugin/commands/clean.md plugin/tests/test_cmd_clean.bats
git commit -m "feat: add /cxpak:clean command"
```

---

### Task 8: Plugin README and LICENSE

**Files:**
- Create: `plugin/README.md`
- Create: `plugin/LICENSE`
- Create: `plugin/tests/test_plugin_structure.bats`

**Context:** The README documents installation and usage. The LICENSE matches the cxpak repo (MIT). A structural test validates the plugin has all required files.

**Step 1: Write the structural test**

Create `plugin/tests/test_plugin_structure.bats`:

```bash
#!/usr/bin/env bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
    PLUGIN_DIR="${SCRIPT_DIR}/.."
}

@test "plugin has README.md" {
    [ -f "${PLUGIN_DIR}/README.md" ]
}

@test "plugin has LICENSE" {
    [ -f "${PLUGIN_DIR}/LICENSE" ]
}

@test "plugin has lib/ensure-cxpak" {
    [ -x "${PLUGIN_DIR}/lib/ensure-cxpak" ]
}

@test "plugin has skills/codebase-context/SKILL.md" {
    [ -f "${PLUGIN_DIR}/skills/codebase-context/SKILL.md" ]
}

@test "plugin has skills/diff-context/SKILL.md" {
    [ -f "${PLUGIN_DIR}/skills/diff-context/SKILL.md" ]
}

@test "plugin has commands/overview.md" {
    [ -f "${PLUGIN_DIR}/commands/overview.md" ]
}

@test "plugin has commands/trace.md" {
    [ -f "${PLUGIN_DIR}/commands/trace.md" ]
}

@test "plugin has commands/diff.md" {
    [ -f "${PLUGIN_DIR}/commands/diff.md" ]
}

@test "plugin has commands/clean.md" {
    [ -f "${PLUGIN_DIR}/commands/clean.md" ]
}

@test "ensure-cxpak is executable" {
    [ -x "${PLUGIN_DIR}/lib/ensure-cxpak" ]
}

@test "no hooks directory (design says no hooks)" {
    [ ! -d "${PLUGIN_DIR}/hooks" ]
}

@test "no agents directory (design says no agents)" {
    [ ! -d "${PLUGIN_DIR}/agents" ]
}
```

**Step 2: Run tests to verify they fail**

Run:
```bash
bats plugin/tests/test_plugin_structure.bats
```

Expected: README and LICENSE tests FAIL (others should pass from previous tasks)

**Step 3: Write README.md**

Create `plugin/README.md`:

```markdown
# cxpak — Claude Code Plugin

Structured codebase context for Claude Code, powered by [cxpak](https://github.com/lyubomir-bozhinov/cxpak).

## What It Does

- **Auto-context:** Claude automatically runs `cxpak overview` when you ask about codebase structure
- **Auto-diff:** Claude automatically runs `cxpak diff` when you ask to review changes
- **On-demand commands:** `/cxpak:overview`, `/cxpak:trace`, `/cxpak:diff`, `/cxpak:clean`

## Installation

### Prerequisites

cxpak is auto-downloaded on first use if not already installed. To install manually:

```bash
# Via Homebrew
brew tap lyubomir-bozhinov/tap
brew install cxpak

# Via cargo
cargo install cxpak
```

### Add the Plugin

Add this plugin to your Claude Code configuration. The plugin directory is `plugin/` within the cxpak repository.

## Skills (Auto-Invoked)

| Skill | Triggers When |
|-------|---------------|
| `codebase-context` | You ask about project structure, architecture, or how components relate |
| `diff-context` | You ask to review changes, prepare a PR description, or understand modifications |

## Commands (User-Invoked)

| Command | Description |
|---------|-------------|
| `/cxpak:overview` | Structured codebase summary |
| `/cxpak:trace <symbol>` | Trace a symbol through the dependency graph |
| `/cxpak:diff` | Changes with surrounding dependency context |
| `/cxpak:clean` | Clear cache and output files |

All commands ask for a token budget (default: 50k).

## License

MIT
```

**Step 4: Copy LICENSE from repo root**

Run:
```bash
cp LICENSE plugin/LICENSE
```

If no LICENSE file exists at repo root, create `plugin/LICENSE` with the MIT license text matching the `Cargo.toml` declaration.

**Step 5: Run tests**

Run:
```bash
bats plugin/tests/test_plugin_structure.bats
```

Expected: All 12 tests PASS

**Step 6: Run ALL tests to verify nothing broke**

Run:
```bash
bats plugin/tests/*.bats
```

Expected: All tests across all files PASS (8 + 9 + 10 + 6 + 7 + 6 + 5 + 12 = 63 tests)

**Step 7: Commit**

```bash
git add plugin/README.md plugin/LICENSE plugin/tests/test_plugin_structure.bats
git commit -m "feat: add plugin README, LICENSE, and structural tests"
```

---

### Task 9: Integration Test — End-to-End

**Files:**
- Create: `plugin/tests/test_integration.bats`

**Context:** Integration tests that actually run cxpak through the plugin's ensure-cxpak script against a real temp git repo. These verify the full pipeline works — ensure-cxpak resolves the binary, and cxpak commands produce expected output.

**Step 1: Write the integration tests**

Create `plugin/tests/test_integration.bats`:

```bash
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
```

**Step 2: Run integration tests**

Run:
```bash
bats plugin/tests/test_integration.bats
```

Expected: All 6 tests PASS (requires cxpak on PATH or auto-download working)

**Step 3: Commit**

```bash
git add plugin/tests/test_integration.bats
git commit -m "feat: add end-to-end integration tests for plugin"
```

---

### Task 10: Final Validation and Push

**Context:** Run all tests, verify the complete plugin, push to GitHub.

**Step 1: Run all plugin tests**

Run:
```bash
bats plugin/tests/*.bats
```

Expected: All tests PASS (63 unit + 6 integration = 69 total)

**Step 2: Run existing cxpak tests to ensure nothing broke**

Run:
```bash
cargo test --verbose
```

Expected: All 238 existing tests PASS

**Step 3: Push**

```bash
git push
```
