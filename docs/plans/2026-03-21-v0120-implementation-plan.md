# v0.12.0 Implementation Plan: Data Layer Awareness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make cxpak understand the data layer — SQL schemas with column-level detail, ORM models, embedded SQL in application code, migration ordering, and infrastructure database definitions — all through typed dependency graph edges.

**Architecture:** New `src/schema/` module with four files (types, extraction, detection, linking). The `DependencyGraph` in `src/index/graph.rs` gains typed edges (full migration of all consumers). Schema module runs post-parse, builds a `SchemaIndex` stored on `CodebaseIndex`, and produces typed edges that feed into the unified graph. Builds on v0.11.0's `context_quality` module for annotations and expansion integration.

**Tech Stack:** Rust, tree-sitter (existing SQL parser), regex (column extraction + embedded SQL detection + Cypher), serde (schema serialization)

**Spec:** `docs/superpowers/specs/2026-03-21-v0120-design.md`

---

## File Structure

### New Files
- `src/schema/mod.rs` — public types (`SchemaIndex`, `TableSchema`, `ColumnSchema`, `ForeignKeyRef`, `IndexSchema`, `ViewSchema`, `DbFunctionSchema`, `OrmModelSchema`, `OrmFieldSchema`, `OrmFramework`, `MigrationChain`, `MigrationEntry`, `MigrationFramework`, `EdgeType`, `TypedEdge`)
- `src/schema/extract.rs` — SQL column extraction, CQL, Cypher regex, Elasticsearch JSON pattern
- `src/schema/detect.rs` — ORM pattern matchers (Django, SQLAlchemy, TypeORM, ActiveRecord, Prisma), Terraform tagging, migration detection
- `src/schema/link.rs` — embedded SQL detection, ORM→table linking, `build_schema_edges()`

### Modified Files
- `src/main.rs` — add `pub mod schema;`
- `src/index/graph.rs` — `DependencyGraph` edges → `HashSet<TypedEdge>`, all API methods updated, `build_dependency_graph()` gains optional `&SchemaIndex` parameter
- `src/index/mod.rs` — add `pub schema: Option<SchemaIndex>` to `CodebaseIndex`, populate during build
- `src/index/ranking.rs` — extract `.target` from `TypedEdge`
- `src/relevance/seed.rs` — extract `.target` from `TypedEdge`
- `src/commands/trace.rs` — extract `.target`, display edge types
- `src/commands/diff.rs` — extract `.target`
- `src/commands/overview.rs` — new: display edge types in dependency graph section
- `src/commands/serve.rs` — extract `.target` in MCP handlers, add schema annotation to pack_context

---

## Stream 1: Typed Dependency Graph

### Task 1: Define `EdgeType` and `TypedEdge` in schema module

**Files:**
- Create: `src/schema/mod.rs`
- Create: `src/schema/extract.rs` (empty scaffold)
- Create: `src/schema/detect.rs` (empty scaffold)
- Create: `src/schema/link.rs` (empty scaffold)
- Modify: `src/main.rs`

- [ ] **Step 1: Create module scaffold**

`src/schema/mod.rs`:
```rust
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
```

Empty scaffolds for `extract.rs`, `detect.rs`, `link.rs`.

- [ ] **Step 2: Add `pub mod schema;` to `src/main.rs`**

- [ ] **Step 3: Write tests for types**

```rust
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
        set.insert(TypedEdge { target: "a.rs".into(), edge_type: EdgeType::Import });
        set.insert(TypedEdge { target: "a.rs".into(), edge_type: EdgeType::ForeignKey });
        assert_eq!(set.len(), 2, "same target, different types = different edges");
    }

    #[test]
    fn test_schema_index_empty() {
        let idx = SchemaIndex::empty();
        assert!(idx.is_empty());
    }

    #[test]
    fn test_schema_index_not_empty() {
        let mut idx = SchemaIndex::empty();
        idx.tables.insert("users".into(), TableSchema {
            name: "users".into(),
            columns: vec![],
            primary_key: None,
            indexes: vec![],
            file_path: "schema.sql".into(),
            start_line: 1,
        });
        assert!(!idx.is_empty());
    }
}
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo test schema --verbose`

- [ ] **Step 5: Commit**

```bash
git add src/schema/ src/main.rs
git commit -m "feat: scaffold schema module with types for v0.12.0"
```

### Task 2: Migrate `DependencyGraph` to typed edges

**Files:**
- Modify: `src/index/graph.rs`

- [ ] **Step 1: Write failing tests for typed graph API**

```rust
#[test]
fn test_add_typed_edge() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs", EdgeType::Import);
    let deps = graph.dependencies("a.rs").unwrap();
    assert_eq!(deps.len(), 1);
    let edge = deps.iter().next().unwrap();
    assert_eq!(edge.target, "b.rs");
    assert_eq!(edge.edge_type, EdgeType::Import);
}

#[test]
fn test_multiple_edge_types_same_target() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs", EdgeType::Import);
    graph.add_edge("a.rs", "b.rs", EdgeType::EmbeddedSql);
    let deps = graph.dependencies("a.rs").unwrap();
    assert_eq!(deps.len(), 2, "same target with different types = 2 edges");
}

#[test]
fn test_dependents_returns_typed_edges() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs", EdgeType::ForeignKey);
    let deps = graph.dependents("b.rs");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].target, "a.rs");
    assert_eq!(deps[0].edge_type, EdgeType::ForeignKey);
}

#[test]
fn test_reachable_from_with_typed_edges() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs", EdgeType::Import);
    graph.add_edge("b.rs", "c.rs", EdgeType::ForeignKey);
    let reachable = graph.reachable_from(&["a.rs"]);
    assert!(reachable.contains("a.rs"));
    assert!(reachable.contains("b.rs"));
    assert!(reachable.contains("c.rs"));
}
```

- [ ] **Step 2: Rewrite `DependencyGraph` with `TypedEdge`**

```rust
use crate::schema::{EdgeType, TypedEdge};

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<TypedEdge>>,
    pub reverse_edges: HashMap<String, HashSet<TypedEdge>>,
}

impl DependencyGraph {
    pub fn new() -> Self { Self::default() }

    pub fn add_edge(&mut self, from: &str, to: &str, edge_type: EdgeType) {
        self.edges.entry(from.to_string()).or_default().insert(TypedEdge {
            target: to.to_string(),
            edge_type: edge_type.clone(),
        });
        self.reverse_edges.entry(to.to_string()).or_default().insert(TypedEdge {
            target: from.to_string(),
            edge_type,
        });
    }

    pub fn dependents(&self, path: &str) -> Vec<&TypedEdge> {
        self.reverse_edges.get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<TypedEdge>> {
        self.edges.get(path)
    }

    pub fn remove_edges_for(&mut self, source: &str) {
        if let Some(targets) = self.edges.remove(source) {
            for edge in &targets {
                if let Some(rev) = self.reverse_edges.get_mut(&edge.target) {
                    rev.retain(|e| e.target != source);
                    if rev.is_empty() {
                        self.reverse_edges.remove(&edge.target);
                    }
                }
            }
        }
    }

    pub fn reachable_from(&self, start_files: &[&str]) -> HashSet<String> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        for &path in start_files {
            if visited.insert(path.to_string()) {
                queue.push_back(path.to_string());
            }
        }
        while let Some(current) = queue.pop_front() {
            if let Some(deps) = self.edges.get(&current) {
                for edge in deps {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }
            if let Some(importers) = self.reverse_edges.get(&current) {
                for edge in importers {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }
        }
        visited
    }
}
```

- [ ] **Step 3: Update `build_dependency_graph()` to pass `EdgeType::Import`**

```rust
pub fn build_dependency_graph(index: &CodebaseIndex, schema: Option<&crate::schema::SchemaIndex>) -> DependencyGraph {
    // ... existing import resolution logic ...
    // Change: graph.add_edge(&file.relative_path, candidate);
    // To:     graph.add_edge(&file.relative_path, candidate, EdgeType::Import);

    // After import edges, add schema edges if present
    if let Some(schema_index) = schema {
        let schema_edges = crate::schema::link::build_schema_edges(index, schema_index);
        for (from, to, edge_type) in schema_edges {
            graph.add_edge(&from, &to, edge_type);
        }
    }

    graph
}
```

- [ ] **Step 4: Update all existing graph tests**

Every test that calls `add_edge("a", "b")` becomes `add_edge("a", "b", EdgeType::Import)`.
Every test that checks `deps.contains("b.rs")` becomes `deps.iter().any(|e| e.target == "b.rs")`.
Every test that checks `dependents()` returns `Vec<&str>` now checks `Vec<&TypedEdge>`.
`build_dependency_graph()` calls gain `None` as second argument (no schema).

**Direct field access tests also need migration:**
- `test_reverse_edges_maintained` (lines 251-254): `reverse_edges.get("b.rs").unwrap().contains("a.rs")` → `.iter().any(|e| e.target == "a.rs")` — `HashSet<TypedEdge>` does not implement `Contains<&str>`.
- `test_remove_and_readd_edges` (line 295): `graph.edges["a.rs"].contains("d.rs")` → `.iter().any(|e| e.target == "d.rs")`.
- `test_remove_and_readd_edges` (line 298): `dependents("d.rs")` returns `Vec<&TypedEdge>` — assertion `vec!["a.rs"]` must become `.iter().any(|e| e.target == "a.rs")`.

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test index::graph --verbose`

- [ ] **Step 6: Commit**

```bash
git add src/index/graph.rs
git commit -m "feat: migrate DependencyGraph to typed edges with EdgeType"
```

### Task 3: Migrate all consumers to typed edges

**Files:**
- Modify: `src/index/ranking.rs`
- Modify: `src/relevance/seed.rs`
- Modify: `src/commands/trace.rs`
- Modify: `src/commands/diff.rs`
- Modify: `src/commands/overview.rs`
- Modify: `src/commands/serve.rs`

**IMPORTANT:** Six files call graph APIs. The most common patterns to fix:

| Old pattern | New pattern |
|---|---|
| `deps.iter().cloned()` (into `HashSet<String>`) | `deps.iter().map(\|e\| e.target.clone())` |
| `dep.to_string()` (from `dependents()`) | `dep.target.to_string()` |
| `d.iter().map(String::as_str).collect()` | `d.iter().map(\|e\| e.target.as_str()).collect()` |
| `seen.insert(dep.clone())` | `seen.insert(dep.target.clone())` |
| `build_dependency_graph(&index)` | `build_dependency_graph(&index, index.schema.as_ref())` |
| `add_edge("a", "b")` (in tests) | `add_edge("a", "b", EdgeType::Import)` |

- [ ] **Step 1: Update `src/index/ranking.rs`**

- `graph.dependencies(path).map(|d| d.len())` → `.len()` still works on `HashSet<TypedEdge>`, no change needed.
- `apply_focus()`: `deps.iter().cloned()` → `deps.iter().map(|e| e.target.clone())` (returns String for HashSet<String>).
- `dep.to_string()` (from dependents) → `dep.target.to_string()`.
- Update test `add_edge` calls to include `EdgeType::Import`.

- [ ] **Step 2: Update `src/relevance/seed.rs`**

```rust
// Change: neighbors.extend(deps.iter().cloned());
// To:
neighbors.extend(deps.iter().map(|e| e.target.clone()));

// Change: neighbors.push(dep.to_string());
// To:
neighbors.push(dep.target.to_string());
```

Update `build_dependency_graph(&index)` at line 61 to `build_dependency_graph(&index, None)`.
**Pass `None` for schema** — when seed.rs builds its own graph (fallback path), schema edges are not needed because the caller (serve.rs) uses the prebuilt graph that already includes schema edges.

Update test `add_edge` calls.

- [ ] **Step 3: Update `src/commands/trace.rs`**

- `deps.iter().cloned()` → `deps.iter().map(|e| e.target.clone())`
- `dep.to_string()` → `dep.target.to_string()`
- `build_dependency_graph(&index)` → `build_dependency_graph(&index, index.schema.as_ref())`

- [ ] **Step 4: Update `src/commands/diff.rs`**

- `deps.iter().cloned()` → `deps.iter().map(|e| e.target.clone())`
- `dep.to_string()` → `dep.target.to_string()`
- `build_dependency_graph(&index)` → `build_dependency_graph(&index, index.schema.as_ref())`

- [ ] **Step 5: Update `src/commands/overview.rs`**

- `build_dependency_graph(&index)` → `build_dependency_graph(&index, index.schema.as_ref())`
- Any graph iteration → extract `.target`

- [ ] **Step 6: Update `src/commands/serve.rs`**

- Line ~774: `d.iter().map(String::as_str).collect()` → `d.iter().map(|e| e.target.as_str()).collect()`
- Line ~860: `seen.insert(dep.clone())` → `seen.insert(dep.target.clone())`
- Line ~862: `target_files.push((dep.clone(), ...))` → `target_files.push((dep.target.clone(), ...))` — BOTH uses of `dep` in the pack_context loop must become `dep.target`
- `build_dependency_graph(index)` → `build_dependency_graph(index, index.schema.as_ref())`
- All other `deps.iter()` patterns → extract `.target`
- Test `test_mcp_pack_context_with_dependencies` exercises this code path — will compile correctly once the above fixes are applied

- [ ] **Step 7: Run full test suite**

Run: `cargo test --verbose`
Expected: ALL tests pass. Zero failures.

- [ ] **Step 8: Commit**

```bash
git add src/index/ranking.rs src/relevance/seed.rs src/commands/trace.rs src/commands/diff.rs src/commands/overview.rs src/commands/serve.rs
git commit -m "feat: migrate all graph consumers to typed edges"
```

### Task 4: Add `SchemaIndex` to `CodebaseIndex`

**Files:**
- Modify: `src/index/mod.rs`

- [ ] **Step 1: Add `schema` field to `CodebaseIndex`**

```rust
use crate::schema::SchemaIndex;

pub struct CodebaseIndex {
    // ... existing fields ...
    pub schema: Option<SchemaIndex>,
}
```

- [ ] **Step 2: Initialize as `None` in `build()` and `build_with_content()`**

Add `schema: None` to the `Self { ... }` struct literal in both constructors. This is a temporary state — Task 11 will restructure both constructors to `let mut index = Self { ... }; index.schema = build_schema_index(&index); index` pattern. For now, just add the field.

- [ ] **Step 3: Run tests, verify no regressions**

Run: `cargo test --verbose`

- [ ] **Step 4: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: add Optional SchemaIndex field to CodebaseIndex"
```

---

## Stream 2: Schema Extraction

### Task 5: SQL column-level extraction

**Files:**
- Modify: `src/schema/extract.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_columns_basic() {
        let body = "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  name TEXT NOT NULL,\n  email VARCHAR(255) UNIQUE\n);";
        let table = extract_table_schema(body, "users", "schema.sql", 1);
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[0].data_type, "INTEGER");
        assert!(table.columns[1].constraints.contains(&"NOT NULL".to_string()));
        assert!(table.columns[2].constraints.contains(&"UNIQUE".to_string()));
    }

    #[test]
    fn test_extract_primary_key() {
        let body = "CREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  name TEXT\n);";
        let table = extract_table_schema(body, "users", "schema.sql", 1);
        assert_eq!(table.primary_key, Some(vec!["id".to_string()]));
    }

    #[test]
    fn test_extract_foreign_key_inline() {
        let body = "CREATE TABLE orders (\n  id INTEGER PRIMARY KEY,\n  user_id INTEGER REFERENCES users(id)\n);";
        let table = extract_table_schema(body, "orders", "schema.sql", 1);
        let fk_col = table.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = fk_col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_extract_foreign_key_table_level() {
        let body = "CREATE TABLE orders (\n  id INTEGER,\n  user_id INTEGER,\n  FOREIGN KEY (user_id) REFERENCES users(id)\n);";
        let table = extract_table_schema(body, "orders", "schema.sql", 1);
        let fk_col = table.columns.iter().find(|c| c.name == "user_id").unwrap();
        assert!(fk_col.foreign_key.is_some());
    }

    #[test]
    fn test_extract_nullable_default() {
        let body = "CREATE TABLE t (\n  status VARCHAR(20) DEFAULT 'active',\n  deleted_at TIMESTAMP NULL\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].default, Some("'active'".to_string()));
        assert!(table.columns[1].nullable);
    }

    #[test]
    fn test_extract_not_null_is_not_nullable() {
        let body = "CREATE TABLE t (\n  name TEXT NOT NULL\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert!(!table.columns[0].nullable);
    }

    #[test]
    fn test_extract_postgresql_types() {
        let body = "CREATE TABLE t (\n  data JSONB,\n  tags TEXT[],\n  id SERIAL\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].data_type, "JSONB");
        assert_eq!(table.columns[1].data_type, "TEXT[]");
        assert_eq!(table.columns[2].data_type, "SERIAL");
    }

    #[test]
    fn test_extract_view_source_tables() {
        let body = "CREATE VIEW active_users AS SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id WHERE u.active = 1;";
        let view = extract_view_schema(body, "active_users", "views.sql");
        assert!(view.source_tables.contains(&"users".to_string()));
        assert!(view.source_tables.contains(&"orders".to_string()));
    }

    #[test]
    fn test_extract_function_references() {
        let body = "CREATE FUNCTION get_user(uid INT) RETURNS TEXT AS $$ SELECT name FROM users WHERE id = uid; $$ LANGUAGE sql;";
        let func = extract_function_schema(body, "get_user", "funcs.sql");
        assert!(func.referenced_tables.contains(&"users".to_string()));
    }

    #[test]
    fn test_extract_empty_table() {
        let body = "CREATE TABLE empty ();";
        let table = extract_table_schema(body, "empty", "schema.sql", 1);
        assert!(table.columns.is_empty());
    }

    #[test]
    fn test_extract_clickhouse_engine_skipped() {
        let body = "CREATE TABLE events (\n  id UInt64,\n  ts DateTime\n) ENGINE = MergeTree() ORDER BY ts;";
        let table = extract_table_schema(body, "events", "ch.sql", 1);
        assert_eq!(table.columns.len(), 2);
    }

    #[test]
    fn test_extract_table_level_primary_key() {
        let body = "CREATE TABLE t (\n  a INT,\n  b INT,\n  PRIMARY KEY (a, b)\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.primary_key, Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn test_extract_multiple_foreign_keys() {
        let body = "CREATE TABLE order_items (\n  order_id INT REFERENCES orders(id),\n  product_id INT REFERENCES products(id)\n);";
        let table = extract_table_schema(body, "order_items", "schema.sql", 1);
        assert!(table.columns[0].foreign_key.is_some());
        assert!(table.columns[1].foreign_key.is_some());
    }

    #[test]
    fn test_extract_mysql_auto_increment() {
        let body = "CREATE TABLE t (\n  id INT AUTO_INCREMENT PRIMARY KEY\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].data_type, "INT");
    }

    #[test]
    fn test_extract_multi_word_type() {
        let body = "CREATE TABLE t (\n  ts TIMESTAMP WITH TIME ZONE DEFAULT NOW(),\n  val DOUBLE PRECISION NOT NULL\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].data_type, "TIMESTAMP WITH TIME ZONE");
        assert_eq!(table.columns[1].data_type, "DOUBLE PRECISION");
    }

    #[test]
    fn test_extract_decimal_with_parens() {
        let body = "CREATE TABLE t (\n  price DECIMAL(10,2) NOT NULL,\n  name VARCHAR(255) UNIQUE\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].data_type, "DECIMAL(10,2)");
        assert_eq!(table.columns[1].data_type, "VARCHAR(255)");
    }

    #[test]
    fn test_extract_references_without_column() {
        let body = "CREATE TABLE orders (\n  user_id INT REFERENCES users\n);";
        let table = extract_table_schema(body, "orders", "schema.sql", 1);
        let fk = table.columns[0].foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert!(fk.target_column.is_empty(), "implicit PK reference has empty column");
    }

    #[test]
    fn test_extract_references_on_delete_cascade() {
        let body = "CREATE TABLE orders (\n  user_id INT REFERENCES users(id) ON DELETE CASCADE ON UPDATE SET NULL\n);";
        let table = extract_table_schema(body, "orders", "schema.sql", 1);
        let fk = table.columns[0].foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_extract_quoted_identifier() {
        let body = r#"CREATE TABLE t ("user id" INT, "created at" TIMESTAMP);"#;
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns[0].name, "user id");
        assert_eq!(table.columns[1].name, "created at");
    }

    #[test]
    fn test_extract_table_level_check_skipped() {
        let body = "CREATE TABLE t (\n  age INT,\n  CHECK (age >= 0 AND age <= 150)\n);";
        let table = extract_table_schema(body, "t", "t.sql", 1);
        assert_eq!(table.columns.len(), 1, "table-level CHECK should not be parsed as column");
        assert_eq!(table.columns[0].name, "age");
    }

    #[test]
    fn test_extract_schema_qualified_reference() {
        let body = "CREATE TABLE orders (\n  user_id INT REFERENCES public.users(id)\n);";
        let table = extract_table_schema(body, "orders", "schema.sql", 1);
        let fk = table.columns[0].foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "public.users");
    }
}
```

- [ ] **Step 2: Implement `extract_table_schema()`, `extract_view_schema()`, `extract_function_schema()`**

Regex-based extraction from `Symbol.body` text:
- Split column definitions by commas at top paren level (track paren depth to handle `DECIMAL(10,2)`, `CHECK(...)`, `DEFAULT(...)`)
- Column name = first token (handle quoted identifiers: strip `"` quotes)
- Column type = **all tokens after name until first recognized constraint keyword**. Constraint keywords: `NOT`, `NULL`, `UNIQUE`, `PRIMARY`, `DEFAULT`, `REFERENCES`, `CHECK`, `CONSTRAINT`, `COLLATE`, `GENERATED`. This correctly captures multi-word types: `TIMESTAMP WITH TIME ZONE`, `DOUBLE PRECISION`, `CHARACTER VARYING(255)`.
- Scan remaining tokens for constraint keywords: NOT NULL, UNIQUE, PRIMARY KEY, DEFAULT ..., REFERENCES ..., CHECK (...)
- FOREIGN KEY detection: match `REFERENCES table_name(column_name)` — stop capture at `)`, ignore trailing `ON DELETE/UPDATE` clauses. Also handle `REFERENCES table_name` without column (implicit PK reference) → `ForeignKeyRef { target_column: "".to_string() }`
- Handle table-level constraints: FOREIGN KEY, PRIMARY KEY, CHECK at end of column list — skip as non-column entries
- Handle schema-qualified names: `REFERENCES public.users(id)` → `target_table: "public.users"`

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/extract.rs
git commit -m "feat: SQL column-level extraction with regex-based parsing"
```

### Task 6: CQL, Cypher, Elasticsearch extraction

**Files:**
- Modify: `src/schema/extract.rs`

- [ ] **Step 1: Write failing tests for CQL**

```rust
#[test]
fn test_cql_create_table() {
    let body = "CREATE TABLE users (\n  id uuid PRIMARY KEY,\n  name text\n) WITH clustering ORDER BY (name ASC);";
    let table = extract_table_schema(body, "users", "schema.cql", 1);
    assert_eq!(table.columns.len(), 2);
    assert_eq!(table.columns[0].data_type, "uuid");
}

#[test]
fn test_cql_with_clause_skipped() {
    let body = "CREATE TABLE t (\n  id int PRIMARY KEY\n) WITH compaction = {'class': 'LeveledCompactionStrategy'};";
    let table = extract_table_schema(body, "t", "t.cql", 1);
    assert_eq!(table.columns.len(), 1);
}
```

- [ ] **Step 2: Write failing tests for Cypher**

```rust
#[test]
fn test_cypher_constraint() {
    let content = "CREATE CONSTRAINT unique_user_email FOR (u:User) REQUIRE u.email IS UNIQUE;";
    let entries = extract_cypher_schema(content, "schema.cypher");
    assert!(!entries.is_empty());
    assert!(entries.iter().any(|e| e.contains_label("User")));
}

#[test]
fn test_cypher_index() {
    let content = "CREATE INDEX user_name_idx FOR (u:User) ON (u.name);";
    let entries = extract_cypher_schema(content, "schema.cypher");
    assert!(entries.iter().any(|e| e.contains_label("User")));
}

#[test]
fn test_cypher_node_labels() {
    let content = "MATCH (u:User)-[:FOLLOWS]->(f:User) RETURN u, f;";
    let labels = extract_cypher_labels(content);
    assert!(labels.contains("User"));
    assert!(labels.contains("FOLLOWS"));
}
```

- [ ] **Step 3: Write failing tests for Elasticsearch**

```rust
#[test]
fn test_elasticsearch_mappings() {
    let content = r#"{"mappings":{"properties":{"name":{"type":"text"},"age":{"type":"integer"}}}}"#;
    let schema = extract_elasticsearch_schema(content, "users.json");
    assert!(schema.is_some());
    let s = schema.unwrap();
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn test_elasticsearch_nested_properties() {
    let content = r#"{"mappings":{"properties":{"address":{"properties":{"city":{"type":"text"}}}}}}"#;
    let schema = extract_elasticsearch_schema(content, "idx.json");
    assert!(schema.is_some());
}

#[test]
fn test_non_elasticsearch_json() {
    let content = r#"{"name":"cxpak","version":"0.12.0"}"#;
    let schema = extract_elasticsearch_schema(content, "package.json");
    assert!(schema.is_none());
}
```

- [ ] **Step 4: Implement CQL (reuse SQL extraction), Cypher (regex), Elasticsearch (JSON pattern)**

CQL: `extract_table_schema()` already handles CQL — just verify WITH clause handling.
Cypher: regex patterns for `CREATE CONSTRAINT ... FOR (n:Label)`, `CREATE INDEX ... FOR (n:Label)`, `(:Label)` node patterns, `-[:TYPE]->` relationship patterns.
Elasticsearch: parse JSON string, check for `"mappings"` → `"properties"` path, extract field names and types.

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add src/schema/extract.rs
git commit -m "feat: add CQL, Cypher, and Elasticsearch schema extraction"
```

### Task 7: Prisma schema enrichment

**Files:**
- Modify: `src/schema/extract.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_prisma_model_to_table_schema() {
    let body = "model User {\n  id    Int     @id @default(autoincrement())\n  email String  @unique\n  posts Post[]\n}";
    let schema = extract_prisma_schema(body, "User", "schema.prisma", 1);
    assert_eq!(schema.table_name, "user");
    assert_eq!(schema.fields.len(), 3);
    assert!(schema.fields[2].is_relation);
}

#[test]
fn test_prisma_map_override() {
    let body = "model User {\n  id Int @id\n  @@map(\"users\")\n}";
    let schema = extract_prisma_schema(body, "User", "schema.prisma", 1);
    assert_eq!(schema.table_name, "users");
}

#[test]
fn test_prisma_relation_field() {
    let body = "model Post {\n  author   User @relation(fields: [authorId], references: [id])\n  authorId Int\n}";
    let schema = extract_prisma_schema(body, "Post", "schema.prisma", 1);
    let relation_field = schema.fields.iter().find(|f| f.name == "author").unwrap();
    assert!(relation_field.is_relation);
    assert_eq!(relation_field.related_model, Some("User".to_string()));
}
```

- [ ] **Step 2: Implement `extract_prisma_schema()`**

Parse model body lines. Field format: `name Type [@modifiers]`.
- `@id` → primary key
- `@unique` → unique constraint
- `@default(...)` → default value
- `@relation(...)` → relation field
- `@@map("table")` → table name override
- Field types ending in `[]` → relation/array

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/extract.rs
git commit -m "feat: add Prisma schema extraction with relation detection"
```

---

## Stream 3: Schema Detection

### Task 8: ORM pattern matchers

**Files:**
- Modify: `src/schema/detect.rs`

- [ ] **Step 1: Write failing tests for each ORM (12 tests)**

```rust
#[test]
fn test_detect_django_model() { /* class User(models.Model): → table "user" */ }
#[test]
fn test_detect_django_db_table_override() { /* class Meta: db_table = "custom_users" */ }
#[test]
fn test_detect_sqlalchemy_model() { /* class User(Base): with sqlalchemy import → __tablename__ = "users" */ }
#[test]
fn test_detect_sqlalchemy_default_name() { /* (Base) + sqlalchemy import, no __tablename__ → "user" */ }
#[test]
fn test_detect_sqlalchemy_false_positive_no_import() { /* class Foo(Base): WITHOUT sqlalchemy import → NOT detected */ }
#[test]
fn test_detect_typeorm_entity() { /* class with @Column() in body → detected as TypeORM */ }
#[test]
fn test_detect_typeorm_entity_name() { /* @Entity("users") in file.content on line before class → table name "users" */ }
#[test]
fn test_detect_typeorm_via_member_decorators() { /* body contains @PrimaryColumn or @ManyToOne → TypeORM */ }
#[test]
fn test_detect_activerecord_model() { /* class User < ActiveRecord::Base → "users" */ }
#[test]
fn test_detect_activerecord_application_record() { /* class User < ApplicationRecord → "users" */ }
#[test]
fn test_detect_prisma_model() { /* kind: Struct from prisma parser → tagged */ }
#[test]
fn test_detect_non_orm_class() { /* class UserService(Base): → NOT detected (no model fields) */ }
#[test]
fn test_detect_multiple_models_one_file() { /* two Django models → both detected */ }
#[test]
fn test_detect_no_orm_patterns() { /* plain Rust file → empty */ }
```

- [ ] **Step 2: Implement ORM detection**

```rust
pub fn detect_orm_models(index: &CodebaseIndex) -> Vec<OrmModelSchema> {
    let mut models = Vec::new();
    for file in &index.files {
        let Some(pr) = &file.parse_result else { continue };
        let lang = file.language.as_deref().unwrap_or("");
        for symbol in &pr.symbols {
            if let Some(model) = detect_single_orm_model(symbol, lang, &file.relative_path) {
                models.push(model);
            }
        }
    }
    models
}

fn detect_single_orm_model(
    symbol: &Symbol,
    language: &str,
    file_path: &str,
    imports: &[Import],
    file_content: &str,
) -> Option<OrmModelSchema> {
    match language {
        "python" => detect_django_or_sqlalchemy(symbol, file_path, imports),
        "typescript" => detect_typeorm(symbol, file_path, file_content),
        "ruby" => detect_activerecord(symbol, file_path),
        "prisma" => detect_prisma_model(symbol, file_path),
        _ => None,
    }
}
```

**ORM detection specifics:**

**SQLAlchemy:** Check signature for `(Base)` or `(DeclarativeBase)` BUT also require an import guard: `imports.iter().any(|i| i.source.contains("sqlalchemy"))`. This prevents false positives from non-ORM classes inheriting `Base`.

**TypeORM:** The `@Entity()` decorator is a **sibling node** in the tree-sitter AST — it is NOT inside the class symbol's `signature` or `body`. Instead, detect TypeORM by checking **member decorators** inside the class body: if `symbol.body` contains `@Column` or `@PrimaryColumn` or `@PrimaryGeneratedColumn` or `@ManyToOne` or `@OneToMany` or `@ManyToMany` or `@OneToOne`, it's a TypeORM entity. For table name extraction, scan `file_content` for `@Entity("table_name")` on the line immediately before the class declaration.

```rust
```

Each sub-function checks symbol signature/body for the known patterns and extracts table name + fields.

**ActiveRecord pluralization:**
```rust
fn pluralize(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with("ss") || lower.ends_with("sh") || lower.ends_with("ch")
        || lower.ends_with('x') || lower.ends_with('z') || lower.ends_with('s') {
        format!("{lower}es")
    } else if lower.ends_with('y') && !lower.ends_with("ay") && !lower.ends_with("ey")
        && !lower.ends_with("oy") && !lower.ends_with("uy") {
        format!("{}ies", &lower[..lower.len()-1])
    } else {
        format!("{lower}s")
    }
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/detect.rs
git commit -m "feat: ORM detection for Django, SQLAlchemy, TypeORM, ActiveRecord, Prisma"
```

### Task 9: Terraform tagging

**Files:**
- Modify: `src/schema/detect.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_detect_terraform_dynamodb() { /* resource "aws_dynamodb_table" "orders" → tagged */ }
#[test]
fn test_detect_terraform_rds() { /* resource "aws_rds_cluster" "main" → tagged */ }
#[test]
fn test_detect_terraform_non_db_resource() { /* resource "aws_s3_bucket" → NOT tagged */ }
```

- [ ] **Step 2: Implement Terraform DB resource detection**

Match HCL block names against `DB_RESOURCE_PREFIXES` list (26 prefixes from spec).

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/detect.rs
git commit -m "feat: Terraform/HCL database resource tagging"
```

### Task 10: Migration detection

**Files:**
- Modify: `src/schema/detect.rs`

- [ ] **Step 1: Write failing tests (10 tests)**

One per framework (Rails, Alembic, Flyway, Django, Knex, Prisma, Drizzle, Generic) + no migrations + mixed frameworks.

- [ ] **Step 2: Implement migration detection**

```rust
pub fn detect_migrations(index: &CodebaseIndex) -> Vec<MigrationChain> {
    let paths: Vec<&str> = index.files.iter().map(|f| f.relative_path.as_str()).collect();
    let mut chains = Vec::new();
    // Try each framework's pattern against the file paths
    // For each match: extract sequence from filename, sort, build chain
    chains
}
```

Each framework has a directory pattern and filename regex. Sort by extracted sequence.

**Note: Alembic** is the exception — Alembic migration filenames use a hash prefix, not a sortable timestamp. The sequence is determined by reading `revision = "..."` from the file body (`file.content`). This is the only framework that requires content reading.

**Dependency note:** The `regex` crate is already in `Cargo.toml` (added in v0.10.0). No new dependencies needed.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/detect.rs
git commit -m "feat: migration detection for 7 frameworks + generic"
```

### Task 11: Build `SchemaIndex` orchestrator

**Files:**
- Modify: `src/schema/detect.rs`

- [ ] **Step 1: Implement `build_schema_index()`**

```rust
pub fn build_schema_index(index: &CodebaseIndex) -> Option<SchemaIndex> {
    let mut schema = SchemaIndex::empty();

    // 1. Extract SQL schemas from SQL/CQL files
    for file in &index.files {
        let lang = file.language.as_deref().unwrap_or("");
        if lang == "sql" || file.relative_path.ends_with(".cql") {
            if let Some(pr) = &file.parse_result {
                for symbol in &pr.symbols {
                    // Route to extract_table_schema, extract_view_schema, etc.
                    // based on symbol.kind
                }
            }
        }
        // Cypher files
        if file.relative_path.ends_with(".cypher") {
            // extract_cypher_schema from content
        }
        // Elasticsearch JSON
        if lang == "json" {
            // extract_elasticsearch_schema from content
        }
    }

    // 2. Detect ORM models
    let orm_models = detect_orm_models(index);
    for model in orm_models {
        schema.orm_models.insert(model.class_name.clone(), model);
    }

    // 3. Detect migrations
    schema.migrations = detect_migrations(index);

    // 4. Detect Terraform DB resources
    detect_terraform_schemas(index, &mut schema);

    // 5. Enrich Prisma models (extract.rs provides field-level detail;
    //    detect.rs ORM detection already tagged them — merge, don't duplicate.
    //    Use class_name as key to update existing OrmModelSchema entries
    //    with field detail from extract_prisma_schema().)
    extract_prisma_schemas(index, &mut schema);

    if schema.is_empty() { None } else { Some(schema) }
}
```

- [ ] **Step 2: Wire into `CodebaseIndex::build()` and `build_with_content()`**

After building the base index, call `build_schema_index(&index)` and store result:
```rust
// In build() and build_with_content(), after constructing Self:
let mut index = Self { /* ... */ schema: None };
index.schema = crate::schema::detect::build_schema_index(&index);
index
```

- [ ] **Step 3: Write integration test**

```rust
#[test]
fn test_schema_index_built_for_sql_repo() {
    // Create temp repo with .sql file containing CREATE TABLE
    // Build index
    // Verify schema is Some and tables populated
}

#[test]
fn test_schema_index_none_for_plain_repo() {
    // Create temp repo with only .rs files
    // Build index
    // Verify schema is None
}
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/schema/detect.rs src/index/mod.rs
git commit -m "feat: build SchemaIndex during index construction"
```

---

## Stream 4: Schema Linking

### Task 12: Embedded SQL detection

**Files:**
- Modify: `src/schema/link.rs`

- [ ] **Step 1: Write failing tests (10 tests)**

```rust
#[test]
fn test_detect_select_from() {
    let refs = detect_embedded_sql(r#"let q = "SELECT * FROM users WHERE id = 1";"#);
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].table_name, "users");
}

#[test]
fn test_detect_insert_into() {
    let refs = detect_embedded_sql(r#"db.execute("INSERT INTO orders VALUES (1, 2)");"#);
    assert_eq!(refs[0].table_name, "orders");
}

#[test]
fn test_detect_update() {
    let refs = detect_embedded_sql(r#"cursor.execute("UPDATE inventory SET qty = 0")"#);
    assert_eq!(refs[0].table_name, "inventory");
}

#[test]
fn test_detect_join_multiple_tables() {
    let refs = detect_embedded_sql(r#""SELECT u.name FROM users u JOIN orders o ON u.id = o.user_id""#);
    assert!(refs.iter().any(|r| r.table_name == "users"));
    assert!(refs.iter().any(|r| r.table_name == "orders"));
}

#[test]
fn test_not_sql_string() {
    let refs = detect_embedded_sql(r#"let msg = "SELECT the best option for you";"#);
    assert!(refs.is_empty(), "no FROM/INTO/TABLE = not SQL");
}

#[test]
fn test_parameterized_no_table() {
    let refs = detect_embedded_sql(r#""SELECT * FROM $1 WHERE id = $2""#);
    // $1 is not a table name — skip
    assert!(refs.is_empty() || refs.iter().all(|r| !r.table_name.starts_with('$')));
}

#[test]
fn test_multiline_sql() {
    let refs = detect_embedded_sql("let q = \"SELECT *\n  FROM users\n  WHERE active = 1\";");
    assert!(refs.iter().any(|r| r.table_name == "users"));
}

#[test]
fn test_delete_from() {
    let refs = detect_embedded_sql(r#""DELETE FROM sessions WHERE expired = true""#);
    assert_eq!(refs[0].table_name, "sessions");
}

#[test]
fn test_create_table_in_code() {
    let refs = detect_embedded_sql(r#""CREATE TABLE temp_results (id INT)""#);
    assert_eq!(refs[0].table_name, "temp_results");
}

#[test]
fn test_empty_string() {
    let refs = detect_embedded_sql("");
    assert!(refs.is_empty());
}
```

- [ ] **Step 2: Implement `detect_embedded_sql()`**

```rust
pub struct EmbeddedSqlRef {
    pub table_name: String,
    pub sql_fragment: String,
    pub line: usize,
}

pub fn detect_embedded_sql(content: &str) -> Vec<EmbeddedSqlRef> {
    let mut refs = Vec::new();
    // Find quoted strings in content
    // For each: check for SQL DML keyword AND structural keyword (FROM/INTO/TABLE)
    // Extract table names by position after FROM, JOIN, INTO, UPDATE, TABLE
    // Filter out: variables ($1, ?), keywords, non-identifier characters
    refs
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/link.rs
git commit -m "feat: embedded SQL detection in string literals"
```

### Task 13: Build schema edges

**Files:**
- Modify: `src/schema/link.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_fk_edges() {
    // Two tables, orders.user_id REFERENCES users(id)
    // → ForeignKey edge from orders file to users file
}

#[test]
fn test_view_reference_edges() {
    // View referencing users and orders
    // → ViewReference edges to both table files
}

#[test]
fn test_embedded_sql_edges() {
    // Python file with "SELECT * FROM users"
    // → EmbeddedSql edge from Python file to users SQL file
}

#[test]
fn test_orm_model_edges() {
    // Django model User with table_name "users" matching SQL table
    // → OrmModel edge from model file to SQL file
}

#[test]
fn test_migration_sequence_edges() {
    // 3 migrations → 2 MigrationSequence edges (3→2, 2→1)
}

#[test]
fn test_circular_fk_no_panic() {
    // Table A FK→B, Table B FK→A
    // → Both edges created, no loop
}
```

- [ ] **Step 2: Implement `build_schema_edges()`**

Per the spec: iterate tables for FK/view/function/trigger/index edges, iterate all files for embedded SQL, iterate ORM models for model→table edges, iterate migrations for sequence edges.

**Embedded SQL scanning:** Scan BOTH symbol bodies (for fine-grained attribution) AND `file.content` directly (for module-level SQL, files without parse results). Deduplicate by table name per file before creating edges.

**Trigger/Index edges:** CREATE TRIGGER and CREATE INDEX are already extracted as symbols by the SQL parser. Link trigger → target table file (`TriggerTarget`), index → target table file (`IndexTarget`). Extract target table name from the symbol body (ON table_name pattern).

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/schema/link.rs
git commit -m "feat: build typed schema edges for dependency graph"
```

---

## Stream 5: Integration

### Task 14: Wire schema into trace command

**Files:**
- Modify: `src/commands/trace.rs`

- [ ] **Step 1: Update trace to display edge types**

When printing dependency context, include the edge type:
```
// Dependency: schema/tables.sql (via: foreign_key)
// Dependency: src/api/orders.py (via: embedded_sql)
```

- [ ] **Step 2: Write test**

```rust
#[test]
fn test_trace_shows_schema_edge_types() {
    // Build index with SQL file + Python file with embedded SQL
    // Trace the table → verify output includes edge types
}
```

- [ ] **Step 3: Commit**

```bash
git add src/commands/trace.rs
git commit -m "feat: display edge types in trace output"
```

### Task 15: Wire schema into overview

**Files:**
- Modify: `src/commands/overview.rs`

- [ ] **Step 1: Add edge types to dependency graph section**

When rendering the dependency graph, show edge types:
```
- src/api/orders.py → schema/tables.sql (embedded_sql)
- schema/tables.sql::orders → schema/tables.sql::users (foreign_key)
```

- [ ] **Step 2: Write test**

- [ ] **Step 3: Commit**

```bash
git add src/commands/overview.rs
git commit -m "feat: display typed edges in overview dependency graph"
```

### Task 16: Wire schema into pack_context annotations

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add schema annotation line**

When a packed file contains schema definitions, add a `schema:` line to annotations:
```
// schema: table "orders" — 8 columns, 2 FKs (users, products), 1 index
```

When edge type is available, include it in the parent annotation:
```
// score: 0.82 | role: dependency | parent: src/api/orders.py (via: embedded_sql)
```

- [ ] **Step 2: Add schema-aware query expansion**

In `context_for_task`, when `SchemaIndex` exists and Database domain is active, add table and column names to expansion terms.

- [ ] **Step 3: Write MCP round-trip tests**

```rust
#[test]
fn test_pack_context_schema_annotation() { ... }
#[test]
fn test_context_for_task_schema_expansion() { ... }
```

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: schema annotations and schema-aware expansion in MCP tools"
```

### Task 17: Full pipeline integration tests

**Files:**
- Add tests

- [ ] **Step 1: Write end-to-end tests**

```rust
#[test]
fn test_full_pipeline_sql_repo() {
    // Create temp repo with:
    // - schema/tables.sql (CREATE TABLE users, orders with FK)
    // - src/api.py (with "SELECT * FROM users")
    // Build index → verify:
    // - SchemaIndex has 2 tables with columns
    // - FK edge between orders→users
    // - EmbeddedSql edge from api.py→tables.sql
    // - trace "users" returns both SQL and Python files
}

#[test]
fn test_full_pipeline_django_project() {
    // Create temp repo with:
    // - models/user.py (class User(models.Model))
    // - schema.sql (CREATE TABLE user)
    // Build index → verify:
    // - ORM model detected
    // - OrmModel edge from model file to SQL file
}

#[test]
fn test_full_pipeline_migrations() {
    // Create temp repo with db/migrate/ and 3 Rails migrations
    // Verify MigrationSequence edges in correct order
}

#[test]
fn test_full_pipeline_no_schema() {
    // Plain Rust repo with no SQL/ORM
    // Verify schema is None, all features degrade gracefully
}

#[test]
fn test_full_pipeline_terraform() {
    // .tf file with aws_dynamodb_table
    // Verify tagged as schema
}

#[test]
fn test_full_pipeline_mcp_roundtrip() {
    // MCP call with schema-enriched index
    // Verify responses include edge types and schema annotations
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 3: Commit**

```bash
git add tests/ src/
git commit -m "test: full pipeline integration tests for data layer awareness"
```

---

## Stream 6: Documentation + Version + QA

### Task 18: Update documentation

**Files:**
- Modify: `README.md`
- Modify: `.claude/CLAUDE.md`
- Modify: `plugin/README.md`

- [ ] **Step 1: Update README.md**

- Document data layer awareness
- Schema detection: SQL, CQL, Cypher, Elasticsearch, Prisma
- ORM detection: Django, SQLAlchemy, TypeORM, ActiveRecord
- Typed dependency graph with 9 edge types
- Migration framework support
- Cross-language embedded SQL linking

- [ ] **Step 2: Update CLAUDE.md**

- Add schema module to architecture notes
- Document SchemaIndex, edge types
- Update pipeline description

- [ ] **Step 3: Update plugin/README.md**

- Document schema-enriched MCP tool responses

- [ ] **Step 4: Commit**

```bash
git add README.md .claude/CLAUDE.md plugin/README.md
git commit -m "docs: document data layer awareness for v0.12.0"
```

### Task 19: Version bump

**Files:**
- Modify: `Cargo.toml`, `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `plugin/lib/ensure-cxpak`

- [ ] **Step 1: Bump version to 0.12.0 in all four files**

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 0.12.0"
```

### Task 20: Pre-Release QA + CI Validation

**This task MUST pass before tagging and pushing.**

- [ ] **Step 1: Run full test suite locally**

Run: `cargo test --verbose`
Expected: ALL tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: Zero warnings.

- [ ] **Step 3: Run formatter**

Run: `cargo fmt -- --check`
Expected: Clean.

- [ ] **Step 4: Run coverage**

Run: `cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out json`
Expected: ≥90% overall. 100% on `src/schema/`. ≥95% on modified files.

- [ ] **Step 5: Manual QA — schema extraction**

```bash
# Create temp repo with SQL + Python
mkdir /tmp/qa-test && cd /tmp/qa-test && git init
cat > schema.sql << 'SQL'
CREATE TABLE users (id INT PRIMARY KEY, name TEXT NOT NULL, email VARCHAR(255) UNIQUE);
CREATE TABLE orders (id INT PRIMARY KEY, user_id INT REFERENCES users(id), total DECIMAL);
SQL
cat > api.py << 'PY'
def get_orders():
    db.execute("SELECT o.* FROM orders o JOIN users u ON o.user_id = u.id")
PY
cargo run -- overview --tokens 10k /tmp/qa-test
```
Verify: dependency graph shows ForeignKey and EmbeddedSql edge types.

- [ ] **Step 6: Manual QA — ORM detection**

Create temp repo with Django model, verify ORM detection in trace output.

- [ ] **Step 7: Manual QA — migration ordering**

Create temp repo with `db/migrate/` and 3 timestamped files, verify chronological ordering.

- [ ] **Step 8: Simulate CI jobs locally**

```bash
cargo build --verbose
cargo test --verbose
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90
```

- [ ] **Step 9: Tag and push (only after all above pass)**

```bash
git tag v0.12.0
git push origin main --tags
```

---

## Task Summary

| Stream | Tasks | Dependencies |
|---|---|---|
| 1. Typed Graph | Tasks 1-4 | Sequential (types → graph migration → consumer migration → CodebaseIndex) |
| 2. Schema Extraction | Tasks 5-7 | Task 1 (types needed) |
| 3. Schema Detection | Tasks 8-11 | Task 1 (types), Task 5 (extraction functions) |
| 4. Schema Linking | Tasks 12-13 | Task 1 (types), Task 5 (extraction), Task 8 (ORM models) |
| 5. Integration | Tasks 14-17 | All of Streams 1-4 |
| 6. Docs + Version + QA | Tasks 18-20 | All prior |

**Parallelizable:** After Stream 1, Streams 2-4 can partially overlap:
- Tasks 5-7 (extraction) are independent of Tasks 8-10 (detection)
- Task 11 (orchestrator) needs both extraction and detection
- Tasks 12-13 (linking) need extraction + detection outputs

**Critical path:** Tasks 1-4 → (Tasks 5-7 ∥ Tasks 8-10) → Task 11 → Tasks 12-13 → Tasks 14-17 → Tasks 18-20

**Total: 20 tasks, ~115 new tests, 100% branch coverage on `src/schema/`, 95%+ on modified modules, 90%+ overall CI gate. Task 20 is the release gate.**
