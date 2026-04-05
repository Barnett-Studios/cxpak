use regex::Regex;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SecretPattern {
    pub file: String,
    pub line: usize,
    pub pattern_name: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct SqlInjectionRisk {
    pub file: String,
    pub line: usize,
    pub language: String,
    pub snippet: String,
    pub interpolation_type: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationGap {
    pub file: String,
    pub function_name: String,
    pub parameter: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct UnprotectedEndpoint {
    pub file: String,
    pub method: String,
    pub path: String,
    pub handler: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct ExposureEntry {
    pub path: String,
    pub pub_symbol_count: usize,
    pub inbound_edges: usize,
    pub test_coverage: f64,
    pub exposure_score: f64,
}

#[derive(Debug, Serialize)]
pub struct SecuritySurface {
    pub unprotected_endpoints: Vec<UnprotectedEndpoint>,
    pub input_validation_gaps: Vec<ValidationGap>,
    pub secret_patterns: Vec<SecretPattern>,
    pub sql_injection_surface: Vec<SqlInjectionRisk>,
    pub exposure_scores: Vec<ExposureEntry>,
}

// ---------------------------------------------------------------------------
// Exclusion helpers
// ---------------------------------------------------------------------------

fn should_exclude_from_secret_scan(path: &str) -> bool {
    let lower = path.to_lowercase();
    if lower.contains("test") || lower.contains("spec") || lower.contains("__tests__") {
        return true;
    }
    let lock_files = [
        "cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "gemfile.lock",
        "poetry.lock",
        "composer.lock",
        "pipfile.lock",
    ];
    for lf in &lock_files {
        if lower.ends_with(lf) {
            return true;
        }
    }
    if lower.contains(".env.example") || lower.contains(".env.sample") {
        return true;
    }
    if lower.ends_with(".md") || lower.ends_with(".txt") || lower.ends_with(".rst") {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Secret pattern scanning
// ---------------------------------------------------------------------------

struct SecretSpec {
    name: &'static str,
    pattern: &'static str,
}

const SECRET_PATTERNS: &[SecretSpec] = &[
    SecretSpec {
        name: "aws_access_key",
        pattern: r"AKIA[0-9A-Z]{16}",
    },
    SecretSpec {
        name: "github_pat",
        pattern: r"ghp_[a-zA-Z0-9]{36}",
    },
    SecretSpec {
        name: "password_assignment",
        pattern: r#"(?i)(password|secret|api_key|token)\s*[:=]\s*["'][^"']{8,}["']"#,
    },
    SecretSpec {
        name: "connection_string",
        pattern: r"://[^:]+:[^@]+@",
    },
    SecretSpec {
        name: "slack_token",
        pattern: r"xox[baprs]-[0-9a-zA-Z-]{10,}",
    },
];

pub fn scan_secret_patterns(content: &str, file_path: &str) -> Vec<SecretPattern> {
    if should_exclude_from_secret_scan(file_path) {
        return vec![];
    }

    let mut results = Vec::new();

    for spec in SECRET_PATTERNS {
        let re = match Regex::new(spec.pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for cap in re.find_iter(content) {
            let line = content[..cap.start()]
                .chars()
                .filter(|&c| c == '\n')
                .count()
                + 1;
            let matched = cap.as_str();
            let snippet = if matched.len() > 4 {
                format!("{}...", &matched[..4])
            } else {
                "...".to_string()
            };
            results.push(SecretPattern {
                file: file_path.to_string(),
                line,
                pattern_name: spec.name.to_string(),
                snippet,
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// SQL injection scanning
// ---------------------------------------------------------------------------

fn detect_language_from_path(path: &str) -> &'static str {
    if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs") {
        "javascript"
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        "typescript"
    } else if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".java") {
        "java"
    } else {
        "unknown"
    }
}

fn is_parameterized(sql_fragment: &str) -> bool {
    Regex::new(r"\$\d+|\?|:\w+|@\w+")
        .map(|re| re.is_match(sql_fragment))
        .unwrap_or(false)
}

pub fn scan_sql_injection(content: &str, file_path: &str) -> Vec<SqlInjectionRisk> {
    let lang = detect_language_from_path(file_path);
    let mut results = Vec::new();

    let patterns: &[(&str, &str)] = match lang {
        "python" => &[
            (
                r#"f["']([^"']*SELECT[^"']*\{[^}]+\}[^"']*)["']"#,
                "f-string",
            ),
            (
                r#"f["']([^"']*INSERT[^"']*\{[^}]+\}[^"']*)["']"#,
                "f-string",
            ),
            (
                r#"f["']([^"']*UPDATE[^"']*\{[^}]+\}[^"']*)["']"#,
                "f-string",
            ),
            (
                r#"f["']([^"']*DELETE[^"']*\{[^}]+\}[^"']*)["']"#,
                "f-string",
            ),
            (
                r#"["']([^"']*SELECT[^"']*%s[^"']*)["']\s*%"#,
                "percent-format",
            ),
        ],
        "javascript" | "typescript" => &[(
            r"`([^`]*(?:SELECT|INSERT|UPDATE|DELETE)[^`]*\$\{[^}]+\}[^`]*)`",
            "template-literal",
        )],
        "rust" => &[(
            r#"format!\s*\(\s*"([^"]*(?:SELECT|INSERT|UPDATE|DELETE)[^"]*\{\}[^"]*)""#,
            "format-macro",
        )],
        "java" => &[(
            r#"["']([^"']*(?:SELECT|INSERT|UPDATE|DELETE)[^"']*)["']\s*\+"#,
            "string-concat",
        )],
        _ => &[],
    };

    for (pattern, interpolation_type) in patterns {
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for cap in re.captures_iter(content) {
            let full_match = cap.get(0).unwrap();
            let sql_fragment = cap.get(1).map(|m| m.as_str()).unwrap_or("");

            if is_parameterized(sql_fragment) {
                continue;
            }

            let line = content[..full_match.start()]
                .chars()
                .filter(|&c| c == '\n')
                .count()
                + 1;
            let snippet_len = sql_fragment.len().min(60);
            results.push(SqlInjectionRisk {
                file: file_path.to_string(),
                line,
                language: lang.to_string(),
                snippet: sql_fragment[..snippet_len].to_string(),
                interpolation_type: interpolation_type.to_string(),
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Input validation gaps
// ---------------------------------------------------------------------------

pub fn scan_validation_gaps(content: &str, file_path: &str, pagerank: f64) -> Vec<ValidationGap> {
    if pagerank < 0.5 {
        return vec![];
    }

    let validation_keywords = [
        "validate", "sanitize", "check", "parse", "regex", "is_valid", "assert", "guard", "ensure",
        "verify", "clean",
    ];

    let mut gaps = Vec::new();

    let re_rust_fn =
        match Regex::new(r"pub\s+(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(([^)]*)\)") {
            Ok(r) => r,
            Err(_) => return vec![],
        };

    let re_string_param = match Regex::new(r"(\w+)\s*:\s*(?:String|&str|&String)") {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    for fn_cap in re_rust_fn.captures_iter(content) {
        let fn_name = fn_cap[1].to_string();
        let params = &fn_cap[2];
        let fn_start = fn_cap.get(0).unwrap().start();
        let line = content[..fn_start].chars().filter(|&c| c == '\n').count() + 1;

        let after_sig = &content[fn_start..];
        let body_start = after_sig.find('{').unwrap_or(0);
        let body_end = find_matching_brace(after_sig, body_start).unwrap_or(body_start + 1);
        let body = &after_sig[body_start..body_end];

        let has_validation = validation_keywords.iter().any(|kw| body.contains(kw));
        if has_validation {
            continue;
        }

        for param_cap in re_string_param.captures_iter(params) {
            gaps.push(ValidationGap {
                file: file_path.to_string(),
                function_name: fn_name.clone(),
                parameter: param_cap[1].to_string(),
                line,
            });
        }
    }

    gaps
}

fn find_matching_brace(s: &str, start_pos: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if start_pos >= bytes.len() || bytes[start_pos] != b'{' {
        return None;
    }
    let mut depth = 0usize;
    for (i, &b) in bytes[start_pos..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start_pos + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Exposure score
// ---------------------------------------------------------------------------

pub fn compute_exposure_entry(
    path: &str,
    pub_symbol_count: usize,
    inbound_edges: usize,
    test_coverage: f64,
    max_possible: usize,
) -> ExposureEntry {
    let raw = pub_symbol_count as f64 * inbound_edges as f64 * (1.0 - test_coverage);
    let score = if max_possible == 0 || raw == 0.0 {
        0.0
    } else {
        (raw / max_possible as f64).clamp(0.0, 1.0)
    };
    ExposureEntry {
        path: path.to_string(),
        pub_symbol_count,
        inbound_edges,
        test_coverage,
        exposure_score: score,
    }
}

// ---------------------------------------------------------------------------
// Unprotected endpoint detection
// ---------------------------------------------------------------------------

pub const DEFAULT_AUTH_PATTERNS: &[&str] = &[
    "auth",
    "authenticate",
    "authorize",
    "require_auth",
    "login_required",
    "authenticated",
    "guard",
    "middleware",
    "jwt",
    "bearer",
    "token_required",
    "permission_required",
];

pub fn endpoint_is_protected(content: &str, handler: &str, auth_patterns: &[&str]) -> bool {
    if handler == "handler" || handler == "<anonymous>" {
        let lower = content.to_lowercase();
        return auth_patterns.iter().any(|p| lower.contains(p));
    }

    let handler_pos = content.find(handler).unwrap_or(0);
    let start = handler_pos.saturating_sub(200);
    let end = (handler_pos + 2000).min(content.len());
    let window = &content[start..end];
    let lower = window.to_lowercase();
    auth_patterns.iter().any(|p| lower.contains(p))
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

pub fn build_security_surface(
    index: &crate::index::CodebaseIndex,
    auth_patterns: &[&str],
    focus: Option<&str>,
) -> SecuritySurface {
    use crate::intelligence::api_surface::detect_routes;

    let mut secret_patterns = Vec::new();
    let mut sql_injection_surface = Vec::new();
    let mut input_validation_gaps = Vec::new();
    let mut unprotected_endpoints = Vec::new();

    let max_pub_symbols = index
        .files
        .iter()
        .map(|f| {
            f.parse_result
                .as_ref()
                .map(|pr| {
                    pr.symbols
                        .iter()
                        .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
                        .count()
                })
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(1);
    let max_inbound = index
        .files
        .iter()
        .map(|f| index.graph.dependents(&f.relative_path).len())
        .max()
        .unwrap_or(1);
    let max_possible = max_pub_symbols * max_inbound;

    let mut exposure_scores = Vec::new();

    for file in &index.files {
        if let Some(focus_prefix) = focus {
            if !file.relative_path.starts_with(focus_prefix) {
                continue;
            }
        }

        let path = &file.relative_path;
        let content = &file.content;
        let pagerank = index.pagerank.get(path).copied().unwrap_or(0.0);

        secret_patterns.extend(scan_secret_patterns(content, path));
        sql_injection_surface.extend(scan_sql_injection(content, path));
        input_validation_gaps.extend(scan_validation_gaps(content, path, pagerank));

        let routes = detect_routes(content, path);
        for route in routes {
            if !endpoint_is_protected(content, &route.handler, auth_patterns) {
                unprotected_endpoints.push(UnprotectedEndpoint {
                    file: path.clone(),
                    method: route.method,
                    path: route.path,
                    handler: route.handler,
                    line: route.line,
                });
            }
        }

        let pub_count = file
            .parse_result
            .as_ref()
            .map(|pr| {
                pr.symbols
                    .iter()
                    .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
                    .count()
            })
            .unwrap_or(0);
        let inbound = index.graph.dependents(path).len();
        let has_tests = index.test_map.contains_key(path);
        let test_cov = if has_tests { 1.0 } else { 0.0 };

        let entry = compute_exposure_entry(path, pub_count, inbound, test_cov, max_possible);
        if entry.exposure_score > 0.0 {
            exposure_scores.push(entry);
        }
    }

    exposure_scores.sort_by(|a, b| {
        b.exposure_score
            .partial_cmp(&a.exposure_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    SecuritySurface {
        unprotected_endpoints,
        input_validation_gaps,
        secret_patterns,
        sql_injection_surface,
        exposure_scores,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_aws_key() {
        let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
        let matches = scan_secret_patterns(content, "src/config.rs");
        assert!(
            matches.iter().any(|m| m.pattern_name == "aws_access_key"),
            "AWS key must be detected: {:?}",
            matches
        );
    }

    #[test]
    fn test_secret_github_pat() {
        let content = "token = \"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij\";";
        let matches = scan_secret_patterns(content, "src/github.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "github_pat"));
    }

    #[test]
    fn test_secret_password_assignment() {
        let content = "password = \"supersecretpassword123\"";
        let matches = scan_secret_patterns(content, "src/auth.rs");
        assert!(matches
            .iter()
            .any(|m| m.pattern_name == "password_assignment"));
    }

    #[test]
    fn test_secret_connection_string() {
        let content = "url = \"postgres://admin:password123@localhost/mydb\"";
        let matches = scan_secret_patterns(content, "src/db.rs");
        assert!(matches
            .iter()
            .any(|m| m.pattern_name == "connection_string"));
    }

    #[test]
    fn test_secret_slack_token() {
        let content = "SLACK_TOKEN=xoxb-1234567890-abcdefghij";
        let matches = scan_secret_patterns(content, "src/notify.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "slack_token"));
    }

    #[test]
    fn test_secret_excluded_test_file() {
        let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
        let matches = scan_secret_patterns(content, "tests/test_config.rs");
        assert!(
            matches.is_empty(),
            "test files must be excluded from secret scanning"
        );
    }

    #[test]
    fn test_secret_excluded_lock_file() {
        let content = "password = \"supersecret\"";
        for lock_file in &[
            "Cargo.lock",
            "package-lock.json",
            "yarn.lock",
            "Gemfile.lock",
            "poetry.lock",
        ] {
            let matches = scan_secret_patterns(content, lock_file);
            assert!(matches.is_empty(), "{lock_file} must be excluded");
        }
    }

    #[test]
    fn test_secret_excluded_env_example() {
        let content = "API_KEY=your_api_key_here";
        let matches = scan_secret_patterns(content, ".env.example");
        assert!(matches.is_empty(), ".env.example must be excluded");
    }

    #[test]
    fn test_secret_short_password_ignored() {
        let content = "password = \"short\"";
        let matches = scan_secret_patterns(content, "src/config.rs");
        assert!(
            !matches
                .iter()
                .any(|m| m.pattern_name == "password_assignment"),
            "short password must not match"
        );
    }

    #[test]
    fn test_secret_aws_key_must_be_20_chars() {
        let short = "AKIA123";
        let matches = scan_secret_patterns(short, "src/config.rs");
        assert!(
            !matches.iter().any(|m| m.pattern_name == "aws_access_key"),
            "short AKIA prefix must not match"
        );
    }

    #[test]
    fn test_secret_snippet_redaction() {
        let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
        let matches = scan_secret_patterns(content, "src/config.rs");
        let secret = matches
            .iter()
            .find(|m| m.pattern_name == "aws_access_key")
            .unwrap();
        assert!(secret.snippet.ends_with("..."), "snippet must be redacted");
        assert!(
            secret.snippet.len() < 20,
            "snippet must not expose full secret"
        );
    }

    #[test]
    fn test_sql_injection_python_fstring() {
        let content = r#"query = f"SELECT * FROM users WHERE id = {user_id}""#;
        let risks = scan_sql_injection(content, "src/repo.py");
        assert!(
            !risks.is_empty(),
            "f-string SQL interpolation must be detected"
        );
        assert_eq!(risks[0].language, "python");
    }

    #[test]
    fn test_sql_injection_js_template_literal() {
        let content = "const q = `SELECT * FROM orders WHERE id = ${orderId}`;";
        let risks = scan_sql_injection(content, "src/db.js");
        assert!(
            !risks.is_empty(),
            "JS template literal SQL must be detected"
        );
        assert_eq!(risks[0].language, "javascript");
    }

    #[test]
    fn test_sql_injection_rust_format() {
        let content = r#"let q = format!("SELECT * FROM products WHERE name = '{}'", name);"#;
        let risks = scan_sql_injection(content, "src/repo.rs");
        assert!(!risks.is_empty(), "Rust format! SQL must be detected");
        assert_eq!(risks[0].language, "rust");
    }

    #[test]
    fn test_sql_injection_java_concatenation() {
        let content = r#"String q = "SELECT * FROM accounts WHERE id = " + accountId;"#;
        let risks = scan_sql_injection(content, "src/AccountRepo.java");
        assert!(
            !risks.is_empty(),
            "Java string concatenation SQL must be detected"
        );
        assert_eq!(risks[0].language, "java");
    }

    #[test]
    fn test_sql_injection_parameterized_safe() {
        let content = r#"db.query("SELECT * FROM users WHERE id = $1", [userId])"#;
        let risks = scan_sql_injection(content, "src/repo.js");
        assert!(
            risks.is_empty(),
            "parameterized query must not be flagged as injection risk"
        );
    }

    #[test]
    fn test_sql_injection_parameterized_question_mark_safe() {
        let content = r#"db.prepare("SELECT * FROM users WHERE id = ?").bind(id)"#;
        let risks = scan_sql_injection(content, "src/repo.js");
        assert!(
            risks.is_empty(),
            "? parameterized query must not be flagged"
        );
    }

    #[test]
    fn test_sql_injection_no_sql_keywords_not_flagged() {
        let content = r#"const msg = `Hello ${name}`;"#;
        let risks = scan_sql_injection(content, "src/greet.js");
        assert!(
            risks.is_empty(),
            "template literal without SQL keywords must not be flagged"
        );
    }

    #[test]
    fn test_sql_injection_rust_no_format_macro_not_flagged() {
        let content = r#"let msg = format!("Hello {}", name);"#;
        let risks = scan_sql_injection(content, "src/greet.rs");
        assert!(
            risks.is_empty(),
            "format! without SQL keywords must not be flagged"
        );
    }

    #[test]
    fn test_exposure_score_range() {
        let entry = compute_exposure_entry("src/api.rs", 10, 5, 0.0, 100);
        assert!(entry.exposure_score >= 0.0);
        assert!(entry.exposure_score <= 1.0);
    }

    #[test]
    fn test_exposure_score_fully_tested_is_lower() {
        let untested = compute_exposure_entry("src/a.rs", 10, 5, 0.0, 100);
        let tested = compute_exposure_entry("src/b.rs", 10, 5, 1.0, 100);
        assert!(
            untested.exposure_score > tested.exposure_score,
            "untested file must have higher exposure"
        );
    }

    #[test]
    fn test_exposure_score_zero_symbols_is_zero() {
        let entry = compute_exposure_entry("src/empty.rs", 0, 0, 0.0, 100);
        assert_eq!(entry.exposure_score, 0.0);
    }

    #[test]
    fn test_exposure_max_possible_zero_returns_zero() {
        let entry = compute_exposure_entry("src/x.rs", 5, 3, 0.0, 0);
        assert_eq!(
            entry.exposure_score, 0.0,
            "max_possible=0 must produce score 0"
        );
    }

    #[test]
    fn test_exposure_score_clamped_to_one() {
        let entry = compute_exposure_entry("src/x.rs", 100, 100, 0.0, 1);
        assert!(entry.exposure_score <= 1.0);
    }

    #[test]
    fn test_validation_gap_public_string_param_no_validation() {
        let content = r#"
pub fn create_user(name: String) {
    db.insert(name);
}
"#;
        let gaps = scan_validation_gaps(content, "src/user.rs", 0.8);
        assert!(
            !gaps.is_empty(),
            "unvalidated String param must be detected"
        );
    }

    #[test]
    fn test_validation_gap_with_validate_call_not_flagged() {
        let content = r#"
pub fn create_user(name: String) {
    validate(&name);
    db.insert(name);
}
"#;
        let gaps = scan_validation_gaps(content, "src/user.rs", 0.8);
        assert!(
            gaps.is_empty(),
            "function with validate() call must not be flagged"
        );
    }

    #[test]
    fn test_validation_gap_low_pagerank_skipped() {
        let content = r#"
pub fn do_thing(input: String) {
    process(input);
}
"#;
        let gaps = scan_validation_gaps(content, "src/util.rs", 0.1);
        assert!(
            gaps.is_empty(),
            "low-pagerank file must not be scanned for validation gaps"
        );
    }

    #[test]
    fn test_validation_gap_sanitize_keyword_not_flagged() {
        let content = r#"
pub fn process_input(data: String) {
    let clean = sanitize(&data);
    store(clean);
}
"#;
        let gaps = scan_validation_gaps(content, "src/proc.rs", 0.9);
        assert!(
            gaps.is_empty(),
            "function with sanitize() call must not be flagged"
        );
    }

    #[test]
    fn test_endpoint_protected_by_file_level_auth_keyword() {
        let content = "app.use(authenticate); app.get('/admin', adminHandler);";
        assert!(
            endpoint_is_protected(content, "adminHandler", DEFAULT_AUTH_PATTERNS),
            "file containing authenticate keyword must be considered protected"
        );
    }

    #[test]
    fn test_endpoint_unprotected_no_auth_keywords() {
        let content = "app.get('/public', publicHandler);";
        assert!(
            !endpoint_is_protected(content, "publicHandler", DEFAULT_AUTH_PATTERNS),
            "file with no auth keywords must be unprotected"
        );
    }
}
