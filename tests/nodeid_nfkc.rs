//! NFKC identifier normalization tests (Task 0.3).
//!
//! Verifies that Unicode-equivalent identifier forms collapse to a single term
//! key in the symbol cross-reference map and that symbol_importance scores them
//! consistently.  The "ghost-node" bug: a non-ASCII identifier written in NFC
//! in one file and NFD in another produced two distinct map keys, so a
//! genuinely cross-referenced symbol was mis-scored as un-referenced.
//!
//! NFC/NFD pair chosen: Cyrillic small letter IO (ё).
//!   NFC  — U+0451 (1 codepoint, 2 bytes: 0xD1 0x91)
//!   NFD  — U+0435 U+0308 (2 codepoints, 3 bytes: 0xD0 0xB5 0xCC 0x88)
//! NFKC normalizes both forms to U+0451, so they share a single term key after
//! normalization.

use std::collections::HashMap;

use cxpak::core_graph::index::{compute_term_frequencies, normalize_identifier, split_identifier};
use cxpak::intelligence::pagerank::{build_symbol_cross_refs, symbol_importance};
use cxpak::parser::language::{Symbol, SymbolKind, Visibility};

// ── constants ────────────────────────────────────────────────────────────────

/// ё in NFC: single codepoint U+0451 (2 bytes in UTF-8).
const YO_NFC: &str = "\u{0451}";

/// ё in NFD: base е (U+0435) + combining diaeresis (U+0308) (3 bytes in UTF-8).
const YO_NFD: &str = "\u{0435}\u{0308}";

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_symbol(name: &str, vis: Visibility) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: vis,
        signature: format!("fn {}()", name),
        body: "{}".to_string(),
        start_line: 1,
        end_line: 1,
    }
}

// ── test 1: raw bytes differ but NFKC-equal ───────────────────────────────────

#[test]
fn yo_bytes_differ_but_nfkc_equal() {
    // Prove the pair is genuinely different bytes so the test cannot
    // silently pass on identical input.
    assert_ne!(
        YO_NFC.as_bytes(),
        YO_NFD.as_bytes(),
        "NFC and NFD byte representations must differ"
    );

    // Prove normalize_identifier maps both forms to the same string.
    let n_nfc = normalize_identifier(YO_NFC);
    let n_nfd = normalize_identifier(YO_NFD);
    assert_eq!(
        n_nfc, n_nfd,
        "normalize_identifier must map NFC and NFD to the same string"
    );
}

// ── test 2: ghost-node collapse ───────────────────────────────────────────────
//
// File A's content mentions the identifier in NFC form; file B's in NFD.
// After normalization both should produce the SAME term key, so
// build_symbol_cross_refs yields ONE entry containing BOTH files.

#[test]
fn ghost_nodes_collapse_to_one_entry() {
    // File A: contains the NFC form of ё as a standalone "identifier"
    let content_a = format!("function {} end", YO_NFC);
    // File B: contains the NFD form
    let content_b = format!("function {} end", YO_NFD);

    let tf_a = compute_term_frequencies(&content_a);
    let tf_b = compute_term_frequencies(&content_b);

    // Both files must produce at least one term from the identifier.
    assert!(
        !tf_a.is_empty(),
        "file A should produce at least one term from the NFC identifier"
    );
    assert!(
        !tf_b.is_empty(),
        "file B should produce at least one term from the NFD identifier"
    );

    let mut term_frequencies: HashMap<String, HashMap<String, u32>> = HashMap::new();
    term_frequencies.insert("file_a.rs".to_string(), tf_a);
    term_frequencies.insert("file_b.rs".to_string(), tf_b);

    let cross_refs = build_symbol_cross_refs(&term_frequencies);

    // The normalized key for ё (NFKC-lowercased).
    let normalized_key = normalize_identifier(YO_NFC);

    let entry = cross_refs.get(&normalized_key).unwrap_or_else(|| {
        panic!(
            "expected cross_refs to contain key {:?} (normalized ё); \
             keys present: {:?}",
            normalized_key,
            cross_refs.keys().collect::<Vec<_>>()
        )
    });

    assert!(
        entry.contains("file_a.rs"),
        "cross-ref entry for ё must include file_a.rs"
    );
    assert!(
        entry.contains("file_b.rs"),
        "cross-ref entry for ё must include file_b.rs"
    );

    // There must NOT be a second ghost key for the NFD form.
    let nfd_key = {
        // What a naïve to_lowercase() would produce for the NFD form.
        let mut nfd_only_key = YO_NFD.to_string();
        nfd_only_key = nfd_only_key.to_lowercase();
        nfd_only_key
    };
    if nfd_key != normalized_key {
        assert!(
            !cross_refs.contains_key(&nfd_key),
            "cross_refs must NOT contain a separate ghost entry for the NFD form"
        );
    }
}

// ── test 3: symbol_importance scores cross-form-referenced symbol 1.0 ────────
//
// Public symbol defined in file A (NFC name), referenced in file B (NFD form).
// symbol_importance must return 1.0 (cross-referenced weight), not 0.7 (isolated).
// This proves both production and lookup sites normalize identically.

#[test]
fn cross_form_reference_scores_1_0() {
    // Term frequencies: NFC form in file_a, NFD form in file_b.
    let tf_a = compute_term_frequencies(&format!("function {} end", YO_NFC));
    let tf_b = compute_term_frequencies(&format!("call {} here", YO_NFD));

    let mut term_frequencies: HashMap<String, HashMap<String, u32>> = HashMap::new();
    term_frequencies.insert("file_a.rs".to_string(), tf_a);
    term_frequencies.insert("file_b.rs".to_string(), tf_b);

    let cross_refs = build_symbol_cross_refs(&term_frequencies);

    // Symbol defined in file_a with the NFC name.
    let sym = make_symbol(YO_NFC, Visibility::Public);

    // file_pagerank of 1.0 isolates the weight factor.
    let importance = symbol_importance(&sym, 1.0, &cross_refs, "file_a.rs");

    assert!(
        (importance - 1.0).abs() < 1e-9,
        "public symbol cross-referenced across NFC/NFD forms must score 1.0 \
         (weight = 1.0), got {importance}"
    );
}

// ── test 4: ASCII identifier produces byte-identical terms ───────────────────
//
// Normalization must be a no-op for pure ASCII identifiers so that existing
// behaviour and the spa_determinism golden fixture are unaffected.

#[test]
fn ascii_identifier_is_unchanged() {
    let ident = "computePageRank";

    let before: Vec<String> = {
        // Reproduce the pre-normalization logic: split_identifier already
        // lowercases each part.
        let mut parts = Vec::new();
        for segment in ident.split('_') {
            if segment.is_empty() {
                continue;
            }
            let mut current = String::new();
            let chars: Vec<char> = segment.chars().collect();
            for (i, &ch) in chars.iter().enumerate() {
                if i > 0 && ch.is_uppercase() {
                    if !current.is_empty() {
                        parts.push(current.to_lowercase());
                    }
                    current = String::new();
                }
                current.push(ch);
            }
            if !current.is_empty() {
                parts.push(current.to_lowercase());
            }
        }
        parts
    };

    let after = split_identifier(ident);

    assert_eq!(
        before, after,
        "split_identifier must produce byte-identical results for ASCII identifiers"
    );
}
