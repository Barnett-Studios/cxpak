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
