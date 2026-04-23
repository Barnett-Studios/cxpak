// WCAG AA contrast audit on the critical color pairs.

fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.03928 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(hex: &str) -> f64 {
    let h = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&h[0..2], 16).unwrap() as f64 / 255.0;
    let g = u8::from_str_radix(&h[2..4], 16).unwrap() as f64 / 255.0;
    let b = u8::from_str_radix(&h[4..6], 16).unwrap() as f64 / 255.0;
    0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
}

fn contrast(a: &str, b: &str) -> f64 {
    let la = luminance(a);
    let lb = luminance(b);
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn extract_color(css: &str, var_name: &str, selector: &str) -> String {
    // Find the selector's block, then the var within it.
    let sel_pos = css.find(selector).expect("selector present");
    let block = &css[sel_pos..];
    let block_end = block.find('}').unwrap();
    let block = &block[..block_end];
    let re = regex::Regex::new(&format!(
        r"{}:\s*(#[0-9a-fA-F]{{6}})",
        regex::escape(var_name)
    ))
    .unwrap();
    re.captures(block)
        .expect(var_name)
        .get(1)
        .unwrap()
        .as_str()
        .to_string()
}

#[test]
fn text_dim_passes_wcag_aa_in_dark_mode() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").expect("assets/cxpak-visual.css");
    let bg = extract_color(&css, "--bg-primary", ":root");
    let td = extract_color(&css, "--text-dim", ":root");
    let c = contrast(&bg, &td);
    assert!(
        c >= 4.5,
        "dark --text-dim ({td}) on --bg-primary ({bg}) = {c:.2}:1, fails AA"
    );
}

#[test]
fn text_dim_passes_wcag_aa_in_light_mode() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").expect("assets/cxpak-visual.css");
    let bg = extract_color(&css, "--bg-primary", r#":root[data-theme="light"]"#);
    let td = extract_color(&css, "--text-dim", r#":root[data-theme="light"]"#);
    let c = contrast(&bg, &td);
    assert!(
        c >= 4.5,
        "light --text-dim ({td}) on --bg-primary ({bg}) = {c:.2}:1, fails AA"
    );
}
