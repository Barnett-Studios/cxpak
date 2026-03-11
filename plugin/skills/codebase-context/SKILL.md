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
