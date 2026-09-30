fn main() {
    let src_dir = std::path::Path::new("src");

    let mut c_config = cc::Build::new();
    c_config.include(&src_dir);
    c_config
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");
    let parser_path = src_dir.join("parser.c");
    c_config.file(&parser_path);

    // If your language uses an external scanner written in C,
    // then include this block of code:

    /*
    let scanner_path = src_dir.join("scanner.c");
    c_config.file(&scanner_path);
    println!("cargo:rerun-if-changed={}", scanner_path.to_str().unwrap());
    */

    c_config.compile("parser");
    println!("cargo:rerun-if-changed={}", parser_path.to_str().unwrap());

    // If your language uses an external scanner written in C++,
    // then include this block of code:

    let mut cpp_config = cc::Build::new();
    cpp_config.cpp(true);
    cpp_config.include(&src_dir);
    cpp_config
        .define("TREE_SITTER_MARKDOWN_AVOID_CRASH", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable");
    // TREE_SITTER_MARKDOWN_REPORT_SIZE makes the scanner print the size of the
    // state it is about to serialize, which is how the capacities in
    // src/tree_sitter_markdown/serialization_limit.h were chosen.  Off unless
    // asked for; it writes a line per external token.
    if std::env::var_os("TREE_SITTER_MARKDOWN_REPORT_SIZE").is_some() {
        cpp_config.define("TREE_SITTER_MARKDOWN_REPORT_SIZE", None);
    }
    println!("cargo:rerun-if-env-changed=TREE_SITTER_MARKDOWN_REPORT_SIZE");

    let scanner_path = src_dir.join("scanner.cc");
    cpp_config.file(&scanner_path);
    cpp_config.compile("scanner");

    // scanner.cc #includes every other translation unit (tree-sitter allows the
    // external scanner only one file), so a change to any of them has to
    // retrigger the build.  Watching scanner.cc alone left edits to the headers
    // and the .cc files under src/tree_sitter_markdown/ silently uncompiled.
    println!("cargo:rerun-if-changed={}", scanner_path.to_str().unwrap());
    for entry in std::fs::read_dir(src_dir.join("tree_sitter_markdown"))
        .expect("src/tree_sitter_markdown must be readable")
    {
        println!(
            "cargo:rerun-if-changed={}",
            entry.expect("readable dir entry").path().to_str().unwrap()
        );
    }
}
