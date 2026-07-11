pub mod detect;
pub mod extract;
pub mod introspect;
pub mod link;

// Graph + schema data types live in `core_graph` (cxpak 3.0.0 Phase 0
// de-cycle). Re-exported at the historical `crate::schema::{...}` paths so
// every existing reference (detect/extract/link, intelligence, conventions,
// commands, tests) keeps resolving unchanged.
pub use crate::core_graph::graph::{EdgeConfidence, EdgeType, TypedEdge};
pub use crate::core_graph::schema::{
    column_node_id, ColumnSchema, DbFunctionSchema, ForeignKeyRef, IndexSchema, MigrationChain,
    MigrationEntry, MigrationFramework, OrmFieldSchema, OrmFramework, OrmModelSchema, SchemaIndex,
    TableSchema, ViewSchema,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_type_equality() {
        // EdgeType is re-exported from core_graph; verify the re-export works.
        assert_eq!(EdgeType::Import, EdgeType::Import);
        assert_ne!(EdgeType::Import, EdgeType::ForeignKey);
    }

    #[test]
    fn test_typed_edge_hash() {
        // TypedEdge is re-exported from core_graph; verify the re-export works.
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::Import,
            confidence: EdgeConfidence::Extracted,
        });
        set.insert(TypedEdge {
            target: "a.rs".into(),
            edge_type: EdgeType::ForeignKey,
            confidence: EdgeConfidence::Extracted,
        });
        assert_eq!(
            set.len(),
            2,
            "same target, different types = different edges"
        );
    }

    #[test]
    fn test_schema_index_reexport() {
        // SchemaIndex is re-exported from core_graph; verify the re-export works.
        let idx = SchemaIndex::empty();
        assert!(idx.is_empty());
    }
}
