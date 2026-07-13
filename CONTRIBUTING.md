# Contributing to cxpak

Thanks for your interest in cxpak. It's a Rust CLI that indexes a codebase with
tree-sitter and produces token-budgeted context for LLMs. This guide covers how
to get set up, the quality bar every change has to clear, and the conventions
that keep the project consistent.

By contributing you agree that your contributions are licensed under the
project's [MIT license](LICENSE) (inbound = outbound).

## Ways to contribute

- **Report a bug** — use the [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) form.
- **Request a feature** — use the [feature request](.github/ISSUE_TEMPLATE/feature_request.yml) form.
- **Add a language** — cxpak supports 43 languages; adding one is a well-scoped
  contribution with a [dedicated template](.github/ISSUE_TEMPLATE/language_support.yml)
  and a recipe [below](#adding-a-language).
- **Improve docs** — the README, the ADRs, or this guide.
- **Fix a bug or land a feature** — open an issue first for anything non-trivial
  so the approach can be agreed before you invest the time.

## Development setup

cxpak is a standard Cargo project. You need a Rust toolchain at or above the
project's MSRV.

```bash
# Rust — MSRV is 1.91 (declared as rust-version in Cargo.toml)
rustup toolchain install stable
rustup component add rustfmt clippy

# Clone and build
git clone https://github.com/Barnett-Studios/cxpak
cd cxpak
cargo build

# Install the pre-commit hook (runs fmt + clippy + tests before every commit)
bash scripts/install-hooks.sh
```

`.mise.toml` pins a toolchain if you use [mise](https://mise.jdx.dev/); it's
optional — plain `rustup` + `cargo` is fully supported.

## The quality bar

Every PR has to pass the same gates CI enforces. Run them locally before pushing —
`scripts/install-hooks.sh` wires the first three into a pre-commit hook:

| Gate | Command | Requirement |
|---|---|---|
| Format | `cargo fmt -- --check` | clean |
| Lint | `cargo clippy --all-targets -- -D warnings` | zero warnings |
| Tests | `cargo test` | all pass |
| Coverage | tarpaulin (CI) | **≥ 90%** |
| MSRV | build on 1.91 | compiles |
| Bench gate | `cargo test --features bench --test bench_gate` | recall gate holds |

Tests are authored **together with** the code they cover — a feature PR without
tests won't pass the coverage gate, and a bug fix should come with a test that
fails before the fix and passes after.

### Feature flags

cxpak has several optional features — `visual`, `lsp`, `daemon`, `embeddings`,
`plugins`, `data-introspect` — on top of the per-language `lang-*` flags. A
change that touches feature-gated code can compile under the default set and
still break another combination. Verify the matrix before pushing:

```bash
bash scripts/feature-matrix.sh          # build + test every meaningful combo
bash scripts/feature-matrix.sh build    # build only (faster)
```

If your change needs a live database (the `data-introspect` MySQL/Postgres
path), its integration tests are `#[ignore]`d and read a DSN from
`CXPAK_MYSQL_DSN` / `CXPAK_PG_DSN`; run them with `-- --ignored` against a local
instance.

## Commit conventions

We use [Conventional Commits](https://www.conventionalcommits.org/) with an
optional scope. Keep the subject in the imperative and reference an ADR when one
governs the change:

```
feat(visual): risk treemap colours by percentile, not collapsed score (ADR-0195)
fix(schema): resolve item imports to file-level edges
deps: bump mysql_async 0.34 -> 0.37 (security: lru advisory)
docs: correct LSP method count in the README
test(intelligence): RED — within-repo risk percentile
```

Common types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `deps`,
`chore`. Keep commits logically scoped — the project preserves per-commit
history on merge, so a clean series reviews far better than one giant commit.

## Architecture Decision Records

Any architecturally significant choice — a new edge type, a scoring-signal
change, a surface (MCP/HTTP/LSP) addition, a distribution change — gets an ADR
in [`docs/adrs/`](docs/adrs/). Copy [`docs/adrs/template.md`](docs/adrs/template.md),
fill in Context / Options considered / Decision / Consequences / Revisit-if,
take the next number, and add it to [`docs/adrs/INDEX.md`](docs/adrs/INDEX.md).
Cite the ADR in the code comment and the commit that implements it.

You don't need an ADR for a bug fix, a doc tweak, or a dependency bump.

## Adding a language

The most common structured contribution. To add `foo`:

1. Add `tree-sitter-foo` to `Cargo.toml` as an **optional** dependency.
2. Add a feature flag `lang-foo = ["dep:tree-sitter-foo"]` and include it in the
   `default` feature set.
3. Map the file extension in `src/scanner/mod.rs` → `detect_language()`.
4. Create `src/parser/languages/foo.rs` implementing the `LanguageSupport` trait.
5. Register it in `src/parser/languages/mod.rs` and `src/parser/mod.rs`.
6. Add unit tests in the language file — cover functions, classes/types,
   imports, and exports for a small real snippet.

Then run `bash scripts/feature-matrix.sh` to confirm the flag composes cleanly.
Update the language count and list in the README in the same PR (docs land with
the code that changes them).

## Pull requests

1. Branch off `main` (fetch first — `main` moves).
2. Keep the PR focused; a smaller diff reviews faster and lands sooner.
3. Make sure `fmt`, `clippy`, `test`, and the feature matrix are green locally.
4. Fill in the PR template — the checklist mirrors the CI gates.
5. Update any documentation your change touches (README, ADRs) in the **same**
   PR. Changes to a documented surface land with their doc update.
6. Open the PR against `main`; CI runs the full gate set. A maintainer reviews
   and merges (preserving history — we don't squash).

## Reporting a security issue

Please **don't** open a public issue for a security vulnerability. Report it
privately via GitHub's [security advisory](https://github.com/Barnett-Studios/cxpak/security/advisories/new)
form so it can be fixed before disclosure.

---

Questions that don't fit an issue? Open a
[discussion](https://github.com/Barnett-Studios/cxpak/discussions). Thanks for
contributing.
