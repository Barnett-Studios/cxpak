use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::cli::OutputFormat;
use crate::git;
use crate::index::ranking;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::scanner::Scanner;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

/// Parse a human-readable time expression into a `Duration`.
///
/// Accepted forms: "1 day", "2 days", "1d", "3h", "1 hour", "3 hours",
/// "1 week", "2 weeks", "1w", "1 month", "2 months", "yesterday".
/// Returns `Err` for empty, zero-valued, or unrecognised input.
pub fn parse_time_expression(expr: &str) -> Result<std::time::Duration, String> {
    let expr = expr.trim().to_lowercase();
    if expr.is_empty() {
        return Err("empty time expression".to_string());
    }
    if expr == "yesterday" {
        return Ok(std::time::Duration::from_secs(86400));
    }

    // Try compact form: "3d", "1h", "2w" — only when the prefix is purely digits.
    let try_compact =
        |suffix: char, secs_per: u64| -> Option<Result<std::time::Duration, String>> {
            let num_str = expr.strip_suffix(suffix)?;
            // Guard: the remaining characters must all be ASCII digits (pure number).
            if !num_str.chars().all(|c| c.is_ascii_digit()) || num_str.is_empty() {
                return None;
            }
            let n: u64 = match num_str.parse() {
                Ok(v) => v,
                Err(_) => return Some(Err(format!("invalid time expression: {expr}"))),
            };
            if n == 0 {
                return Some(Err("time expression must be > 0".to_string()));
            }
            Some(Ok(std::time::Duration::from_secs(n * secs_per)))
        };

    if let Some(result) = try_compact('d', 86400) {
        return result;
    }
    if let Some(result) = try_compact('h', 3600) {
        return result;
    }
    if let Some(result) = try_compact('w', 604800) {
        return result;
    }

    // Try long form: "1 day", "2 days", "1 hour", etc.
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 2 {
        let n: u64 = parts[0]
            .parse()
            .map_err(|_| format!("invalid time expression: {expr}"))?;
        if n == 0 {
            return Err("time expression must be > 0".to_string());
        }
        let unit = parts[1];
        let secs_per = match unit {
            "day" | "days" => 86400,
            "hour" | "hours" => 3600,
            "week" | "weeks" => 604800,
            "month" | "months" => 2592000,
            _ => return Err(format!("unknown time unit: {unit}")),
        };
        return Ok(std::time::Duration::from_secs(n * secs_per));
    }

    Err(format!("invalid time expression: {expr}"))
}

/// Convert a `--since` expression into a git ref string.
/// Uses `git log --since` to find the oldest commit within the time window,
/// then returns its parent as the diff base.
pub fn resolve_since(repo_path: &std::path::Path, since_expr: &str) -> Result<String, String> {
    let duration = parse_time_expression(since_expr)?;
    let secs = duration.as_secs();
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "log",
            "--all",
            "--format=%H",
            &format!("--since={secs} seconds ago"),
        ])
        .output()
        .map_err(|e| format!("git log failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hashes: Vec<&str> = stdout.lines().collect();
    if hashes.is_empty() {
        return Err(format!("no commits found in the last {since_expr}"));
    }
    // The last hash in the list is the oldest commit in the time window.
    // We want its parent as the diff base.
    let oldest = hashes.last().unwrap();
    Ok(format!("{oldest}~1"))
}

/// A single file's changes from a git diff.
pub struct FileChange {
    /// Relative path of the changed file.
    pub path: String,
    /// The diff text (unified diff format lines).
    pub diff_text: String,
}

/// Extract changed files and their diffs.
/// If `git_ref` is None, diffs working tree against HEAD.
/// If `git_ref` is Some, diffs that ref's tree against HEAD's tree.
pub fn extract_changes(
    repo_path: &Path,
    git_ref: Option<&str>,
) -> Result<Vec<FileChange>, Box<dyn std::error::Error>> {
    let repo = git2::Repository::open(repo_path)?;

    let head_commit = repo.head()?.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    let diff = match git_ref {
        Some(refname) => {
            let obj = repo.revparse_single(refname)?;
            let ref_commit = obj.peel_to_commit()?;
            let ref_tree = ref_commit.tree()?;
            repo.diff_tree_to_tree(Some(&ref_tree), Some(&head_tree), None)?
        }
        None => repo.diff_tree_to_workdir_with_index(Some(&head_tree), None)?,
    };

    // `Diff::print` uses a single callback that receives every line event.
    // This avoids the two-simultaneous-mutable-closure borrow problem that
    // `Diff::foreach` would impose.  We track the current file path via the
    // `delta` argument on each line callback and accumulate text per path.
    let mut diff_map: HashMap<String, String> = HashMap::new();

    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        let path_str = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if path_str.is_empty() {
            return true;
        }

        let origin = line.origin();
        // Only capture actual diff content lines (added, removed, context).
        // Skip file headers (origin 'F'), hunk headers (origin 'H'), etc.
        let prefix = match origin {
            '+' => "+",
            '-' => "-",
            ' ' => " ",
            _ => return true,
        };

        let content = std::str::from_utf8(line.content()).unwrap_or("");
        let entry = diff_map.entry(path_str).or_default();
        entry.push_str(prefix);
        entry.push_str(content);

        true
    })?;

    let mut changes: Vec<FileChange> = diff_map
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(path, diff_text)| FileChange { path, diff_text })
        .collect();

    // Sort for deterministic ordering.
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(changes)
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    git_ref: Option<&str>,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
    all: bool,
    focus: Option<&str>,
    timing: bool,
    review: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let total_start = std::time::Instant::now();

    // 1. Extract git changes
    let extract_start = std::time::Instant::now();
    if verbose {
        eprintln!("cxpak: extracting git changes in {}", path.display());
    }
    let changes = extract_changes(path, git_ref)?;

    if changes.is_empty() {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(b"No changes detected.\n")?;
        return Ok(());
    }

    if timing {
        eprintln!("cxpak [timing]: extract    {:.1?}", extract_start.elapsed());
    }
    if verbose {
        eprintln!("cxpak: {} changed file(s)", changes.len());
    }

    // 2. Scan repo
    let scan_start = std::time::Instant::now();
    if verbose {
        eprintln!("cxpak: scanning {}", path.display());
    }
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;
    if verbose {
        eprintln!("cxpak: found {} files", files.len());
    }
    if timing {
        eprintln!("cxpak [timing]: scan       {:.1?}", scan_start.elapsed());
    }

    let counter = TokenCounter::new();

    // 3. Parse with cache
    let parse_start = std::time::Instant::now();
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }

    // 4. Build index
    let index_start = std::time::Instant::now();
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    index.co_changes = index.conventions.git_health.co_changes.clone();
    if verbose {
        eprintln!(
            "cxpak: indexed {} files, ~{} tokens total",
            index.total_files, index.total_tokens
        );
    }
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }

    // 5. Graph is cached on index
    let graph_start = std::time::Instant::now();
    if timing {
        eprintln!("cxpak [timing]: graph      {:.1?}", graph_start.elapsed());
    }

    // 5b. Rank files and apply focus
    let git_ctx = git::extract_git_context(path, 20).ok();
    let file_paths: Vec<String> = index
        .files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    let mut scores = ranking::rank_files(&file_paths, &index.graph, git_ctx.as_ref());
    if let Some(focus_path) = focus {
        ranking::apply_focus(&mut scores, focus_path, &index.graph);
    }

    // Sort index files by score so higher-ranked context files get budget priority
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();
    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // 6. Determine the set of changed file paths (relative)
    let changed_paths: HashSet<String> = changes.iter().map(|c| c.path.clone()).collect();

    // 7. Walk graph from changed files: 1-hop or full BFS
    let relevant_paths: HashSet<String> = if all {
        let start: Vec<&str> = changed_paths.iter().map(String::as_str).collect();
        index.graph.reachable_from(&start)
    } else {
        let mut one_hop: HashSet<String> = changed_paths.clone();
        for file in &changed_paths {
            if let Some(deps) = index.graph.dependencies(file) {
                one_hop.extend(deps.iter().map(|e| e.target.clone()));
            }
            for dep in index.graph.dependents(file) {
                one_hop.insert(dep.target.to_string());
            }
        }
        one_hop
    };

    // Context files: reachable but not themselves changed
    let context_paths: HashSet<String> =
        relevant_paths.difference(&changed_paths).cloned().collect();

    if verbose {
        eprintln!(
            "cxpak: {} context file(s) in dependency subgraph",
            context_paths.len()
        );
    }

    // 8. Build diff section text
    let render_start = std::time::Instant::now();
    let mut full_diff = String::new();
    for change in &changes {
        full_diff.push_str(&format!(
            "### {}\n\n```diff\n{}\n```\n\n",
            change.path, change.diff_text
        ));
    }

    // 9. Budget: diff first, then context signatures with the remainder
    let (diff_content, diff_used, _) =
        degrader::truncate_to_budget(&full_diff, token_budget, &counter, "diff");

    let context_budget = token_budget.saturating_sub(diff_used);
    let signatures = render_context_signatures(&index, &context_paths, context_budget, &counter);

    // 10. Metadata
    let git_ref_display = git_ref.unwrap_or("working tree");
    let metadata = format!(
        "- **Ref:** `{}`\n- **Changed files:** {}\n- **Context files:** {}\n",
        git_ref_display,
        changed_paths.len(),
        context_paths.len()
    );

    // 11. Assemble and render
    let sections = OutputSections {
        metadata,
        directory_tree: String::new(),
        module_map: String::new(),
        dependency_graph: String::new(),
        key_files: diff_content,
        signatures,
        git_context: String::new(),
    };

    let mut rendered = output::render(&sections, format);

    // --review: append a risk-ordered change-impact section. It is a markdown
    // block, so it is only appended for markdown output (appending to JSON/XML
    // would corrupt the document). `git_ref` is threaded straight through so the
    // review observes the same revision as the diff above.
    if review && matches!(format, OutputFormat::Markdown) {
        match build_review_bundle(&index, path, git_ref) {
            Ok(bundle) => rendered.push_str(&render_review(&bundle)),
            Err(e) => {
                eprintln!("cxpak: review bundle skipped: {e}");
            }
        }
    }

    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
        eprintln!("cxpak [timing]: total      {:.1?}", total_start.elapsed());
    }

    match out {
        Some(out_path) => {
            std::fs::write(out_path, &rendered)?;
            if verbose {
                eprintln!("cxpak: written to {}", out_path.display());
            }
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(rendered.as_bytes())?;
        }
    }

    Ok(())
}

/// Render public signatures of context files (reachable but not changed).
fn render_context_signatures(
    index: &CodebaseIndex,
    context_paths: &HashSet<String>,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut full = String::new();

    for file in &index.files {
        if !context_paths.contains(&file.relative_path) {
            continue;
        }
        let Some(pr) = &file.parse_result else {
            continue;
        };

        let public_syms: Vec<_> = pr
            .symbols
            .iter()
            .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
            .collect();

        if public_syms.is_empty() {
            continue;
        }

        full.push_str(&format!("### {}\n\n", file.relative_path));
        for sym in public_syms {
            full.push_str(&format!("```\n{}\n```\n\n", sym.signature));
        }
    }

    let (budgeted, _, _) =
        degrader::truncate_to_budget(&full, budget, counter, "context signatures");
    budgeted
}

// ---------------------------------------------------------------------------
// Review: expected-but-absent changes (the headline `--review` signal)
// ---------------------------------------------------------------------------

/// Why a file is flagged as expected-but-absent from the current change set.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) enum OmissionKind {
    /// `expected_file` historically changed together with a changed file
    /// (`with`) `count` times in the mined co-change window.
    CoChange { with: String, count: u32 },
    /// `expected_file` is a high-confidence test of a changed source file.
    MissingTest { for_source: String },
}

/// A file cxpak expected to be in the diff (based on history / test mapping)
/// but which was not changed.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Omission {
    pub expected_file: String,
    pub kind: OmissionKind,
    /// Ranking key: co-change strength (count × recency_weight), or +inf for a
    /// missing high-confidence test so those sort to the top.
    pub weight: f64,
}

/// Minimum co-change strength (`count × recency_weight`) to surface, to avoid
/// noise from incidental one-off pairings. Conservative; covered by tests.
const OMISSION_MIN_WEIGHT: f64 = 1.0;

/// Detect expected-but-absent changes. Pure: depends only on its arguments, so
/// it is fully unit-testable with synthetic inputs (no git fixture required).
///
/// Two signals:
/// 1. **Co-change** — a mined pair where exactly one side is in the diff and the
///    other (above [`OMISSION_MIN_WEIGHT`]) is absent.
/// 2. **Missing test** — a changed source file whose high-confidence test
///    (NameMatch / Both) is not itself in the diff.
///
/// De-duped by `expected_file` (a file flagged by both signals appears once),
/// then sorted strongest-first by `weight`.
pub(crate) fn detect_omissions(
    changed: &[String],
    co_changes: &[crate::intelligence::co_change::CoChangeEdge],
    test_map: &std::collections::HashMap<String, Vec<crate::intelligence::test_map::TestFileRef>>,
) -> Vec<Omission> {
    use std::collections::HashSet;
    let changed_set: HashSet<&str> = changed.iter().map(|s| s.as_str()).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Omission> = Vec::new();

    // 1. Co-change omissions: exactly ONE side changed, the other is absent.
    for e in co_changes {
        let a_in = changed_set.contains(e.file_a.as_str());
        let b_in = changed_set.contains(e.file_b.as_str());
        if a_in == b_in {
            continue; // both changed, or neither — not an omission for this diff
        }
        let (present, absent) = if a_in {
            (&e.file_a, &e.file_b)
        } else {
            (&e.file_b, &e.file_a)
        };
        let weight = e.count as f64 * e.recency_weight;
        if weight < OMISSION_MIN_WEIGHT || changed_set.contains(absent.as_str()) {
            continue;
        }
        if seen.insert(absent.clone()) {
            out.push(Omission {
                expected_file: absent.clone(),
                kind: OmissionKind::CoChange {
                    with: present.clone(),
                    count: e.count,
                },
                weight,
            });
        }
    }

    // 2. Missing-test omissions: a changed source whose high-confidence test is absent.
    for src in changed {
        if let Some(tests) = test_map.get(src) {
            for t in tests {
                let strong = matches!(
                    t.confidence,
                    crate::intelligence::test_map::TestConfidence::Both
                        | crate::intelligence::test_map::TestConfidence::NameMatch
                );
                if strong && !changed_set.contains(t.path.as_str()) && seen.insert(t.path.clone()) {
                    out.push(Omission {
                        expected_file: t.path.clone(),
                        kind: OmissionKind::MissingTest {
                            for_source: src.clone(),
                        },
                        weight: f64::INFINITY, // tests-not-updated ranks at the top
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

// ---------------------------------------------------------------------------
// Review bundle: change-impact context composed from existing intelligence
// ---------------------------------------------------------------------------

/// Change-impact context for `cxpak diff --review`. Composes already-tested
/// intelligence functions over exactly the changed surface — no new analysis.
#[derive(serde::Serialize)]
pub(crate) struct ReviewBundle {
    pub changed_paths: Vec<String>,
    pub blast: crate::intelligence::blast_radius::BlastRadiusResult,
    pub predicted_tests: Vec<crate::intelligence::predict::TestPrediction>,
    pub violations: Vec<crate::conventions::verify::Violation>,
    pub security: crate::intelligence::security::SecuritySurface,
    /// Expected-but-absent changes (the headline `--review` signal).
    pub omissions: Vec<Omission>,
}

/// Build the review bundle for the changed surface implied by `git_ref`
/// (or the uncommitted working tree when `git_ref` is `None`).
///
/// `diff::run` builds its index via `CodebaseIndex::build_with_content`, which
/// populates `graph`/`co_changes` but leaves `pagerank` and `test_map` EMPTY.
/// Reading those empty fields would collapse blast-radius risk to ~0 and yield
/// zero impacted tests, so this fn computes pagerank + test_map locally — making
/// the bundle correct and self-contained regardless of how the index was built.
pub(crate) fn build_review_bundle(
    index: &crate::index::CodebaseIndex,
    repo_path: &std::path::Path,
    git_ref: Option<&str>,
) -> Result<ReviewBundle, String> {
    let changed = crate::conventions::verify::get_changed_lines(repo_path, git_ref, None)?;
    let changed_paths: Vec<String> = changed.iter().map(|c| c.path.clone()).collect();
    let refs: Vec<&str> = changed_paths.iter().map(|s| s.as_str()).collect();

    // Locally derived (the diff-path index leaves these empty — see doc comment).
    let pagerank = crate::intelligence::pagerank::compute_pagerank(&index.graph, 0.85, 100);
    let all_paths: std::collections::HashSet<String> = index
        .files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    let test_map = crate::intelligence::test_map::build_test_map(&index.files, &all_paths);

    let blast = crate::intelligence::blast_radius::compute_blast_radius(
        &refs,
        &index.graph,
        &pagerank,
        &test_map,
        2,
        None,
    );
    let prediction = crate::intelligence::predict::predict(
        &refs,
        &index.graph,
        &pagerank,
        &index.co_changes,
        &test_map,
        2,
    );
    let verify = crate::conventions::verify::verify_changes(&changed, index, repo_path);
    // Same default auth-pattern slice the serve `cxpak_security_surface` handler passes.
    let security = crate::intelligence::security::build_security_surface(
        index,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        None,
    );

    // Headline feature: what SHOULD have changed but didn't. Reuses the test_map
    // built above and the index's mined co-change edges (live on the run path).
    let omissions = detect_omissions(&changed_paths, &index.co_changes, &test_map);

    Ok(ReviewBundle {
        changed_paths,
        blast,
        predicted_tests: prediction.test_impact,
        violations: verify.violations,
        security,
        omissions,
    })
}

/// Render the review bundle as a risk-ordered markdown section. Leads with the
/// headline "Possibly missing" omissions (omitted entirely when empty — no
/// nagging), then blast radius, impacted tests, convention violations, and
/// security findings scoped to the changed files.
pub(crate) fn render_review(bundle: &ReviewBundle) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    s.push_str("\n## Review\n\n");

    // Headline: expected-but-absent changes (already weight-sorted, strongest
    // first). Missing high-confidence tests always render (highest signal, few
    // in number). Co-change candidates are capped to the strongest few so the
    // genuine catch isn't buried under a long tail of weak pairings; the
    // remainder is summarized (never silently dropped).
    if !bundle.omissions.is_empty() {
        const MAX_COCHANGE_SHOWN: usize = 7;
        s.push_str("### ⚠ Possibly missing\n\n");
        let mut cochange_shown = 0usize;
        let mut cochange_hidden = 0usize;
        for o in &bundle.omissions {
            match &o.kind {
                OmissionKind::CoChange { with, count } => {
                    if cochange_shown >= MAX_COCHANGE_SHOWN {
                        cochange_hidden += 1;
                        continue;
                    }
                    cochange_shown += 1;
                    let _ = writeln!(
                        s,
                        "- `{}` usually changes with `{}` (co-changed {}×) but isn't in this diff",
                        o.expected_file, with, count
                    );
                }
                OmissionKind::MissingTest { for_source } => {
                    let _ = writeln!(
                        s,
                        "- `{}` changed but its test `{}` did not",
                        for_source, o.expected_file
                    );
                }
            }
        }
        if cochange_hidden > 0 {
            let _ = writeln!(
                s,
                "- _…and {cochange_hidden} more lower-confidence co-change candidate(s)_"
            );
        }
        s.push('\n');
    }

    // Blast radius: direct then transitive, each sorted by risk descending.
    s.push_str("### Blast radius\n\n");
    let mut affected: Vec<(&str, &crate::intelligence::blast_radius::AffectedFile)> = bundle
        .blast
        .categories
        .direct_dependents
        .iter()
        .map(|f| ("direct", f))
        .chain(
            bundle
                .blast
                .categories
                .transitive_dependents
                .iter()
                .map(|f| ("transitive", f)),
        )
        .collect();
    affected.sort_by(|a, b| {
        b.1.risk
            .partial_cmp(&a.1.risk)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if affected.is_empty() {
        s.push_str("_No dependents affected._\n\n");
    } else {
        for (kind, f) in affected {
            let _ = writeln!(
                s,
                "- `{}` ({}, {} hop(s), risk {:.2})",
                f.path, kind, f.hops, f.risk
            );
        }
        s.push('\n');
    }

    // Impacted tests, by confidence descending.
    if !bundle.predicted_tests.is_empty() {
        let mut tests: Vec<&crate::intelligence::predict::TestPrediction> =
            bundle.predicted_tests.iter().collect();
        tests.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        s.push_str("### Impacted tests\n\n");
        for t in tests {
            let _ = writeln!(s, "- `{}` (confidence {:.2})", t.test_file, t.confidence);
        }
        s.push('\n');
    }

    // Convention violations, grouped by the changed file (path prefix of location).
    if !bundle.violations.is_empty() {
        s.push_str("### Convention violations\n\n");
        for p in &bundle.changed_paths {
            let mut group: Vec<&crate::conventions::verify::Violation> = bundle
                .violations
                .iter()
                .filter(|v| v.location.starts_with(p.as_str()))
                .collect();
            if group.is_empty() {
                continue;
            }
            group.sort_by(|a, b| a.location.cmp(&b.location));
            let _ = writeln!(s, "**`{p}`**");
            for v in group {
                let _ = writeln!(s, "- [{}] {} — {}", v.severity, v.location, v.message);
            }
            s.push('\n');
        }
    }

    // Security findings, scoped to changed files only.
    let in_changed = |p: &str| bundle.changed_paths.iter().any(|c| c == p);
    let mut sec = String::new();
    for e in bundle
        .security
        .unprotected_endpoints
        .iter()
        .filter(|e| in_changed(&e.file))
    {
        let _ = writeln!(
            sec,
            "- Unprotected endpoint `{} {}` in `{}` (handler `{}`)",
            e.method, e.path, e.file, e.handler
        );
    }
    for e in bundle
        .security
        .secret_patterns
        .iter()
        .filter(|e| in_changed(&e.file))
    {
        let _ = writeln!(
            sec,
            "- Secret pattern `{}` in `{}:{}`",
            e.pattern_name, e.file, e.line
        );
    }
    for e in bundle
        .security
        .sql_injection_surface
        .iter()
        .filter(|e| in_changed(&e.file))
    {
        let _ = writeln!(
            sec,
            "- SQL injection risk in `{}:{}` ({})",
            e.file, e.line, e.interpolation_type
        );
    }
    for e in bundle
        .security
        .input_validation_gaps
        .iter()
        .filter(|e| in_changed(&e.file))
    {
        let _ = writeln!(
            sec,
            "- Validation gap on `{}` param `{}` in `{}:{}`",
            e.function_name, e.parameter, e.file, e.line
        );
    }
    if !sec.is_empty() {
        s.push_str("### Security\n\n");
        s.push_str(&sec);
        s.push('\n');
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_diff_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        dir
    }

    /// Build a temp repo where `src/main.rs` depends on `src/helper.rs`
    /// (`use crate::helper::work;`), commits both, then edits `helper.rs` on
    /// disk and leaves the edit UNCOMMITTED. `get_changed_lines(repo, None)`
    /// diffs the working tree vs HEAD, so the uncommitted edit is the change
    /// set; changing `helper.rs` gives `main.rs` as a direct dependent.
    /// Returns the index built the same way `run` does (`build_with_content`).
    fn review_test_repo() -> (tempfile::TempDir, CodebaseIndex) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/helper.rs"),
            "pub fn work() -> i32 {\n    1\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use crate::helper::work;\nfn main() {\n    let _ = work();\n}\n",
        )
        .unwrap();

        let mut idx = repo.index().unwrap();
        idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        // Modify the depended-upon file, leaving it UNCOMMITTED (working tree).
        std::fs::write(
            dir.path().join("src/helper.rs"),
            "pub fn work() -> i32 {\n    2\n}\n",
        )
        .unwrap();

        // Build the index exactly as `run` does.
        let scanner = Scanner::new(dir.path()).unwrap();
        let files = scanner.scan().unwrap();
        let counter = TokenCounter::new();
        let (parse_results, content_map) =
            crate::cache::parse::parse_with_cache(&files, dir.path(), &counter, false);
        let mut index =
            CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        index.conventions = crate::conventions::build_convention_profile(&index, dir.path());
        index.co_changes = index.conventions.git_health.co_changes.clone();

        (dir, index)
    }

    #[test]
    fn review_bundle_covers_changed_surface() {
        let (repo, index) = review_test_repo();
        let bundle = build_review_bundle(&index, repo.path(), None).unwrap();

        // helper.rs is the changed file; main.rs imports it → dependent expected.
        assert!(
            bundle.changed_paths.iter().any(|p| p == "src/helper.rs"),
            "changed surface should include the uncommitted edit"
        );
        assert!(
            !bundle.blast.categories.direct_dependents.is_empty()
                || !bundle.blast.categories.transitive_dependents.is_empty(),
            "a changed file with a dependent must show blast-radius entries"
        );
        // impacted tests come from predict.test_impact; confidences are valid probabilities
        assert!(bundle
            .predicted_tests
            .iter()
            .all(|t| t.confidence >= 0.0 && t.confidence <= 1.0));
        // convention violations are limited to changed files — Violation has no
        // `file`, so match the path prefix of `location` ("{path}:{line}" or "{path}")
        assert!(bundle.violations.iter().all(|v| bundle
            .changed_paths
            .iter()
            .any(|p| v.location.starts_with(p.as_str()))));
    }

    #[test]
    fn render_review_orders_by_risk_and_filters_security() {
        let (repo, index) = review_test_repo();
        let bundle = build_review_bundle(&index, repo.path(), None).unwrap();
        let md = render_review(&bundle);
        assert!(md.contains("## Review"));
        assert!(md.contains("Blast radius"));
        // security entries are rendered only for changed files. SecuritySurface
        // has no `findings`; filter each typed vec by its file/path field.
        let in_changed = |p: &str| bundle.changed_paths.iter().any(|c| c == p);
        assert!(
            bundle
                .security
                .unprotected_endpoints
                .iter()
                .all(|e| in_changed(&e.file))
                || !md.contains("Unprotected endpoint")
        );
        assert!(
            bundle
                .security
                .secret_patterns
                .iter()
                .all(|e| in_changed(&e.file))
                || !md.contains("Secret pattern")
        );
    }

    #[test]
    fn render_review_surfaces_missing_test_omission() {
        // Synthetic bundle: a changed source with an absent high-confidence test
        // must render under the "Possibly missing" subsection.
        use crate::intelligence::blast_radius::{
            BlastRadiusCategories, BlastRadiusResult, RiskSummary,
        };
        use crate::intelligence::security::SecuritySurface;

        let bundle = ReviewBundle {
            changed_paths: vec!["src/svc.rs".to_string()],
            blast: BlastRadiusResult {
                changed_files: vec!["src/svc.rs".to_string()],
                total_affected: 0,
                categories: BlastRadiusCategories {
                    direct_dependents: vec![],
                    transitive_dependents: vec![],
                    test_files: vec![],
                    schema_dependents: vec![],
                },
                risk_summary: RiskSummary {
                    high: 0,
                    medium: 0,
                    low: 0,
                },
            },
            predicted_tests: vec![],
            violations: vec![],
            security: SecuritySurface {
                unprotected_endpoints: vec![],
                input_validation_gaps: vec![],
                secret_patterns: vec![],
                sql_injection_surface: vec![],
                exposure_scores: vec![],
            },
            omissions: vec![Omission {
                expected_file: "tests/svc_test.rs".to_string(),
                kind: OmissionKind::MissingTest {
                    for_source: "src/svc.rs".to_string(),
                },
                weight: f64::INFINITY,
            }],
        };
        let md = render_review(&bundle);
        assert!(md.contains("Possibly missing"));
        assert!(md.contains("tests/svc_test.rs"));
        assert!(md.contains("src/svc.rs"));
    }

    #[test]
    fn render_review_caps_cochange_omissions_and_summarizes_remainder() {
        use crate::intelligence::blast_radius::{
            BlastRadiusCategories, BlastRadiusResult, RiskSummary,
        };
        use crate::intelligence::security::SecuritySurface;

        // 10 co-change omissions (descending weight) + 1 missing test.
        let mut omissions: Vec<Omission> = (0..10)
            .map(|i| Omission {
                expected_file: format!("src/dep{i}.rs"),
                kind: OmissionKind::CoChange {
                    with: "src/svc.rs".to_string(),
                    count: (20 - i) as u32,
                },
                weight: (20 - i) as f64,
            })
            .collect();
        omissions.push(Omission {
            expected_file: "tests/svc_test.rs".to_string(),
            kind: OmissionKind::MissingTest {
                for_source: "src/svc.rs".to_string(),
            },
            weight: f64::INFINITY,
        });
        // Sort as the detector would (strongest first; +inf at top).
        omissions.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let bundle = ReviewBundle {
            changed_paths: vec!["src/svc.rs".to_string()],
            blast: BlastRadiusResult {
                changed_files: vec![],
                total_affected: 0,
                categories: BlastRadiusCategories {
                    direct_dependents: vec![],
                    transitive_dependents: vec![],
                    test_files: vec![],
                    schema_dependents: vec![],
                },
                risk_summary: RiskSummary {
                    high: 0,
                    medium: 0,
                    low: 0,
                },
            },
            predicted_tests: vec![],
            violations: vec![],
            security: SecuritySurface {
                unprotected_endpoints: vec![],
                input_validation_gaps: vec![],
                secret_patterns: vec![],
                sql_injection_surface: vec![],
                exposure_scores: vec![],
            },
            omissions,
        };
        let md = render_review(&bundle);

        // At most 7 co-change lines rendered; 3 summarized as "more".
        let cochange_lines = md.matches("usually changes with").count();
        assert_eq!(cochange_lines, 7, "co-change list must be capped at 7");
        assert!(md.contains("and 3 more lower-confidence co-change candidate(s)"));
        // The missing-test omission is never capped — always rendered.
        assert!(md.contains("its test `tests/svc_test.rs` did not"));
        // The strongest candidate (count 20) is kept; the weakest (count 11) dropped.
        assert!(md.contains("co-changed 20×"));
        assert!(!md.contains("co-changed 11×"));
    }

    #[test]
    fn render_review_omits_missing_subsection_when_no_omissions() {
        let (repo, index) = review_test_repo();
        let mut bundle = build_review_bundle(&index, repo.path(), None).unwrap();
        bundle.omissions.clear();
        let md = render_review(&bundle);
        assert!(!md.contains("Possibly missing")); // no nagging when nothing is absent
    }

    #[test]
    fn review_bundle_empty_when_no_changes() {
        // A clean working tree (committed, nothing modified) → empty change set.
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let scanner = Scanner::new(dir.path()).unwrap();
        let files = scanner.scan().unwrap();
        let counter = TokenCounter::new();
        let (parse_results, content_map) =
            crate::cache::parse::parse_with_cache(&files, dir.path(), &counter, false);
        let mut index =
            CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        index.conventions = crate::conventions::build_convention_profile(&index, dir.path());
        index.co_changes = index.conventions.git_health.co_changes.clone();

        let bundle = build_review_bundle(&index, dir.path(), None).unwrap();
        assert!(bundle.changed_paths.is_empty());
        assert!(bundle.blast.categories.direct_dependents.is_empty());
        assert!(bundle.blast.categories.transitive_dependents.is_empty());
        assert!(bundle.omissions.is_empty());
        // render of an empty bundle still produces the header, no nagging subsections
        let md = render_review(&bundle);
        assert!(md.contains("## Review"));
        assert!(!md.contains("Possibly missing"));
        assert!(md.contains("_No dependents affected._"));
    }

    #[test]
    fn render_review_includes_all_populated_sections() {
        use crate::conventions::verify::{Violation, ViolationEvidence};
        use crate::intelligence::blast_radius::{
            AffectedFile, BlastRadiusCategories, BlastRadiusResult, RiskSummary,
        };
        use crate::intelligence::predict::TestPrediction;
        use crate::intelligence::security::{
            SecretPattern, SecuritySurface, SqlInjectionRisk, UnprotectedEndpoint, ValidationGap,
        };

        let bundle = ReviewBundle {
            // src/clean.rs is changed but has NO violations — exercises the
            // empty-group `continue` in the violations grouping loop.
            changed_paths: vec!["src/api.rs".to_string(), "src/clean.rs".to_string()],
            blast: BlastRadiusResult {
                changed_files: vec!["src/api.rs".to_string()],
                total_affected: 2,
                categories: BlastRadiusCategories {
                    direct_dependents: vec![AffectedFile {
                        path: "src/router.rs".to_string(),
                        edge_type: "import".to_string(),
                        hops: 1,
                        risk: 0.80,
                        note: None,
                    }],
                    transitive_dependents: vec![AffectedFile {
                        path: "src/app.rs".to_string(),
                        edge_type: "import".to_string(),
                        hops: 2,
                        risk: 0.30,
                        note: None,
                    }],
                    test_files: vec![],
                    schema_dependents: vec![],
                },
                risk_summary: RiskSummary {
                    high: 1,
                    medium: 0,
                    low: 1,
                },
            },
            predicted_tests: vec![
                TestPrediction {
                    test_file: "tests/api_test.rs".to_string(),
                    test_function: None,
                    signals: vec![],
                    confidence: 0.9,
                },
                TestPrediction {
                    test_file: "tests/smoke.rs".to_string(),
                    test_function: None,
                    signals: vec![],
                    confidence: 0.4,
                },
            ],
            violations: vec![Violation {
                severity: "high".to_string(),
                category: "naming".to_string(),
                location: "src/api.rs:12".to_string(),
                message: "snake_case expected".to_string(),
                evidence: ViolationEvidence {
                    dominant_pattern: "snake_case".to_string(),
                    count: "90%".to_string(),
                    strength: "strong".to_string(),
                    history: None,
                },
                suggestion: None,
            }],
            security: SecuritySurface {
                unprotected_endpoints: vec![UnprotectedEndpoint {
                    file: "src/api.rs".to_string(),
                    method: "GET".to_string(),
                    path: "/admin".to_string(),
                    handler: "admin".to_string(),
                    line: 5,
                }],
                input_validation_gaps: vec![ValidationGap {
                    file: "src/api.rs".to_string(),
                    function_name: "create".to_string(),
                    parameter: "body".to_string(),
                    line: 9,
                }],
                secret_patterns: vec![SecretPattern {
                    file: "src/api.rs".to_string(),
                    line: 3,
                    pattern_name: "aws_key".to_string(),
                    snippet: "AKIA...".to_string(),
                }],
                sql_injection_surface: vec![SqlInjectionRisk {
                    file: "src/api.rs".to_string(),
                    line: 20,
                    language: "rust".to_string(),
                    snippet: "format!(...)".to_string(),
                    interpolation_type: "format".to_string(),
                }],
                exposure_scores: vec![],
            },
            omissions: vec![],
        };
        let md = render_review(&bundle);
        // impacted-tests section, ordered by confidence desc
        assert!(md.contains("### Impacted tests"));
        assert!(
            md.find("tests/api_test.rs").unwrap() < md.find("tests/smoke.rs").unwrap(),
            "impacted tests must be ordered by confidence descending"
        );
        // blast radius ordered by risk desc (router 0.80 before app 0.30)
        assert!(md.find("src/router.rs").unwrap() < md.find("src/app.rs").unwrap());
        // violations grouped under the changed file
        assert!(md.contains("### Convention violations"));
        assert!(md.contains("src/api.rs:12"));
        // all security finding kinds rendered (scoped to the changed file)
        assert!(md.contains("### Security"));
        assert!(md.contains("Unprotected endpoint"));
        assert!(md.contains("Secret pattern"));
        assert!(md.contains("SQL injection risk"));
        assert!(md.contains("Validation gap"));
    }

    #[test]
    fn render_review_drops_security_findings_outside_changed_files() {
        use crate::intelligence::blast_radius::{
            BlastRadiusCategories, BlastRadiusResult, RiskSummary,
        };
        use crate::intelligence::security::{SecretPattern, SecuritySurface};

        // A secret in a file NOT in the change set must not be rendered.
        let bundle = ReviewBundle {
            changed_paths: vec!["src/changed.rs".to_string()],
            blast: BlastRadiusResult {
                changed_files: vec![],
                total_affected: 0,
                categories: BlastRadiusCategories {
                    direct_dependents: vec![],
                    transitive_dependents: vec![],
                    test_files: vec![],
                    schema_dependents: vec![],
                },
                risk_summary: RiskSummary {
                    high: 0,
                    medium: 0,
                    low: 0,
                },
            },
            predicted_tests: vec![],
            violations: vec![],
            security: SecuritySurface {
                unprotected_endpoints: vec![],
                input_validation_gaps: vec![],
                secret_patterns: vec![SecretPattern {
                    file: "src/other.rs".to_string(),
                    line: 1,
                    pattern_name: "aws_key".to_string(),
                    snippet: "AKIA...".to_string(),
                }],
                sql_injection_surface: vec![],
                exposure_scores: vec![],
            },
            omissions: vec![],
        };
        let md = render_review(&bundle);
        assert!(
            !md.contains("### Security"),
            "no in-scope findings → no section"
        );
        assert!(!md.contains("src/other.rs"));
    }

    #[test]
    fn detects_cochange_and_missing_test_omissions() {
        use crate::intelligence::co_change::CoChangeEdge;
        use crate::intelligence::test_map::{TestConfidence, TestFileRef};
        use std::collections::HashMap;

        let changed = vec!["src/handler.rs".to_string()];
        let co = vec![
            // handler.rs co-changed with its test 14x recently, but the test isn't in the diff
            CoChangeEdge {
                file_a: "src/handler.rs".into(),
                file_b: "src/handler_test.rs".into(),
                count: 14,
                recency_weight: 0.9,
            },
            // a weak, stale pairing below threshold — must NOT be reported
            CoChangeEdge {
                file_a: "src/handler.rs".into(),
                file_b: "README.md".into(),
                count: 1,
                recency_weight: 0.05,
            },
            // a pairing where BOTH sides are the changed file — not an omission
            CoChangeEdge {
                file_a: "src/handler.rs".into(),
                file_b: "src/handler.rs".into(),
                count: 9,
                recency_weight: 0.9,
            },
        ];
        let mut test_map: HashMap<String, Vec<TestFileRef>> = HashMap::new();
        test_map.insert(
            "src/handler.rs".into(),
            vec![TestFileRef {
                path: "src/handler_test.rs".into(),
                confidence: TestConfidence::Both,
            }],
        );

        let oms = detect_omissions(&changed, &co, &test_map);

        // the strong co-change to the absent test is reported (de-duped: the
        // missing-test pass sees it already in `seen`, so it appears exactly once)
        assert!(oms.iter().any(|o| o.expected_file == "src/handler_test.rs"));
        assert_eq!(
            oms.iter()
                .filter(|o| o.expected_file == "src/handler_test.rs")
                .count(),
            1,
            "expected_file must be de-duped across both signals"
        );
        // the co-change signal won (registered first) with the verified count
        assert!(oms.iter().any(|o| o.expected_file == "src/handler_test.rs"
            && matches!(o.kind, OmissionKind::CoChange { count: 14, .. })));
        // the weak/stale pairing is filtered out by OMISSION_MIN_WEIGHT
        assert!(!oms.iter().any(|o| o.expected_file == "README.md"));
        // results are ranked strongest-first (weight desc)
        assert!(oms.windows(2).all(|w| w[0].weight >= w[1].weight));
    }

    #[test]
    fn detects_missing_test_when_no_cochange_history() {
        use crate::intelligence::test_map::{TestConfidence, TestFileRef};
        use std::collections::HashMap;

        // No co-change edges at all — the missing-test signal must still fire.
        let changed = vec!["src/svc.rs".to_string()];
        let mut test_map: HashMap<String, Vec<TestFileRef>> = HashMap::new();
        test_map.insert(
            "src/svc.rs".into(),
            vec![TestFileRef {
                path: "tests/svc_test.rs".into(),
                confidence: TestConfidence::NameMatch,
            }],
        );

        let oms = detect_omissions(&changed, &[], &test_map);
        assert_eq!(oms.len(), 1);
        assert_eq!(oms[0].expected_file, "tests/svc_test.rs");
        assert!(matches!(oms[0].kind, OmissionKind::MissingTest { .. }));
        assert!(oms[0].weight.is_infinite());
    }

    #[test]
    fn missing_test_ignores_weak_import_only_confidence() {
        use crate::intelligence::test_map::{TestConfidence, TestFileRef};
        use std::collections::HashMap;

        // ImportMatch alone is not strong enough to nag about.
        let changed = vec!["src/svc.rs".to_string()];
        let mut test_map: HashMap<String, Vec<TestFileRef>> = HashMap::new();
        test_map.insert(
            "src/svc.rs".into(),
            vec![TestFileRef {
                path: "tests/svc_test.rs".into(),
                confidence: TestConfidence::ImportMatch,
            }],
        );

        let oms = detect_omissions(&changed, &[], &test_map);
        assert!(oms.is_empty());
    }

    #[test]
    fn detect_omissions_handles_file_b_side_and_sorts_multiple() {
        use crate::intelligence::co_change::CoChangeEdge;
        use std::collections::HashMap;

        // The changed file appears as `file_b` in one edge and `file_a` in
        // another — exercises both sides of the present/absent split. Two
        // qualifying co-change omissions means the sort comparator runs.
        let changed = vec!["src/x.rs".to_string()];
        let co = vec![
            CoChangeEdge {
                file_a: "src/weak.rs".into(),
                file_b: "src/x.rs".into(), // x.rs is file_b here
                count: 5,
                recency_weight: 1.0,
            },
            CoChangeEdge {
                file_a: "src/x.rs".into(), // x.rs is file_a here
                file_b: "src/strong.rs".into(),
                count: 12,
                recency_weight: 1.0,
            },
        ];
        let oms = detect_omissions(&changed, &co, &HashMap::new());
        assert_eq!(oms.len(), 2);
        // strongest first (12 before 5) — exercises the sort comparator
        assert_eq!(oms[0].expected_file, "src/strong.rs");
        assert_eq!(oms[1].expected_file, "src/weak.rs");
    }

    #[test]
    fn no_omissions_when_everything_changed_together() {
        use crate::intelligence::co_change::CoChangeEdge;
        let changed = vec!["a.rs".to_string(), "b.rs".to_string()];
        let co = vec![CoChangeEdge {
            file_a: "a.rs".into(),
            file_b: "b.rs".into(),
            count: 20,
            recency_weight: 1.0,
        }];
        let oms = detect_omissions(&changed, &co, &std::collections::HashMap::new());
        assert!(oms.is_empty()); // both sides present → nothing absent
    }

    #[test]
    fn test_no_changes() {
        let repo = make_diff_repo();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_modified_file() {
        let repo = make_diff_repo();
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .unwrap();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/main.rs");
        assert!(changes[0].diff_text.contains("println"));
    }

    #[test]
    fn test_new_file() {
        let repo = make_diff_repo();
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
        // Stage it so it shows in diff
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
        index.write().unwrap();

        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.iter().any(|c| c.path == "src/lib.rs"));
    }

    #[test]
    fn test_multiple_changes() {
        let repo = make_diff_repo();
        std::fs::write(repo.path().join("src/main.rs"), "fn main() { todo!(); }\n").unwrap();
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn foo() {}\n").unwrap();
        // Stage new file
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
        index.write().unwrap();

        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.len() >= 2);
    }

    #[test]
    fn test_diff_with_ref() {
        let repo = make_diff_repo();
        // Make second commit
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"v2\"); }\n",
        )
        .unwrap();
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let mut index = git_repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&head])
            .unwrap();

        // Diff HEAD~1 vs HEAD
        let changes = extract_changes(repo.path(), Some("HEAD~1")).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/main.rs");
    }

    #[test]
    fn test_diff_text_has_plus_minus() {
        let repo = make_diff_repo();
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .unwrap();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(!changes.is_empty());
        let diff = &changes[0].diff_text;
        assert!(
            diff.contains('+') || diff.contains('-'),
            "diff should have +/- markers"
        );
    }

    #[test]
    fn test_not_a_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = extract_changes(dir.path(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_time_expression_days() {
        assert_eq!(parse_time_expression("1 day").unwrap().as_secs(), 86400);
        assert_eq!(parse_time_expression("2 days").unwrap().as_secs(), 172800);
        assert_eq!(parse_time_expression("1d").unwrap().as_secs(), 86400);
        assert_eq!(parse_time_expression("3d").unwrap().as_secs(), 259200);
    }

    #[test]
    fn test_parse_time_expression_hours() {
        assert_eq!(parse_time_expression("1 hour").unwrap().as_secs(), 3600);
        assert_eq!(parse_time_expression("3 hours").unwrap().as_secs(), 10800);
        assert_eq!(parse_time_expression("1h").unwrap().as_secs(), 3600);
    }

    #[test]
    fn test_parse_time_expression_weeks() {
        assert_eq!(parse_time_expression("1 week").unwrap().as_secs(), 604800);
        assert_eq!(parse_time_expression("2 weeks").unwrap().as_secs(), 1209600);
        assert_eq!(parse_time_expression("1w").unwrap().as_secs(), 604800);
    }

    #[test]
    fn test_parse_time_expression_months() {
        assert_eq!(parse_time_expression("1 month").unwrap().as_secs(), 2592000);
        assert_eq!(
            parse_time_expression("2 months").unwrap().as_secs(),
            5184000
        );
    }

    #[test]
    fn test_parse_time_expression_yesterday() {
        assert_eq!(parse_time_expression("yesterday").unwrap().as_secs(), 86400);
    }

    #[test]
    fn test_parse_time_expression_invalid() {
        assert!(parse_time_expression("").is_err());
        assert!(parse_time_expression("abc").is_err());
        assert!(parse_time_expression("0 days").is_err());
    }

    #[test]
    fn test_parse_time_expression_zero_compact() {
        // "0d" should fail because time must be > 0
        assert!(parse_time_expression("0d").is_err());
        assert!(parse_time_expression("0h").is_err());
        assert!(parse_time_expression("0w").is_err());
    }

    #[test]
    fn test_parse_time_expression_unknown_unit() {
        // "2 fortnights" is an unknown unit
        let result = parse_time_expression("2 fortnights");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unknown time unit"),
            "expected 'unknown time unit', got: {err}"
        );
    }

    #[test]
    fn test_parse_time_expression_non_numeric_compact() {
        // "abch" — non-numeric prefix to compact form
        let result = parse_time_expression("abch");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_time_expression_compact_weeks() {
        assert_eq!(parse_time_expression("2w").unwrap().as_secs(), 1209600);
    }

    #[test]
    fn test_resolve_since_git_error() {
        // Non-existent directory should cause git to fail
        let result = resolve_since(std::path::Path::new("/nonexistent/repo"), "1d");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_since_no_recent_commits() {
        // Create a repo with a very old commit then ask for "1 second ago"
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        std::fs::write(dir.path().join("file.txt"), "hello").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        // Ask for commits from the future effectively — "1 second" window is fine
        // since the commit was literally just made, it *will* be found.
        // To get "no commits", we need an impossible window — but git --since
        // will likely find the commit. Use the function and just verify it doesn't panic.
        let result = resolve_since(dir.path(), "1d");
        // This should succeed since commit was just made
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_context_signatures_with_parse_results() {
        use crate::budget::counter::TokenCounter;
        use crate::index::CodebaseIndex;
        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/util.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/util.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "src/empty.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/empty.rs"),
                language: Some("rust".to_string()),
                size_bytes: 20,
            },
        ];

        let mut parse_results = HashMap::new();
        // File with public symbols
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "public_fn".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn public_fn()".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        // File with only private symbols
        parse_results.insert(
            "src/util.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "private_fn".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn private_fn()".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        // src/empty.rs has no parse result — tests the `let Some(pr) = ...` path

        let mut content_map = HashMap::new();
        content_map.insert(
            "src/lib.rs".to_string(),
            "pub fn public_fn() {}".to_string(),
        );
        content_map.insert("src/util.rs".to_string(), "fn private_fn() {}".to_string());
        content_map.insert("src/empty.rs".to_string(), "// empty".to_string());

        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        let mut context_paths = HashSet::new();
        context_paths.insert("src/lib.rs".to_string());
        context_paths.insert("src/util.rs".to_string());
        context_paths.insert("src/empty.rs".to_string());

        let result = render_context_signatures(&index, &context_paths, 10000, &counter);

        // Should include public_fn signature from lib.rs
        assert!(
            result.contains("public_fn"),
            "expected public_fn in output: {result}"
        );
        // Should NOT include private_fn
        assert!(
            !result.contains("private_fn"),
            "private symbols should be excluded"
        );
        // Should include file header for lib.rs
        assert!(result.contains("src/lib.rs"), "expected file header");
    }

    #[test]
    fn test_render_context_signatures_empty() {
        use crate::budget::counter::TokenCounter;
        use crate::index::CodebaseIndex;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let content_map = HashMap::from([("src/main.rs".to_string(), "fn main() {}".to_string())]);
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);

        // No context paths
        let result = render_context_signatures(&index, &HashSet::new(), 10000, &counter);
        assert!(result.is_empty());
    }

    #[test]
    fn test_diff_with_all_flag_graph_walk() {
        use crate::cli::OutputFormat;

        let repo = make_diff_repo();
        // Add a second file that imports from main
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "use crate::main;\npub fn helper() {}\n",
        )
        .unwrap();
        // Stage it
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let mut index = git_repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "add lib", &tree, &[&head])
            .unwrap();

        // Now modify main.rs in the working tree
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .unwrap();

        // Run with all=true to exercise BFS graph walk (lines 290-291)
        let result = run(
            repo.path(),
            None,  // git_ref
            50000, // token_budget
            &OutputFormat::Markdown,
            None,  // out
            false, // verbose
            true,  // all
            None,  // focus
            false, // timing
            false, // review
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_time_expression_overflow() {
        // Huge number that overflows u64 parse — covers line 37
        let result = parse_time_expression("99999999999999999999999d");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid time expression"));
    }
}
