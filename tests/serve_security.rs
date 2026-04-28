//! Security adversarial tests for `cxpak serve`.
//!
//! Locks two invariants identified by v2.1.0 final-validation review:
//! 1. Non-loopback bind without --token must be REFUSED at startup.
//! 2. Every response from `cxpak serve` carries a strict CSP and the
//!    standard defense-in-depth security header set.
#![cfg(feature = "daemon")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cxpak::budget::counter::TokenCounter;
use cxpak::commands::serve::{build_router_for_test, build_router_for_test_with_token};
use cxpak::index::CodebaseIndex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

fn make_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new())
}

fn shared() -> Arc<RwLock<Arc<CodebaseIndex>>> {
    Arc::new(RwLock::new(Arc::new(make_index())))
}

fn shared_path() -> Arc<PathBuf> {
    Arc::new(PathBuf::from("/tmp"))
}

// ── Defect #1: non-loopback bind without token ──────────────────────────────

use std::net::SocketAddr;

#[test]
fn validate_refuses_ipv4_non_loopback_without_token() {
    let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    let err = cxpak::commands::serve::validate_bind_security(&addr, None)
        .expect_err("0.0.0.0 + no token must Err");
    assert!(
        err.contains("--token") || err.contains("authenticated"),
        "error must explain the token requirement; got: {err}"
    );
    assert!(
        err.contains("loopback") || err.contains("0.0.0.0"),
        "error must name the non-loopback address; got: {err}"
    );
}

#[test]
fn validate_refuses_ipv6_unspecified_without_token() {
    let addr: SocketAddr = "[::]:8080".parse().unwrap();
    assert!(
        cxpak::commands::serve::validate_bind_security(&addr, None).is_err(),
        ":: (IPv6 unspecified) without token must be refused"
    );
}

#[test]
fn validate_refuses_arbitrary_lan_address_without_token() {
    let addr: SocketAddr = "192.168.1.42:8080".parse().unwrap();
    assert!(
        cxpak::commands::serve::validate_bind_security(&addr, None).is_err(),
        "LAN address without token must be refused"
    );
}

#[test]
fn validate_allows_ipv4_loopback_without_token() {
    let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    cxpak::commands::serve::validate_bind_security(&addr, None)
        .expect("127.0.0.1 without token is permitted (local-OS-only access)");
}

#[test]
fn validate_allows_ipv6_loopback_without_token() {
    let addr: SocketAddr = "[::1]:8080".parse().unwrap();
    cxpak::commands::serve::validate_bind_security(&addr, None)
        .expect("::1 without token is permitted (local-OS-only access)");
}

#[test]
fn validate_allows_non_loopback_with_token() {
    let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    cxpak::commands::serve::validate_bind_security(&addr, Some("secret"))
        .expect("0.0.0.0 + --token is the documented production deployment");
}

#[test]
fn validate_rejects_empty_string_token_at_startup() {
    // `--token ""` was previously accepted by validate_bind_security and
    // only rejected per-request by the bearer middleware.  An operator
    // mistake belongs at startup where they can see and fix it, not on
    // every subsequent 401.  Tightened in v2.1.3.
    let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    let err = cxpak::commands::serve::validate_bind_security(&addr, Some(""))
        .expect_err("--token \"\" must Err at startup, not pass through");
    assert!(
        err.contains("non-empty"),
        "error must explain the non-empty requirement; got: {err}"
    );
}

#[test]
fn validate_rejects_ipv4_mapped_ipv6_loopback_without_token() {
    // ::ffff:127.0.0.1 is the IPv4-mapped form of 127.0.0.1.  Per current
    // Rust stdlib (1.94), `Ipv6Addr::is_loopback()` only matches `::1`
    // exactly — IPv4-mapped does NOT count as loopback, so our guard
    // correctly REJECTS this address without a token.  Pinning so a
    // future stdlib extension that flips is_loopback() for the mapped
    // form (which would silently weaken our defense) breaks this test
    // and forces an explicit decision.
    let addr: SocketAddr = "[::ffff:127.0.0.1]:8080".parse().unwrap();
    assert!(
        cxpak::commands::serve::validate_bind_security(&addr, None).is_err(),
        "::ffff:127.0.0.1 (IPv4-mapped IPv6) MUST be treated as non-loopback \
         and require a token — pin this so a future stdlib change cannot \
         silently weaken the guard"
    );
}

// ── Defect #2: security headers ─────────────────────────────────────────────

async fn fetch_headers(uri: &str) -> axum::http::HeaderMap {
    let app = build_router_for_test(shared(), shared_path());
    let req = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    response.headers().clone()
}

#[tokio::test]
async fn responses_carry_strict_content_security_policy() {
    let h = fetch_headers("/health").await;
    let csp = h
        .get("content-security-policy")
        .expect("CSP header must be present on every response")
        .to_str()
        .unwrap();
    assert!(
        csp.contains("default-src 'none'"),
        "CSP must lock default-src to 'none' (no script, no style, no img); got: {csp}"
    );
    assert!(
        csp.contains("frame-ancestors 'none'"),
        "CSP must include frame-ancestors 'none' (clickjacking defense); got: {csp}"
    );
}

#[tokio::test]
async fn responses_carry_x_content_type_options_nosniff() {
    let h = fetch_headers("/health").await;
    let v = h
        .get("x-content-type-options")
        .expect("X-Content-Type-Options must be present")
        .to_str()
        .unwrap();
    assert_eq!(
        v, "nosniff",
        "X-Content-Type-Options must be 'nosniff' to prevent MIME sniffing"
    );
}

#[tokio::test]
async fn responses_carry_referrer_policy_no_referrer() {
    let h = fetch_headers("/health").await;
    let v = h
        .get("referrer-policy")
        .expect("Referrer-Policy must be present")
        .to_str()
        .unwrap();
    assert_eq!(
        v, "no-referrer",
        "Referrer-Policy must be 'no-referrer' so internal paths leak nothing on outbound clicks"
    );
}

#[tokio::test]
async fn responses_carry_x_frame_options_deny() {
    let h = fetch_headers("/health").await;
    let v = h
        .get("x-frame-options")
        .expect("X-Frame-Options must be present")
        .to_str()
        .unwrap();
    assert_eq!(
        v, "DENY",
        "X-Frame-Options must be 'DENY' (defense-in-depth alongside CSP frame-ancestors)"
    );
}

#[tokio::test]
async fn v1_routes_also_carry_security_headers() {
    // Security middleware MUST cover v1 routes too — they were merged
    // into the parent router after the legacy routes were defined, and
    // a wrong layer ordering would leave them naked.
    let app = build_router_for_test_with_token(shared(), shared_path(), Some("t".into()));
    let req = Request::builder()
        .uri("/v1/health")
        .method("GET")
        .header("authorization", "Bearer t")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let h = response.headers();
    assert!(
        h.get("content-security-policy").is_some(),
        "v1 routes must also carry CSP — layer ordering bug if missing"
    );
    assert!(
        h.get("x-content-type-options").is_some(),
        "v1 routes must carry X-Content-Type-Options"
    );
}
