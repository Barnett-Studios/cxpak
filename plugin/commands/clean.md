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

3. **Run `cxpak clean`:**
   ```bash
   "$CXPAK" clean <path>
   ```

4. **Confirm.** Tell the user: "Cleared `.cxpak/` cache and output files."
