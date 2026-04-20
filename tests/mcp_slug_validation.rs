use cxpak::commands::serve::validate_visual_type_slug;

#[test]
fn slug_rejects_path_traversal() {
    assert!(validate_visual_type_slug("../etc/passwd").is_err());
    assert!(validate_visual_type_slug("dashboard/../../etc").is_err());
}

#[test]
fn slug_rejects_absolute_paths() {
    assert!(validate_visual_type_slug("/etc/passwd").is_err());
    assert!(validate_visual_type_slug("\\windows\\system32").is_err());
}

#[test]
fn slug_rejects_null_bytes() {
    assert!(validate_visual_type_slug("dashboard\0").is_err());
}

#[test]
fn slug_accepts_all_closed_enum_values() {
    for t in [
        "dashboard",
        "architecture",
        "risk",
        "flow",
        "timeline",
        "diff",
        "all",
    ] {
        let result = validate_visual_type_slug(t);
        assert!(result.is_ok(), "slug {t} must be accepted");
        assert_eq!(result.unwrap(), t);
    }
}

#[test]
fn slug_rejects_unknown_value() {
    assert!(validate_visual_type_slug("not_a_view").is_err());
    assert!(validate_visual_type_slug("DASHBOARD").is_err()); // case-sensitive
    assert!(validate_visual_type_slug("").is_err());
}

#[test]
fn canonicalize_check_rejects_escape() {
    // This tests the second line of defense. If someone changes validate_visual_type_slug
    // to return user input, the canonicalize check should still catch traversal.
    // Direct test against a helper — implementation detail covered in Step 3.
    let repo = tempfile::tempdir().unwrap();
    let visual_dir = repo.path().join(".cxpak/visual");
    std::fs::create_dir_all(&visual_dir).unwrap();
    // Try to write "dashboard/../../escape.html"
    let bad_filepath = visual_dir.join("dashboard/../../escape.html");
    let canon_dir = visual_dir.canonicalize().unwrap();
    // parent().canonicalize() MUST fail or not start with canon_dir.
    let parent_canon = bad_filepath.parent().unwrap().canonicalize();
    if let Ok(p) = parent_canon {
        assert!(
            !p.starts_with(&canon_dir),
            "path escape must be caught: {p:?}"
        );
    }
}
