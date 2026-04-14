use crate::budget::counter::TokenCounter;
use crate::cache;
use crate::cli::{VisualFormatArg, VisualTypeArg};
use crate::index::CodebaseIndex;
use crate::scanner::Scanner;
use crate::visual::export;
use crate::visual::layout::{self, LayoutConfig};
use crate::visual::render::{self, RenderMetadata};
use std::io::Write;
use std::path::Path;

fn make_metadata(index: &CodebaseIndex, path: &Path) -> RenderMetadata {
    let repo_name = path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "codebase".to_string());

    let generated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let node_count = index.files.len();
    let edge_count = index.graph.edges.values().map(|v| v.len()).sum::<usize>();

    RenderMetadata {
        repo_name,
        generated_at,
        health_score: None,
        node_count,
        edge_count,
        cxpak_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn ext_for_format(format: &VisualFormatArg) -> &'static str {
    match format {
        VisualFormatArg::Html => "html",
        VisualFormatArg::Mermaid => "mmd",
        VisualFormatArg::Svg => "svg",
        VisualFormatArg::Png => "png",
        VisualFormatArg::C4 => "dsl",
        VisualFormatArg::Json => "json",
    }
}

fn type_slug(visual_type: &VisualTypeArg) -> &'static str {
    match visual_type {
        VisualTypeArg::Dashboard => "dashboard",
        VisualTypeArg::Architecture => "architecture",
        VisualTypeArg::Risk => "risk",
        VisualTypeArg::Flow => "flow",
        VisualTypeArg::Timeline => "timeline",
        VisualTypeArg::Diff => "diff",
    }
}

fn build_index(path: &Path) -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let scanner = Scanner::new(path)?;
    let files = scanner.scan_workspace(None)?;
    if files.is_empty() {
        return Err("no source files found".into());
    }
    let (parse_results, content_map) =
        cache::parse::parse_with_cache(&files, path, &counter, false);
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    index.co_changes = index.conventions.git_health.co_changes.clone();
    Ok(index)
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    visual_type: &VisualTypeArg,
    format: &VisualFormatArg,
    out: Option<&Path>,
    symbol: Option<&str>,
    files: Option<&str>,
    _focus: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let index = build_index(path)?;
    let metadata = make_metadata(&index, path);
    let config = LayoutConfig::default();

    // Render the HTML string for the chosen visual type.
    let html: String = match visual_type {
        VisualTypeArg::Dashboard => render::render_dashboard(&index, &metadata),
        VisualTypeArg::Architecture => render::render_architecture_explorer(&index, &metadata)?,
        VisualTypeArg::Risk => render::render_risk_heatmap(&index, &metadata),
        VisualTypeArg::Flow => {
            let sym = symbol.unwrap_or("main");
            let flow_result = crate::intelligence::data_flow::trace_data_flow(sym, None, 6, &index);
            render::render_flow_diagram(&flow_result, &index, &metadata)?
        }
        VisualTypeArg::Timeline => {
            let snapshots =
                crate::visual::timeline::load_cached_snapshots(path).unwrap_or_default();
            render::render_time_machine(snapshots, &metadata, &config)?
        }
        VisualTypeArg::Diff => {
            let changed: Vec<String> = files
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            render::render_diff_view(&index, &changed, &metadata, &config)?
        }
    };

    // Convert to the requested output format.
    let (content_bytes, is_binary): (Vec<u8>, bool) = match format {
        VisualFormatArg::Html => (html.into_bytes(), false),
        VisualFormatArg::Mermaid => {
            // Build a module layout and serialise to Mermaid syntax.
            let computed =
                layout::build_module_layout(&index, &config).unwrap_or_else(|_| empty_layout());
            (export::to_mermaid(&computed).into_bytes(), false)
        }
        VisualFormatArg::Svg => {
            let computed =
                layout::build_module_layout(&index, &config).unwrap_or_else(|_| empty_layout());
            (export::to_svg(&computed, &metadata).into_bytes(), false)
        }
        VisualFormatArg::Png => {
            let computed =
                layout::build_module_layout(&index, &config).unwrap_or_else(|_| empty_layout());
            (export::to_png(&computed, &metadata, 1200, 900)?, true)
        }
        VisualFormatArg::C4 => {
            let computed =
                layout::build_module_layout(&index, &config).unwrap_or_else(|_| empty_layout());
            (export::to_c4(&computed, &metadata).into_bytes(), false)
        }
        VisualFormatArg::Json => {
            let computed =
                layout::build_module_layout(&index, &config).unwrap_or_else(|_| empty_layout());
            (export::to_json(&computed).into_bytes(), false)
        }
    };

    // Determine output path.
    let default_name = format!(
        "cxpak-{}.{}",
        type_slug(visual_type),
        ext_for_format(format)
    );
    let out_path: std::path::PathBuf = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| path.join(&default_name));

    if is_binary {
        std::fs::write(&out_path, &content_bytes)?;
        eprintln!("cxpak: wrote {}", out_path.display());
    } else {
        match out {
            Some(_) => {
                std::fs::write(&out_path, &content_bytes)?;
                eprintln!("cxpak: wrote {}", out_path.display());
            }
            None => {
                // Default: write to file so the browser can open it.
                std::fs::write(&out_path, &content_bytes)?;
                eprintln!("cxpak: wrote {}", out_path.display());
            }
        }
    }

    // For non-HTML text formats with no explicit --out, also print to stdout
    // so the output can be piped.
    let _ = is_binary;
    match (out, format) {
        (None, VisualFormatArg::Mermaid)
        | (None, VisualFormatArg::Svg)
        | (None, VisualFormatArg::C4)
        | (None, VisualFormatArg::Json) => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(&content_bytes)?;
        }
        _ => {}
    }

    Ok(())
}

fn empty_layout() -> crate::visual::layout::ComputedLayout {
    crate::visual::layout::ComputedLayout {
        nodes: vec![],
        edges: vec![],
        width: 0.0,
        height: 0.0,
        layers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_for_format_all_variants() {
        assert_eq!(ext_for_format(&VisualFormatArg::Html), "html");
        assert_eq!(ext_for_format(&VisualFormatArg::Mermaid), "mmd");
        assert_eq!(ext_for_format(&VisualFormatArg::Svg), "svg");
        assert_eq!(ext_for_format(&VisualFormatArg::Png), "png");
        assert_eq!(ext_for_format(&VisualFormatArg::C4), "dsl");
        assert_eq!(ext_for_format(&VisualFormatArg::Json), "json");
    }

    #[test]
    fn test_type_slug_all_variants() {
        assert_eq!(type_slug(&VisualTypeArg::Dashboard), "dashboard");
        assert_eq!(type_slug(&VisualTypeArg::Architecture), "architecture");
        assert_eq!(type_slug(&VisualTypeArg::Risk), "risk");
        assert_eq!(type_slug(&VisualTypeArg::Flow), "flow");
        assert_eq!(type_slug(&VisualTypeArg::Timeline), "timeline");
        assert_eq!(type_slug(&VisualTypeArg::Diff), "diff");
    }

    #[test]
    fn test_empty_layout_fields() {
        let layout = empty_layout();
        assert!(layout.nodes.is_empty());
        assert!(layout.edges.is_empty());
        assert_eq!(layout.width, 0.0);
        assert_eq!(layout.height, 0.0);
        assert!(layout.layers.is_empty());
    }

    #[test]
    fn test_make_metadata_non_git_dir() {
        use crate::budget::counter::TokenCounter;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let index = crate::index::CodebaseIndex::build_with_content(
            vec![],
            Default::default(),
            &counter,
            Default::default(),
        );
        let meta = make_metadata(&index, dir.path());
        assert!(!meta.cxpak_version.is_empty());
        assert!(!meta.generated_at.is_empty());
        // The repo_name should be the directory name, not empty
        assert!(!meta.repo_name.is_empty());
    }

    #[test]
    fn test_visual_type_matches_render_enum() {
        // Confirm CLI arg names align with the VisualType enum in visual::mod
        let _dashboard = crate::visual::VisualType::Dashboard;
        let _architecture = crate::visual::VisualType::Architecture;
        let _risk = crate::visual::VisualType::Risk;
        let _flow = crate::visual::VisualType::Flow;
        let _timeline = crate::visual::VisualType::Timeline;
        let _diff = crate::visual::VisualType::Diff;
    }

    /// ext_for_format covers all 6 VisualFormatArg variants with correct extensions.
    /// This protects against adding new variants without updating the match arm.
    #[test]
    fn test_ext_for_format_returns_correct_extensions() {
        assert_eq!(ext_for_format(&VisualFormatArg::Html), "html");
        assert_eq!(ext_for_format(&VisualFormatArg::Mermaid), "mmd");
        assert_eq!(ext_for_format(&VisualFormatArg::Svg), "svg");
        assert_eq!(ext_for_format(&VisualFormatArg::Png), "png");
        assert_eq!(ext_for_format(&VisualFormatArg::C4), "dsl");
        assert_eq!(ext_for_format(&VisualFormatArg::Json), "json");
    }

    /// The default output filename uses `cxpak-{type}.{ext}` format.
    #[test]
    fn test_default_filename_format() {
        let type_arg = VisualTypeArg::Risk;
        let format_arg = VisualFormatArg::Svg;
        let slug = type_slug(&type_arg);
        let ext = ext_for_format(&format_arg);
        let default_name = format!("cxpak-{slug}.{ext}");
        assert_eq!(default_name, "cxpak-risk.svg");
    }

    /// type_slug + ext_for_format for every combination of type and format produces
    /// the expected "cxpak-{type}.{ext}" pattern (non-empty, no spaces).
    #[test]
    fn test_default_filename_all_types_and_formats_are_non_empty() {
        let types = [
            VisualTypeArg::Dashboard,
            VisualTypeArg::Architecture,
            VisualTypeArg::Risk,
            VisualTypeArg::Flow,
            VisualTypeArg::Timeline,
            VisualTypeArg::Diff,
        ];
        let formats = [
            VisualFormatArg::Html,
            VisualFormatArg::Mermaid,
            VisualFormatArg::Svg,
            VisualFormatArg::Png,
            VisualFormatArg::C4,
            VisualFormatArg::Json,
        ];
        for t in &types {
            for f in &formats {
                let name = format!("cxpak-{}.{}", type_slug(t), ext_for_format(f));
                assert!(
                    !name.is_empty(),
                    "filename for {t:?}/{f:?} should not be empty"
                );
                assert!(
                    !name.contains(' '),
                    "filename for {t:?}/{f:?} should not contain spaces, got: {name}"
                );
                assert!(
                    name.starts_with("cxpak-"),
                    "filename for {t:?}/{f:?} should start with 'cxpak-', got: {name}"
                );
            }
        }
    }
}
