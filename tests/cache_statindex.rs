/// Integration tests for the stat-index fast-path (Task 0.2).
///
/// The stat-index accelerates `content_fingerprint` by skipping SHA-256
/// re-computation for files whose `(mtime_ns, size)` are unchanged.  These
/// tests verify:
///
///   1. **Zero-rehash** – unchanged files are served entirely from the index;
///      the content-hash function is never invoked for them.
///   2. **Invalidation / correctness** – a file that changes (mtime_ns or size)
///      is re-hashed and the fingerprint changes; unchanged siblings are not.
///   3. **Markdown frontmatter stripping** – a `.md` file whose frontmatter
///      changes but body is identical produces the same fingerprint; a body
///      change flips it.
///   4. **Cross-clone / portability** – computing the fingerprint without a
///      stat-index (fresh) produces the same value as via the fast-path.
use cxpak::cache::{content_fingerprint_with_stat_index, StatIndex, StatIndexEntry};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A SHA-256 implementation that counts every invocation.  The counting is
/// coordinated through an `Arc<AtomicUsize>` so that multiple files can be
/// processed and the total call count can be inspected by the test.
struct CountingHasher {
    counter: Arc<AtomicUsize>,
}

impl CountingHasher {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        Self { counter }
    }

    fn hash(&self, content: &str) -> String {
        self.counter.fetch_add(1, Ordering::SeqCst);
        // Delegate to the real SHA-256 so the fingerprint values are genuine.
        sha256_of(content)
    }
}

/// Compute the canonical SHA-256 (after frontmatter stripping), matching
/// what the production path uses.
fn sha256_of(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(cxpak::cache::strip_md_frontmatter(content).as_bytes())
    )
}

/// A file entry as passed to `content_fingerprint_with_stat_index`:
/// `(relative_path, content, mtime_ns, size_bytes)`.
type FileEntry = (String, String, u64, u64);

/// Build a small in-memory file set with stable sentinel mtime values.
/// `mtime_ns` is set to a stable value per file based on its position
/// so each file has a unique, repeatable mtime.
fn make_files(items: &[(&str, &str)]) -> Vec<FileEntry> {
    items
        .iter()
        .enumerate()
        .map(|(i, (p, c))| {
            let mtime_ns = 1_700_000_000_000_000_000_u64 + i as u64 * 1_000_000_000;
            let size_bytes = c.len() as u64;
            (p.to_string(), c.to_string(), mtime_ns, size_bytes)
        })
        .collect()
}

/// Populate a `StatIndex` so every supplied file is a "hit" (matching
/// mtime_ns and size_bytes).
fn build_stat_index_for(files: &[FileEntry]) -> StatIndex {
    let mut idx = StatIndex::default();
    for (path, content, mtime_ns, size_bytes) in files {
        let hash = sha256_of(content);
        idx.entries.insert(
            path.clone(),
            StatIndexEntry {
                mtime_ns: *mtime_ns,
                size_bytes: *size_bytes,
                content_sha256: hash,
            },
        );
    }
    idx
}

// ---------------------------------------------------------------------------
// Test 1: Zero-rehash — unchanged files must not invoke the content hasher.
// ---------------------------------------------------------------------------
#[test]
fn zero_rehash_on_unchanged_files() {
    let files = make_files(&[
        ("src/a.rs", "fn a() {}"),
        ("src/b.rs", "fn b() {}"),
        ("src/c.rs", "fn c() {}"),
    ]);

    // Build a stat-index that matches the current (mtime_ns, size) for all files.
    let mut stat_index = build_stat_index_for(&files);

    let call_count = Arc::new(AtomicUsize::new(0));
    let hasher = CountingHasher::new(Arc::clone(&call_count));

    let _fp = content_fingerprint_with_stat_index(&files, "head-abc", &mut stat_index, |content| {
        hasher.hash(content)
    });

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "content hasher must not be called for any file when all are in the stat-index"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Invalidation — a changed file is re-hashed; siblings are not.
// ---------------------------------------------------------------------------
#[test]
fn invalidation_rehashes_changed_file_only() {
    let files_original = make_files(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);

    // Pre-populate stat-index for all files.
    let mut stat_index = build_stat_index_for(&files_original);

    // Simulate a content change to `src/a.rs`: new content with different
    // length so size_bytes changes, and bump mtime_ns.
    let new_content_a = "fn a() { 42 }"; // different content & length
    let files_updated = vec![
        (
            "src/a.rs".to_string(),
            new_content_a.to_string(),
            // Bump mtime_ns to simulate a file write.
            1_700_000_001_000_000_000_u64,
            new_content_a.len() as u64,
        ),
        // src/b.rs is unchanged — same mtime_ns and size as in stat_index.
        files_original[1].clone(),
    ];

    let call_count = Arc::new(AtomicUsize::new(0));
    let hasher = CountingHasher::new(Arc::clone(&call_count));

    let fp_updated = content_fingerprint_with_stat_index(
        &files_updated,
        "head-abc",
        &mut stat_index,
        |content| hasher.hash(content),
    );

    // Exactly one file was re-hashed (src/a.rs); src/b.rs came from the index.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "only the changed file should trigger a re-hash"
    );

    // The fingerprint must differ from the original (content-sensitivity preserved).
    let mut stat_index2 = build_stat_index_for(&files_original);
    let call_count2 = Arc::new(AtomicUsize::new(0));
    let hasher2 = CountingHasher::new(Arc::clone(&call_count2));
    let fp_original = content_fingerprint_with_stat_index(
        &files_original,
        "head-abc",
        &mut stat_index2,
        |content| hasher2.hash(content),
    );

    assert_ne!(
        fp_updated, fp_original,
        "fingerprint must change when file content changes"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Markdown frontmatter stripping.
// ---------------------------------------------------------------------------
#[test]
fn markdown_frontmatter_stripped_before_hash() {
    // Only the frontmatter changes — fingerprint must be identical.
    let frontmatter_v1 = "---\ndate: 2024-01-01\n---\n# Hello\nbody text";
    let frontmatter_v2 = "---\ndate: 2025-06-01\nupdated: true\n---\n# Hello\nbody text";

    // Different mtime_ns to prevent stat-index from short-circuiting — we want
    // to exercise the hasher path with frontmatter stripping active.
    let files_v1: Vec<FileEntry> = vec![(
        "README.md".to_string(),
        frontmatter_v1.to_string(),
        1_700_000_000_000_000_001_u64,
        frontmatter_v1.len() as u64,
    )];
    let files_v2: Vec<FileEntry> = vec![(
        "README.md".to_string(),
        frontmatter_v2.to_string(),
        1_700_000_000_000_000_002_u64, // different mtime
        frontmatter_v2.len() as u64,   // different size (different YAML header)
    )];

    // Empty stat-index — both calls go through the hasher.
    let mut empty_idx1 = StatIndex::default();
    let mut empty_idx2 = StatIndex::default();

    let fp1 = content_fingerprint_with_stat_index(&files_v1, "head", &mut empty_idx1, sha256_of);
    let fp2 = content_fingerprint_with_stat_index(&files_v2, "head", &mut empty_idx2, sha256_of);

    assert_eq!(
        fp1, fp2,
        "frontmatter-only change must not alter fingerprint"
    );

    // Body change MUST flip the fingerprint.
    let body_changed: Vec<FileEntry> = vec![(
        "README.md".to_string(),
        "---\ndate: 2024-01-01\n---\n# Hello\nDIFFERENT".to_string(),
        1_700_000_000_000_000_003_u64,
        45_u64,
    )];
    let mut empty_idx3 = StatIndex::default();
    let fp3 =
        content_fingerprint_with_stat_index(&body_changed, "head", &mut empty_idx3, sha256_of);

    assert_ne!(fp1, fp3, "body change in markdown must alter fingerprint");
}

// ---------------------------------------------------------------------------
// Test 4: Cross-clone portability — fast-path and full-hash agree.
// ---------------------------------------------------------------------------
#[test]
fn cross_clone_fast_path_equals_full_hash() {
    let files = make_files(&[
        ("src/lib.rs", "pub fn main() {}"),
        ("src/util.rs", "pub fn helper() {}"),
        ("README.md", "---\ntitle: test\n---\n# Readme"),
    ]);

    // Fresh clone: empty stat-index forces full hash computation.
    let mut empty_idx = StatIndex::default();
    let fp_full =
        content_fingerprint_with_stat_index(&files, "head-xyz", &mut empty_idx, sha256_of);

    // Warm clone: populated stat-index skips all hashes.
    let mut warm_idx = build_stat_index_for(&files);
    let call_count = Arc::new(AtomicUsize::new(0));
    let hasher = CountingHasher::new(Arc::clone(&call_count));

    let fp_fast =
        content_fingerprint_with_stat_index(&files, "head-xyz", &mut warm_idx, |content| {
            hasher.hash(content)
        });

    assert_eq!(
        fp_full, fp_fast,
        "fast-path and full-hash must produce identical fingerprints"
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "warm stat-index must skip all SHA-256 calls"
    );
}

// ---------------------------------------------------------------------------
// Test 5: strip_md_frontmatter unit tests.
// ---------------------------------------------------------------------------
#[test]
fn strip_md_frontmatter_leaves_non_md_content_untouched() {
    let rust_code = "fn main() { println!(\"hi\"); }";
    assert_eq!(cxpak::cache::strip_md_frontmatter(rust_code), rust_code);
}

#[test]
fn strip_md_frontmatter_removes_leading_yaml_block() {
    let md = "---\ntitle: test\ndate: 2024\n---\n# Body\ncontent here";
    let stripped = cxpak::cache::strip_md_frontmatter(md);
    assert_eq!(stripped, "# Body\ncontent here");
}

#[test]
fn strip_md_frontmatter_no_closing_delimiter_is_left_untouched() {
    // No closing `---` → not valid frontmatter; return unchanged.
    let md = "---\ntitle: test\n# Body without closing delimiter";
    assert_eq!(cxpak::cache::strip_md_frontmatter(md), md);
}

// ---------------------------------------------------------------------------
// Test 6: CACHE_VERSION is 6 (bumped in Task B3d for DerivedCache.base_commit).
// ---------------------------------------------------------------------------
#[test]
fn cache_version_is_6() {
    assert_eq!(
        cxpak::cache::CACHE_VERSION,
        6,
        "CACHE_VERSION must be 6 (bumped in Task B3d for base_commit on DerivedCache)"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Stale v3 cache is discarded, not mis-deserialized.
// ---------------------------------------------------------------------------
#[test]
fn stale_v3_stat_index_is_discarded() {
    let dir = tempfile::tempdir().unwrap();

    // Write a v3-format stat_index.json (wrong version).
    let stale = serde_json::json!({
        "version": 3,
        "entries": {}
    });
    std::fs::write(dir.path().join("stat_index.json"), stale.to_string()).unwrap();

    let loaded = StatIndex::load(dir.path());
    assert!(
        loaded.entries.is_empty(),
        "stale v3 stat-index must be discarded and return empty entries"
    );
}

// ---------------------------------------------------------------------------
// Test 8: StatIndex persists across save/load.
// ---------------------------------------------------------------------------
#[test]
fn stat_index_roundtrip_save_load() {
    let dir = tempfile::tempdir().unwrap();

    let mut idx = StatIndex::default();
    idx.entries.insert(
        "src/main.rs".to_string(),
        StatIndexEntry {
            mtime_ns: 1_700_000_000_000_000_042_u64,
            size_bytes: 128,
            content_sha256: "aabbcc".to_string(),
        },
    );

    idx.save(dir.path()).expect("save stat_index");

    let loaded = StatIndex::load(dir.path());
    assert_eq!(loaded.entries.len(), 1);
    let entry = loaded.entries.get("src/main.rs").unwrap();
    assert_eq!(entry.mtime_ns, 1_700_000_000_000_000_042_u64);
    assert_eq!(entry.size_bytes, 128);
    assert_eq!(entry.content_sha256, "aabbcc");
}

// ---------------------------------------------------------------------------
// Test 9: Body ending in `\n---` must NOT be over-stripped.
//
// Regression for the bug where `rest.ends_with("\n---")` caused the scanner
// to return `""` for any content whose body contained a horizontal rule at
// EOF — violating ADR-0167 content-sensitivity.
// ---------------------------------------------------------------------------
#[test]
fn strip_md_frontmatter_body_ending_in_hr_is_preserved() {
    // Valid frontmatter followed by a body that ends with a `---` HR (no
    // trailing newline).  The body must be returned intact.
    let md = "---\ntitle: X\n---\n# Body\n---";
    let stripped = cxpak::cache::strip_md_frontmatter(md);
    assert_eq!(
        stripped, "# Body\n---",
        "body ending in \\n--- must be preserved, not over-stripped"
    );

    // Same document but body ends with `---\n` (trailing newline).
    let md_nl = "---\ntitle: X\n---\n# Body\n---\n";
    let stripped_nl = cxpak::cache::strip_md_frontmatter(md_nl);
    assert_eq!(
        stripped_nl, "# Body\n---\n",
        "body ending in ---\\n must also be preserved"
    );

    // Two DISTINCT bodies ending in `\n---` must produce DISTINCT fingerprints
    // (content-sensitivity restored after the over-strip bug fix).
    let doc_a: Vec<(String, String, u64, u64)> = vec![(
        "doc.md".to_string(),
        "---\ntitle: A\n---\n# Alpha\n---".to_string(),
        1_u64,
        27_u64,
    )];
    let doc_b: Vec<(String, String, u64, u64)> = vec![(
        "doc.md".to_string(),
        "---\ntitle: B\n---\n# Beta body\n---".to_string(),
        2_u64,
        31_u64,
    )];

    let mut idx_a = StatIndex::default();
    let mut idx_b = StatIndex::default();

    let fp_a = content_fingerprint_with_stat_index(&doc_a, "head", &mut idx_a, sha256_of);
    let fp_b = content_fingerprint_with_stat_index(&doc_b, "head", &mut idx_b, sha256_of);

    assert_ne!(
        fp_a, fp_b,
        "distinct bodies ending in \\n--- must produce distinct fingerprints"
    );
}
