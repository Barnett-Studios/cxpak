//! Identifier-level ranking fused with conventions DNA (cxpak 3.0.0, Phase C —
//! ADR-0181).
//!
//! File-level relevance (the `MultiSignalScorer`) answers "which files?"; this
//! layer answers "which *identifiers*?" and folds that answer back into the file
//! score. The unit of ranking is a `(file, identifier)` pair. Five deterministic
//! signals shape each pair's score:
//!
//!   1. **naming-pattern match ×10** — an identifier whose split/normalized name
//!      tokens intersect the query's tokens is strongly boosted.
//!   2. **ambiguity penalty ×0.1** — an identifier defined in more than
//!      [`AMBIGUITY_DEF_THRESHOLD`] places across the codebase is down-weighted.
//!   3. **`_`-prefix penalty ×0.1** — underscore-prefixed (private/internal)
//!      identifiers are down-weighted.
//!   4. **mention personalization** — the file mass fed to redistribution is the
//!      *personalized* PageRank seeded from the query's mentioned files, biasing
//!      ranking toward what the query names (see
//!      [`crate::intelligence::pagerank::compute_pagerank_personalized`]).
//!   5. **edge-rank redistribution** — each file's (personalized) PageRank mass
//!      is redistributed down to its identifiers proportionally to each symbol's
//!      visibility weight, so ranking is per-identifier, not per-file.
//!
//! These are then **fused with the conventions DNA**: an identifier whose naming
//! style and visibility match the codebase's dominant conventions
//! ([`ConventionProfile`]) earns a bounded conformance bonus, so idiomatic
//! identifiers rank consistently above one-off outliers.
//!
//! ## File-level fusion is boost-only (recall-safe)
//!
//! The penalties above operate *within* the identifier ranking — they change
//! which identifier is a file's best and the relative order of `(file, ident)`
//! pairs. When the best-identifier signal is folded back into the file score it
//! is applied as a **boost-only** multiplier (`≥ 1.0`): a file's relevance can
//! only rise, never fall, from this layer. That keeps the D2 recall gate
//! monotone — no file that the base scorer would have surfaced can be dropped by
//! identifier ranking — while still letting query-matching, convention-conforming
//! identifiers pull their file up the ranking. See ADR-0181.

use crate::core_graph::index::normalize_identifier;
use crate::index::CodebaseIndex;
use crate::intelligence::pagerank::{
    build_symbol_cross_refs, compute_pagerank_personalized, symbol_importance,
};
use crate::parser::language::{Symbol, SymbolKind};
use std::collections::{HashMap, HashSet};

/// Naming-pattern match multiplier: an identifier whose tokens intersect the
/// query's tokens is the single strongest identifier-level signal.
pub const NAME_MATCH_MULT: f64 = 10.0;

/// Ambiguity multiplier: identifiers defined in more than
/// [`AMBIGUITY_DEF_THRESHOLD`] places carry little locating power.
pub const AMBIGUITY_MULT: f64 = 0.1;

/// Underscore-prefix multiplier: `_name` marks private/internal symbols that are
/// rarely the subject of a query.
pub const UNDERSCORE_MULT: f64 = 0.1;

/// An identifier is "ambiguous" once it has strictly more than this many
/// definitions across the codebase.
pub const AMBIGUITY_DEF_THRESHOLD: usize = 5;

/// Conventions-DNA conformance bonus: the multiplier for a fully idiomatic
/// identifier is `1.0 + DNA_MATCH_BONUS`; a non-conforming one stays at `1.0`.
pub const DNA_MATCH_BONUS: f64 = 0.5;

/// File-level fusion gain: the fraction by which the best-identifier signal
/// (normalized to `[0, 1]`) lifts a file's base relevance. Boost-only.
///
/// Held at `0.0` (identifier ranking present but recall-neutral). A gate-repo D2
/// A/B (ripgrep + flask + express — 31 of the 62-PR corpus, the repos the gate
/// measures; inert 0.0 vs a 0.05 boost) measured the boost as net-neutral on the
/// gate: on the committed pinned subset it was equal @8k and +0.03 @32k / +0.05
/// MRR, but across all gate repos (31 entries) it *regressed* recall@8k by 0.03
/// — a single boundary flip (flask/5962, recall@8k 1.0→0.0) where the
/// fill-then-overflow packer swaps seeds at a budget edge, not a ranking error.
/// The measured @32k/MRR gains were within noise. Per the D2 rule (`active ≥
/// baseline on BOTH budgets` → ship active; else inert), the boost ships off.
/// The per-identifier signals still compute and surface as the `identifier_rank`
/// signal detail; raise this gain (and re-run the D2 A/B) to activate the boost.
/// See ADR-0181.
pub const IDENT_FUSION_GAIN: f64 = 0.0;

/// Mention-personalization gain: a file the query names (or that the mention
/// walk reaches) has its identifiers' rank lifted by up to this fraction.
pub const MENTION_GAIN: f64 = 1.0;

/// Redistribution floor: every file carrying symbols participates in identifier
/// ranking even when the dependency graph gives it no PageRank mass (e.g. an
/// isolated file), so identifier ranking never silently ignores a file.
const PR_FLOOR: f64 = 0.05;

/// PageRank parameters, matching the index-build defaults so the personalized
/// walk converges on the same graph the file-level ranks came from.
const PR_DAMPING: f64 = 0.85;
const PR_MAX_ITER: usize = 100;

/// A single scored `(file, identifier)` unit.
#[derive(Debug, Clone)]
pub struct ScoredIdentifier {
    pub file: String,
    pub name: String,
    /// Personalized-PageRank mass redistributed to this identifier (signal #5).
    pub redistributed_base: f64,
    /// Product of the naming/ambiguity/underscore/DNA multipliers.
    pub multiplier: f64,
    /// `redistributed_base * multiplier` — the identifier's final rank.
    pub score: f64,
}

/// The result of ranking every identifier for one query: the per-`(file, ident)`
/// scores, plus the derived per-file boost factor and `[0, 1]` signal the
/// file-level scorer consumes.
#[derive(Debug, Clone, Default)]
pub struct IdentifierRanking {
    /// Boost-only multiplier per file (`>= 1.0`).
    pub file_factors: HashMap<String, f64>,
    /// Best-identifier signal per file, normalized to `[0, 1]`.
    pub file_signal: HashMap<String, f64>,
    /// Every scored `(file, identifier)` unit (for introspection and tests).
    pub scored: Vec<ScoredIdentifier>,
}

impl IdentifierRanking {
    /// Boost factor for `file` (defaults to the neutral `1.0`).
    pub fn factor(&self, file: &str) -> f64 {
        self.file_factors.get(file).copied().unwrap_or(1.0)
    }

    /// Normalized best-identifier signal for `file` (defaults to `0.0`).
    pub fn signal(&self, file: &str) -> f64 {
        self.file_signal.get(file).copied().unwrap_or(0.0)
    }
}

/// Does an identifier's split/normalized name tokens intersect the query tokens?
fn matches_query_pattern(name: &str, query_tokens: &HashSet<String>) -> bool {
    if query_tokens.is_empty() {
        return false;
    }
    crate::relevance::signals::tokenize(name)
        .iter()
        .any(|t| query_tokens.contains(t))
}

/// Conformance of a symbol to the codebase's dominant conventions, in `[0, 1]`.
///
/// Blends a naming-style component (does the symbol's case style match the
/// dominant style for its kind?) and a visibility component (does its visibility
/// match the dominant one?), each weighted by how strong that convention is
/// (`percentage / 100`). Returns `0.0` when the codebase has no dominant pattern
/// to conform to.
fn dna_conformance(symbol: &Symbol, index: &CodebaseIndex) -> f64 {
    use crate::conventions::naming::classify_name;

    let naming = &index.conventions.naming;
    // Pick the observation relevant to this symbol's kind.
    let style_obs = match symbol.kind {
        SymbolKind::Function | SymbolKind::Method => naming.function_style.as_ref(),
        SymbolKind::Struct
        | SymbolKind::Enum
        | SymbolKind::Trait
        | SymbolKind::Interface
        | SymbolKind::Class
        | SymbolKind::TypeAlias => naming.type_style.as_ref(),
        SymbolKind::Constant => naming.constant_style.as_ref(),
        _ => None,
    };

    let naming_conf = style_obs
        .filter(|obs| classify_name(&symbol.name).to_string() == obs.dominant)
        .map(|obs| (obs.percentage / 100.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);

    // The visibility convention's `dominant` is "public" / "private".
    let vis_str = match symbol.visibility {
        crate::parser::language::Visibility::Public => "public",
        crate::parser::language::Visibility::Private => "private",
    };
    let vis_dominant = index
        .conventions
        .visibility
        .public_ratio
        .as_ref()
        .filter(|obs| vis_str == obs.dominant)
        .map(|obs| (obs.percentage / 100.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);

    // Equal weight to naming and visibility conformance.
    0.5 * naming_conf + 0.5 * vis_dominant
}

/// Build a personalization vector over files from the query's mentions.
///
/// A file earns seed mass for every symbol whose name matches the query pattern,
/// plus mass when its path tokens match the query. The result feeds
/// [`compute_pagerank_personalized`], biasing the walk toward what the query
/// names. Returns an empty map (⇒ classic PageRank) when nothing matches.
fn mention_seeds(index: &CodebaseIndex, query_tokens: &HashSet<String>) -> HashMap<String, f64> {
    let mut seeds: HashMap<String, f64> = HashMap::new();
    for file in &index.files {
        let mut mass = 0.0_f64;
        if let Some(pr) = &file.parse_result {
            for sym in &pr.symbols {
                if matches_query_pattern(&sym.name, query_tokens) {
                    mass += 1.0;
                }
            }
        }
        // A path whose tokens the query names is itself a mention.
        let path_tokens: HashSet<String> = file
            .relative_path
            .split(['/', '.', '_', '-'])
            .flat_map(crate::index::split_identifier)
            .filter(|t| t.len() >= 2)
            .collect();
        if path_tokens.iter().any(|t| query_tokens.contains(t)) {
            mass += 1.0;
        }
        if mass > 0.0 {
            seeds.insert(file.relative_path.clone(), mass);
        }
    }
    seeds
}

/// Rank every `(file, identifier)` unit for `query_tokens` and derive per-file
/// boost factors. `query_tokens` are the already-expanded, normalized/split
/// query tokens the file-level scorer uses (so identifier matching is consistent
/// with the other signals).
pub fn build_identifier_ranking(
    index: &CodebaseIndex,
    query_tokens: &HashSet<String>,
) -> IdentifierRanking {
    // Definition counts across the codebase: normalized symbol name → #defs.
    let mut def_counts: HashMap<String, usize> = HashMap::new();
    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            for sym in &pr.symbols {
                *def_counts
                    .entry(normalize_identifier(&sym.name))
                    .or_default() += 1;
            }
        }
    }

    // Cross-references (for the visibility weight inside `symbol_importance`).
    let cross_refs = build_symbol_cross_refs(&index.term_frequencies);

    // Signal #4: mention personalization. `mention_seeds` marks the files the
    // query names; those seeds are then propagated through the dependency graph
    // by a personalized (topic-sensitive) PageRank walk so the bias reaches the
    // seeds' neighbors too. The per-file mention strength blends the direct seed
    // mass (robust even for graph-isolated files) with the graph-propagated
    // personalized rank, normalized to [0, 1].
    let seeds = mention_seeds(index, query_tokens);
    let personalized_pr =
        compute_pagerank_personalized(&index.graph, PR_DAMPING, PR_MAX_ITER, &seeds);
    let seed_max = seeds.values().copied().fold(0.0_f64, f64::max);
    let mention_strength = |path: &str| -> f64 {
        let direct = if seed_max > 0.0 {
            seeds.get(path).copied().unwrap_or(0.0) / seed_max
        } else {
            0.0
        };
        let propagated = personalized_pr.get(path).copied().unwrap_or(0.0);
        direct.max(propagated).clamp(0.0, 1.0)
    };

    let mut scored: Vec<ScoredIdentifier> = Vec::new();

    for file in &index.files {
        let pr = match &file.parse_result {
            Some(pr) if !pr.symbols.is_empty() => pr,
            _ => continue,
        };
        // Signal #5 base: the file's dependency-graph PageRank mass, floored so
        // every symbol-bearing file participates.
        let file_pr = index
            .pagerank
            .get(file.relative_path.as_str())
            .copied()
            .unwrap_or(0.0)
            .max(PR_FLOOR);
        let m_mention = 1.0 + MENTION_GAIN * mention_strength(&file.relative_path);

        // Signal #5: total visibility weight in the file — the denominator that
        // redistributes the file's PageRank mass across its identifiers.
        let total_weight: f64 = pr
            .symbols
            .iter()
            .map(|s| symbol_importance(s, 1.0, &cross_refs, &file.relative_path))
            .sum();
        if total_weight <= 0.0 {
            continue;
        }

        for sym in &pr.symbols {
            let w_vis = symbol_importance(sym, 1.0, &cross_refs, &file.relative_path);
            let redistributed_base = file_pr * w_vis / total_weight;

            // Signal #1: naming-pattern match ×10.
            let m_name = if matches_query_pattern(&sym.name, query_tokens) {
                NAME_MATCH_MULT
            } else {
                1.0
            };
            // Signal #2: ambiguity penalty ×0.1 (>5 definitions).
            let defs = def_counts
                .get(&normalize_identifier(&sym.name))
                .copied()
                .unwrap_or(1);
            let m_ambig = if defs > AMBIGUITY_DEF_THRESHOLD {
                AMBIGUITY_MULT
            } else {
                1.0
            };
            // Signal #3: `_`-prefix penalty ×0.1.
            let m_under = if sym.name.starts_with('_') {
                UNDERSCORE_MULT
            } else {
                1.0
            };
            // Conventions-DNA fusion: idiomatic identifiers earn a bounded bonus.
            let m_dna = 1.0 + DNA_MATCH_BONUS * dna_conformance(sym, index);

            let multiplier = m_name * m_ambig * m_under * m_dna * m_mention;
            let score = redistributed_base * multiplier;

            scored.push(ScoredIdentifier {
                file: file.relative_path.clone(),
                name: sym.name.clone(),
                redistributed_base,
                multiplier,
                score,
            });
        }
    }

    // Per-file best-identifier score, then normalize to [0, 1] against the global
    // maximum so the file signal is comparable across queries.
    let mut best_per_file: HashMap<String, f64> = HashMap::new();
    for s in &scored {
        let e = best_per_file.entry(s.file.clone()).or_insert(0.0);
        if s.score > *e {
            *e = s.score;
        }
    }
    let global_max = best_per_file.values().copied().fold(0.0_f64, f64::max);

    let mut file_signal: HashMap<String, f64> = HashMap::new();
    let mut file_factors: HashMap<String, f64> = HashMap::new();
    for (file, best) in &best_per_file {
        let signal = if global_max > 0.0 {
            (best / global_max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        file_signal.insert(file.clone(), signal);
        // Boost-only: factor in [1.0, 1.0 + IDENT_FUSION_GAIN].
        file_factors.insert(file.clone(), 1.0 + IDENT_FUSION_GAIN * signal);
    }

    IdentifierRanking {
        file_factors,
        file_signal,
        scored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn sym(name: &str, kind: SymbolKind, vis: Visibility) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            visibility: vis,
            signature: format!("fn {name}()"),
            body: "{}".into(),
            start_line: 1,
            end_line: 1,
        }
    }

    /// Build an index from `(relative_path, source, symbols)` triples. The source
    /// is written so term-frequency / cross-ref machinery sees real content.
    fn build(files: &[(&str, &str, Vec<Symbol>)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        for (path, src, symbols) in files {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, src).unwrap();
            scanned.push(ScannedFile {
                relative_path: (*path).into(),
                absolute_path: abs,
                language: Some("rust".into()),
                size_bytes: src.len() as u64,
            });
            parse_results.insert(
                (*path).to_string(),
                ParseResult {
                    symbols: symbols.clone(),
                    imports: vec![],
                    exports: vec![],
                },
            );
        }
        let mut index = CodebaseIndex::build(scanned, parse_results, &counter);
        // Populate conventions the way serve::build_index does, so DNA fusion has
        // a profile to fuse against. A nonexistent repo path keeps git-health
        // empty and the fixture hermetic (naming/visibility come from symbols).
        let conv = crate::conventions::build_convention_profile(
            &index,
            std::path::Path::new("/nonexistent/cxpak-ident-test"),
        );
        index.conventions = conv;
        index
    }

    fn tokens(words: &[&str]) -> HashSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn find<'a>(r: &'a IdentifierRanking, file: &str, name: &str) -> &'a ScoredIdentifier {
        r.scored
            .iter()
            .find(|s| s.file == file && s.name == name)
            .unwrap_or_else(|| panic!("no scored identifier {file}::{name}"))
    }

    #[test]
    fn naming_pattern_match_boosts_x10() {
        // Two public functions in the same file; one matches the query token
        // "rate", the other doesn't. Identical redistributed base ⇒ the matcher's
        // score must be exactly NAME_MATCH_MULT× the non-matcher's.
        let index = build(&[(
            "src/a.rs",
            "pub fn rate_limit() {} pub fn other_thing() {}",
            vec![
                sym("rate_limit", SymbolKind::Function, Visibility::Public),
                sym("other_thing", SymbolKind::Function, Visibility::Public),
            ],
        )]);
        let r = build_identifier_ranking(&index, &tokens(&["rate"]));
        let matcher = find(&r, "src/a.rs", "rate_limit");
        let other = find(&r, "src/a.rs", "other_thing");
        // Same DNA conformance and base weight, so the ratio is purely the
        // naming multiplier.
        assert!(
            (matcher.multiplier / other.multiplier - NAME_MATCH_MULT).abs() < 1e-9,
            "expected ×{NAME_MATCH_MULT} boost, got {} vs {}",
            matcher.multiplier,
            other.multiplier
        );
        assert!(matcher.score > other.score);
    }

    #[test]
    fn ambiguity_penalty_x01_above_threshold() {
        // "handle" is defined in 6 files (>5) ⇒ ambiguous; "unique_name" once.
        let mut files: Vec<(&str, &str, Vec<Symbol>)> = Vec::new();
        let paths = ["a", "b", "c", "d", "e", "f"];
        for p in paths {
            let leaked: &'static str = Box::leak(format!("src/{p}.rs").into_boxed_str());
            files.push((
                leaked,
                "pub fn handle() {}",
                vec![sym("handle", SymbolKind::Function, Visibility::Public)],
            ));
        }
        files.push((
            "src/uniq.rs",
            "pub fn unique_name() {}",
            vec![sym("unique_name", SymbolKind::Function, Visibility::Public)],
        ));
        let index = build(&files);
        let r = build_identifier_ranking(&index, &tokens(&["nomatch"]));
        let ambiguous = find(&r, "src/a.rs", "handle");
        let unique = find(&r, "src/uniq.rs", "unique_name");
        // Both are public snake_case functions with no query match, so their
        // naming/underscore/DNA/mention multipliers are identical — the ratio
        // isolates the ambiguity penalty. 6 defs > threshold ⇒ ×0.1.
        assert!(
            (ambiguous.multiplier / unique.multiplier - AMBIGUITY_MULT).abs() < 1e-9,
            "ambiguity ratio should be {AMBIGUITY_MULT}, got {} / {}",
            ambiguous.multiplier,
            unique.multiplier
        );
    }

    #[test]
    fn underscore_prefix_penalty_x01() {
        let index = build(&[(
            "src/a.rs",
            "pub fn _internal() {} pub fn external() {}",
            vec![
                sym("_internal", SymbolKind::Function, Visibility::Public),
                sym("external", SymbolKind::Function, Visibility::Public),
            ],
        )]);
        let r = build_identifier_ranking(&index, &tokens(&["nomatch"]));
        let under = find(&r, "src/a.rs", "_internal");
        let plain = find(&r, "src/a.rs", "external");
        assert!(
            (plain.multiplier / under.multiplier - 1.0 / UNDERSCORE_MULT).abs() < 1e-6,
            "underscore should divide multiplier by {UNDERSCORE_MULT}: {} vs {}",
            under.multiplier,
            plain.multiplier
        );
    }

    #[test]
    fn redistribution_produces_per_identifier_scores() {
        // A public+referenced symbol (weight 1.0) and a private one (weight 0.3)
        // in the same file must receive different redistributed bases summing to
        // the file's PageRank mass.
        let index = build(&[
            (
                "src/a.rs",
                "pub fn shared() {} fn helper() {}",
                vec![
                    sym("shared", SymbolKind::Function, Visibility::Public),
                    sym("helper", SymbolKind::Function, Visibility::Private),
                ],
            ),
            // A second file references `shared` so it counts as public+referenced.
            (
                "src/b.rs",
                "pub fn call() { shared(); }",
                vec![sym("call", SymbolKind::Function, Visibility::Public)],
            ),
        ]);
        let r = build_identifier_ranking(&index, &tokens(&["nomatch"]));
        let shared = find(&r, "src/a.rs", "shared");
        let helper = find(&r, "src/a.rs", "helper");
        assert!(
            shared.redistributed_base > helper.redistributed_base,
            "public+referenced ({}) should get more mass than private ({})",
            shared.redistributed_base,
            helper.redistributed_base
        );
        assert!(
            helper.redistributed_base > 0.0,
            "every identifier must get a positive share"
        );
    }

    #[test]
    fn mention_personalization_biases_toward_seed() {
        // Two files with generically-named symbols that do NOT match the query, so
        // the naming multiplier is neutral for both. The query token "payments"
        // matches only the PATH of the first file, so mention personalization is
        // the sole differentiator — its identifier score must exceed the other's.
        let index = build(&[
            (
                "src/payments_module.rs",
                "pub fn run_task() {}",
                vec![sym("run_task", SymbolKind::Function, Visibility::Public)],
            ),
            (
                "src/other_module.rs",
                "pub fn run_task_two() {}",
                vec![sym(
                    "run_task_two",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
            ),
        ]);
        let r = build_identifier_ranking(&index, &tokens(&["payments"]));
        let a = find(&r, "src/payments_module.rs", "run_task");
        let b = find(&r, "src/other_module.rs", "run_task_two");
        // Neither symbol name matches "payments" (m_name == 1 for both), so the
        // score gap is entirely the mention multiplier.
        assert!(
            (a.multiplier - 1.0).abs() > 1e-9,
            "mentioned file's identifier should carry a mention boost, got {}",
            a.multiplier
        );
        assert!(
            a.score > b.score,
            "mentioned file's identifier score ({}) should exceed unmentioned ({})",
            a.score,
            b.score
        );
    }

    #[test]
    fn conventions_dna_fusion_rewards_conforming_identifier() {
        // A codebase overwhelmingly snake_case: a snake_case function conforms and
        // earns the DNA bonus; a camelCase outlier does not. Neither matches the
        // query, so only the DNA multiplier differs.
        let mut files: Vec<(&str, &str, Vec<Symbol>)> = Vec::new();
        // 8 conforming snake_case functions establish the dominant convention.
        let names = [
            "load_config",
            "save_config",
            "parse_input",
            "write_output",
            "read_bytes",
            "flush_buffer",
            "open_stream",
            "close_stream",
        ];
        for (i, n) in names.iter().enumerate() {
            let path: &'static str = Box::leak(format!("src/c{i}.rs").into_boxed_str());
            let src: &'static str = Box::leak(format!("pub fn {n}() {{}}").into_boxed_str());
            files.push((
                path,
                src,
                vec![sym(n, SymbolKind::Function, Visibility::Public)],
            ));
        }
        // The comparison file: one conforming, one non-conforming function.
        files.push((
            "src/x.rs",
            "pub fn tidy_state() {} pub fn tidyState() {}",
            vec![
                sym("tidy_state", SymbolKind::Function, Visibility::Public),
                sym("tidyState", SymbolKind::Function, Visibility::Public),
            ],
        ));
        let index = build(&files);
        // Sanity: snake_case is the dominant function style.
        assert_eq!(
            index
                .conventions
                .naming
                .function_style
                .as_ref()
                .map(|o| o.dominant.as_str()),
            Some("snake_case"),
            "fixture must establish snake_case dominance"
        );
        let r = build_identifier_ranking(&index, &tokens(&["nomatch"]));
        let conforming = find(&r, "src/x.rs", "tidy_state");
        let outlier = find(&r, "src/x.rs", "tidyState");
        assert!(
            conforming.multiplier > outlier.multiplier,
            "snake_case conformer ({}) should out-multiply camelCase outlier ({})",
            conforming.multiplier,
            outlier.multiplier
        );
    }

    #[test]
    fn file_factor_is_boost_only() {
        // Every file factor must be >= 1.0 (recall-safe boost-only fusion), and
        // the file holding the query-matching identifier must carry the stronger
        // raw signal. The signal is gain-independent — the fusion's substance;
        // the factor is `1 + IDENT_FUSION_GAIN * signal`, so factors only diverge
        // when the boost is active. At the shipped neutral gain (0.0) all factors
        // collapse to 1.0, which is precisely why recall is unperturbed.
        let index = build(&[
            (
                "src/match.rs",
                "pub fn rate_limiter() {}",
                vec![sym(
                    "rate_limiter",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
            ),
            (
                "src/plain.rs",
                "pub fn something() {}",
                vec![sym("something", SymbolKind::Function, Visibility::Public)],
            ),
        ]);
        let r = build_identifier_ranking(&index, &tokens(&["rate"]));
        for f in r.file_factors.values() {
            assert!(*f >= 1.0, "factor must be boost-only, got {f}");
            assert!(
                *f <= 1.0 + IDENT_FUSION_GAIN + 1e-9,
                "factor must stay within the gain bound, got {f}"
            );
        }
        assert!(
            r.signal("src/match.rs") > r.signal("src/plain.rs"),
            "matching file signal ({}) should exceed non-matching ({})",
            r.signal("src/match.rs"),
            r.signal("src/plain.rs")
        );
    }

    #[test]
    fn deterministic_across_runs() {
        let index = build(&[(
            "src/a.rs",
            "pub fn rate_limit() {} fn _helper() {}",
            vec![
                sym("rate_limit", SymbolKind::Function, Visibility::Public),
                sym("_helper", SymbolKind::Function, Visibility::Private),
            ],
        )]);
        let a = build_identifier_ranking(&index, &tokens(&["rate"]));
        let b = build_identifier_ranking(&index, &tokens(&["rate"]));
        assert_eq!(a.file_factors, b.file_factors);
        assert_eq!(a.file_signal, b.file_signal);
    }
}
