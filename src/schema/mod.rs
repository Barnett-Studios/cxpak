pub mod detect;
pub mod extract;
pub mod link;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    Import,
    ForeignKey,
    ViewReference,
    TriggerTarget,
    IndexTarget,
    FunctionReference,
    EmbeddedSql,
    OrmModel,
    MigrationSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target: String,
    pub edge_type: EdgeType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaIndex {
    pub tables: HashMap<String, TableSchema>,
    pub views: HashMap<String, ViewSchema>,
    pub functions: HashMap<String, DbFunctionSchema>,
    pub orm_models: HashMap<String, OrmModelSchema>,
    pub migrations: Vec<MigrationChain>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    pub primary_key: Option<Vec<String>>,
    pub indexes: Vec<IndexSchema>,
    pub file_path: String,
    pub start_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub constraints: Vec<String>,
    pub foreign_key: Option<ForeignKeyRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyRef {
    pub target_table: String,
    pub target_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSchema {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSchema {
    pub name: String,
    pub source_tables: Vec<String>,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFunctionSchema {
    pub name: String,
    pub referenced_tables: Vec<String>,
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrmModelSchema {
    pub class_name: String,
    pub table_name: String,
    pub framework: OrmFramework,
    pub file_path: String,
    pub fields: Vec<OrmFieldSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrmFieldSchema {
    pub name: String,
    pub field_type: String,
    pub is_relation: bool,
    pub related_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrmFramework {
    Django,
    SqlAlchemy,
    TypeOrm,
    ActiveRecord,
    Prisma,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationChain {
    pub framework: MigrationFramework,
    pub directory: String,
    pub migrations: Vec<MigrationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationEntry {
    pub file_path: String,
    pub sequence: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MigrationFramework {
    Rails,
    Alembic,
    Flyway,
    Django,
    Knex,
    Prisma,
    Drizzle,
    Generic,
}

impl SchemaIndex {
    pub fn empty() -> Self {
        Self {
            tables: HashMap::new(),
            views: HashMap::new(),
            functions: HashMap::new(),
            orm_models: HashMap::new(),
            migrations: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
            && self.views.is_empty()
            && self.functions.is_empty()
            && self.orm_models.is_empty()
            && self.migrations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_type_equality() {
        assert_eq!(EdgeType::Import, EdgeType::Import);
        assert_ne!(EdgeType::Import, EdgeType::ForeignKey);
    }

    #[test]
    fn test_typed_edge_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::Import,
        });
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::ForeignKey,
        });
        assert_eq!(
            set.len(),
            2,
            "same target, different types = different edges"
        );
    }

    #[test]
    fn test_schema_index_empty() {
        let idx = SchemaIndex::empty();
        assert!(idx.is_empty());
    }

    #[test]
    fn test_schema_index_not_empty() {
        let mut idx = SchemaIndex::empty();
        idx.tables.insert(
            "users".into(),
            TableSchema {
                name: "users".into(),
                columns: vec![],
                primary_key: None,
                indexes: vec![],
                file_path: "schema.sql".into(),
                start_line: 1,
            },
        );
        assert!(!idx.is_empty());
    }
}
