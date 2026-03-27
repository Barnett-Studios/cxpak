---
name: setup
description: Set up the cxpak MCP server for the current project. Use when the user wants to enable cxpak tools or when cxpak MCP is not connected.
---

# cxpak Setup

Set up cxpak as an MCP server for this project. Run this command:

```bash
claude mcp add cxpak -- cxpak serve --mcp .
```

This adds cxpak to the project's MCP configuration. After running, use `/mcp` to verify the connection.

If cxpak is not installed, install it first:
```bash
brew tap Barnett-Studios/tap && brew install cxpak
```

Or via cargo:
```bash
cargo install cxpak
```
