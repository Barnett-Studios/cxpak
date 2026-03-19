pub mod language;
pub mod languages;

use language::LanguageSupport;
use std::collections::HashMap;

pub struct LanguageRegistry {
    languages: HashMap<String, Box<dyn LanguageSupport>>,
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            languages: HashMap::new(),
        };
        registry.register_defaults();
        registry
    }

    fn register_defaults(&mut self) {
        #[cfg(feature = "lang-rust")]
        self.register(Box::new(languages::rust::RustLanguage));

        #[cfg(feature = "lang-typescript")]
        self.register(Box::new(languages::typescript::TypeScriptLanguage));

        #[cfg(feature = "lang-javascript")]
        self.register(Box::new(languages::javascript::JavaScriptLanguage));

        #[cfg(feature = "lang-python")]
        self.register(Box::new(languages::python::PythonLanguage));

        #[cfg(feature = "lang-java")]
        self.register(Box::new(languages::java::JavaLanguage));

        #[cfg(feature = "lang-go")]
        self.register(Box::new(languages::go::GoLanguage));

        #[cfg(feature = "lang-c")]
        self.register(Box::new(languages::c::CLanguage));

        #[cfg(feature = "lang-cpp")]
        self.register(Box::new(languages::cpp::CppLanguage));

        #[cfg(feature = "lang-ruby")]
        self.register(Box::new(languages::ruby::RubyLanguage));

        #[cfg(feature = "lang-csharp")]
        self.register(Box::new(languages::csharp::CSharpLanguage));

        #[cfg(feature = "lang-swift")]
        self.register(Box::new(languages::swift::SwiftLanguage));

        #[cfg(feature = "lang-kotlin")]
        self.register(Box::new(languages::kotlin::KotlinLanguage));

        // Tier 1 — new programming languages
        #[cfg(feature = "lang-bash")]
        self.register(Box::new(languages::bash::BashLanguage));

        #[cfg(feature = "lang-php")]
        self.register(Box::new(languages::php::PhpLanguage));

        #[cfg(feature = "lang-dart")]
        self.register(Box::new(languages::dart::DartLanguage));

        #[cfg(feature = "lang-scala")]
        self.register(Box::new(languages::scala::ScalaLanguage));

        #[cfg(feature = "lang-lua")]
        self.register(Box::new(languages::lua::LuaLanguage));

        #[cfg(feature = "lang-elixir")]
        self.register(Box::new(languages::elixir::ElixirLanguage));

        #[cfg(feature = "lang-zig")]
        self.register(Box::new(languages::zig::ZigLanguage));

        #[cfg(feature = "lang-haskell")]
        self.register(Box::new(languages::haskell::HaskellLanguage));

        #[cfg(feature = "lang-groovy")]
        self.register(Box::new(languages::groovy::GroovyLanguage));

        #[cfg(feature = "lang-objc")]
        self.register(Box::new(languages::objc::ObjcLanguage));

        #[cfg(feature = "lang-r")]
        self.register(Box::new(languages::r::RLanguage));

        #[cfg(feature = "lang-julia")]
        self.register(Box::new(languages::julia::JuliaLanguage));

        #[cfg(feature = "lang-ocaml")]
        {
            self.register(Box::new(languages::ocaml::OcamlLanguage));
            self.register(Box::new(languages::ocaml::OcamlInterfaceLanguage));
        }

        #[cfg(feature = "lang-matlab")]
        self.register(Box::new(languages::matlab::MatlabLanguage));

        // Tier 2 — structural/config languages
        #[cfg(feature = "lang-css")]
        self.register(Box::new(languages::css::CssLanguage));

        #[cfg(feature = "lang-scss")]
        self.register(Box::new(languages::scss::ScssLanguage));

        #[cfg(feature = "lang-markdown")]
        self.register(Box::new(languages::markdown::MarkdownLanguage));

        #[cfg(feature = "lang-json")]
        self.register(Box::new(languages::json_lang::JsonLangLanguage));

        #[cfg(feature = "lang-yaml")]
        self.register(Box::new(languages::yaml::YamlLanguage));

        #[cfg(feature = "lang-toml")]
        self.register(Box::new(languages::toml_lang::TomlLangLanguage));

        #[cfg(feature = "lang-dockerfile")]
        self.register(Box::new(languages::dockerfile::DockerfileLanguage));

        #[cfg(feature = "lang-hcl")]
        self.register(Box::new(languages::hcl::HclLanguage));

        #[cfg(feature = "lang-proto")]
        self.register(Box::new(languages::proto::ProtoLanguage));

        #[cfg(feature = "lang-svelte")]
        self.register(Box::new(languages::svelte::SvelteLanguage));

        #[cfg(feature = "lang-makefile")]
        self.register(Box::new(languages::makefile::MakefileLanguage));

        #[cfg(feature = "lang-html")]
        self.register(Box::new(languages::html::HtmlLanguage));

        #[cfg(feature = "lang-graphql")]
        self.register(Box::new(languages::graphql::GraphqlLanguage));

        #[cfg(feature = "lang-xml")]
        self.register(Box::new(languages::xml::XmlLanguage));
    }

    pub fn register(&mut self, lang: Box<dyn LanguageSupport>) {
        self.languages.insert(lang.name().to_string(), lang);
    }

    pub fn get(&self, name: &str) -> Option<&dyn LanguageSupport> {
        self.languages.get(name).map(|l| l.as_ref())
    }

    pub fn supported_languages(&self) -> Vec<&str> {
        self.languages.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_creates_registry() {
        let registry = LanguageRegistry::default();
        assert!(registry.get("rust").is_some());
    }

    #[test]
    fn test_supported_languages_returns_all() {
        let registry = LanguageRegistry::new();
        let langs = registry.supported_languages();
        assert!(
            langs.len() >= 40,
            "expected at least 40 languages, got {}",
            langs.len()
        );
        assert!(langs.contains(&"rust"));
        assert!(langs.contains(&"python"));
    }

    #[test]
    fn test_ts_language_all_registered() {
        // Exercises ts_language() on every registered language, which is otherwise
        // uncovered because unit tests use make_parser() directly.
        let registry = LanguageRegistry::new();
        for lang_name in registry.supported_languages() {
            let lang = registry
                .get(lang_name)
                .expect("registered language missing");
            let ts_lang = lang.ts_language();
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&ts_lang)
                .unwrap_or_else(|e| panic!("failed to set language for {}: {}", lang_name, e));
        }
    }

    #[test]
    fn test_get_nonexistent_language() {
        let registry = LanguageRegistry::new();
        assert!(registry.get("brainfuck").is_none());
    }
}
