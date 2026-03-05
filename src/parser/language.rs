use tree_sitter::Language as TsLanguage;

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub signature: String,
    pub body: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Method,
    Constant,
    TypeAlias,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub source: String,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub name: String,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone)]
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
