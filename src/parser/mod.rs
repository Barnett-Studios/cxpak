pub mod language;
pub mod languages;

use language::LanguageSupport;
use std::collections::HashMap;

pub struct LanguageRegistry {
    languages: HashMap<String, Box<dyn LanguageSupport>>,
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
