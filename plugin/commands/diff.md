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
