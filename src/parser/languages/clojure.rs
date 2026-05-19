use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ClojureLanguage;

impl ClojureLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Collect the direct named children of a node, skipping anonymous
    /// punctuation nodes (`(`, `)`, `[`, `]`, etc.).
    fn named_children<'a>(node: &tree_sitter::Node<'a>) -> Vec<tree_sitter::Node<'a>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|c| c.is_named())
            .collect()
    }

    /// Return the text of the first named child of a list_lit.
    /// This is the "head" of the form — e.g. "defn", "ns", "def".
    fn list_head_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        let children = Self::named_children(node);
        children
            .first()
            .map(|c| Self::node_text(c, source).to_string())
            .unwrap_or_default()
    }

    /// Return the second named child of a list_lit — usually the definition name.
    ///
    /// In tree-sitter-clojure, `^metadata` annotations are stored as `meta_lit`
    /// nodes attached to the `sym_lit` via a "meta" field, where `meta_lit.value`
    /// holds the metadata form (e.g. `kwd_lit(":private")`).  The sym_lit's own
    /// text therefore includes the annotation prefix (`"^:private secret"`), so
    /// we must extract the `sym_name` sub-child for the bare name, and inspect
    /// the `meta_lit` children to detect `:private`.
    fn list_name_text(node: &tree_sitter::Node, source: &[u8]) -> (String, bool) {
        let children = Self::named_children(node);
        let name_node = match children.get(1) {
            Some(n) => n,
            None => return (String::new(), false),
        };

        let sub = Self::named_children(name_node);

        // The bare symbol name is carried by the sym_name child.
        let actual_name = sub
            .iter()
            .find(|c| c.kind() == "sym_name")
            .map(|c| Self::node_text(c, source).to_string())
            .unwrap_or_else(|| Self::node_text(name_node, source).to_string());

        // Each ^annotation is a meta_lit whose `value` field holds the metadata
        // form (kwd_lit ":private" or map_lit containing :private).
        let is_private = sub.iter().any(|c| {
            if c.kind() == "meta_lit" {
                if let Some(val) = c.child_by_field_name("value") {
                    let text = Self::node_text(&val, source);
                    text == ":private" || (val.kind() == "map_lit" && text.contains(":private"))
                } else {
                    false
                }
            } else {
                false
            }
        });

        (actual_name, is_private)
    }

    /// Extract `:require` imports from a namespace form's body.
    ///
    /// Handles the two main require styles:
    ///   `[clojure.string :as str]`   — vector with optional alias
    ///   `clojure.string`             — bare symbol
    fn extract_ns_imports(ns_node: &tree_sitter::Node, source: &[u8]) -> Vec<Import> {
        let mut imports = Vec::new();
        let children = Self::named_children(ns_node);

        // Each (:require ...) clause is a list_lit inside the ns form.
        for child in children.iter().skip(2) {
            if child.kind() != "list_lit" {
                continue;
            }
            let clause_children = Self::named_children(child);
            let head = clause_children
                .first()
                .map(|c| Self::node_text(c, source))
                .unwrap_or("");
            if head != ":require" && head != ":use" {
                continue;
            }

            for require_entry in clause_children.iter().skip(1) {
                match require_entry.kind() {
                    "vec_lit" => {
                        // [some.ns :as alias] or [some.ns :refer [f1 f2]]
                        let vec_children = Self::named_children(require_entry);
                        if let Some(ns_sym) = vec_children.first() {
                            let ns_name = Self::node_text(ns_sym, source).to_string();
                            if !ns_name.is_empty() {
                                let short =
                                    ns_name.rsplit('.').next().unwrap_or(&ns_name).to_string();
                                imports.push(Import {
                                    source: ns_name,
                                    names: vec![short],
                                });
                            }
                        }
                    }
                    "sym_lit" => {
                        // bare require: clojure.string
                        let ns_name = Self::node_text(require_entry, source).to_string();
                        if !ns_name.is_empty() && !ns_name.starts_with(':') {
                            let short = ns_name.rsplit('.').next().unwrap_or(&ns_name).to_string();
                            imports.push(Import {
                                source: ns_name,
                                names: vec![short],
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        imports
    }

    /// Walk one top-level form and push extracted symbols/imports/exports.
    fn process_top_level(
        node: &tree_sitter::Node,
        source: &[u8],
        ns: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        exports: &mut Vec<Export>,
    ) {
        if node.kind() != "list_lit" {
            return;
        }

        let head = Self::list_head_text(node, source);

        match head.as_str() {
            "ns" => {
                // (ns some.namespace (:require ...) ...)
                let (ns_name, _) = Self::list_name_text(node, source);
                if !ns_name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    exports.push(Export {
                        name: ns_name.clone(),
                        kind: SymbolKind::Class,
                    });
                    symbols.push(Symbol {
                        name: ns_name,
                        kind: SymbolKind::Class,
                        visibility: Visibility::Public,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
                let ns_imports = Self::extract_ns_imports(node, source);
                imports.extend(ns_imports);
            }

            "defn" => {
                let (name, is_meta_private) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let visibility = if is_meta_private {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    if visibility == Visibility::Public {
                        exports.push(Export {
                            name: qualified.clone(),
                            kind: SymbolKind::Function,
                        });
                    }
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Function,
                        visibility,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "defn-" => {
                // Explicitly private function (defn- is Clojure's private defn shorthand).
                let (name, _) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "defmacro" => {
                let (name, is_meta_private) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let visibility = if is_meta_private {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    if visibility == Visibility::Public {
                        exports.push(Export {
                            name: qualified.clone(),
                            kind: SymbolKind::Macro,
                        });
                    }
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Macro,
                        visibility,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "def" => {
                let (name, is_meta_private) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let visibility = if is_meta_private {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    if visibility == Visibility::Public {
                        exports.push(Export {
                            name: qualified.clone(),
                            kind: SymbolKind::Constant,
                        });
                    }
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Constant,
                        visibility,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "defprotocol" => {
                let (name, _) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    exports.push(Export {
                        name: qualified.clone(),
                        kind: SymbolKind::Interface,
                    });
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Interface,
                        visibility: Visibility::Public,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "defrecord" | "deftype" => {
                let (name, _) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    exports.push(Export {
                        name: qualified.clone(),
                        kind: SymbolKind::Struct,
                    });
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "defmulti" => {
                let (name, _) = Self::list_name_text(node, source);
                if !name.is_empty() {
                    let sig = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    let qualified = if ns.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", ns, name)
                    };
                    exports.push(Export {
                        name: qualified.clone(),
                        kind: SymbolKind::Function,
                    });
                    symbols.push(Symbol {
                        name: qualified,
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: sig,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            _ => {}
        }
    }
}

impl LanguageSupport for ClojureLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_clojure::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "clojure"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // First pass: find the namespace name so we can qualify symbols.
        let mut ns = String::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "list_lit"
                && ClojureLanguage::list_head_text(&child, source_bytes) == "ns"
            {
                let (ns_name, _) = ClojureLanguage::list_name_text(&child, source_bytes);
                ns = ns_name;
                break;
            }
        }

        // Second pass: extract all top-level definitions.
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            Self::process_top_level(
                &child,
                source_bytes,
                &ns,
                &mut symbols,
                &mut imports,
                &mut exports,
            );
        }

        ParseResult {
            symbols,
            imports,
            exports,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_clojure::LANGUAGE.into())
            .expect("failed to set Clojure language");
        parser
    }

    fn parse_and_extract(source: &str) -> ParseResult {
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        ClojureLanguage.extract(source, &tree)
    }

    #[test]
    fn test_empty_source() {
        let result = parse_and_extract("");
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_ns_extracted_as_namespace_symbol() {
        let src = "(ns my.app.core)\n";
        let result = parse_and_extract(src);
        let ns_syms: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "my.app.core")
            .collect();
        assert!(!ns_syms.is_empty(), "expected ns as Class symbol");
        assert_eq!(ns_syms[0].visibility, Visibility::Public);
        assert!(
            result.exports.iter().any(|e| e.name == "my.app.core"),
            "ns should be exported"
        );
    }

    #[test]
    fn test_public_defn_extracted() {
        let src = "(ns my.ns)\n(defn greet [name] (str \"Hello, \" name))\n";
        let result = parse_and_extract(src);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "my.ns/greet")
            .collect();
        assert!(!funcs.is_empty(), "expected public defn 'greet'");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(
            result.exports.iter().any(|e| e.name == "my.ns/greet"),
            "public defn should be exported"
        );
    }

    #[test]
    fn test_private_defn_minus_not_exported() {
        let src = "(ns my.ns)\n(defn- helper [x] (* x 2))\n";
        let result = parse_and_extract(src);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "my.ns/helper")
            .collect();
        assert!(!funcs.is_empty(), "expected private defn- 'helper'");
        assert_eq!(funcs[0].visibility, Visibility::Private);
        assert!(
            !result.exports.iter().any(|e| e.name == "my.ns/helper"),
            "defn- should not be exported"
        );
    }

    #[test]
    fn test_defmacro_extracted_with_macro_kind() {
        let src = "(ns my.ns)\n(defmacro when-pos [x & body] `(when (pos? ~x) ~@body))\n";
        let result = parse_and_extract(src);
        let macros: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Macro && s.name == "my.ns/when-pos")
            .collect();
        assert!(!macros.is_empty(), "expected defmacro as Macro symbol");
        assert_eq!(macros[0].visibility, Visibility::Public);
        assert!(
            result.exports.iter().any(|e| e.name == "my.ns/when-pos"),
            "public defmacro should be exported"
        );
    }

    #[test]
    fn test_def_var_extracted() {
        let src = "(ns my.ns)\n(def max-retries 3)\n";
        let result = parse_and_extract(src);
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant && s.name == "my.ns/max-retries")
            .collect();
        assert!(!vars.is_empty(), "expected def as Constant symbol");
        assert_eq!(vars[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_defprotocol_extracted_as_interface() {
        let src = "(ns my.ns)\n(defprotocol Store\n  (get-val [this k])\n  (put-val [this k v]))\n";
        let result = parse_and_extract(src);
        let protos: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface && s.name == "my.ns/Store")
            .collect();
        assert!(!protos.is_empty(), "expected defprotocol as Interface");
    }

    #[test]
    fn test_defrecord_extracted_as_struct() {
        let src = "(ns my.ns)\n(defrecord Point [x y])\n";
        let result = parse_and_extract(src);
        let recs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct && s.name == "my.ns/Point")
            .collect();
        assert!(!recs.is_empty(), "expected defrecord as Struct");
    }

    #[test]
    fn test_defmulti_extracted() {
        let src = "(ns my.ns)\n(defmulti area :shape)\n";
        let result = parse_and_extract(src);
        let multis: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "my.ns/area")
            .collect();
        assert!(!multis.is_empty(), "expected defmulti as Function");
    }

    #[test]
    fn test_require_imports_extracted() {
        let src = "(ns my.ns\n  (:require [clojure.string :as str]\n            [clojure.set :as set]))\n";
        let result = parse_and_extract(src);
        assert!(
            result.imports.iter().any(|i| i.source == "clojure.string"),
            "expected clojure.string import"
        );
        assert!(
            result.imports.iter().any(|i| i.source == "clojure.set"),
            "expected clojure.set import"
        );
    }

    #[test]
    fn test_bare_require_extracted() {
        let src = "(ns my.ns\n  (:require clojure.string))\n";
        let result = parse_and_extract(src);
        assert!(
            result.imports.iter().any(|i| i.source == "clojure.string"),
            "expected bare require import"
        );
    }

    #[test]
    fn test_use_clause_extracted() {
        let src = "(ns my.ns\n  (:use [clojure.string :only [join]]))\n";
        let result = parse_and_extract(src);
        assert!(
            result.imports.iter().any(|i| i.source == "clojure.string"),
            "expected :use import"
        );
    }

    #[test]
    fn test_private_def_via_metadata() {
        let src = "(ns my.ns)\n(def ^:private secret 42)\n";
        let result = parse_and_extract(src);
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "my.ns/secret")
            .collect();
        assert!(!vars.is_empty(), "expected ^:private def 'secret'");
        assert_eq!(vars[0].visibility, Visibility::Private);
        assert!(
            !result.exports.iter().any(|e| e.name == "my.ns/secret"),
            "^:private def should not be exported"
        );
    }

    #[test]
    fn test_symbol_line_numbers() {
        let src = "(ns my.ns)\n\n(defn foo []\n  :ok)\n";
        let result = parse_and_extract(src);
        let foo: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "my.ns/foo")
            .collect();
        assert!(!foo.is_empty(), "expected defn 'foo'");
        assert_eq!(foo[0].start_line, 3, "defn should start at line 3");
    }

    #[test]
    fn test_no_ns_symbols_unqualified() {
        let src = "(defn standalone [] :ok)\n";
        let result = parse_and_extract(src);
        // Without ns, name should not have a / prefix
        assert!(
            result.symbols.iter().any(|s| s.name == "standalone"),
            "unqualified defn should have plain name"
        );
    }

    #[test]
    fn test_multiple_definitions() {
        let src = r#"
(ns my.app
  (:require [clojure.string :as str]))

(def version "1.0.0")

(defn- build-url [path]
  (str "https://example.com" path))

(defn fetch [endpoint]
  (build-url endpoint))

(defprotocol Repository
  (find-by-id [this id])
  (save [this entity]))

(defrecord InMemoryRepo [data]
  Repository
  (find-by-id [_ id] (get @data id))
  (save [_ entity] (swap! data assoc (:id entity) entity)))

(defmacro with-retry [n & body]
  `(dotimes [~'_ ~n] ~@body))
"#;
        let result = parse_and_extract(src);

        assert!(result.symbols.iter().any(|s| s.kind == SymbolKind::Class));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Constant && s.name == "my.app/version"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Function && s.visibility == Visibility::Private));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Function && s.name == "my.app/fetch"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Interface));
        assert!(result.symbols.iter().any(|s| s.kind == SymbolKind::Struct));
        assert!(result.symbols.iter().any(|s| s.kind == SymbolKind::Macro));
        assert!(result.imports.iter().any(|i| i.source == "clojure.string"));
    }
}
