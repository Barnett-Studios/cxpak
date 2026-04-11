# cxpak v0.6.0 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make cxpak smarter (importance-weighted budget allocation), faster (parallel parsing), and bulletproof (95%+ test coverage targeting 100%).

**Architecture:** Three workstreams executed sequentially — test coverage first (safety net), smart context second (new feature validated by tests), speed third (rayon changes internals, tests catch regressions). All new code requires 100% test coverage.

**Tech Stack:** Rust, tree-sitter, rayon (new), git2, clap, tiktoken-rs

**Design doc:** `docs/plans/2026-03-12-v060-smart-context-speed-coverage-design.md`

---

## Workstream 1: Test Coverage (76% → 95%+)

### Task 1: C parser coverage (57% → 95%+)

**Files:**
- Modify: `src/parser/languages/c.rs` (tests section)

**Step 1: Write failing tests for untested C constructs**

Add these tests to the existing `#[cfg(test)] mod tests` block in `src/parser/languages/c.rs`:

```rust
#[test]
fn test_extract_struct() {
    let source = "struct Point {\n    int x;\n    int y;\n};\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    let structs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
    assert!(!structs.is_empty(), "expected struct symbol");
    assert_eq!(structs[0].name, "Point");
    assert_eq!(structs[0].visibility, Visibility::Public);
}

#[test]
fn test_extract_enum() {
    let source = "enum Color {\n    RED,\n    GREEN,\n    BLUE\n};\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    let enums: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Enum).collect();
    assert!(!enums.is_empty(), "expected enum symbol");
    assert_eq!(enums[0].name, "Color");
}

#[test]
fn test_extract_multiple_includes() {
    let source = "#include <stdlib.h>\n#include <string.h>\n#include \"local.h\"\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    assert_eq!(result.imports.len(), 3);
}

#[test]
fn test_extract_function_pointer_param() {
    let source = "void register_callback(void (*cb)(int)) {\n    // store cb\n}\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    let funcs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
    assert!(!funcs.is_empty(), "expected function with pointer param");
    assert_eq!(funcs[0].name, "register_callback");
}

#[test]
fn test_extract_static_function() {
    let source = "static int helper(void) {\n    return 42;\n}\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    // static functions are still top-level function_definitions in C grammar
    let funcs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
    assert!(!funcs.is_empty(), "expected static function");
    assert_eq!(funcs[0].name, "helper");
}

#[test]
fn test_extract_struct_in_declaration() {
    let source = "struct Node {\n    int value;\n    struct Node* next;\n} node;\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    let structs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
    assert!(!structs.is_empty(), "expected struct from declaration");
    assert_eq!(structs[0].name, "Node");
}

#[test]
fn test_empty_source() {
    let source = "";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    assert!(result.symbols.is_empty());
    assert!(result.imports.is_empty());
    assert!(result.exports.is_empty());
}

#[test]
fn test_typedef_struct() {
    let source = "typedef struct {\n    float x;\n    float y;\n} Vec2;\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    let typedefs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::TypeAlias).collect();
    assert!(!typedefs.is_empty(), "expected typedef for struct");
    assert_eq!(typedefs[0].name, "Vec2");
}

#[test]
fn test_multiple_functions() {
    let source = "int foo(void) { return 1; }\nint bar(int x) { return x * 2; }\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    assert_eq!(result.symbols.len(), 2);
    assert_eq!(result.exports.len(), 2);
}

#[test]
fn test_function_line_numbers() {
    let source = "\n\nint foo(void) {\n    return 1;\n}\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CLanguage;
    let result = lang.extract(source, &tree);
    assert_eq!(result.symbols[0].start_line, 3);
    assert_eq!(result.symbols[0].end_line, 5);
}
```

**Step 2: Run tests to verify they pass (these test existing functionality)**

Run: `cargo test --test-threads=1 -- parser::languages::c`

**Step 3: Commit**

```bash
git add src/parser/languages/c.rs
git commit -m "test: add comprehensive C parser tests for structs, enums, typedefs, edge cases"
```

---

### Task 2: C++ parser coverage (57% → 95%+)

**Files:**
- Modify: `src/parser/languages/cpp.rs` (tests section)

**Step 1: Write tests for untested C++ constructs**

Add to existing tests in `src/parser/languages/cpp.rs`:

```rust
#[test]
fn test_extract_namespace() {
    let source = "namespace math {\n    int add(int a, int b) { return a + b; }\n}\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let ns: Vec<_> = result.symbols.iter().filter(|s| s.name == "math").collect();
    assert!(!ns.is_empty(), "expected namespace symbol");
}

#[test]
fn test_extract_struct() {
    let source = "struct Vec3 {\n    float x, y, z;\n};\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let structs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
    assert!(!structs.is_empty(), "expected struct");
    assert_eq!(structs[0].name, "Vec3");
}

#[test]
fn test_extract_enum() {
    let source = "enum Direction {\n    UP,\n    DOWN,\n    LEFT,\n    RIGHT\n};\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let enums: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Enum).collect();
    assert!(!enums.is_empty(), "expected enum");
    assert_eq!(enums[0].name, "Direction");
}

#[test]
fn test_extract_class_in_declaration() {
    let source = "class Widget {\npublic:\n    void draw();\n} widget;\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let classes: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
    assert!(!classes.is_empty(), "expected class from declaration");
    assert_eq!(classes[0].name, "Widget");
}

#[test]
fn test_extract_struct_in_declaration() {
    let source = "struct Data {\n    int value;\n} data;\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let structs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
    assert!(!structs.is_empty(), "expected struct from declaration");
    assert_eq!(structs[0].name, "Data");
}

#[test]
fn test_extract_typedef_cpp() {
    let source = "typedef long long int64;\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let typedefs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::TypeAlias).collect();
    assert!(!typedefs.is_empty(), "expected typedef");
    assert_eq!(typedefs[0].name, "int64");
}

#[test]
fn test_empty_source_cpp() {
    let source = "";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    assert!(result.symbols.is_empty());
    assert!(result.imports.is_empty());
}

#[test]
fn test_multiple_includes_cpp() {
    let source = "#include <vector>\n#include <map>\n#include <algorithm>\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    assert_eq!(result.imports.len(), 3);
}

#[test]
fn test_function_with_reference_param() {
    let source = "void swap(int& a, int& b) {\n    int tmp = a;\n    a = b;\n    b = tmp;\n}\n";
    let mut parser = make_parser();
    let tree = parser.parse(source, None).unwrap();
    let lang = CppLanguage;
    let result = lang.extract(source, &tree);
    let funcs: Vec<_> = result.symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
    assert!(!funcs.is_empty());
    assert_eq!(funcs[0].name, "swap");
}
```

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::cpp`

**Step 3: Commit**

```bash
git add src/parser/languages/cpp.rs
git commit -m "test: add comprehensive C++ parser tests for namespaces, classes, enums, declarations"
```

---

### Task 3: C# parser coverage (56% → 95%+)

**Files:**
- Modify: `src/parser/languages/csharp.rs` (tests section)

**Step 1: Write tests for untested C# constructs**

Follow same pattern as C/C++ — test classes, interfaces, enums, properties, generics, nested types, using statements. Each test creates a `make_parser()`, parses source, calls `extract()`, asserts on symbols/imports/exports.

Key test cases:
- `test_extract_interface` — `interface IDisposable { void Dispose(); }`
- `test_extract_enum_members` — `enum Color { Red, Green, Blue }`
- `test_extract_property` — `class Foo { public int Bar { get; set; } }`
- `test_extract_generic_class` — `class List<T> { }`
- `test_extract_nested_class` — class inside class
- `test_extract_using_statements` — `using System;` as imports
- `test_extract_static_method` — `static void Main(string[] args) { }`
- `test_empty_source`
- `test_multiple_classes`

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::csharp`

**Step 3: Commit**

```bash
git add src/parser/languages/csharp.rs
git commit -m "test: add comprehensive C# parser tests for interfaces, enums, properties, generics"
```

---

### Task 4: Java parser coverage (58% → 95%+)

**Files:**
- Modify: `src/parser/languages/java.rs` (tests section)

**Step 1: Write tests for untested Java constructs**

Key test cases:
- `test_extract_generic_class` — `public class Box<T> { }`
- `test_extract_inner_class` — class inside class
- `test_extract_annotation` — `@Override public void run() { }`
- `test_extract_enum_constants` — `enum Status { ACTIVE, INACTIVE }`
- `test_extract_static_method` — `public static void main(String[] args) { }`
- `test_extract_interface` — `public interface Runnable { void run(); }`
- `test_extract_multiple_imports` — multiple import statements
- `test_empty_source`
- `test_extract_abstract_class`

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::java`

**Step 3: Commit**

```bash
git add src/parser/languages/java.rs
git commit -m "test: add comprehensive Java parser tests for generics, inner classes, annotations, enums"
```

---

### Task 5: TypeScript parser coverage (65% → 95%+)

**Files:**
- Modify: `src/parser/languages/typescript.rs` (tests section)

**Step 1: Write tests for untested TypeScript constructs**

Key test cases:
- `test_extract_generic_function` — `function identity<T>(arg: T): T { return arg; }`
- `test_extract_decorated_class` — class with decorator
- `test_extract_mapped_type` — `type Readonly<T> = { readonly [P in keyof T]: T[P] }`
- `test_extract_conditional_type` — type with conditional
- `test_extract_reexport` — `export { Foo } from './foo'`
- `test_extract_type_alias` — `type StringOrNumber = string | number`
- `test_extract_enum` — `enum Direction { Up, Down }`
- `test_extract_interface` — `interface User { name: string; }`
- `test_empty_source`
- `test_extract_arrow_function_export` — `export const add = (a: number, b: number) => a + b`

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::typescript`

**Step 3: Commit**

```bash
git add src/parser/languages/typescript.rs
git commit -m "test: add comprehensive TypeScript parser tests for generics, decorators, types, re-exports"
```

---

### Task 6: Kotlin parser coverage (68% → 95%+)

**Files:**
- Modify: `src/parser/languages/kotlin.rs` (tests section)

**Step 1: Write tests for untested Kotlin constructs**

Key test cases:
- `test_extract_data_class` — `data class User(val name: String, val age: Int)`
- `test_extract_companion_object` — `companion object { const val MAX = 100 }`
- `test_extract_extension_function` — `fun String.addExclamation(): String = this + "!"`
- `test_extract_sealed_class` — `sealed class Result { }`
- `test_extract_object_declaration` — `object Singleton { }`
- `test_extract_interface` — `interface Drawable { fun draw() }`
- `test_extract_enum_class` — `enum class Color { RED, GREEN, BLUE }`
- `test_empty_source`
- `test_extract_import_statements`
- `test_extract_function_with_default_params`

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::kotlin`

**Step 3: Commit**

```bash
git add src/parser/languages/kotlin.rs
git commit -m "test: add comprehensive Kotlin parser tests for data classes, sealed classes, extensions"
```

---

### Task 7: Swift parser coverage (68% → 95%+)

**Files:**
- Modify: `src/parser/languages/swift.rs` (tests section)

**Step 1: Write tests for untested Swift constructs**

Key test cases:
- `test_extract_protocol` — `protocol Equatable { func isEqual(to: Self) -> Bool }`
- `test_extract_extension` — `extension String { func reversed() -> String { ... } }`
- `test_extract_computed_property` — `var fullName: String { return "\(first) \(last)" }`
- `test_extract_enum_with_cases` — `enum Compass { case north, south, east, west }`
- `test_extract_access_levels` — `public`, `private`, `internal` functions
- `test_extract_struct` — `struct Point { var x: Double; var y: Double }`
- `test_extract_import` — `import Foundation`
- `test_empty_source`
- `test_extract_class_with_init`

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- parser::languages::swift`

**Step 3: Commit**

```bash
git add src/parser/languages/swift.rs
git commit -m "test: add comprehensive Swift parser tests for protocols, extensions, enums, access levels"
```

---

### Task 8: Index module coverage (graph.rs 63%, mod.rs 77% → 95%+)

**Files:**
- Modify: `src/index/graph.rs` (add tests)
- Modify: `src/index/mod.rs` (add tests)

**Step 1: Write graph.rs tests**

Add `#[cfg(test)] mod tests` to `src/index/graph.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert!(graph.edges.is_empty());
        assert!(graph.dependents("any").is_empty());
        assert!(graph.dependencies("any").is_none());
    }

    #[test]
    fn test_add_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        assert!(graph.edges.contains_key("a.rs"));
        assert!(graph.edges["a.rs"].contains("b.rs"));
    }

    #[test]
    fn test_dependents() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("c.rs", "b.rs");
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"a.rs"));
        assert!(deps.contains(&"c.rs"));
    }

    #[test]
    fn test_dependencies() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "c.rs");
        let deps = graph.dependencies("a.rs").unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains("b.rs"));
        assert!(deps.contains("c.rs"));
    }

    #[test]
    fn test_dependencies_none() {
        let graph = DependencyGraph::new();
        assert!(graph.dependencies("nonexistent").is_none());
    }

    #[test]
    fn test_reachable_from_single() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("b.rs", "c.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(reachable.contains("c.rs"));
    }

    #[test]
    fn test_reachable_from_reverse() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        // Starting from b.rs should reach a.rs via incoming edges
        let reachable = graph.reachable_from(&["b.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
    }

    #[test]
    fn test_reachable_from_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("b.rs", "c.rs");
        graph.add_edge("c.rs", "a.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_reachable_from_disconnected() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("c.rs", "d.rs");
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(!reachable.contains("c.rs"));
        assert!(!reachable.contains("d.rs"));
    }

    #[test]
    fn test_reachable_from_empty_start() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        let reachable = graph.reachable_from(&[]);
        assert!(reachable.is_empty());
    }

    #[test]
    fn test_duplicate_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs");
        graph.add_edge("a.rs", "b.rs");
        assert_eq!(graph.edges["a.rs"].len(), 1); // HashSet deduplicates
    }
}
```

**Step 2: Write mod.rs tests**

Add to existing `#[cfg(test)] mod tests` in `src/index/mod.rs`:

```rust
#[test]
fn test_find_symbol_case_insensitive() {
    // build a minimal index with a known symbol
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "pub fn MyFunc() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "test.rs".into(),
        absolute_path: file_path,
        language: Some("rust".into()),
        size_bytes: 18,
    }];
    let mut parse_results = HashMap::new();
    parse_results.insert("test.rs".into(), ParseResult {
        symbols: vec![Symbol {
            name: "MyFunc".into(),
            kind: crate::parser::language::SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "pub fn MyFunc()".into(),
            body: "{}".into(),
            start_line: 1,
            end_line: 1,
        }],
        imports: vec![],
        exports: vec![],
    });
    let index = CodebaseIndex::build(files, parse_results, &counter);

    // Case-insensitive match
    let matches = index.find_symbol("myfunc");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].1.name, "MyFunc");

    // No match
    let no_match = index.find_symbol("nonexistent");
    assert!(no_match.is_empty());
}

#[test]
fn test_find_content_matches() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "fn hello_world() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "test.rs".into(),
        absolute_path: file_path,
        language: Some("rust".into()),
        size_bytes: 20,
    }];
    let index = CodebaseIndex::build(files, HashMap::new(), &counter);

    let matches = index.find_content_matches("hello_world");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0], "test.rs");

    let no_match = index.find_content_matches("xyz_not_found");
    assert!(no_match.is_empty());
}

#[test]
fn test_all_public_symbols() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "pub fn foo() {} fn bar() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "test.rs".into(),
        absolute_path: file_path,
        language: Some("rust".into()),
        size_bytes: 27,
    }];
    let mut parse_results = HashMap::new();
    parse_results.insert("test.rs".into(), ParseResult {
        symbols: vec![
            Symbol {
                name: "foo".into(),
                kind: crate::parser::language::SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn foo()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            },
            Symbol {
                name: "bar".into(),
                kind: crate::parser::language::SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn bar()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            },
        ],
        imports: vec![],
        exports: vec![],
    });
    let index = CodebaseIndex::build(files, parse_results, &counter);
    let public = index.all_public_symbols();
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].1.name, "foo");
}

#[test]
fn test_all_imports() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "use std::io;").unwrap();
    let files = vec![ScannedFile {
        relative_path: "test.rs".into(),
        absolute_path: file_path,
        language: Some("rust".into()),
        size_bytes: 12,
    }];
    let mut parse_results = HashMap::new();
    parse_results.insert("test.rs".into(), ParseResult {
        symbols: vec![],
        imports: vec![Import {
            source: "std::io".into(),
            names: vec!["io".into()],
        }],
        exports: vec![],
    });
    let index = CodebaseIndex::build(files, parse_results, &counter);
    let imports = index.all_imports();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].1.source, "std::io");
}

#[test]
fn test_language_stats() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp1 = dir.path().join("a.rs");
    let fp2 = dir.path().join("b.rs");
    let fp3 = dir.path().join("c.py");
    std::fs::write(&fp1, "fn a() {}").unwrap();
    std::fs::write(&fp2, "fn b() {}").unwrap();
    std::fs::write(&fp3, "def c(): pass").unwrap();
    let files = vec![
        ScannedFile { relative_path: "a.rs".into(), absolute_path: fp1, language: Some("rust".into()), size_bytes: 9 },
        ScannedFile { relative_path: "b.rs".into(), absolute_path: fp2, language: Some("rust".into()), size_bytes: 9 },
        ScannedFile { relative_path: "c.py".into(), absolute_path: fp3, language: Some("python".into()), size_bytes: 13 },
    ];
    let index = CodebaseIndex::build(files, HashMap::new(), &counter);
    assert_eq!(index.language_stats["rust"].file_count, 2);
    assert_eq!(index.language_stats["python"].file_count, 1);
    assert_eq!(index.total_files, 3);
}
```

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- index::`

**Step 3: Commit**

```bash
git add src/index/graph.rs src/index/mod.rs
git commit -m "test: add comprehensive index tests for graph traversal, symbol lookup, content matching"
```

---

### Task 9: Git module coverage (84% → 95%+)

**Files:**
- Modify: `src/git/mod.rs` (add tests)

**Step 1: Write tests for uncovered git edge cases**

Add to existing tests in `src/git/mod.rs`:

```rust
#[test]
fn test_empty_repo_no_commits() {
    let dir = tempfile::TempDir::new().unwrap();
    let _repo = git2::Repository::init(dir.path()).unwrap();
    // No commits yet — push_head should fail gracefully
    let result = extract_git_context(dir.path(), 100);
    assert!(result.is_err(), "expected error for repo with no commits");
}

#[test]
fn test_single_commit() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Alice", "alice@test.com").unwrap();
    make_commit(&repo, &sig, "first", &[("hello.txt", "hi")], None);

    let ctx = extract_git_context(dir.path(), 100).unwrap();
    assert_eq!(ctx.commits.len(), 1);
    assert_eq!(ctx.commits[0].message, "first");
    assert_eq!(ctx.contributors.len(), 1);
    assert_eq!(ctx.contributors[0].name, "Alice");
}

#[test]
fn test_max_commits_limit() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();
    let c1 = make_commit(&repo, &sig, "c1", &[("a.txt", "1")], None);
    let c2 = make_commit(&repo, &sig, "c2", &[("a.txt", "2")], Some(c1));
    let _c3 = make_commit(&repo, &sig, "c3", &[("a.txt", "3")], Some(c2));

    // Limit to 2 commits
    let ctx = extract_git_context(dir.path(), 2).unwrap();
    assert_eq!(ctx.commits.len(), 2);
    // Newest first
    assert_eq!(ctx.commits[0].message, "c3");
    assert_eq!(ctx.commits[1].message, "c2");
}

#[test]
fn test_format_date() {
    assert_eq!(format_date(0), "1970-01-01");
    assert_eq!(format_date(-1), "1970-01-01"); // negative -> epoch fallback
    assert_eq!(format_date(1_700_000_000), "2023-11-14");
}

#[test]
fn test_multiple_contributors() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let alice = git2::Signature::now("Alice", "alice@test.com").unwrap();
    let bob = git2::Signature::now("Bob", "bob@test.com").unwrap();
    let c1 = make_commit(&repo, &alice, "by alice", &[("a.txt", "a")], None);
    let _c2 = make_commit(&repo, &bob, "by bob", &[("b.txt", "b")], Some(c1));

    let ctx = extract_git_context(dir.path(), 100).unwrap();
    assert_eq!(ctx.contributors.len(), 2);
}

#[test]
fn test_file_churn_sorted() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();
    let c1 = make_commit(&repo, &sig, "c1", &[("hot.txt", "1"), ("cold.txt", "1")], None);
    let c2 = make_commit(&repo, &sig, "c2", &[("hot.txt", "2")], Some(c1));
    let _c3 = make_commit(&repo, &sig, "c3", &[("hot.txt", "3")], Some(c2));

    let ctx = extract_git_context(dir.path(), 100).unwrap();
    assert_eq!(ctx.file_churn[0].path, "hot.txt");
    assert_eq!(ctx.file_churn[0].commit_count, 3);
}

#[test]
fn test_not_a_git_repo() {
    let dir = tempfile::TempDir::new().unwrap();
    let result = extract_git_context(dir.path(), 100);
    assert!(result.is_err());
}
```

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -- git::`

**Step 3: Commit**

```bash
git add src/git/mod.rs
git commit -m "test: add git edge case tests for empty repos, commit limits, contributor sorting"
```

---

### Task 10: Commands coverage (overview, trace, diff)

**Files:**
- Modify: integration tests or add targeted unit tests

**Step 1: Add CLI integration tests**

Create tests in `tests/cli_tests.rs` (or add to existing integration tests):

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn make_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() { println!(\"hello\"); }\n").unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").unwrap();

    let mut index = repo.index().unwrap();
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();

    dir
}

#[test]
fn test_overview_markdown() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files:"));
}

#[test]
fn test_overview_json() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--format", "json", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_overview_xml() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--format", "xml", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak"));
}

#[test]
fn test_overview_zero_tokens() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "0", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be greater than 0"));
}

#[test]
fn test_overview_invalid_tokens() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "abc", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid token count"));
}

#[test]
fn test_trace_not_found() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["trace", "--tokens", "10k", "nonexistent_symbol_xyz", repo.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn test_trace_found() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["trace", "--tokens", "10k", "main", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn test_diff_no_changes() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["diff", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes detected"));
}

#[test]
fn test_diff_with_changes() {
    let repo = make_test_repo();
    std::fs::write(repo.path().join("src/main.rs"), "fn main() { println!(\"changed\"); }\n").unwrap();
    Command::cargo_bin("cxpak").unwrap()
        .args(["diff", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("src/main.rs"));
}

#[test]
fn test_clean_command() {
    let repo = make_test_repo();
    // Create .cxpak directory
    std::fs::create_dir_all(repo.path().join(".cxpak/cache")).unwrap();
    std::fs::write(repo.path().join(".cxpak/test.md"), "test").unwrap();

    Command::cargo_bin("cxpak").unwrap()
        .args(["clean", repo.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!repo.path().join(".cxpak").exists());
}

#[test]
fn test_overview_verbose() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--verbose", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("scanning"));
}

#[test]
fn test_bad_subcommand() {
    Command::cargo_bin("cxpak").unwrap()
        .args(["nonsense"])
        .assert()
        .failure();
}

#[test]
fn test_no_subcommand() {
    Command::cargo_bin("cxpak").unwrap()
        .assert()
        .failure();
}

#[test]
fn test_overview_with_output_file() {
    let repo = make_test_repo();
    let out = repo.path().join("output.md");
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--out", out.to_str().unwrap(), repo.path().to_str().unwrap()])
        .assert()
        .success();
    assert!(out.exists());
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("Files:"));
}
```

**Step 2: Run tests**

Run: `cargo test --test-threads=1`

**Step 3: Commit**

```bash
git add tests/
git commit -m "test: add comprehensive CLI integration tests for all commands and error paths"
```

---

### Task 11: Mop-up — remaining uncovered lines

**Files:**
- Modify: various files with small test additions

**Step 1: Run coverage to identify remaining gaps**

Run: `cargo tarpaulin --version 0.31.4 --out Html --output-dir coverage/`

**Step 2: Add targeted tests for remaining uncovered lines**

Focus on:
- `cache/mod.rs` — 2 remaining lines (cache miss/corruption paths)
- `cache/parse.rs` — 2 remaining lines (parse error paths)
- `budget/counter.rs` — 2 remaining lines (edge cases)
- `cli/mod.rs` — 1 remaining line (if any)
- Any language parser lines still uncovered

**Step 3: Run full test suite and verify coverage**

Run: `cargo test --test-threads=1`
Run: `cargo tarpaulin --version 0.31.4 --out Html --output-dir coverage/`

Expected: 95%+ overall coverage

**Step 4: Commit**

```bash
git add -A
git commit -m "test: mop-up coverage for cache, budget, CLI edge cases — target 95%+"
```

---

## Workstream 2: Smart Context

### Task 12: Create `src/index/ranking.rs` with `FileScore` and scoring logic

**Files:**
- Create: `src/index/ranking.rs`
- Modify: `src/index/mod.rs` (add `pub mod ranking;`)

**Step 1: Write failing tests for ranking module**

Create `src/index/ranking.rs` with tests first:

```rust
use std::collections::HashMap;
use crate::index::graph::DependencyGraph;
use crate::git::{GitContext, FileChurn, CommitInfo, ContributorInfo};

#[derive(Debug, Clone)]
pub struct FileScore {
    pub path: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub git_recency: f64,
    pub git_churn: f64,
    pub composite: f64,
}

/// Compute importance scores for all files.
///
/// Weights: in_degree * 0.4 + out_degree * 0.1 + git_recency * 0.3 + git_churn * 0.2
pub fn rank_files(
    file_paths: &[String],
    graph: &DependencyGraph,
    git_context: Option<&GitContext>,
) -> Vec<FileScore> {
    todo!()
}

/// Apply focus boost: 2x for files under focus_path, 1.5x for their direct dependencies.
pub fn apply_focus(
    scores: &mut [FileScore],
    focus_path: &str,
    graph: &DependencyGraph,
) {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_git_context(file_churns: Vec<(&str, usize)>, dates: Vec<&str>) -> GitContext {
        GitContext {
            commits: dates.into_iter().enumerate().map(|(i, d)| CommitInfo {
                hash: format!("{:07}", i),
                message: format!("commit {}", i),
                author: "Test".into(),
                date: d.to_string(),
            }).collect(),
            file_churn: file_churns.into_iter().map(|(p, c)| FileChurn {
                path: p.to_string(),
                commit_count: c,
            }).collect(),
            contributors: vec![ContributorInfo { name: "Test".into(), commit_count: 1 }],
        }
    }

    #[test]
    fn test_rank_files_basic() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("app.rs", "lib.rs");
        graph.add_edge("cli.rs", "lib.rs");
        // lib.rs has in_degree=2, app.rs and cli.rs have in_degree=0

        let paths = vec!["app.rs".into(), "cli.rs".into(), "lib.rs".into()];
        let scores = rank_files(&paths, &graph, None);

        assert_eq!(scores.len(), 3);
        let lib_score = scores.iter().find(|s| s.path == "lib.rs").unwrap();
        let app_score = scores.iter().find(|s| s.path == "app.rs").unwrap();
        assert!(lib_score.composite > app_score.composite, "lib.rs should rank higher (more dependents)");
        assert_eq!(lib_score.in_degree, 2);
    }

    #[test]
    fn test_rank_files_with_git() {
        let graph = DependencyGraph::new();
        let git = make_git_context(
            vec![("hot.rs", 10), ("cold.rs", 1)],
            vec!["2026-03-12"],
        );

        let paths = vec!["hot.rs".into(), "cold.rs".into()];
        let scores = rank_files(&paths, &graph, Some(&git));

        let hot = scores.iter().find(|s| s.path == "hot.rs").unwrap();
        let cold = scores.iter().find(|s| s.path == "cold.rs").unwrap();
        assert!(hot.git_churn > cold.git_churn);
        assert!(hot.composite > cold.composite);
    }

    #[test]
    fn test_rank_files_empty() {
        let graph = DependencyGraph::new();
        let scores = rank_files(&[], &graph, None);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_rank_files_no_graph_no_git() {
        let graph = DependencyGraph::new();
        let paths = vec!["a.rs".into(), "b.rs".into()];
        let scores = rank_files(&paths, &graph, None);
        // All scores should be 0 without any signals
        assert_eq!(scores.len(), 2);
        for s in &scores {
            assert_eq!(s.composite, 0.0);
        }
    }

    #[test]
    fn test_apply_focus() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/auth/mod.rs", "src/db/users.rs");

        let mut scores = vec![
            FileScore { path: "src/auth/mod.rs".into(), in_degree: 0, out_degree: 1, git_recency: 0.0, git_churn: 0.0, composite: 1.0 },
            FileScore { path: "src/db/users.rs".into(), in_degree: 1, out_degree: 0, git_recency: 0.0, git_churn: 0.0, composite: 0.5 },
            FileScore { path: "src/other.rs".into(), in_degree: 0, out_degree: 0, git_recency: 0.0, git_churn: 0.0, composite: 0.3 },
        ];

        apply_focus(&mut scores, "src/auth", &graph);

        let auth = scores.iter().find(|s| s.path == "src/auth/mod.rs").unwrap();
        let db = scores.iter().find(|s| s.path == "src/db/users.rs").unwrap();
        let other = scores.iter().find(|s| s.path == "src/other.rs").unwrap();

        assert!((auth.composite - 2.0).abs() < 0.01, "focus path should be 2x: {}", auth.composite);
        assert!((db.composite - 0.75).abs() < 0.01, "dependency should be 1.5x: {}", db.composite);
        assert!((other.composite - 0.3).abs() < 0.01, "unrelated should be unchanged: {}", other.composite);
    }

    #[test]
    fn test_apply_focus_no_match() {
        let graph = DependencyGraph::new();
        let mut scores = vec![
            FileScore { path: "a.rs".into(), in_degree: 0, out_degree: 0, git_recency: 0.0, git_churn: 0.0, composite: 1.0 },
        ];
        let original = scores[0].composite;
        apply_focus(&mut scores, "nonexistent/path", &graph);
        assert_eq!(scores[0].composite, original, "should be unchanged");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --test-threads=1 -- index::ranking`
Expected: FAIL with `not yet implemented`

**Step 3: Implement ranking logic**

Replace the `todo!()` calls with actual implementations:

```rust
pub fn rank_files(
    file_paths: &[String],
    graph: &DependencyGraph,
    git_context: Option<&GitContext>,
) -> Vec<FileScore> {
    // Build churn map from git context
    let churn_map: HashMap<&str, usize> = git_context
        .map(|g| g.file_churn.iter().map(|f| (f.path.as_str(), f.commit_count)).collect())
        .unwrap_or_default();

    let max_churn = churn_map.values().copied().max().unwrap_or(1) as f64;

    // Compute recency from most recent commit date (simple: all files get same recency for now)
    // A more sophisticated version would track per-file last-modified dates
    let git_recency = git_context
        .and_then(|g| g.commits.first())
        .map(|_| 1.0)  // Has recent commits
        .unwrap_or(0.0);

    file_paths
        .iter()
        .map(|path| {
            let in_degree = graph.dependents(path).len();
            let out_degree = graph.dependencies(path).map(|d| d.len()).unwrap_or(0);
            let file_churn = churn_map.get(path.as_str()).copied().unwrap_or(0) as f64 / max_churn;

            let composite = in_degree as f64 * 0.4
                + out_degree as f64 * 0.1
                + git_recency * 0.3
                + file_churn * 0.2;

            FileScore {
                path: path.clone(),
                in_degree,
                out_degree,
                git_recency,
                git_churn: file_churn,
                composite,
            }
        })
        .collect()
}

pub fn apply_focus(
    scores: &mut [FileScore],
    focus_path: &str,
    graph: &DependencyGraph,
) {
    // Find files under focus path
    let focus_files: Vec<String> = scores
        .iter()
        .filter(|s| s.path.starts_with(focus_path))
        .map(|s| s.path.clone())
        .collect();

    // Find direct dependencies of focus files
    let mut dep_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &focus_files {
        if let Some(deps) = graph.dependencies(f) {
            dep_files.extend(deps.iter().cloned());
        }
        for dep in graph.dependents(f) {
            dep_files.insert(dep.to_string());
        }
    }
    // Remove focus files from dep set (they get 2x, not 1.5x)
    for f in &focus_files {
        dep_files.remove(f);
    }

    for score in scores.iter_mut() {
        if focus_files.contains(&score.path) {
            score.composite *= 2.0;
        } else if dep_files.contains(&score.path) {
            score.composite *= 1.5;
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --test-threads=1 -- index::ranking`
Expected: PASS

**Step 5: Add `pub mod ranking;` to `src/index/mod.rs`**

Add after line 2 (`pub mod symbols;`):
```rust
pub mod ranking;
```

**Step 6: Commit**

```bash
git add src/index/ranking.rs src/index/mod.rs
git commit -m "feat: add graph-based file importance ranking module"
```

---

### Task 13: Integrate ranking into budget allocation

**Files:**
- Modify: `src/commands/overview.rs`

**Step 1: Write a test for ranked overview behavior**

This is an integration-level change. The key behavior: files above median score get full content in module map, files below get signatures only. Add a targeted test that verifies module map ordering changes.

**Step 2: Modify `overview.rs` to use ranking**

In `run()`, after building the index and before budget allocation:

1. Build dependency graph: `let graph = crate::commands::trace::build_dependency_graph(&index);`
2. Extract git context: `let git_ctx = git::extract_git_context(path, 20).ok();`
3. Rank files: `let scores = ranking::rank_files(&file_paths, &graph, git_ctx.as_ref());`
4. Sort `index.files` by score (descending) so high-importance files get budget first

The budget allocation itself doesn't change — the *order* of files in section rendering changes, which means important files get their content before the budget runs out.

**Step 3: Run full test suite**

Run: `cargo test --test-threads=1`

**Step 4: Commit**

```bash
git add src/commands/overview.rs
git commit -m "feat: integrate importance ranking into overview command"
```

---

### Task 14: Add `--focus` flag to CLI

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/commands/overview.rs`
- Modify: `src/commands/trace.rs`
- Modify: `src/commands/diff.rs`
- Modify: `src/main.rs`

**Step 1: Add `--focus` to CLI definition**

In `src/cli/mod.rs`, add to `Overview`, `Trace`, and `Diff` variants:

```rust
#[arg(long)]
focus: Option<String>,
```

**Step 2: Thread `focus` through `main.rs` to command functions**

Update each `run()` signature to accept `focus: Option<&str>`.

**Step 3: Apply focus boost in overview.rs**

After ranking, if focus is Some:
```rust
if let Some(focus_path) = focus {
    ranking::apply_focus(&mut scores, focus_path, &graph);
}
```

**Step 4: Write integration test for --focus**

```rust
#[test]
fn test_overview_with_focus() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--focus", "src", repo.path().to_str().unwrap()])
        .assert()
        .success();
}
```

**Step 5: Run tests**

Run: `cargo test --test-threads=1`

**Step 6: Commit**

```bash
git add src/cli/mod.rs src/commands/overview.rs src/commands/trace.rs src/commands/diff.rs src/main.rs tests/
git commit -m "feat: add --focus flag for importance boosting on overview, trace, diff"
```

---

## Workstream 3: Speed

### Task 15: Add `--timing` flag

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/commands/overview.rs`
- Modify: `src/commands/trace.rs`
- Modify: `src/commands/diff.rs`
- Modify: `src/main.rs`

**Step 1: Add `--timing` to CLI**

In `src/cli/mod.rs`, add to all three command variants:

```rust
#[arg(long)]
timing: bool,
```

**Step 2: Add timing instrumentation to overview.rs**

Wrap each pipeline stage in `std::time::Instant`:

```rust
use std::time::Instant;

// In run():
let t0 = Instant::now();

// After scan:
let t_scan = t0.elapsed();
let t1 = Instant::now();

// After parse:
let t_parse = t1.elapsed();
// ... etc.

if timing {
    let file_count = index.total_files;
    eprintln!("[timing] scan:    {:>4}ms ({} files)", t_scan.as_millis(), file_count);
    eprintln!("[timing] parse:   {:>4}ms", t_parse.as_millis());
    eprintln!("[timing] index:   {:>4}ms", t_index.as_millis());
    eprintln!("[timing] budget:  {:>4}ms", t_budget.as_millis());
    eprintln!("[timing] output:  {:>4}ms", t_output.as_millis());
    eprintln!("[timing] total:   {:>4}ms", t0.elapsed().as_millis());
}
```

**Step 3: Write test for --timing flag**

```rust
#[test]
fn test_overview_timing() {
    let repo = make_test_repo();
    Command::cargo_bin("cxpak").unwrap()
        .args(["overview", "--tokens", "10k", "--timing", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("[timing]"));
}
```

**Step 4: Run tests**

Run: `cargo test --test-threads=1`

**Step 5: Commit**

```bash
git add src/cli/mod.rs src/commands/ src/main.rs tests/
git commit -m "feat: add --timing flag for pipeline performance instrumentation"
```

---

### Task 16: Add rayon for parallel parsing

**Files:**
- Modify: `Cargo.toml` (add rayon dependency)
- Modify: `src/cache/parse.rs` (parallelize parsing loop)

**Step 1: Add rayon dependency**

In `Cargo.toml` under `[dependencies]`:
```toml
rayon = "1"
```

**Step 2: Write a test that parsing produces correct results (before change)**

Run existing tests to establish baseline: `cargo test --test-threads=1`

**Step 3: Modify `parse_with_cache` to use `par_iter()`**

In `src/cache/parse.rs`, change the file parsing loop from sequential to parallel:

```rust
use rayon::prelude::*;

// Change: files.iter().map(...).collect()
// To: files.par_iter().map(...).collect()
```

Key considerations:
- Each thread creates its own `Parser` instance (tree-sitter parsers are not `Send`)
- Cache lookups happen before parsing — cached files skip parsing entirely
- The `TokenCounter` is thread-safe (read-only after initialization)

**Step 4: Run full test suite to verify no regressions**

Run: `cargo test --test-threads=1`

**Step 5: Verify with --timing on a real repo**

Run: `cargo run -- overview --tokens 50k --timing .`

**Step 6: Commit**

```bash
git add Cargo.toml src/cache/parse.rs
git commit -m "perf: parallelize file parsing with rayon for ~2-4x speedup on large repos"
```

---

### Task 17: Final coverage pass and cleanup

**Files:**
- Various

**Step 1: Run final coverage report**

Run: `cargo tarpaulin --version 0.31.4 --out Html --output-dir coverage/`

**Step 2: Add any remaining tests needed to reach 95%+**

Target the specific lines shown as uncovered.

**Step 3: Run full test suite + clippy + fmt**

Run: `cargo test --test-threads=1 && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`

**Step 4: Commit**

```bash
git add -A
git commit -m "test: final coverage pass — all modules at 95%+"
```

---

## Release: v0.6.0

### Task 18: Version bump and release

**Files:**
- Modify: `Cargo.toml` — version `"0.6.0"`
- Modify: `plugin/.claude-plugin/plugin.json` — version `"0.6.0"`
- Modify: `.claude-plugin/marketplace.json` — version `"0.6.0"`

**Step 1: Update versions**

**Step 2: Run full test suite one final time**

Run: `cargo test --test-threads=1`

**Step 3: Commit and tag**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json
git commit -m "release: v0.6.0 — smart context, parallel parsing, 95%+ coverage"
git tag v0.6.0
git push origin main --tags
```

Homebrew tap auto-updates via CI. Plugin versions are manual (updated above).

---

## Summary

| Task | Workstream | Target |
|------|-----------|--------|
| 1-7 | Coverage | Language parsers 57-68% → 95%+ |
| 8 | Coverage | Index module 63-77% → 95%+ |
| 9 | Coverage | Git module 84% → 95%+ |
| 10 | Coverage | CLI integration tests |
| 11 | Coverage | Mop-up remaining gaps |
| 12 | Smart Context | ranking.rs module |
| 13 | Smart Context | Budget integration |
| 14 | Smart Context | --focus flag |
| 15 | Speed | --timing flag |
| 16 | Speed | Rayon parallel parsing |
| 17 | Quality | Final coverage pass |
| 18 | Release | v0.6.0 tag |
