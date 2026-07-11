//! N6 acceptance: the SPA ships the client-side palette system (ADR-0191) with
//! Tokyo Night as the default, several popular schemes, no CDN, and — because
//! the palette is applied at runtime — byte-identical output across renders.
#![cfg(feature = "visual")]

use cxpak::test_support::render_fixture_spa;

#[test]
fn spa_ships_palette_registry_tokyo_night_default_and_stays_deterministic() {
    let html = render_fixture_spa();
    let lower = html.to_lowercase();

    // Tokyo Night bg present as the default palette (hex case-insensitive).
    assert!(lower.contains("#1a1b26"), "Tokyo Night bg present");
    assert!(
        html.contains("tokyo-night"),
        "Tokyo Night is registered/default"
    );
    // The apply function and picker exist.
    assert!(html.contains("applyPalette"), "applyPalette present");
    assert!(
        html.contains("cxpak-palette-select"),
        "palette picker present"
    );
    // A spread of popular schemes shipped.
    assert!(
        html.contains("catppuccin") && html.contains("gruvbox") && html.contains("everforest"),
        "popular palettes shipped"
    );
    // No external origin.
    assert!(!html.contains("cdn.jsdelivr.net"), "no CDN");
    assert!(!html.contains("unpkg.com"), "no unpkg");

    // Palette is client-side runtime state → emitted bytes identical.
    assert_eq!(render_fixture_spa(), html, "byte-identical across renders");
}
