use cxpak::commands::serve::{normalize_path_param, normalize_symbol_param};

#[test]
fn path_rejects_traversal_segment() {
    assert!(normalize_path_param("foo/../bar").is_err());
}
#[test]
fn path_accepts_dots_in_filename() {
    assert!(normalize_path_param("foo..bar.txt").is_ok());
    assert!(normalize_path_param(".eslintrc..backup").is_ok());
}
#[test]
fn path_rejects_absolute() {
    assert!(normalize_path_param("/etc/passwd").is_err());
}
#[test]
fn path_rejects_backslash() {
    assert!(normalize_path_param("a\\b").is_err());
}
#[test]
fn path_rejects_null_byte() {
    assert!(normalize_path_param("a\0b").is_err());
}
#[test]
fn path_rejects_over_limit() {
    let s: String = std::iter::repeat_n('a', 1025).collect();
    assert!(normalize_path_param(&s).is_err());
}
#[test]
fn symbol_allows_generics() {
    assert!(normalize_symbol_param("Vec<String>").is_ok());
    assert!(normalize_symbol_param("std::vector<int>").is_ok());
}
#[test]
fn symbol_rejects_path_separators() {
    assert!(normalize_symbol_param("../secret").is_err());
    assert!(normalize_symbol_param("foo/bar").is_err());
}
#[test]
fn symbol_rejects_shell_chars() {
    for s in ["a`b", "a$b", "a;b", "a|b"] {
        assert!(normalize_symbol_param(s).is_err(), "{s}");
    }
}
