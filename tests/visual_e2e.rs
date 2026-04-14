//! Browser-driven end-to-end tests for all 6 cxpak visual views.
//!
//! Each test:
//!   1. Generates HTML via `cxpak visual --visual-type <view> --format html --out <file>`
//!   2. Serves it from a tiny localhost HTTP server on an ephemeral port
//!   3. Loads it in a real headless Chrome process
//!   4. Asserts exact DOM state (counts, text content, attributes)
//!
//! Gate: `#[cfg(all(feature = "visual", feature = "e2e-browser"))]`
//!
//! If Chrome is not installed the tests print a message and return early, so
//! `cargo test --all-features` still passes on hosts without Chrome.

#![cfg(all(feature = "visual", feature = "e2e-browser"))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use headless_chrome::browser::tab::Tab;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::{Browser, LaunchOptions};
use tempfile::TempDir;

// ─── HTTP server ─────────────────────────────────────────────────────────────

/// Spin up a `tiny_http` server that serves files from `dir` on an ephemeral
/// port.  Returns `(port, join_handle)`.  The server runs until the process
/// exits (the handle is intentionally kept alive by the caller).
fn serve_dir(dir: PathBuf) -> (u16, std::thread::JoinHandle<()>) {
    use tiny_http::{Response, Server};

    let server = Server::http("127.0.0.1:0").expect("failed to bind HTTP server");
    let port = server
        .server_addr()
        .to_ip()
        .expect("server_addr is not an IP")
        .port();

    let handle = std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let url = request.url().to_owned();
            let rel = url.trim_start_matches('/').split('?').next().unwrap_or("");
            let file_path = dir.join(rel);

            match std::fs::read(&file_path) {
                Ok(content) => {
                    let content_type = match file_path.extension().and_then(|e| e.to_str()) {
                        Some("html") => "text/html; charset=utf-8",
                        Some("js") => "application/javascript",
                        Some("css") => "text/css",
                        Some("json") => "application/json",
                        Some("svg") => "image/svg+xml",
                        _ => "application/octet-stream",
                    };
                    let mut resp = Response::from_data(content);
                    resp.add_header(
                        tiny_http::Header::from_bytes(
                            &b"Content-Type"[..],
                            content_type.as_bytes(),
                        )
                        .expect("valid header"),
                    );
                    let _ = request.respond(resp);
                }
                Err(_) => {
                    let _ =
                        request.respond(Response::from_string("not found").with_status_code(404));
                }
            }
        }
    });
    (port, handle)
}

// ─── Browser helpers ──────────────────────────────────────────────────────────

/// Try to launch a headless Chrome browser.  Returns `None` and prints a
/// message if Chrome / Chromium is not available on this host.
fn launch_browser() -> Option<Browser> {
    let opts = LaunchOptions::default_builder()
        .headless(true)
        .window_size(Some((1400, 900)))
        .build()
        .expect("LaunchOptions::build() must not fail");

    match Browser::new(opts) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("Skipping browser E2E test — Chrome/Chromium not available: {e}");
            None
        }
    }
}

/// Collect JS exceptions thrown while the page is loaded and after navigation.
///
/// Must be called **before** `navigate_to` so the listener is registered first.
/// Returns a cloned `Arc<Mutex<Vec<String>>>` that accumulates exception messages.
fn attach_exception_collector(tab: &Arc<Tab>) -> Arc<Mutex<Vec<String>>> {
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let errors_clone = Arc::clone(&errors);

    tab.enable_runtime().expect("enable_runtime must succeed");

    tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::RuntimeExceptionThrown(exc) = event {
            let msg = exc.params.exception_details.text.clone();
            errors_clone.lock().unwrap().push(msg);
        }
    }))
    .expect("add_event_listener must succeed");

    errors
}

// ─── CLI helpers ──────────────────────────────────────────────────────────────

/// Build a git-initialised repo fixture with `src/main.rs` and `src/lib.rs`.
/// The fixture at `tests/fixtures/simple_repo` is a static directory without a
/// git index, so we create a fresh one in a `TempDir` the same way the other
/// visual tests do.
fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\nfn helper() -> i32 { 42 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet() { println!(\"hi\"); }\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    dir
}

/// Run `cxpak visual --visual-type <view> [extra_args…] --format html --out <out_dir>/<view>.html <repo>`.
/// Panics if the process exits non-zero.
fn generate_html(out_dir: &Path, repo: &Path, view: &str, extra_args: &[&str]) {
    let out_file = out_dir.join(format!("{view}.html"));
    let mut cmd = std::process::Command::new(assert_cmd::cargo_bin!("cxpak"));
    cmd.args(["visual", "--visual-type", view, "--format", "html", "--out"])
        .arg(&out_file);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(repo);

    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to run cxpak visual: {e}"));
    assert!(
        status.success(),
        "cxpak visual --visual-type {view} --format html failed (exit {:?})",
        status.code()
    );
}

/// Short wait for D3 / JS to finish rendering after navigation.
fn wait_for_render() {
    std::thread::sleep(Duration::from_millis(600));
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn dashboard_has_four_quadrants_and_numeric_health_score() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    generate_html(out.path(), repo.path(), "dashboard", &[]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/dashboard.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // Exactly 4 quadrants
    let quadrants = tab.find_elements(".cxpak-quadrant").unwrap();
    assert_eq!(
        quadrants.len(),
        4,
        "dashboard must have exactly 4 quadrants, got {}",
        quadrants.len()
    );

    // Health gauge shows a numeric score in [0, 10]
    let gauge = tab.find_element(".cxpak-gauge-score").unwrap();
    let text = gauge.get_inner_text().unwrap();
    let score: f64 = text
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("health gauge text must be numeric, got: {text:?}"));
    assert!(
        (0.0..=10.0).contains(&score),
        "health score {score} is outside [0, 10]"
    );

    // At least 4 nav links
    let nav = tab.find_elements(".cxpak-nav-link").unwrap();
    assert!(
        nav.len() >= 4,
        "nav must have at least 4 links, got {}",
        nav.len()
    );

    // No JS exceptions during page load
    let errs = errors.lock().unwrap();
    assert!(
        errs.is_empty(),
        "dashboard.html threw JS exceptions: {errs:?}"
    );
}

#[test]
fn dashboard_health_quadrant_click_navigates_to_architecture() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    // Generate both views so the server can serve architecture.html
    generate_html(out.path(), repo.path(), "dashboard", &[]);
    generate_html(out.path(), repo.path(), "architecture", &[]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    tab.navigate_to(&format!("http://127.0.0.1:{port}/dashboard.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // The Health Score quadrant (Q1) is the first `.cxpak-quadrant.cxpak-clickable` element
    // and has onclick -> navTo('architecture')
    let clickable = tab.find_element(".cxpak-quadrant.cxpak-clickable").unwrap();
    clickable.click().unwrap();

    // Allow up to 2 s for the navigation to complete
    tab.set_default_timeout(Duration::from_secs(2));
    let _ = tab.wait_until_navigated();
    wait_for_render();

    let url = tab.get_url();
    assert!(
        url.contains("architecture.html"),
        "clicking the health quadrant must navigate to architecture.html, got URL: {url:?}"
    );
}

#[test]
fn architecture_renders_nodes_legend_and_breadcrumb() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    generate_html(out.path(), repo.path(), "architecture", &[]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/architecture.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // At least 2 module nodes rendered by D3
    let nodes = tab.find_elements(".cxpak-node").unwrap();
    assert!(
        nodes.len() >= 2,
        "architecture L1 must render at least 2 nodes, got {}",
        nodes.len()
    );

    // Legend is present and mentions "God file"
    let legend = tab.find_element(".cxpak-legend").unwrap();
    let legend_text = legend.get_inner_text().unwrap();
    assert!(
        legend_text.to_lowercase().contains("god"),
        "legend must mention 'God file', got: {legend_text:?}"
    );

    // Legend has exactly 5 swatches (healthy, mid, unhealthy, god-file overlay, circular-dep)
    let swatches = tab.find_elements(".cxpak-legend-swatch").unwrap();
    assert_eq!(
        swatches.len(),
        5,
        "architecture legend must have 5 swatches, got {}",
        swatches.len()
    );

    // Initial breadcrumb says "Repository"
    let bc = tab.find_element(".cxpak-breadcrumb").unwrap();
    let bc_text = bc.get_inner_text().unwrap();
    assert_eq!(
        bc_text.trim(),
        "Repository",
        "initial breadcrumb must be 'Repository', got: {bc_text:?}"
    );

    let errs = errors.lock().unwrap();
    assert!(
        errs.is_empty(),
        "architecture.html threw JS exceptions: {errs:?}"
    );
}

#[test]
fn architecture_node_click_produces_no_js_exceptions() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    generate_html(out.path(), repo.path(), "architecture", &[]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/architecture.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    let nodes = tab.find_elements(".cxpak-node").unwrap();
    assert!(!nodes.is_empty(), "need at least one node to click");

    // Click the first node — may drill down or do nothing if no level-2 data
    nodes[0].click().unwrap();
    wait_for_render();

    // The click must not throw any JS exceptions
    let errs = errors.lock().unwrap();
    assert!(
        errs.is_empty(),
        "clicking a node produced JS exceptions: {errs:?}"
    );
}

#[test]
fn risk_heatmap_renders_treemap_cells_and_legend() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    generate_html(out.path(), repo.path(), "risk", &[]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/risk.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // At least 1 treemap cell
    let cells = tab.find_elements(".treemap-cell").unwrap();
    assert!(
        !cells.is_empty(),
        "risk heatmap must render at least 1 treemap cell"
    );

    // Legend present and mentions risk levels
    let legend = tab.find_element(".cxpak-legend").unwrap();
    let legend_text = legend.get_inner_text().unwrap();
    let lower = legend_text.to_lowercase();
    assert!(
        lower.contains("risk") || lower.contains("low") || lower.contains("high"),
        "risk legend must mention risk levels, got: {legend_text:?}"
    );

    // Exactly 3 swatches (Low / Medium / High)
    let swatches = tab.find_elements(".cxpak-legend-swatch").unwrap();
    assert_eq!(
        swatches.len(),
        3,
        "risk legend must have 3 swatches, got {}",
        swatches.len()
    );

    let errs = errors.lock().unwrap();
    assert!(errs.is_empty(), "risk.html threw JS exceptions: {errs:?}");
}

#[test]
fn flow_diagram_legend_has_four_swatches() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();

    // `cxpak visual flow` exits 0 if the symbol is found, 1 if not found.
    // Either is acceptable — we just need the HTML file to exist.
    let out_file = out.path().join("flow.html");
    let status = std::process::Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "visual",
            "--visual-type",
            "flow",
            "--symbol",
            "main",
            "--format",
            "html",
            "--out",
            out_file.to_str().unwrap(),
        ])
        .arg(repo.path())
        .status()
        .expect("cxpak visual flow must run");

    let exit_code = status.code().unwrap_or(-1);
    assert!(
        exit_code == 0 || exit_code == 1,
        "flow must exit 0 or 1, got {exit_code}"
    );
    assert!(
        out_file.exists(),
        "flow --out file must exist even when exit code is 1"
    );

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/flow.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // Flow legend must have exactly 4 color swatches
    // (Source, Transform, Sink, Passthrough)
    let swatches = tab.find_elements(".cxpak-legend-swatch").unwrap();
    assert_eq!(
        swatches.len(),
        4,
        "flow legend must have 4 swatches (Source/Transform/Sink/Passthrough), got {}",
        swatches.len()
    );

    let errs = errors.lock().unwrap();
    assert!(errs.is_empty(), "flow.html threw JS exceptions: {errs:?}");
}

#[test]
fn timeline_empty_state_shows_fallback_message_and_disabled_controls() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();

    // timeline exits 0 or 1 depending on whether snapshot history is available
    let out_file = out.path().join("timeline.html");
    let status = std::process::Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "visual",
            "--visual-type",
            "timeline",
            "--format",
            "html",
            "--out",
            out_file.to_str().unwrap(),
        ])
        .arg(repo.path())
        .status()
        .expect("cxpak visual timeline must run");

    let exit_code = status.code().unwrap_or(-1);
    // A single-commit repo has no timeline data; exit 0 or 1 both accepted.
    assert!(
        exit_code == 0 || exit_code == 1,
        "timeline must exit 0 or 1, got {exit_code}"
    );
    if !out_file.exists() {
        // timeline produced no output (e.g. returned early); skip DOM assertions.
        return;
    }

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/timeline.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // 9 control buttons: |<  <  ▶  >|  >|  +  0.5x  1x  2x  4x
    // Actually render.rs creates 5 playback + 4 speed = 9 buttons total
    let btns = tab.find_elements(".cxpak-tm-btn").unwrap();
    assert_eq!(
        btns.len(),
        9,
        "timeline must have exactly 9 control buttons (5 playback + 4 speed), got {}",
        btns.len()
    );

    // With an empty fixture repo (1 commit, no snapshots), the fallback message
    // must be rendered somewhere on the page.
    let html = tab.get_content().unwrap();
    let has_fallback = html.contains("Insufficient")
        || html.contains("No timeline snapshots")
        || html.contains("No timeline");
    assert!(
        has_fallback,
        "timeline empty state must show a fallback message"
    );

    let errs = errors.lock().unwrap();
    assert!(
        errs.is_empty(),
        "timeline.html threw JS exceptions: {errs:?}"
    );
}

#[test]
fn diff_view_renders_two_panels_and_impact_badge() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();
    generate_html(out.path(), repo.path(), "diff", &["--files", "src/main.rs"]);

    let (port, _srv) = serve_dir(out.path().to_path_buf());
    let tab = browser.new_tab().unwrap();
    let errors = attach_exception_collector(&tab);
    tab.navigate_to(&format!("http://127.0.0.1:{port}/diff.html"))
        .unwrap()
        .wait_until_navigated()
        .unwrap();
    wait_for_render();

    // Exactly 2 panels: "before" and "after"
    let panels = tab.find_elements(".cxpak-diff-panel").unwrap();
    assert_eq!(
        panels.len(),
        2,
        "diff view must have exactly 2 panels (before + after), got {}",
        panels.len()
    );

    // "Impact" appears somewhere in the page
    let html = tab.get_content().unwrap();
    assert!(
        html.contains("Impact"),
        "diff view must contain an 'Impact' badge or section"
    );

    let errs = errors.lock().unwrap();
    assert!(errs.is_empty(), "diff.html threw JS exceptions: {errs:?}");
}

#[test]
fn no_js_exceptions_on_any_view() {
    let Some(browser) = launch_browser() else {
        return;
    };
    let repo = make_repo();
    let out = TempDir::new().unwrap();

    // Generate all views that exit predictably with 0
    for (view, extra) in &[
        ("dashboard", vec![]),
        ("architecture", vec![]),
        ("risk", vec![]),
        ("diff", vec!["--files", "src/main.rs"]),
    ] {
        generate_html(out.path(), repo.path(), view, extra);
    }

    // flow and timeline may exit 1 — generate them with status check, not assert
    for (view, extra) in &[("flow", vec!["--symbol", "main"]), ("timeline", vec![])] {
        let out_file = out.path().join(format!("{view}.html"));
        let _ = std::process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["visual", "--visual-type", view, "--format", "html", "--out"])
            .arg(&out_file)
            .args(extra)
            .arg(repo.path())
            .status();
    }

    let (port, _srv) = serve_dir(out.path().to_path_buf());

    let views = [
        "dashboard",
        "architecture",
        "risk",
        "diff",
        "flow",
        "timeline",
    ];

    for view in views {
        let html_path = out.path().join(format!("{view}.html"));
        if !html_path.exists() {
            // View produced no output (e.g. timeline with no history)
            eprintln!("no_js_exceptions_on_any_view: {view}.html not generated, skipping");
            continue;
        }

        let tab = browser.new_tab().unwrap();
        let errors = attach_exception_collector(&tab);
        tab.navigate_to(&format!("http://127.0.0.1:{port}/{view}.html"))
            .unwrap()
            .wait_until_navigated()
            .unwrap();
        wait_for_render();

        let errs = errors.lock().unwrap().clone();
        assert!(errs.is_empty(), "{view}.html threw JS exceptions: {errs:?}");
    }
}
