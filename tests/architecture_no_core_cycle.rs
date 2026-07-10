// Acceptance for Task 0.1 (cxpak 3.0.0, Phase 0): the four core modules
// (`index`, `schema`, `intelligence`, `conventions`) plus the new `core_graph`
// boundary must not form a circular dependency in cxpak's own file-level import
// graph. `build_architecture_map` runs Tarjan SCC over `index.graph.edges`; we
// assert no returned cycle group spans two or more core module prefixes.
//
// This guards the de-cycle refactor: before the refactor cxpak's own
// `cycles` health subscore is dragged by an SCC spanning these modules; after
// it, every SCC is contained within a single core prefix (or absent).

use cxpak::commands::serve::build_index;
use cxpak::intelligence::architecture::build_architecture_map;
use std::path::Path;

#[test]
fn core_modules_have_no_circular_dependency() {
    let index = build_index(Path::new(".")).expect("index builds for repo root");
    let arch = build_architecture_map(&index, 2);

    let core = [
        "src/index",
        "src/schema",
        "src/intelligence",
        "src/conventions",
        "src/core_graph",
    ];

    let offending: Vec<&Vec<String>> = arch
        .circular_deps
        .iter()
        .filter(|group| {
            // Count how many distinct core prefixes this cycle group touches.
            let touched = core
                .iter()
                .filter(|prefix| group.iter().any(|path| path.starts_with(*prefix)))
                .count();
            touched >= 2
        })
        .collect();

    assert!(
        offending.is_empty(),
        "core-spanning cycles remain (a cycle group touching >=2 of {core:?}): {offending:#?}"
    );
}
