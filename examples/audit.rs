use cxpak::budget::counter::TokenCounter;
use cxpak::cache;
use cxpak::conventions;
use cxpak::index::CodebaseIndex;
use cxpak::scanner::Scanner;
use std::path::Path;

fn main() {
    let counter = TokenCounter::new();
    let scanner = Scanner::new(Path::new(".")).unwrap();
    let files = scanner.scan_workspace(None).unwrap();
    let (pr, cm) = cache::parse::parse_with_cache(&files, Path::new("."), &counter, false);
    let mut index = CodebaseIndex::build_with_content(files, pr, &counter, cm);
    index.conventions = conventions::build_convention_profile(&index, Path::new("."));

    println!("=== DEPENDENCY GRAPH ===");
    let total_edges: usize = index.graph.edges.values().map(|s| s.len()).sum();
    println!(
        "files with edges: {}, total edges: {}",
        index.graph.edges.len(),
        total_edges
    );

    println!("\n=== PAGERANK TOP 10 ===");
    let mut prs: Vec<_> = index.pagerank.iter().collect();
    prs.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    for (p, v) in prs.iter().take(10) {
        println!("  {:<50} {:.4}", p, v);
    }
    let zero_count = prs.iter().filter(|(_, v)| **v == 0.0).count();
    println!("zero-pagerank files: {} / {}", zero_count, prs.len());

    println!("\n=== CALL GRAPH ===");
    println!("edges: {}", index.call_graph.edges.len());
    let cross: usize = index
        .call_graph
        .edges
        .iter()
        .filter(|e| e.caller_file != e.callee_file)
        .count();
    let intra: usize = index
        .call_graph
        .edges
        .iter()
        .filter(|e| e.caller_file == e.callee_file)
        .count();
    println!(
        "  cross-file: {}  intra-file: {}  unresolved: {}",
        cross,
        intra,
        index.call_graph.unresolved.len()
    );

    println!("\n=== REVERSE EDGES src/index/mod.rs (expected many inbound) ===");
    println!(
        "  dependents: {}",
        index.graph.dependents("src/index/mod.rs").len()
    );
    for d in index.graph.dependents("src/index/mod.rs").iter().take(5) {
        println!("    from: {} ({:?})", d.target, d.edge_type);
    }
    let count = index
        .graph
        .reverse_edges
        .get("src/index/mod.rs")
        .map(|s| s.len())
        .unwrap_or(0);
    println!("  reverse_edges count: {}", count);

    println!("\n=== BLAST RADIUS src/parser/language.rs ===");
    let b = cxpak::intelligence::blast_radius::compute_blast_radius(
        &["src/parser/language.rs"],
        &index.graph,
        &index.pagerank,
        &index.test_map,
        3,
        None,
    );
    println!(
        "direct: {}  transitive: {}  tests: {}",
        b.categories.direct_dependents.len(),
        b.categories.transitive_dependents.len(),
        b.categories.test_files.len()
    );

    println!("\n=== RISK TOP 10 ===");
    let risks = cxpak::intelligence::risk::compute_risk_ranking(&index);
    for r in risks.iter().take(10) {
        println!(
            "  {:<45} risk={:.3} churn={} blast={} tc={:.1}",
            r.path, r.risk_score, r.churn_30d, r.blast_radius, r.test_coverage
        );
    }

    println!("\n=== ARCHITECTURE ===");
    let arch = cxpak::intelligence::architecture::build_architecture_map(&index, 2);
    println!(
        "modules: {} circular_deps: {}",
        arch.modules.len(),
        arch.circular_deps.len()
    );
    for (i, cycle) in arch.circular_deps.iter().enumerate().take(5) {
        println!("  cycle {}: {:?}", i, cycle);
    }
    let mut mods: Vec<_> = arch.modules.iter().collect();
    mods.sort_by(|a, b| b.coupling.partial_cmp(&a.coupling).unwrap());
    println!("top 5 coupled:");
    for m in mods.iter().take(5) {
        println!(
            "  {:<30} coup={:.2} coh={:.2} files={} gods={}",
            m.prefix,
            m.coupling,
            m.cohesion,
            m.file_count,
            m.god_files.len()
        );
    }

    println!("\n=== SECURITY ===");
    let auth = [
        "require_auth",
        "authenticate",
        "authorize",
        "auth_middleware",
        "check_auth",
    ];
    let sec = cxpak::intelligence::security::build_security_surface(&index, &auth, None);
    println!(
        "unprotected: {}  secrets: {}  sql_injection: {}  validation_gaps: {}",
        sec.unprotected_endpoints.len(),
        sec.secret_patterns.len(),
        sec.sql_injection_surface.len(),
        sec.input_validation_gaps.len()
    );
    for e in sec.unprotected_endpoints.iter().take(3) {
        println!("  UNPROT {}:{} {} {}", e.file, e.line, e.method, e.path);
    }
    for s in sec.secret_patterns.iter().take(5) {
        println!("  SECRET {:?}", s);
    }
    for s in sec.sql_injection_surface.iter().take(3) {
        println!("  SQL {:?}", s);
    }
}
