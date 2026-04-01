// Query expansion with hierarchical synonym maps

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Core synonym map
// ---------------------------------------------------------------------------

static CORE_SYNONYMS: LazyLock<HashMap<&'static str, &'static [&'static str]>> =
    LazyLock::new(|| {
        let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
        m.insert(
            "auth",
            &[
                "authentication",
                "authorize",
                "login",
                "session",
                "jwt",
                "token",
                "credential",
                "oauth",
                "password",
            ],
        );
        m.insert(
            "db",
            &[
                "database",
                "query",
                "sql",
                "migration",
                "schema",
                "table",
                "model",
                "orm",
                "repository",
            ],
        );
        m.insert(
            "api",
            &[
                "endpoint",
                "route",
                "handler",
                "controller",
                "request",
                "response",
                "middleware",
                "rest",
                "graphql",
            ],
        );
        m.insert(
            "config",
            &[
                "configuration",
                "settings",
                "env",
                "environment",
                "options",
                "preferences",
            ],
        );
        m.insert(
            "test",
            &[
                "testing", "spec", "assert", "mock", "fixture", "expect", "describe",
            ],
        );
        m.insert(
            "error",
            &[
                "exception",
                "panic",
                "fault",
                "failure",
                "catch",
                "throw",
                "rescue",
                "recover",
            ],
        );
        m.insert(
            "log",
            &[
                "logging",
                "logger",
                "trace",
                "debug",
                "warn",
                "info",
                "audit",
                "telemetry",
            ],
        );
        m.insert(
            "cache",
            &[
                "caching",
                "memoize",
                "invalidate",
                "ttl",
                "redis",
                "memcached",
            ],
        );
        m.insert(
            "async",
            &[
                "concurrent",
                "parallel",
                "future",
                "promise",
                "await",
                "spawn",
                "thread",
                "task",
            ],
        );
        m.insert(
            "parse",
            &[
                "parser",
                "parsing",
                "tokenize",
                "lexer",
                "deserialize",
                "decode",
                "unmarshal",
            ],
        );
        m.insert(
            "serial",
            &[
                "serialize",
                "encode",
                "marshal",
                "format",
                "json",
                "xml",
                "protobuf",
            ],
        );
        m.insert(
            "http",
            &[
                "request",
                "response",
                "client",
                "server",
                "fetch",
                "curl",
                "websocket",
            ],
        );
        m.insert(
            "file",
            &[
                "filesystem",
                "path",
                "directory",
                "read",
                "write",
                "stream",
                "buffer",
            ],
        );
        m.insert(
            "user",
            &[
                "account",
                "profile",
                "identity",
                "principal",
                "member",
                "role",
            ],
        );
        m.insert(
            "pay",
            &[
                "payment",
                "billing",
                "charge",
                "invoice",
                "subscription",
                "stripe",
            ],
        );
        m.insert(
            "msg",
            &[
                "message",
                "event",
                "notification",
                "queue",
                "publish",
                "subscribe",
                "broker",
            ],
        );
        m.insert(
            "deploy",
            &[
                "deployment",
                "release",
                "rollout",
                "canary",
                "staging",
                "production",
            ],
        );
        m.insert(
            "build",
            &[
                "compile", "bundle", "package", "artifact", "target", "output",
            ],
        );
        m.insert(
            "lint",
            &[
                "linter",
                "format",
                "formatter",
                "style",
                "check",
                "clippy",
                "eslint",
            ],
        );
        m.insert(
            "type",
            &[
                "typing",
                "typecheck",
                "generic",
                "interface",
                "schema",
                "validate",
            ],
        );
        m.insert(
            "render",
            &["display", "paint", "draw", "template", "view", "component"],
        );
        m.insert(
            "route",
            &["routing", "path", "url", "navigate", "redirect", "dispatch"],
        );
        m.insert(
            "store",
            &[
                "storage",
                "persist",
                "save",
                "load",
                "repository",
                "warehouse",
            ],
        );
        m.insert(
            "crypt",
            &[
                "encrypt", "decrypt", "hash", "sign", "verify", "cipher", "tls", "ssl",
            ],
        );
        m.insert(
            "metric",
            &[
                "metrics",
                "monitor",
                "measure",
                "counter",
                "gauge",
                "histogram",
                "prometheus",
            ],
        );
        m.insert(
            "migrate",
            &[
                "migration",
                "upgrade",
                "downgrade",
                "alter",
                "evolve",
                "transform",
            ],
        );
        m.insert(
            "valid",
            &[
                "validate",
                "validation",
                "check",
                "verify",
                "sanitize",
                "constrain",
            ],
        );
        m.insert(
            "perm",
            &[
                "permission",
                "authorize",
                "acl",
                "rbac",
                "policy",
                "grant",
                "deny",
            ],
        );
        m.insert(
            "schedule",
            &[
                "scheduler",
                "cron",
                "job",
                "worker",
                "background",
                "timer",
                "interval",
            ],
        );
        m.insert(
            "retry",
            &[
                "backoff",
                "resilience",
                "circuit",
                "breaker",
                "timeout",
                "fallback",
            ],
        );
        m
    });

// ---------------------------------------------------------------------------
// Domain enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    Web,
    Database,
    Auth,
    Infra,
    Testing,
    Api,
    Mobile,
    ML,
}

// ---------------------------------------------------------------------------
// Domain synonym maps
// ---------------------------------------------------------------------------

static DOMAIN_SYNONYMS: LazyLock<HashMap<Domain, HashMap<&'static str, &'static [&'static str]>>> =
    LazyLock::new(|| {
        let mut outer: HashMap<Domain, HashMap<&'static str, &'static [&'static str]>> =
            HashMap::new();

        // Web
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "component",
                &[
                    "widget", "element", "view", "template", "layout", "partial", "fragment",
                ],
            );
            m.insert(
                "style",
                &[
                    "css",
                    "scss",
                    "sass",
                    "stylesheet",
                    "theme",
                    "design",
                    "responsive",
                ],
            );
            m.insert(
                "state",
                &[
                    "store", "reducer", "context", "provider", "hook", "signal", "reactive",
                ],
            );
            m.insert(
                "dom",
                &[
                    "document", "node", "element", "selector", "query", "event", "listener",
                ],
            );
            outer.insert(Domain::Web, m);
        }

        // Database
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "index",
                &[
                    "btree",
                    "hash",
                    "unique",
                    "composite",
                    "covering",
                    "partial",
                ],
            );
            m.insert(
                "join",
                &["inner", "outer", "left", "cross", "subquery", "lateral"],
            );
            m.insert(
                "txn",
                &[
                    "transaction",
                    "commit",
                    "rollback",
                    "isolation",
                    "lock",
                    "deadlock",
                ],
            );
            m.insert(
                "constraint",
                &[
                    "foreign", "primary", "unique", "check", "default", "nullable",
                ],
            );
            outer.insert(Domain::Database, m);
        }

        // Auth
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert("sso", &["saml", "oidc", "ldap", "federation", "identity"]);
            m.insert("mfa", &["totp", "u2f", "webauthn", "otp", "factor"]);
            m.insert(
                "scope",
                &["claim", "role", "permission", "grant", "audience"],
            );
            outer.insert(Domain::Auth, m);
        }

        // Infra
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "pod",
                &["container", "deployment", "service", "ingress", "daemonset"],
            );
            m.insert(
                "vpc",
                &["subnet", "cidr", "gateway", "route", "peering", "firewall"],
            );
            m.insert("iam", &["role", "policy", "principal", "assume", "trust"]);
            outer.insert(Domain::Infra, m);
        }

        // Testing
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert("stub", &["fake", "spy", "double", "dummy", "mock"]);
            m.insert(
                "coverage",
                &["branch", "line", "statement", "mutation", "threshold"],
            );
            m.insert(
                "e2e",
                &[
                    "integration",
                    "acceptance",
                    "smoke",
                    "regression",
                    "contract",
                ],
            );
            outer.insert(Domain::Testing, m);
        }

        // API
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "pagination",
                &["cursor", "offset", "limit", "page", "next", "previous"],
            );
            m.insert("version", &["v1", "v2", "breaking", "deprecate", "sunset"]);
            m.insert(
                "rate",
                &["throttle", "limit", "quota", "backpressure", "burst"],
            );
            outer.insert(Domain::Api, m);
        }

        // Mobile
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "screen",
                &["activity", "viewcontroller", "widget", "page", "scene"],
            );
            m.insert(
                "nav",
                &["navigation", "router", "stack", "tab", "drawer", "deeplink"],
            );
            m.insert("gesture", &["tap", "swipe", "drag", "pinch", "longpress"]);
            outer.insert(Domain::Mobile, m);
        }

        // ML
        {
            let mut m: HashMap<&'static str, &'static [&'static str]> = HashMap::new();
            m.insert(
                "epoch",
                &["iteration", "batch", "step", "gradient", "backprop"],
            );
            m.insert(
                "feature",
                &[
                    "embedding",
                    "vector",
                    "dimension",
                    "latent",
                    "representation",
                ],
            );
            m.insert(
                "infer",
                &["predict", "forward", "score", "classify", "detect"],
            );
            outer.insert(Domain::ML, m);
        }

        outer
    });

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Expand a query string into a set of terms including synonyms drawn from the
/// core map and any applicable domain-specific maps.
pub fn expand_query(query: &str, domains: &HashSet<Domain>) -> HashSet<String> {
    let tokens = crate::relevance::signals::tokenize(query);
    let mut expanded = tokens.clone();
    for token in &tokens {
        if let Some(synonyms) = CORE_SYNONYMS.get(token.as_str()) {
            expanded.extend(synonyms.iter().map(|s| s.to_string()));
        }
        for domain in domains {
            if let Some(domain_map) = DOMAIN_SYNONYMS.get(domain) {
                if let Some(synonyms) = domain_map.get(token.as_str()) {
                    expanded.extend(synonyms.iter().map(|s| s.to_string()));
                }
            }
        }
    }
    expanded
}

/// Detect which domains are present in the given set of indexed files based on
/// file extensions and path segments.
pub fn detect_domains(files: &[crate::index::IndexedFile]) -> HashSet<Domain> {
    let mut domains = HashSet::new();

    for file in files {
        let path = &file.relative_path;
        let path_lower = path.to_lowercase();

        // Derive the filename (last segment after '/').
        let filename = path_lower.rsplit('/').next().unwrap_or(&path_lower);

        // Extension (everything after the last '.', including the dot for starts_with checks).
        let ext = filename
            .rfind('.')
            .map(|i| &filename[i..])
            .unwrap_or_default();

        // Web: .html, .css, .scss, .svelte, .jsx, .tsx
        if matches!(
            ext,
            ".html" | ".css" | ".scss" | ".svelte" | ".jsx" | ".tsx"
        ) {
            domains.insert(Domain::Web);
        }

        // Database: .sql, .prisma, or "migration" anywhere in path
        if matches!(ext, ".sql" | ".prisma") || path_lower.contains("migration") {
            domains.insert(Domain::Database);
        }

        // Auth: "auth", "login", or "session" anywhere in path (case-insensitive)
        if path_lower.contains("auth")
            || path_lower.contains("login")
            || path_lower.contains("session")
        {
            domains.insert(Domain::Auth);
        }

        // Infra: .tf, .hcl, .tfvars, or filename starts with "Dockerfile"
        if matches!(ext, ".tf" | ".hcl" | ".tfvars") || filename.starts_with("dockerfile") {
            domains.insert(Domain::Infra);
        }

        // Testing: "test" or "spec" anywhere in path (case-insensitive)
        if path_lower.contains("test") || path_lower.contains("spec") {
            domains.insert(Domain::Testing);
        }

        // API: "route", "endpoint", "handler", or "openapi" anywhere in path
        if path_lower.contains("route")
            || path_lower.contains("endpoint")
            || path_lower.contains("handler")
            || path_lower.contains("openapi")
        {
            domains.insert(Domain::Api);
        }

        // Mobile: .swift, .kt, .dart, .xcodeproj, or "android/" in path
        if matches!(ext, ".swift" | ".kt" | ".dart" | ".xcodeproj")
            || path_lower.contains("android/")
        {
            domains.insert(Domain::Mobile);
        }

        // ML: .ipynb
        if ext == ".ipynb" {
            domains.insert(Domain::ML);
        }
    }

    domains
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a minimal IndexedFile with only a relative_path.
    fn make_file(path: &str) -> crate::index::IndexedFile {
        crate::index::IndexedFile {
            relative_path: path.to_string(),
            language: None,
            size_bytes: 0,
            token_count: 0,
            parse_result: None,
            content: String::new(),
            mtime_secs: None,
        }
    }

    // -----------------------------------------------------------------------
    // Core synonym expansion tests (sampling ~10 of the 30 keys)
    // -----------------------------------------------------------------------

    #[test]
    fn test_expand_auth_synonym() {
        let domains = HashSet::new();
        let result = expand_query("auth", &domains);
        assert!(
            result.contains("authentication"),
            "auth should expand to authentication"
        );
        assert!(result.contains("login"), "auth should expand to login");
        assert!(result.contains("jwt"), "auth should expand to jwt");
    }

    #[test]
    fn test_expand_db_synonym() {
        let domains = HashSet::new();
        let result = expand_query("db", &domains);
        assert!(result.contains("database"), "db should expand to database");
        assert!(result.contains("sql"), "db should expand to sql");
        assert!(result.contains("orm"), "db should expand to orm");
    }

    #[test]
    fn test_expand_api_synonym() {
        let domains = HashSet::new();
        let result = expand_query("api", &domains);
        assert!(result.contains("endpoint"), "api should expand to endpoint");
        assert!(result.contains("handler"), "api should expand to handler");
        assert!(
            result.contains("middleware"),
            "api should expand to middleware"
        );
    }

    #[test]
    fn test_expand_cache_synonym() {
        let domains = HashSet::new();
        let result = expand_query("cache", &domains);
        assert!(result.contains("caching"), "cache should expand to caching");
        assert!(result.contains("redis"), "cache should expand to redis");
        assert!(result.contains("ttl"), "cache should expand to ttl");
    }

    #[test]
    fn test_expand_log_synonym() {
        let domains = HashSet::new();
        let result = expand_query("log", &domains);
        assert!(result.contains("logging"), "log should expand to logging");
        assert!(
            result.contains("telemetry"),
            "log should expand to telemetry"
        );
        assert!(result.contains("trace"), "log should expand to trace");
    }

    #[test]
    fn test_expand_error_synonym() {
        let domains = HashSet::new();
        let result = expand_query("error", &domains);
        assert!(
            result.contains("exception"),
            "error should expand to exception"
        );
        assert!(result.contains("panic"), "error should expand to panic");
        assert!(result.contains("recover"), "error should expand to recover");
    }

    #[test]
    fn test_expand_crypt_synonym() {
        let domains = HashSet::new();
        let result = expand_query("crypt", &domains);
        assert!(result.contains("encrypt"), "crypt should expand to encrypt");
        assert!(result.contains("tls"), "crypt should expand to tls");
        assert!(result.contains("hash"), "crypt should expand to hash");
    }

    #[test]
    fn test_expand_parse_synonym() {
        let domains = HashSet::new();
        let result = expand_query("parse", &domains);
        assert!(result.contains("parser"), "parse should expand to parser");
        assert!(
            result.contains("deserialize"),
            "parse should expand to deserialize"
        );
        assert!(result.contains("lexer"), "parse should expand to lexer");
    }

    #[test]
    fn test_expand_metric_synonym() {
        let domains = HashSet::new();
        let result = expand_query("metric", &domains);
        assert!(
            result.contains("metrics"),
            "metric should expand to metrics"
        );
        assert!(
            result.contains("prometheus"),
            "metric should expand to prometheus"
        );
        assert!(
            result.contains("histogram"),
            "metric should expand to histogram"
        );
    }

    #[test]
    fn test_expand_retry_synonym() {
        let domains = HashSet::new();
        let result = expand_query("retry", &domains);
        assert!(result.contains("backoff"), "retry should expand to backoff");
        assert!(result.contains("timeout"), "retry should expand to timeout");
        assert!(
            result.contains("fallback"),
            "retry should expand to fallback"
        );
    }

    // -----------------------------------------------------------------------
    // Domain detection tests — one per domain
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_web_by_extension() {
        let files = vec![make_file("src/App.tsx"), make_file("src/styles/main.scss")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Web),
            "tsx/scss should detect Web domain"
        );
    }

    #[test]
    fn test_detect_database_by_extension() {
        let files = vec![make_file("db/schema.prisma")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Database),
            ".prisma should detect Database domain"
        );
    }

    #[test]
    fn test_detect_database_by_migration_path() {
        let files = vec![make_file("db/migrations/0001_initial.sql")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Database),
            "'migration' in path should detect Database domain"
        );
    }

    #[test]
    fn test_detect_auth_by_path_segment() {
        let files = vec![make_file("src/auth/middleware.rs")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Auth),
            "'auth' in path should detect Auth domain"
        );
    }

    #[test]
    fn test_detect_auth_by_login_segment() {
        let files = vec![make_file("src/handlers/login.py")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Auth),
            "'login' in path should detect Auth domain"
        );
    }

    #[test]
    fn test_detect_infra_by_terraform_extension() {
        let files = vec![make_file("infra/main.tf")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Infra),
            ".tf should detect Infra domain"
        );
    }

    #[test]
    fn test_detect_infra_by_dockerfile() {
        let files = vec![make_file("Dockerfile")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Infra),
            "Dockerfile should detect Infra domain"
        );
    }

    #[test]
    fn test_detect_testing_by_path_segment() {
        let files = vec![make_file("tests/unit/auth_test.go")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Testing),
            "'test' in path should detect Testing domain"
        );
    }

    #[test]
    fn test_detect_testing_by_spec_segment() {
        let files = vec![make_file("spec/models/user_spec.rb")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Testing),
            "'spec' in path should detect Testing domain"
        );
    }

    #[test]
    fn test_detect_api_by_route_segment() {
        let files = vec![make_file("src/routes/users.ts")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Api),
            "'routes' in path should detect API domain"
        );
    }

    #[test]
    fn test_detect_api_by_handler_segment() {
        let files = vec![make_file("src/handlers/payment.rs")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Api),
            "'handler' in path should detect API domain"
        );
    }

    #[test]
    fn test_detect_mobile_by_swift_extension() {
        let files = vec![make_file("ios/App/ContentView.swift")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Mobile),
            ".swift should detect Mobile domain"
        );
    }

    #[test]
    fn test_detect_mobile_by_android_path() {
        let files = vec![make_file("android/app/src/MainActivity.kt")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::Mobile),
            "'android/' in path should detect Mobile domain"
        );
    }

    #[test]
    fn test_detect_ml_by_notebook_extension() {
        let files = vec![make_file("notebooks/training.ipynb")];
        let domains = detect_domains(&files);
        assert!(
            domains.contains(&Domain::ML),
            ".ipynb should detect ML domain"
        );
    }

    // -----------------------------------------------------------------------
    // No domains and multiple domains
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_no_domains_for_plain_rust() {
        let files = vec![
            make_file("src/main.rs"),
            make_file("src/lib.rs"),
            make_file("src/utils/math.rs"),
        ];
        let domains = detect_domains(&files);
        // None of the domain-triggering patterns should be present.
        assert!(
            !domains.contains(&Domain::Web),
            "plain .rs files should not detect Web"
        );
        assert!(
            !domains.contains(&Domain::Database),
            "plain .rs files should not detect Database"
        );
        assert!(
            !domains.contains(&Domain::ML),
            "plain .rs files should not detect ML"
        );
        assert!(
            !domains.contains(&Domain::Mobile),
            "plain .rs files should not detect Mobile"
        );
    }

    #[test]
    fn test_detect_multiple_domains() {
        let files = vec![
            make_file("src/App.tsx"),           // Web
            make_file("db/migrations/001.sql"), // Database (migration path + sql)
            make_file("infra/cluster.tf"),      // Infra
            make_file("notebooks/model.ipynb"), // ML
        ];
        let domains = detect_domains(&files);
        assert!(domains.contains(&Domain::Web));
        assert!(domains.contains(&Domain::Database));
        assert!(domains.contains(&Domain::Infra));
        assert!(domains.contains(&Domain::ML));
    }

    // -----------------------------------------------------------------------
    // Integration tests: expand_query with active domains
    // -----------------------------------------------------------------------

    #[test]
    fn test_expand_with_web_domain_component() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Web);
        let result = expand_query("component", &domains);
        // "component" is not a core synonym key, so only the domain map applies.
        assert!(
            result.contains("component"),
            "original token should be in result"
        );
        assert!(
            result.contains("widget"),
            "Web domain should expand 'component' to 'widget'"
        );
        assert!(
            result.contains("template"),
            "Web domain should expand 'component' to 'template'"
        );
    }

    #[test]
    fn test_expand_with_database_domain_txn() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Database);
        let result = expand_query("txn", &domains);
        assert!(
            result.contains("transaction"),
            "Database domain should expand 'txn' to 'transaction'"
        );
        assert!(
            result.contains("rollback"),
            "Database domain should expand 'txn' to 'rollback'"
        );
    }

    #[test]
    fn test_expand_with_auth_domain_sso() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Auth);
        let result = expand_query("sso", &domains);
        assert!(
            result.contains("saml"),
            "Auth domain should expand 'sso' to 'saml'"
        );
        assert!(
            result.contains("oidc"),
            "Auth domain should expand 'sso' to 'oidc'"
        );
    }

    #[test]
    fn test_expand_with_infra_domain_pod() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Infra);
        let result = expand_query("pod", &domains);
        assert!(
            result.contains("container"),
            "Infra domain should expand 'pod' to 'container'"
        );
        assert!(
            result.contains("ingress"),
            "Infra domain should expand 'pod' to 'ingress'"
        );
    }

    #[test]
    fn test_expand_with_testing_domain_stub() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Testing);
        let result = expand_query("stub", &domains);
        assert!(
            result.contains("fake"),
            "Testing domain should expand 'stub' to 'fake'"
        );
        assert!(
            result.contains("spy"),
            "Testing domain should expand 'stub' to 'spy'"
        );
    }

    #[test]
    fn test_expand_with_api_domain_pagination() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Api);
        let result = expand_query("pagination", &domains);
        assert!(
            result.contains("cursor"),
            "API domain should expand 'pagination' to 'cursor'"
        );
        assert!(
            result.contains("offset"),
            "API domain should expand 'pagination' to 'offset'"
        );
    }

    #[test]
    fn test_expand_with_mobile_domain_screen() {
        let mut domains = HashSet::new();
        domains.insert(Domain::Mobile);
        let result = expand_query("screen", &domains);
        assert!(
            result.contains("activity"),
            "Mobile domain should expand 'screen' to 'activity'"
        );
        assert!(
            result.contains("scene"),
            "Mobile domain should expand 'screen' to 'scene'"
        );
    }

    #[test]
    fn test_expand_with_ml_domain_epoch() {
        let mut domains = HashSet::new();
        domains.insert(Domain::ML);
        let result = expand_query("epoch", &domains);
        assert!(
            result.contains("iteration"),
            "ML domain should expand 'epoch' to 'iteration'"
        );
        assert!(
            result.contains("gradient"),
            "ML domain should expand 'epoch' to 'gradient'"
        );
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_expand_empty_query_returns_empty() {
        let domains = HashSet::new();
        let result = expand_query("", &domains);
        assert!(
            result.is_empty(),
            "empty query should produce empty expansion"
        );
    }

    #[test]
    fn test_expand_unknown_term_passes_through() {
        let domains = HashSet::new();
        let result = expand_query("xyzzy_unknown_term", &domains);
        // tokenize will split "xyzzy_unknown_term" into parts; each part should be
        // present but without any extra expansion since none are synonym keys.
        assert!(
            result.contains("xyzzy"),
            "'xyzzy' should pass through unchanged"
        );
        assert!(
            result.contains("unknown"),
            "'unknown' should pass through unchanged"
        );
        assert!(
            result.contains("term"),
            "'term' should pass through unchanged"
        );
        // Total size: just the tokenised parts, nothing extra.
        assert_eq!(
            result.len(),
            3,
            "no synonym expansion for unknown terms: {:?}",
            result
        );
    }

    #[test]
    fn test_expand_no_domains_gives_core_only() {
        let domains = HashSet::new();
        let result = expand_query("auth config", &domains);
        // Should get core synonyms for "auth" and "config" but no domain expansions.
        assert!(result.contains("authentication"));
        assert!(result.contains("configuration"));
        assert!(result.contains("settings"));
        // No domain-specific terms (e.g. "widget" from Web) should appear.
        assert!(
            !result.contains("widget"),
            "no Web domain synonyms without domain active"
        );
    }

    #[test]
    fn test_expand_original_tokens_always_included() {
        let domains = HashSet::new();
        let result = expand_query("database migration", &domains);
        // "database" and "migration" are not synonym keys, but they should still
        // appear as themselves in the expansion set.
        assert!(
            result.contains("database"),
            "original token 'database' should be preserved"
        );
        assert!(
            result.contains("migration"),
            "original token 'migration' should be preserved"
        );
    }

    #[test]
    fn test_expand_core_and_domain_combined() {
        // "auth" triggers core synonyms; "sso" triggers Auth domain synonyms.
        let mut domains = HashSet::new();
        domains.insert(Domain::Auth);
        let result = expand_query("auth sso", &domains);
        // Core expansion for "auth"
        assert!(result.contains("authentication"));
        assert!(result.contains("jwt"));
        // Domain expansion for "sso"
        assert!(result.contains("saml"));
        assert!(result.contains("oidc"));
    }

    #[test]
    fn test_detect_empty_files_list() {
        let domains = detect_domains(&[]);
        assert!(
            domains.is_empty(),
            "empty file list should produce no domains"
        );
    }
}
