# Multi-File Output (Pack Mode) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When a repo exceeds the token budget, write full untruncated analysis to `.cxpak/` detail files alongside the budgeted overview, with omission markers that point to them.

**Architecture:** The render pipeline splits into two paths after section generation. If `index.total_tokens <= token_budget`, single-file mode (unchanged). Otherwise, pack mode: each section is rendered both budgeted (for the overview) and unbudgeted (for `.cxpak/` detail files). The overview's omission markers are rewritten to point to the detail files.

**Tech Stack:** Rust, std::fs for file I/O, existing OutputSections/render infrastructure.

---

### Task 1: Add `omission_pointer` to degrader

**Files:**
- Modify: `src/budget/degrader.rs`

**Context:** Currently `omission_marker` produces `<!-- section omitted: ~Nk tokens. Use --tokens Mk+ to include -->`. Pack mode needs a different marker: `<!-- full content: .cxpak/filename.md (~Nk tokens) -->`. Add a new function for this; don't change the existing one.

**Step 1: Write the failing test**

Add to `src/budget/degrader.rs` inside `mod tests`:

```rust
#[test]
fn test_omission_pointer() {
    let pointer = omission_pointer("signatures", "signatures.md", 39400);
    assert!(pointer.contains(".cxpak/signatures.md"));
    assert!(pointer.contains("~39.4k tokens"));
    assert!(pointer.contains("full content"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p cxpak degrader::tests::test_omission_pointer`
Expected: FAIL — `omission_pointer` not found.

**Step 3: Write minimal implementation**

Add to `src/budget/degrader.rs` above the `#[cfg(test)]` block:

```rust
pub fn omission_pointer(section: &str, filename: &str, omitted_tokens: usize) -> String {
    let display_tokens = if omitted_tokens >= 1000 {
        format!("~{:.1}k", omitted_tokens as f64 / 1000.0)
    } else {
        format!("~{}", omitted_tokens)
    };
    format!("<!-- {section} full content: .cxpak/{filename} ({display_tokens} tokens) -->")
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p cxpak degrader::tests::test_omission_pointer`
Expected: PASS

**Step 5: Commit**

```bash
git add src/budget/degrader.rs
git commit -m "feat: add omission_pointer for pack mode markers"
```

---

### Task 2: Add `truncate_to_budget_with_pointer` to degrader

**Files:**
- Modify: `src/budget/degrader.rs`

**Context:** The current `truncate_to_budget` uses `omission_marker`. Pack mode needs a variant that uses `omission_pointer` instead. Rather than adding a boolean flag to the existing function (ugly), add a new function that wraps the same truncation logic but uses the pointer-style marker.

**Step 1: Write the failing test**

Add to `src/budget/degrader.rs` inside `mod tests`:

```rust
#[test]
fn test_truncate_with_pointer() {
    let counter = crate::budget::counter::TokenCounter::new();
    let content = (0..100)
        .map(|i| format!("this is line number {} with some padding text", i))
        .collect::<Vec<_>>()
        .join("\n");
    let (result, _used, omitted) =
        truncate_to_budget_with_pointer(&content, 10, &counter, "module map", "modules.md");
    assert!(omitted > 0);
    assert!(result.contains(".cxpak/modules.md"));
    assert!(!result.contains("Use --tokens"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p cxpak degrader::tests::test_truncate_with_pointer`
Expected: FAIL — `truncate_to_budget_with_pointer` not found.

**Step 3: Write minimal implementation**

Add to `src/budget/degrader.rs`:

```rust
pub fn truncate_to_budget_with_pointer(
    content: &str,
    budget: usize,
    counter: &crate::budget::counter::TokenCounter,
    section_name: &str,
    detail_filename: &str,
) -> (String, usize, usize) {
    let total_tokens = counter.count(content);
    if total_tokens <= budget {
        return (content.to_string(), total_tokens, 0);
    }

    let mut lines = Vec::new();
    let mut used = 0;
    for line in content.lines() {
        let line_tokens = counter.count(line) + 1;
        if used + line_tokens > budget.saturating_sub(50) {
            break;
        }
        lines.push(line);
        used += line_tokens;
    }

    let omitted = total_tokens - used;
    let marker = omission_pointer(section_name, detail_filename, omitted);
    let mut truncated = lines.join("\n");
    truncated.push('\n');
    truncated.push_str(&marker);
    (truncated, used, omitted)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p cxpak degrader::tests::test_truncate_with_pointer`
Expected: PASS

**Step 5: Commit**

```bash
git add src/budget/degrader.rs
git commit -m "feat: add truncate_to_budget_with_pointer for pack mode"
```

---

### Task 3: Add `render_section_to_file` to output module

**Files:**
- Modify: `src/output/mod.rs`
- Modify: `src/output/markdown.rs`

**Context:** Detail files need to render a single section as a standalone document. Add a function that takes a section name and content, and renders it as a complete file in the chosen format. Start with markdown only — XML and JSON follow the same pattern.

**Step 1: Write the failing test**

Add to `src/output/markdown.rs` inside `mod tests`:

```rust
#[test]
fn test_render_single_section() {
    let content = "### src/main.rs\n- pub Function: `main`\n";
    let output = render_single_section("Module / Component Map", content);
    assert!(output.starts_with("## Module / Component Map"));
    assert!(output.contains("pub Function"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p cxpak output::markdown::tests::test_render_single_section`
Expected: FAIL — `render_single_section` not found.

**Step 3: Write minimal implementation**

Add to `src/output/markdown.rs`:

```rust
pub fn render_single_section(title: &str, content: &str) -> String {
    format!("## {title}\n\n{content}\n")
}
```

Add to `src/output/mod.rs`:

```rust
pub fn render_single_section(title: &str, content: &str, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => markdown::render_single_section(title, content),
        OutputFormat::Xml => xml::render_single_section(title, content),
        OutputFormat::Json => json::render_single_section(title, content),
    }
}
```

Add to `src/output/xml.rs`:

```rust
pub fn render_single_section(title: &str, content: &str) -> String {
    let tag = title.to_lowercase().replace([' ', '/'], "-");
    let mut out = String::from("<cxpak>\n");
    emit_section(&mut out, &tag, content);
    out.push_str("</cxpak>\n");
    out
}
```

Add to `src/output/json.rs`:

```rust
pub fn render_single_section(title: &str, content: &str) -> String {
    let key = title.to_lowercase().replace([' ', '/'], "_");
    let mut map = serde_json::Map::new();
    map.insert(key, serde_json::Value::String(content.to_string()));
    serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".into())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p cxpak output`
Expected: PASS

**Step 5: Commit**

```bash
git add src/output/mod.rs src/output/markdown.rs src/output/xml.rs src/output/json.rs
git commit -m "feat: add render_single_section for detail file output"
```

---

### Task 4: Add `ensure_gitignore_entry` utility

**Files:**
- Create: `src/util.rs`
- Modify: `src/main.rs` (add `mod util;`)

**Context:** Pack mode needs to append `.cxpak/` to the repo's `.gitignore` if not already present. Small standalone utility.

**Step 1: Write the failing test**

Create `src/util.rs` with:

```rust
use std::path::Path;

pub fn ensure_gitignore_entry(repo_root: &Path) -> std::io::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_creates_gitignore_with_cxpak() {
        let dir = TempDir::new().unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".cxpak/"));
    }

    #[test]
    fn test_appends_to_existing_gitignore() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("target/"));
        assert!(content.contains(".cxpak/"));
    }

    #[test]
    fn test_idempotent_if_already_present() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n.cxpak/\n").unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.matches(".cxpak/").count(), 1);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p cxpak util::tests`
Expected: FAIL — `todo!()` panics.

**Step 3: Write minimal implementation**

Replace the `todo!()` in `ensure_gitignore_entry`:

```rust
pub fn ensure_gitignore_entry(repo_root: &Path) -> std::io::Result<()> {
    let gitignore_path = repo_root.join(".gitignore");
    let entry = ".cxpak/";

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        if content.lines().any(|line| line.trim() == entry) {
            return Ok(());
        }
        let separator = if content.ends_with('\n') { "" } else { "\n" };
        std::fs::write(&gitignore_path, format!("{content}{separator}{entry}\n"))
    } else {
        std::fs::write(&gitignore_path, format!("{entry}\n"))
    }
}
```

Add `mod util;` to `src/main.rs` (alongside the other `mod` declarations).

**Step 4: Run tests to verify they pass**

Run: `cargo test -p cxpak util::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/util.rs src/main.rs
git commit -m "feat: add ensure_gitignore_entry utility for pack mode"
```

---

### Task 5: Refactor overview.rs render functions to return both budgeted and full content

**Files:**
- Modify: `src/commands/overview.rs:81-100`

**Context:** Currently each `render_*` function returns a single `String` (the budgeted version). For pack mode, we need both the budgeted content and the full (untruncated) content. The cleanest approach: render functions produce the full content first, then a separate step truncates for the overview. This means splitting each render function into "generate full content" + "truncate for budget."

Introduce a `SectionContent` struct that holds both versions:

```rust
struct SectionContent {
    budgeted: String,
    full: String,
    was_truncated: bool,
}
```

**Step 1: Add the struct and refactor `render_directory_tree`**

At the top of `src/commands/overview.rs`, add:

```rust
struct SectionContent {
    budgeted: String,
    full: String,
    was_truncated: bool,
}
```

Refactor `render_directory_tree` (currently at line 156) to:

```rust
fn render_directory_tree(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
) -> SectionContent {
    let mut full = String::new();
    for file in &index.files {
        full.push_str(&file.relative_path);
        full.push('\n');
    }

    if pack_mode {
        let (budgeted, _, omitted) = degrader::truncate_to_budget_with_pointer(
            &full, budget, counter, "directory tree", "tree.md",
        );
        SectionContent {
            was_truncated: omitted > 0,
            budgeted,
            full,
        }
    } else {
        let (budgeted, _, omitted) =
            degrader::truncate_to_budget(&full, budget, counter, "directory tree");
        SectionContent {
            was_truncated: omitted > 0,
            budgeted,
            full,
        }
    }
}
```

Apply the same pattern to ALL render functions: `render_module_map`, `render_dependency_graph`, `render_key_files`, `render_signatures`, `render_git_context`. Each generates the full content first, then truncates to budget, returning `SectionContent`.

For `render_key_files`, the full version should render all key files without any budget — just iterate and render each file's content in full.

**Step 2: Update the call site in `run()`**

Replace lines 82-100 in `overview.rs` with:

```rust
    let pack_mode = index.total_tokens > token_budget;

    let alloc = BudgetAllocation::allocate(token_budget);

    let metadata = render_metadata(&index, token_budget, pack_mode);
    let directory_tree = render_directory_tree(&index, alloc.directory_tree, &counter, pack_mode);
    let module_map = render_module_map(&index, alloc.module_map, &counter, pack_mode);
    let dependency_graph = render_dependency_graph(&index, alloc.dependency_graph, &counter, pack_mode);
    let key_files = render_key_files(&index, alloc.key_files, &counter, pack_mode);
    let signatures = render_signatures(&index, alloc.signatures, &counter, pack_mode);
    let git_context = render_git_context(path, alloc.git_context, &counter, pack_mode);

    let sections = OutputSections {
        metadata,
        directory_tree: directory_tree.budgeted,
        module_map: module_map.budgeted,
        dependency_graph: dependency_graph.budgeted,
        key_files: key_files.budgeted,
        signatures: signatures.budgeted,
        git_context: git_context.budgeted,
    };
```

Note: `render_metadata` changes to accept `token_budget` and `pack_mode` so it can add the "Detail files" and "Token budget" lines.

**Step 3: Run all tests**

Run: `cargo test -p cxpak`
Expected: All existing tests still PASS. The refactor doesn't change single-file behavior — when `pack_mode` is false, `SectionContent.budgeted` is identical to the old return value.

**Step 4: Commit**

```bash
git add src/commands/overview.rs
git commit -m "refactor: render functions return SectionContent with full+budgeted"
```

---

### Task 6: Write `.cxpak/` detail files in pack mode

**Files:**
- Modify: `src/commands/overview.rs` (the `run` function, after section rendering)

**Context:** After rendering sections, if `pack_mode` is true, write each truncated section's full content to `.cxpak/`. Use `output::render_single_section` from Task 3.

**Step 1: Write the integration test**

Add to `tests/overview_test.rs`:

```rust
/// Create a temp repo with enough content to exceed a tiny budget.
fn make_large_temp_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    // Create enough files to exceed a 500-token budget
    for i in 0..20 {
        let content = format!(
            "pub fn function_{i}(x: i32) -> i32 {{\n    x + {i}\n}}\n\npub fn helper_{i}() -> String {{\n    String::from(\"hello_{i}\")\n}}\n"
        );
        std::fs::write(src_dir.join(format!("mod_{i}.rs")), &content).unwrap();
    }

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"large\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# Large Test Project\n").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
        .unwrap();

    dir
}

#[test]
fn test_pack_mode_creates_cxpak_dir() {
    let repo = make_large_temp_repo();
    let out_file = repo.path().join("../overview.md");

    Command::cargo_bin("cxpak")
        .unwrap()
        .args([
            "overview",
            "--tokens",
            "500",
            "--out",
            out_file.to_str().unwrap(),
        ])
        .arg(repo.path())
        .assert()
        .success();

    // .cxpak/ directory should exist in the repo
    let cxpak_dir = repo.path().join(".cxpak");
    assert!(cxpak_dir.exists(), ".cxpak/ directory should be created");
    assert!(cxpak_dir.join("modules.md").exists() || cxpak_dir.join("signatures.md").exists(),
        "at least one detail file should exist");
}

#[test]
fn test_pack_mode_overview_has_pointers() {
    let repo = make_large_temp_repo();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".cxpak/"));
}

#[test]
fn test_pack_mode_gitignore_updated() {
    let repo = make_large_temp_repo();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success();

    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".cxpak/"));
}

#[test]
fn test_single_file_mode_no_cxpak_dir() {
    let repo = make_temp_repo();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success();

    let cxpak_dir = repo.path().join(".cxpak");
    assert!(!cxpak_dir.exists(), ".cxpak/ should NOT be created when repo fits in budget");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p cxpak test_pack_mode`
Expected: FAIL — `.cxpak/` not created, pointers not in output.

**Step 3: Write implementation**

Add to `src/commands/overview.rs`, in the `run` function, after the `let sections = OutputSections { ... };` block and before `let rendered = output::render(...)`:

```rust
    // 4b. Write detail files in pack mode
    if pack_mode {
        let cxpak_dir = path.join(".cxpak");
        std::fs::create_dir_all(&cxpak_dir)?;

        let detail_sections: &[(&str, &SectionContent, &str, &str)] = &[
            ("Directory Tree", &directory_tree, "tree", "tree.md"),
            ("Module / Component Map", &module_map, "modules", "modules.md"),
            ("Dependency Graph", &dependency_graph, "dependencies", "dependencies.md"),
            ("Key Files", &key_files, "key-files", "key-files.md"),
            ("Function / Type Signatures", &signatures, "signatures", "signatures.md"),
            ("Git Context", &git_context, "git", "git.md"),
        ];

        for (title, section, _slug, filename) in detail_sections {
            if section.was_truncated {
                let rendered_detail = output::render_single_section(title, &section.full, format);
                let detail_path = cxpak_dir.join(filename);
                std::fs::write(&detail_path, &rendered_detail)?;
                if verbose {
                    eprintln!("cxpak: wrote {}", detail_path.display());
                }
            }
        }

        crate::util::ensure_gitignore_entry(path)?;

        if verbose {
            eprintln!("cxpak: pack mode — detail files in {}", cxpak_dir.display());
        }
    }
```

**Step 4: Run all tests**

Run: `cargo test`
Expected: ALL tests pass — both old single-file tests and new pack mode tests.

**Step 5: Commit**

```bash
git add src/commands/overview.rs tests/overview_test.rs
git commit -m "feat: write .cxpak/ detail files in pack mode"
```

---

### Task 7: Add metadata pack mode fields

**Files:**
- Modify: `src/commands/overview.rs` (`render_metadata` function)

**Context:** In pack mode, metadata should include `- **Token budget:** 50k` and `- **Detail files:** \`.cxpak/\` (full untruncated analysis)`.

**Step 1: Write the test**

Add to `tests/overview_test.rs`:

```rust
#[test]
fn test_pack_mode_metadata_has_budget_and_detail_info() {
    let repo = make_large_temp_repo();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Token budget"))
        .stdout(predicate::str::contains("Detail files"));
}

#[test]
fn test_single_file_mode_no_detail_info() {
    let repo = make_temp_repo();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Detail files").not());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p cxpak test_pack_mode_metadata test_single_file_mode_no_detail`
Expected: FAIL

**Step 3: Implement**

Update `render_metadata` signature and body:

```rust
fn render_metadata(index: &CodebaseIndex, token_budget: usize, pack_mode: bool) -> String {
    let mut out = String::new();

    out.push_str(&format!("- **Files:** {}\n", index.total_files));
    out.push_str(&format!(
        "- **Total size:** {:.1} KB\n",
        index.total_bytes as f64 / 1024.0
    ));
    out.push_str(&format!(
        "- **Estimated tokens:** ~{}k\n",
        index.total_tokens / 1000
    ));

    if pack_mode {
        let budget_display = if token_budget >= 1000 {
            format!("{}k", token_budget / 1000)
        } else {
            format!("{}", token_budget)
        };
        out.push_str(&format!("- **Token budget:** {}\n", budget_display));
        out.push_str("- **Detail files:** `.cxpak/` (full untruncated analysis)\n");
    }

    if !index.language_stats.is_empty() {
        out.push_str("- **Languages:**\n");
        let mut langs: Vec<_> = index.language_stats.iter().collect();
        langs.sort_by(|a, b| b.1.file_count.cmp(&a.1.file_count));
        for (lang, stats) in &langs {
            let pct = if index.total_files > 0 {
                (stats.file_count as f64 / index.total_files as f64 * 100.0) as usize
            } else {
                0
            };
            out.push_str(&format!(
                "  - {} — {} files ({}%)\n",
                lang, stats.file_count, pct
            ));
        }
    }

    out
}
```

**Step 4: Run all tests**

Run: `cargo test`
Expected: ALL pass

**Step 5: Commit**

```bash
git add src/commands/overview.rs tests/overview_test.rs
git commit -m "feat: add token budget and detail files info to pack mode metadata"
```

---

### Task 8: Final validation — clippy, fmt, full test suite

**Files:**
- All modified files

**Step 1: Run cargo fmt**

Run: `cargo fmt`

**Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings. Fix any that appear.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass (existing + new).

**Step 4: Run against real repos**

Run: `cargo build --release`

Test single-file mode (small repo):
```bash
./target/release/cxpak overview --tokens 50k .
# Should NOT create .cxpak/
```

Test pack mode (large repo):
```bash
./target/release/cxpak overview --tokens 50k --out /tmp/meridian-overview.md ../meridian
ls ../meridian/.cxpak/
# Should see detail files: modules.md, signatures.md, etc.
cat /tmp/meridian-overview.md | head -20
# Should see "Detail files: .cxpak/" in metadata
# Should see ".cxpak/signatures.md" in omission pointers
```

**Step 5: Commit any final fixes**

```bash
git add -A
git commit -m "chore: fmt + clippy fixes for pack mode"
```
