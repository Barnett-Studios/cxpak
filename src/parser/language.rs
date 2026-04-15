use serde::{Deserialize, Serialize};
use tree_sitter::Language as TsLanguage;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub signature: String,
    pub body: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SymbolKind {
    // Existing
    Function,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Method,
    Constant,
    TypeAlias,
    // Tier 2 structural units
    Selector,
    Mixin,
    Variable,
    Heading,
    Section,
    Key,
    Table,
    Block,
    Target,
    Rule,
    Element,
    Message,
    Service,
    Query,
    Mutation,
    Type,
    Instruction,
    Macro,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Import {
    pub source: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

pub trait LanguageSupport: Send + Sync {
    fn ts_language(&self) -> TsLanguage;
    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult;
    fn name(&self) -> &str;
}
