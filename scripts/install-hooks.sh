#!/bin/bash
# Install git hooks for cxpak development

HOOK_DIR=$(git rev-parse --git-dir)/hooks

cat > "$HOOK_DIR/pre-commit" << 'HOOK'
#!/bin/bash
set -e

# Format check
if ! cargo fmt -- --check 2>/dev/null; then
    echo "❌ cargo fmt failed. Run: cargo fmt"
    exit 1
fi

# Clippy
if ! cargo clippy --all-targets -- -D warnings 2>/dev/null; then
    echo "❌ clippy failed. Fix warnings above."
    exit 1
fi

# Tests
if ! cargo test --quiet 2>/dev/null; then
    echo "❌ Tests failed. Fix failing tests."
    exit 1
fi

echo "✅ Pre-commit checks passed (coverage enforced in CI at 90%)"
HOOK

chmod +x "$HOOK_DIR/pre-commit"
echo "✅ Pre-commit hook installed"
