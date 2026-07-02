# ADR Index

178 architecture decision records. 0001–0162 reconstructed retroactively from internal design docs and shipped code (v0.1.0 → v2.2.1); 0163 onward written at decision time. See [README](./README.md) for methodology.

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-distribution-cratesio-and-github-binaries.md) | Distribute via crates.io (cargo install) and pre-built GitHub Release binaries; no Homebrew | ACCEPTED | 2026-03-05 |
| [0002](0002-git2-library-no-shelling-out.md) | Access git history via the git2 library rather than shelling out to the git binary | ACCEPTED | 2026-03-05 |
| [0003](0003-index-as-central-data-structure.md) | The Index is the single source of truth populated by Scanner/Parser and read by Budget/Output | ACCEPTED | 2026-03-05 |
| [0004](0004-language-support-trait-boundary.md) | Per-language extraction behind a LanguageSupport trait with a runtime registry | ACCEPTED | 2026-03-05 |
| [0005](0005-no-config-file-flags-only.md) | Configuration via CLI flags only, no config file | ACCEPTED | 2026-03-05 |
| [0006](0006-out-of-band-indexing-vs-llm-exploration.md) | Index the codebase out-of-band and hand the LLM a token-budgeted briefing instead of letting it explore | ACCEPTED | 2026-03-05 |
| [0007](0007-pipeline-stage-module-boundaries.md) | Organize the system as a linear pipeline of single-responsibility modules with explicit I/O boundaries | ACCEPTED | 2026-03-05 |
| [0008](0008-single-binary-grammars-compiled-in-behind-feature-flags.md) | Ship a single binary with all tree-sitter grammars compiled in, each behind a Cargo feature flag | ACCEPTED | 2026-03-05 |
| [0009](0009-synchronous-no-async-runtime.md) | No async runtime — synchronous filesystem and CPU-bound pipeline | ACCEPTED | 2026-03-05 |
| [0010](0010-tdd-with-coverage-gate.md) | Strict Red-Green-Refactor TDD with a CI coverage gate enforced per task | ACCEPTED | 2026-03-05 |
| [0011](0011-three-layer-ignore-rules-via-ignore-crate.md) | Three-layer ignore model (.gitignore + built-in defaults + .cxpakignore) using the ripgrep ignore crate | ACCEPTED | 2026-03-05 |
| [0012](0012-three-output-formats-markdown-xml-json.md) | Render to three output formats (markdown default, XML, JSON) via a format-dispatch boundary | ACCEPTED | 2026-03-05 |
| [0013](0013-tiktoken-cl100k-count-once-cache.md) | Token counting via tiktoken-rs cl100k_base, counted once during indexing and cached | ACCEPTED | 2026-03-05 |
| [0014](0014-top-down-degradation-omission-markers.md) | Top-down progressive degradation with explicit omission markers | ACCEPTED | 2026-03-05 |
| [0015](0015-trace-call-graph-budget-controlled-depth.md) | trace command walks the dependency/call graph with budget-controlled hop depth and ambiguity handling | ACCEPTED | 2026-03-05 |
| [0016](0016-weighted-section-budget-with-overflow.md) | Weighted per-section token budget with fixed metadata floor (design-time overflow not implemented) | ACCEPTED | 2026-03-05 |
| [0017](0017-cxpak-gitignore-injection.md) | Pack mode appends .cxpak/ to the target repo's .gitignore idempotently | ACCEPTED | 2026-03-08 |
| [0018](0018-omission-pointer-marker-format.md) | Pointer-style omission markers reference .cxpak/ detail files instead of suggesting a higher token budget | ACCEPTED | 2026-03-08 |
| [0019](0019-pack-mode-detail-files-on-budget-overflow.md) | Pack mode: write full analysis to .cxpak/ detail files when a repo exceeds the token budget | ACCEPTED | 2026-03-08 |
| [0020](0020-detail-file-extension-matches-format.md) | Detail file extensions track the --format flag (.md/.json/.xml) | ACCEPTED | 2026-03-09 |
| [0021](0021-language-support-trait-tree-sitter.md) | New languages are added via the LanguageSupport trait backed by per-language tree-sitter crates behind feature flags | ACCEPTED | 2026-03-09 |
| [0022](0022-trace-command-dependency-graph-bfs.md) | Trace command locates a symbol then walks the dependency graph (1-hop default, full BFS with --all) | ACCEPTED | 2026-03-09 |
| [0023](0023-xml-omission-pointer-as-element.md) | XML output emits omission pointers as <detail-ref> elements rather than HTML comments | ACCEPTED | 2026-03-09 |
| [0024](0024-cache-parseresult-serde-serialization.md) | Make ParseResult and its components serde-serializable so parse results persist in the cache | ACCEPTED | 2026-03-10 |
| [0025](0025-cache-preserved-during-stale-output-cleanup.md) | Stale .cxpak/ cleanup preserves the cache/ subdirectory; only output files are wiped | ACCEPTED | 2026-03-10 |
| [0026](0026-cxpak-clean-full-wipe-command.md) | cxpak clean subcommand fully removes the .cxpak/ directory (cache + outputs) | ACCEPTED | 2026-03-10 |
| [0027](0027-diff-command-diff-first-budget.md) | Diff command combines git2 diff with trace-style context using a diff-first budget strategy | ACCEPTED | 2026-03-10 |
| [0028](0028-homebrew-tap-prebuilt-tarballs.md) | Distribute via a dedicated Homebrew tap that downloads prebuilt release tarballs, auto-updated by CI | ACCEPTED | 2026-03-10 |
| [0029](0029-parse-cache-mtime-size-key.md) | Parse cache keyed on file path + mtime + size, stored as JSON in .cxpak/cache/ | ACCEPTED | 2026-03-10 |
| [0030](0030-shared-parse-with-cache-function.md) | Extract a single parse_with_cache function shared by overview, trace, and diff | ACCEPTED | 2026-03-10 |
| [0031](0031-bats-shell-test-tdd-with-dryrun-and-mocked-uname.md) | BATS as the test framework with TDD, mocked uname, and a --dry-run seam | ACCEPTED | 2026-03-11 |
| [0032](0032-default-50k-token-budget-always-ask.md) | Default 50k token budget, always prompt the user (except clean) | ACCEPTED | 2026-03-11 |
| [0033](0033-ensure-cxpak-binary-resolution-and-autodownload.md) | Shared ensure-cxpak bash script resolves the binary: PATH, then cached install, then auto-download | ACCEPTED | 2026-03-11 |
| [0034](0034-github-release-tarball-distribution.md) | First-use distribution via GitHub Releases tarballs with uname-based platform mapping | ACCEPTED | 2026-03-11 |
| [0035](0035-markdown-hardcoded-output-format.md) | Hardcode --format markdown for all plugin invocations | ACCEPTED | 2026-03-11 |
| [0036](0036-plugin-no-agents-no-hooks-skills-commands-only.md) | Plugin surface limited to auto-invoked skills and user-invoked commands; no agents, no hooks | ACCEPTED | 2026-03-11 |
| [0037](0037-plugin-versioned-in-repo-distribution.md) | Plugin lives in-repo and is versioned/released lockstep with the CLI | ACCEPTED | 2026-03-11 |
| [0038](0038-daemon-feature-flag-boundary.md) | Gate daemon (notify/axum/tokio) behind a non-default feature flag | ACCEPTED | 2026-03-12 |
| [0039](0039-default-token-budget-50k.md) | Default --tokens to 50k across all commands | ACCEPTED | 2026-03-12 |
| [0040](0040-focus-flag-score-boost.md) | --focus CLI flag with multiplicative score boost | ACCEPTED | 2026-03-12 |
| [0041](0041-github-action-separate-repo-composite.md) | GitHub Action as a separate composite-action repo posting PR comments | ACCEPTED | 2026-03-12 |
| [0042](0042-graph-git-importance-ranking.md) | Graph-and-git-based file importance ranking for budget allocation | ACCEPTED | 2026-03-12 |
| [0043](0043-mcp-server-jsonrpc-over-stdio.md) | MCP server mode as JSON-RPC over stdio (cxpak serve --mcp) | ACCEPTED | 2026-03-12 |
| [0044](0044-pass-content-through-pipeline-no-double-read.md) | Pass file content from parser to indexer to eliminate double disk reads | ACCEPTED | 2026-03-12 |
| [0045](0045-per-file-git-recency-from-churn.md) | Per-file git recency derived from churn rank instead of a binary commit check | ACCEPTED | 2026-03-12 |
| [0046](0046-rayon-parallel-parsing.md) | Parallelize file parsing with rayon | ACCEPTED | 2026-03-12 |
| [0047](0047-reverse-adjacency-index-dependents.md) | Reverse adjacency index for O(1) dependents lookup | ACCEPTED | 2026-03-12 |
| [0048](0048-serve-http-api-shared-rwlock-index.md) | cxpak serve HTTP API over a shared Arc<RwLock> hot index | ACCEPTED | 2026-03-12 |
| [0049](0049-since-time-expression-to-commit.md) | --since flag resolves a time expression to a git commit | ACCEPTED | 2026-03-12 |
| [0050](0050-template-based-narrative-no-llm.md) | Template-based codebase narrative generated from index signals (no LLM) | ACCEPTED | 2026-03-12 |
| [0051](0051-tiktoken-o200k-base.md) | Switch token counting to the o200k_base tokenizer | ACCEPTED | 2026-03-12 |
| [0052](0052-timing-flag-stderr-instrumentation.md) | --timing flag emitting per-stage pipeline durations to stderr | ACCEPTED | 2026-03-12 |
| [0053](0053-file-watcher-notify-debounced.md) | File watcher via notify with channel-based debounced drain | ACCEPTED | 2026-03-13 |
| [0054](0054-graph-cache-deferred.md) | Defer dependency-graph caching to a later release | ACCEPTED | 2026-03-13 |
| [0055](0055-incremental-in-memory-index-updates.md) | Incremental in-memory index and graph updates (upsert/remove) | ACCEPTED | 2026-03-13 |
| [0056](0056-fill-then-omit-token-budget-packing.md) | Token-budget packing: fill by relevance rank, omit overflow with reasons | ACCEPTED | 2026-03-17 |
| [0057](0057-mcp-plugin-wiring-via-mcp-json.md) | Wire MCP server into the Claude Code plugin via .mcp.json + ensure-cxpak-serve wrapper | ACCEPTED | 2026-03-17 |
| [0058](0058-multi-signal-weighted-relevance-scoring.md) | Multi-signal weighted-sum relevance scoring with five deterministic signals | ACCEPTED | 2026-03-17 |
| [0059](0059-relevance-module-as-core-crate.md) | Extract relevance scoring into a standalone src/relevance/ core module | ACCEPTED | 2026-03-17 |
| [0060](0060-seed-selection-dependency-fanout.md) | Seed selection by score threshold with 1-hop dependency fan-out at a discount | ACCEPTED | 2026-03-17 |
| [0061](0061-term-frequency-index-extension.md) | Store per-file term frequencies as a HashMap on CodebaseIndex, computed at parse time | ACCEPTED | 2026-03-17 |
| [0062](0062-two-tool-context-handshake.md) | Two-tool MCP surface: cxpak_context_for_task (rank) + cxpak_pack_context (bundle) | ACCEPTED | 2026-03-17 |
| [0063](0063-cxpak-search-regex-mcp-tool.md) | Add cxpak_search regex MCP tool with line-based content search | ACCEPTED | 2026-03-18 |
| [0064](0064-filename-before-extension-language-detection.md) | Filename match takes priority over extension in detect_language | ACCEPTED | 2026-03-18 |
| [0065](0065-focus-prefix-path-filtering.md) | Add focus param as a starts_with prefix filter on all MCP tools | ACCEPTED | 2026-03-18 |
| [0066](0066-search-http-endpoint-post-not-get.md) | Expose search as POST /search rather than GET | ACCEPTED | 2026-03-18 |
| [0067](0067-tier2-symbolkind-structural-variants.md) | Extend SymbolKind with 17 structural variants for Tier 2 languages | ACCEPTED | 2026-03-18 |
| [0068](0068-tree-sitter-grammars-30-languages.md) | Adopt 30 additional tree-sitter grammars to reach 42-language coverage | ACCEPTED | 2026-03-18 |
| [0069](0069-composite-degradation-priority-score.md) | Order degradation by composite score: 0.7 relevance + 0.3 concept priority | ACCEPTED | 2026-03-19 |
| [0070](0070-hierarchical-query-expansion-core-plus-domain.md) | Hierarchical query expansion: always-on core synonyms + heuristic-activated domain synonyms | ACCEPTED | 2026-03-19 |
| [0071](0071-language-aware-context-annotations.md) | Prepend per-file [cxpak] annotations using language-correct comment syntax | ACCEPTED | 2026-03-19 |
| [0072](0072-max-symbol-token-chunk-splitting.md) | Split symbols over 4000 tokens via AST-aware then blank-line then hard fallback | ACCEPTED | 2026-03-19 |
| [0073](0073-progressive-degradation-five-detail-levels.md) | Five-level progressive degradation model for budget-constrained rendering | ACCEPTED | 2026-03-19 |
| [0074](0074-allocate-with-degradation-by-reference.md) | allocate_with_degradation operates on &[(&IndexedFile, FileRole, f64)] references, not owned files | ACCEPTED | 2026-03-20 |
| [0075](0075-expansion-via-optional-tokens-no-trait-change.md) | Wire query expansion through optional expanded_tokens without changing the RelevanceScorer trait | ACCEPTED | 2026-03-20 |
| [0076](0076-convention-based-orm-table-name-resolution.md) | Resolve ORM model table names by convention with deterministic override detection, no inflector dependency | ACCEPTED | 2026-03-21 |
| [0077](0077-directory-pattern-migration-ordering.md) | Order migrations by directory pattern + filename sequence, content-reading only for Alembic | ACCEPTED | 2026-03-21 |
| [0078](0078-language-agnostic-embedded-sql-detection.md) | Detect embedded SQL by scanning string literals with a structural-keyword guard, not via framework/API awareness | ACCEPTED | 2026-03-21 |
| [0079](0079-regex-based-sql-ddl-extraction.md) | Extract SQL column-level schema via regex on the DDL body rather than tree-sitter re-parse | ACCEPTED | 2026-03-21 |
| [0080](0080-schema-index-as-separate-optional-structure.md) | Model the data layer as a separate Option<SchemaIndex> on CodebaseIndex, not embedded in Symbol | ACCEPTED | 2026-03-21 |
| [0081](0081-typed-dependency-graph-edges.md) | Replace string edge sets with TypedEdge carrying an EdgeType enum | ACCEPTED | 2026-03-21 |
| [0082](0082-api-surface-signatures-only-regex-routes.md) | Extract API surface as signatures-only plus regex-based HTTP route detection for 12 frameworks | ACCEPTED | 2026-03-22 |
| [0083](0083-auto-context-one-call-pipeline.md) | Compose all cxpak intelligence into a single cxpak_auto_context tool with a fixed JSON briefing | ACCEPTED | 2026-03-22 |
| [0084](0084-blast-radius-reverse-bfs-multiplicative-risk.md) | Compute blast radius via reverse BFS with a multiplicative risk score and single-category assignment | ACCEPTED | 2026-03-22 |
| [0085](0085-cache-dependency-graph-on-codebaseindex.md) | Build and cache the DependencyGraph once on CodebaseIndex instead of on-demand per call site | ACCEPTED | 2026-03-22 |
| [0086](0086-context-diff-in-memory-snapshot.md) | Implement cxpak_context_diff via an in-memory snapshot in a separate Arc<RwLock>, with git-ref fallback | ACCEPTED | 2026-03-22 |
| [0087](0087-fill-then-overflow-budget-allocation.md) | Allocate the auto_context token budget by strict-priority fill-then-overflow, not fixed proportions | ACCEPTED | 2026-03-22 |
| [0088](0088-local-embeddings-minilm-with-byok-providers.md) | Add a 7th embedding signal via local candle MiniLM by default with optional BYOK remote providers | ACCEPTED | 2026-03-22 |
| [0089](0089-pagerank-on-dependency-graph-as-importance.md) | Use standard PageRank over the typed dependency graph for file importance, computed at build time | ACCEPTED | 2026-03-22 |
| [0090](0090-test-mapping-naming-plus-imports.md) | Map tests to sources via naming conventions plus import analysis, not content matching | ACCEPTED | 2026-03-22 |
| [0091](0091-three-layer-noise-filtering.md) | Filter context noise in three layers: blocklist, Jaccard similarity dedup, and a 0.15 relevance floor | ACCEPTED | 2026-03-22 |
| [0092](0092-v1-mcp-api-semver-contract.md) | Freeze the MCP tool API under semver at v1.0.0; CLI/HTTP/internal structure remain unstable | ACCEPTED | 2026-03-22 |
| [0093](0093-convention-profile-cached-on-index.md) | Convention profile built at index time and cached on CodebaseIndex with incremental updates | ACCEPTED | 2026-03-27 |
| [0094](0094-convention-strength-thresholds.md) | Tiered convention strength labels (Convention >=90%, Trend 70-89%, Mixed 50-69%, below 50% unreported) | ACCEPTED | 2026-03-27 |
| [0095](0095-deterministic-suggestions-only.md) | Verify emits suggestions only when deterministic, null when judgment is required | ACCEPTED | 2026-03-27 |
| [0096](0096-dna-section-budget-tiering.md) | DNA section is step 0 of auto_context, deducted before fill-then-overflow, never degraded, budget-tiered | ACCEPTED | 2026-03-27 |
| [0097](0097-report-what-is-not-should-be.md) | Conventions report what IS (evidence-based), never what SHOULD BE (prescriptive) | ACCEPTED | 2026-03-27 |
| [0098](0098-two-git-churn-windows.md) | Git health uses two churn windows (30d and 180d) rather than one | ACCEPTED | 2026-03-27 |
| [0099](0099-verify-changed-lines-only-via-git2.md) | cxpak_verify checks only changed lines, scoped via git2 (no git CLI) | ACCEPTED | 2026-03-27 |
| [0100](0100-briefing-mode-optional-content-type.md) | Briefing mode shares full mode's structure with content as Option<String> (None in briefing) | ACCEPTED | 2026-03-31 |
| [0101](0101-call-graph-tiered-confidence.md) | Call graph uses tree-sitter extraction for Tier 1 languages and regex for Tier 2, tagged by confidence | ACCEPTED | 2026-03-31 |
| [0102](0102-co-change-mining-threshold-and-decay.md) | Co-change edges mined from git, recency-decayed, piggybacking the git_health walk (>=3-commit threshold designed but unwired) | ACCEPTED | 2026-03-31 |
| [0103](0103-composite-health-score-six-dimensions.md) | Composite health score from six weighted dimensions with renormalization for null dead_code | ACCEPTED | 2026-03-31 |
| [0104](0104-convention-export-portable-artifact.md) | Conventions become a portable, versioned, checksummed export artifact (.cxpak/conventions.json) | ACCEPTED | 2026-03-31 |
| [0105](0105-cross-language-edge-type-graph-migration.md) | Cross-language edges added as EdgeType::CrossLanguage(BridgeType); EdgeType+TypedEdge moved to index/graph.rs | ACCEPTED | 2026-03-31 |
| [0106](0106-data-flow-structural-not-taint.md) | Data flow is structural call-graph tracing with confidence tagging, explicitly not taint analysis | ACCEPTED | 2026-03-31 |
| [0107](0107-dead-code-deterministic-classification.md) | Dead code is a deterministic binary classification; liveness_score is only a sort key | ACCEPTED | 2026-03-31 |
| [0108](0108-drift-snapshot-baseline-not-git-reconstruction.md) | Architecture drift uses stored snapshots plus a baseline, not git-diff reconstruction | ACCEPTED | 2026-03-31 |
| [0109](0109-incremental-indexing-via-mutation-api.md) | Incremental indexing: file-level mtime/size invalidation for parsing, full recompute for graph-derived scores | ACCEPTED | 2026-03-31 |
| [0110](0110-intelligence-api-versioned-localhost-bearer.md) | Intelligence HTTP API is versioned (/v1/), localhost-bound by default, optional bearer token, path-traversal guarded | ACCEPTED | 2026-03-31 |
| [0111](0111-lsp-supplementary-intelligence-server.md) | LSP server is supplementary intelligence over stdio (14 custom cxpak/* methods), gated behind lsp feature depending on daemon | ACCEPTED | 2026-03-31 |
| [0112](0112-multiplicative-risk-score-with-floor.md) | Standing risk score is multiplicative with a 0.01 floor and percentile-rank churn normalization | ACCEPTED | 2026-03-31 |
| [0113](0113-onboarding-reading-order-four-factors.md) | Onboarding map computes reading order from PageRank, topological dependency order, module grouping, and complexity progression | ACCEPTED | 2026-03-31 |
| [0114](0114-precomputed-sugiyama-layout-self-contained-html.md) | Visualizations use Rust-precomputed Sugiyama layout shipped as self-contained HTML with inlined D3 | ACCEPTED | 2026-03-31 |
| [0115](0115-prediction-signal-confidence-merging.md) | Change-impact test prediction merges three independent signals into seven confidence tiers | ACCEPTED | 2026-03-31 |
| [0116](0116-recency-scoring-signal-weight.md) | Recency becomes the 5th relevance signal at weight 0.05 with linear 90-day decay, rebalancing all weights | ACCEPTED | 2026-03-31 |
| [0117](0117-security-surface-per-type-secret-regex.md) | Security surface uses five deterministic detections with per-type secret regexes, not entropy matching | ACCEPTED | 2026-03-31 |
| [0118](0118-tarjan-scc-for-circular-deps.md) | Circular dependency detection via Tarjan's SCC, not full cycle enumeration | ACCEPTED | 2026-03-31 |
| [0119](0119-wasm-plugin-sdk-sandboxing.md) | Plugin SDK uses WASM (wasmtime) with declared file-pattern scoping and 1MB return limits for sandboxing | ACCEPTED | 2026-03-31 |
| [0120](0120-architecture-drift-snapshot-persistence.md) | Architecture drift detection via persisted JSON snapshots and a stored baseline | ACCEPTED | 2026-04-01 |
| [0121](0121-architecture-explorer-three-level-lazy-level3.md) | Three-level semantic zoom with lazy level-3 (top-20 by PageRank for static export) | ACCEPTED | 2026-04-01 |
| [0122](0122-architecture-quality-cohesion-boundary-god-files.md) | Per-module architecture quality metrics: cohesion ratio, root-file boundary violations, mean+2σ god files | ACCEPTED | 2026-04-01 |
| [0123](0123-briefing-mode-content-option-string.md) | Briefing mode: strip source by making PackedFile.content an Option<String> set to None | ACCEPTED | 2026-04-01 |
| [0124](0124-call-graph-two-tier-confidence.md) | Cross-file call graph with Exact/Approximate confidence and tree-sitter + regex two-tier extraction | ACCEPTED | 2026-04-01 |
| [0125](0125-change-impact-prediction-three-signal-confidence.md) | Change-impact prediction merging three signals into seven discrete confidence levels | ACCEPTED | 2026-04-01 |
| [0126](0126-co-change-mining-180d-window-threshold-3.md) | Git co-change mining: 180-day window, >=3 co-commit threshold, linear recency decay to 0.3 | ACCEPTED | 2026-04-01 |
| [0127](0127-cognitive-limit-7-plus-minus-2-clustering.md) | Cap graph layers and onboarding phases at 9 nodes (7±2 cognitive limit) | ACCEPTED | 2026-04-01 |
| [0128](0128-convention-export-versioned-checksummed-artifact.md) | Versioned, SHA256-checksummed convention export with deterministic canonical JSON | ACCEPTED | 2026-04-01 |
| [0129](0129-cross-language-bridge-detection-six-types.md) | Cross-language bridge detection across six bridge types, injected post-build as graph edges | ACCEPTED | 2026-04-01 |
| [0130](0130-data-flow-tracing-depth-10-three-confidence.md) | Structural data-flow tracing over the call graph: max depth 10, three-level confidence, four boundary flags | ACCEPTED | 2026-04-01 |
| [0131](0131-dead-code-detection-entry-point-rules.md) | Dead-code detection: zero callers AND not an entry point AND no test reference, ranked by liveness score | ACCEPTED | 2026-04-01 |
| [0132](0132-edgetype-relocation-cross-language-variant.md) | Relocate EdgeType/TypedEdge into index::graph and add CrossLanguage(BridgeType) variant | ACCEPTED | 2026-04-01 |
| [0133](0133-incremental-rebuild-mtime-tracking.md) | Incremental index rebuild via mtime/size tracking on IndexedFile | ACCEPTED | 2026-04-01 |
| [0134](0134-lsp-server-tower-lsp-feature-gated.md) | LSP server over stdio via tower-lsp behind an lsp feature flag, with 4 standard + 14 custom methods | ACCEPTED | 2026-04-01 |
| [0135](0135-mcp-html-inline-1mb-spill-to-disk.md) | Spill MCP visual HTML responses over 1MB to disk, return file path | ACCEPTED | 2026-04-01 |
| [0136](0136-monorepo-workspace-prefix-scoping.md) | Monorepo workspace support via path-prefix scoping and per-workspace cache namespaces | ACCEPTED | 2026-04-01 |
| [0137](0137-onboarding-canonical-in-intelligence-thin-visual-layer.md) | Canonical onboarding logic in intelligence module; visual layer is render-only | ACCEPTED | 2026-04-01 |
| [0138](0138-onboarding-topo-order-kahn-lexicographic-cyclebreak.md) | Dependency-first onboarding order via Kahn topological sort with lexicographic cycle-break | ACCEPTED | 2026-04-01 |
| [0139](0139-plugin-security-model.md) | Plugin security model: checksum + pattern scoping + content opt-in + size limits | ACCEPTED | 2026-04-01 |
| [0140](0140-png-via-resvg-rasterize-svg.md) | Render PNG by rasterizing self-generated SVG via resvg | ACCEPTED | 2026-04-01 |
| [0141](0141-precompute-layout-in-rust-no-sigma.md) | Pre-compute graph layout in Rust (simplified Sugiyama), not client-side JS | ACCEPTED | 2026-04-01 |
| [0142](0142-security-surface-five-deterministic-detections.md) | Security surface: five deterministic regex/heuristic detections with exclusions and redaction | ACCEPTED | 2026-04-01 |
| [0143](0143-self-contained-html-inlined-d3-bundle.md) | Self-contained HTML output with an inlined custom D3 bundle (no CDN) | ACCEPTED | 2026-04-01 |
| [0144](0144-standing-risk-multiplicative-percentile-churn.md) | Standing per-file risk = floored-percentile-churn x normalized-blast (no floor) x floored-lack-of-tests | ACCEPTED | 2026-04-01 |
| [0145](0145-timeline-snapshots-git-diff-no-reparse.md) | Time Machine snapshots from lightweight tree-walk deltas (no re-parse), sampled and cached | ACCEPTED | 2026-04-01 |
| [0146](0146-versioned-http-intelligence-api-bearer-auth.md) | Versioned /v1/ HTTP Intelligence API with bearer-token auth and path-traversal defense | ACCEPTED | 2026-04-01 |
| [0147](0147-visual-export-format-matrix.md) | Six-format visual export matrix (HTML, Mermaid, SVG, PNG, C4 DSL, JSON) | ACCEPTED | 2026-04-01 |
| [0148](0148-visual-plugins-feature-flag-boundary.md) | Gate visual (resvg) and plugins (wasmtime) behind feature flags | ACCEPTED | 2026-04-01 |
| [0149](0149-wasm-plugin-host-via-wasmtime.md) | WASM as the plugin execution model, hosted via wasmtime | ACCEPTED | 2026-04-01 |
| [0150](0150-command-palette-search-index.md) | Rust-precomputed search index with subsequence fuzzy match and 20k entry cap | ACCEPTED | 2026-04-17 |
| [0151](0151-cross-process-golden-fixture-determinism.md) | Cross-process golden-fixture determinism test with centralized timestamp redaction | ACCEPTED | 2026-04-17 |
| [0152](0152-csp-accepted-inline-risk-serve-header.md) | Accept inline JS/CSS (unsafe-inline) as required; cxpak serve sets a restrictive CSP, file:// runs without CSP | ACCEPTED | 2026-04-17 |
| [0153](0153-data-integrity-single-source-intelligence.md) | Every SPA number must equal the intelligence function output via JSON round-trip comparison | ACCEPTED | 2026-04-17 |
| [0154](0154-dom-injection-safety-textcontent-setattribute.md) | All user-derived strings reach the DOM only via textContent/setAttribute; all JSON tags pass through escape_script_tag | ACCEPTED | 2026-04-17 |
| [0155](0155-layoutnode-aria-label-serde-default.md) | aria_label field on LayoutNode built in Rust, with serde(default) for cache back-compat and a content allowlist | ACCEPTED | 2026-04-17 |
| [0156](0156-mcp-visual-slug-closed-enum-canonicalize.md) | MCP visual_type slug validated against a closed enum returning &'static str, plus canonicalize-and-verify on the output path | ACCEPTED | 2026-04-17 |
| [0157](0157-risk-ranking-explicit-tiebreak.md) | compute_risk_ranking gets an explicit path-ascending secondary sort key | ACCEPTED | 2026-04-17 |
| [0158](0158-serve-loopback-only-without-token.md) | cxpak serve without --token binds loopback only and rejects non-loopback binds; header-only bearer auth | ACCEPTED | 2026-04-17 |
| [0159](0159-spa-controller-as-includestr-asset.md) | SPA controller JS lives in an asset file inlined via include_str!, not a Rust string literal | ACCEPTED | 2026-04-17 |
| [0160](0160-spa-single-html-hash-routing.md) | Single self-contained HTML SPA with client-side hash routing for all six views | ACCEPTED | 2026-04-17 |
| [0161](0161-theme-attribute-not-light-dark.md) | Attribute-based theming (data-theme on <html>) instead of CSS light-dark() | ACCEPTED | 2026-04-17 |
| [0162](0162-v1-api-param-normalization-two-normalizers.md) | v1 API parameter validation: two normalizers (path vs symbol), JSON error envelope, and caps | ACCEPTED | 2026-04-17 |
| [0163](0163-windows-build-git2-no-default-features.md) | Enable Windows builds by dropping git2 default features (cxpak does only local git) | ACCEPTED | 2026-06-14 |
| [0164](0164-v230-single-additive-release-no-gates.md) | Ship v2.3.0 as a single additive release (scale + cost + review), no feature gates | ACCEPTED | 2026-06-14 |
| [0165](0165-warm-started-pagerank.md) | Warm-started PageRank via an optional seed vector, converging to the same threshold | ACCEPTED | 2026-06-14 |
| [0166](0166-incremental-graph-edge-delta.md) | Incremental dependency-graph updates as an edge-delta extension, with the full rebuild as parity oracle | ACCEPTED | 2026-06-14 |
| [0167](0167-persistent-derived-index-cache.md) | Persist the derived index with a fail-closed, content-addressed fingerprint | ACCEPTED | 2026-06-16 |
| [0168](0168-efficiency-report-and-filtered-tokens.md) | Efficiency report as decision-support (relevant-set coverage + budget-margin), aggregated from existing data; cost estimate opt-in | ACCEPTED | 2026-06-16 |
| [0169](0169-versioned-context-format-schema.md) | Publish a versioned context-format schema over the existing JSON output, not a new format | ACCEPTED | 2026-06-14 |
| [0170](0170-official-container-images-from-ci.md) | Publish official multi-arch container images from CI, built from release artifacts and signed | ACCEPTED | 2026-06-18 |
| [0171](0171-review-mode-extends-diff.md) | Review mode as a --review extension of cxpak diff, composing existing intelligence + expected-but-absent detection | ACCEPTED | 2026-06-16 |
| [0172](0172-recall-regression-ci-gate.md) | CI recall-regression gate on a bounded subset; gate recall@budget, track MRR | ACCEPTED | 2026-06-22 |
| [0173](0173-live-db-introspection-rustls.md) | Live DB introspection + schema drift; rustls drivers (tokio-postgres + mysql_async), feature-gated, DSN-scrubbed, read-only | ACCEPTED | 2026-06-24 |
| [0174](0174-column-level-lineage-blast-radius.md) | Column-level lineage + column-granular blast radius; columns as graph nodes, `EdgeType::ColumnReference`, per-edge confidence | ACCEPTED | 2026-06-24 |
| [0175](0175-surface-edge-confidence-in-outputs.md) | Surface `EdgeConfidence`: tag only `Inferred` edges (`, inferred`) across overview/trace/auto_context renderings + an `INFORMATION` LSP diagnostic; canonical `EdgeType::label()` | ACCEPTED | 2026-06-30 |
| [0176](0176-graph-query-capability-four-surfaces.md) | Deterministic graph-query (node/neighbors/path/subgraph) from one `intelligence::graph_query` core, projected to CLI/HTTP/LSP live + MCP via catalog adapter (≤8); lex-min shortest-path tiebreak | ACCEPTED | 2026-06-30 |
| [0177](0177-cypher-graphml-graph-export.md) | Cypher + GraphML dependency-graph export: serialize `DependencyGraph` directly (honest `EdgeType`+`EdgeConfidence`), idempotent `MERGE` + fixed `:DEPENDS_ON`, reuse `xml_escape`, no new deps, no live Neo4j (validity via structural+escaping+determinism asserts) | ACCEPTED | 2026-06-30 |
| [0178](0178-post-commit-rebuild-union-merge-driver.md) | Post-commit auto-rebuild + union-merge driver via `cxpak hook`: line-oriented committable `.cxpak/graph.edges`, best-effort non-fatal post-commit (exit 0, `CXPAK_NO_HOOK`), commutative union merge driver, idempotent install into target repo only; reuses parse cache + asserts incremental==full | ACCEPTED | 2026-06-30 |
