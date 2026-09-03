//! #43: every markdown fence around repo text is sized by `output::fenced`.
//!
//! The six sites that packed a file body, a symbol body or a diff hunk each wrote
//! ``` by hand, and a README with its own code block closed the block early. Fixing
//! the six is not the same as stopping a seventh, and the unit tests pin only the
//! sites they name. This is the completeness half.
//!
//! Its denominator comes from the SOURCE TREE, not from a list of known sites — a
//! guard that enumerated the call sites it already knows about could only ever
//! confirm those, which is the failure mode this file exists to avoid.

use std::path::Path;

/// Lines outside `#[cfg(test)]`, with `//` comments stripped.
///
/// Depth-counted rather than "everything after the first `mod tests`": a file can
/// have several test modules, and code after one of them is production code.
fn production_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut test_depth: Option<i32> = None;
    let mut pending_cfg_test = false;

    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if let Some(depth) = test_depth.as_mut() {
            *depth += raw.matches('{').count() as i32 - raw.matches('}').count() as i32;
            if *depth <= 0 {
                test_depth = None;
            }
            continue;
        }
        if line.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        if pending_cfg_test {
            pending_cfg_test = false;
            let depth = raw.matches('{').count() as i32 - raw.matches('}').count() as i32;
            if depth > 0 {
                test_depth = Some(depth);
                continue;
            }
        }
        let code = match line.find("//") {
            Some(0) => continue,
            Some(n) => &line[..n],
            None => line,
        };
        out.push((i + 1, code.to_string()));
    }
    out
}

#[test]
fn no_command_writes_a_markdown_fence_by_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let mut scanned = 0usize;
    let mut offenders: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&root).expect("src/commands must exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("readable source");
        scanned += 1;
        for (line_no, code) in production_lines(&src) {
            if code.contains("```") {
                offenders.push(format!(
                    "{}:{line_no}: {code}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }

    // Positive control: a guard that scanned nothing would pass silently, and a
    // renamed or moved directory is exactly how that happens.
    assert!(
        scanned >= 5,
        "only {scanned} file(s) scanned under {} — the guard is not looking where it thinks",
        root.display()
    );

    assert!(
        offenders.is_empty(),
        "a fence written by hand cannot be closed by the content it wraps — use \
         `crate::output::fenced`:\n  {}",
        offenders.join("\n  ")
    );
}

/// The control for the control: `production_lines` must actually see production
/// code and actually skip test code. Without this, a bug that made it return
/// nothing would turn the guard above permanently green.
#[test]
fn the_scanner_separates_production_from_test_code() {
    let src = "fn a() { let x = \"```\"; }\n\
               #[cfg(test)]\n\
               mod tests {\n\
               fn t() { let y = \"```\"; }\n\
               }\n\
               fn b() { let z = \"```\"; }\n";
    let lines = production_lines(src);
    let hits: Vec<usize> = lines
        .iter()
        .filter(|(_, c)| c.contains("```"))
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        hits,
        vec![1, 6],
        "lines 1 and 6 are production, line 4 is not"
    );
}
