// Progressive degradation for context quality

use crate::parser::language::Symbol;

pub const MAX_SYMBOL_TOKENS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetailLevel {
    Full = 0,
    Trimmed = 1,
    Documented = 2,
    Signature = 3,
    Stub = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileRole {
    Selected,
    Dependency,
}

#[derive(Debug, Clone)]
pub struct DegradedSymbol {
    pub symbol: Symbol,
    pub level: DetailLevel,
    pub rendered: String,
    pub rendered_tokens: usize,
    pub chunk_index: Option<usize>,
    pub chunk_total: Option<usize>,
    pub parent_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detail_level_ordering() {
        assert!(DetailLevel::Full < DetailLevel::Trimmed);
        assert!(DetailLevel::Trimmed < DetailLevel::Documented);
        assert!(DetailLevel::Documented < DetailLevel::Signature);
        assert!(DetailLevel::Signature < DetailLevel::Stub);
    }

    #[test]
    fn test_detail_level_equality() {
        assert_eq!(DetailLevel::Full, DetailLevel::Full);
        assert_ne!(DetailLevel::Full, DetailLevel::Stub);
    }

    #[test]
    fn test_file_role_variants() {
        let selected = FileRole::Selected;
        let dep = FileRole::Dependency;
        assert_ne!(selected, dep);
    }
}
