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
