// SQL column-level extraction, CQL, Cypher regex, Elasticsearch JSON pattern

use crate::schema::{
    ColumnSchema, DbFunctionSchema, ForeignKeyRef, OrmFieldSchema, OrmFramework, OrmModelSchema,
    TableSchema, ViewSchema,
};
use regex::Regex;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Compile-once regex statics — avoids recompiling on every call.
// ---------------------------------------------------------------------------

static RE_VIEW_SOURCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:FROM|JOIN)\s+([a-zA-Z_][a-zA-Z0-9_.]*)").expect("RE_VIEW_SOURCES")
});

static RE_FUNCTION_SOURCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+([a-zA-Z_][a-zA-Z0-9_.]*)")
        .expect("RE_FUNCTION_SOURCES")
});

/// Strip an optional `schema.` prefix from a table reference.
///
/// `"public.users"` → `"users"`, `"users"` → `"users"`.
fn strip_schema_prefix(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(_, t)| t)
}

// ---------------------------------------------------------------------------
// Task 5 – SQL column-level extraction
// ---------------------------------------------------------------------------

/// Extract the content between the first `(` and its matching `)`, tracking
/// paren depth so nested parens are handled correctly.
fn extract_paren_body(sql: &str) -> Option<&str> {
    let start = sql.find('(')?;
    let bytes = sql.as_bytes();
    let mut depth = 0usize;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    Some(&sql[start + 1..end])
}

/// Split a comma-separated list of column/constraint definitions at the **top
/// paren level only**, so `DECIMAL(10,2)` and `CHECK(a,b)` are kept intact.
fn split_top_level_commas(body: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
            }
            b',' if depth == 0 => {
                result.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&body[start..]);
    result
}

/// Constraint keywords that terminate the column type token stream.
const CONSTRAINT_KEYWORDS: &[&str] = &[
    "NOT",
    "NULL",
    "UNIQUE",
    "PRIMARY",
    "DEFAULT",
    "REFERENCES",
    "CHECK",
    "CONSTRAINT",
    "COLLATE",
    "GENERATED",
    "AUTO_INCREMENT",
];

/// Return true if the token (upper-cased) is a constraint keyword.
fn is_constraint_kw(tok: &str) -> bool {
    let up = tok.to_uppercase();
    CONSTRAINT_KEYWORDS.contains(&up.as_str())
}

/// Consume tokens that form a type, including any paren group that follows
/// (e.g. `VARCHAR(255)`, `DECIMAL(10,2)`).  Advances `idx` past consumed
/// tokens and returns the collected type string.
fn collect_type(tokens: &[&str], idx: &mut usize) -> String {
    let mut parts: Vec<String> = Vec::new();

    while *idx < tokens.len() {
        let tok = tokens[*idx];
        if is_constraint_kw(tok) {
            break;
        }
        // Accumulate this token into the type string.
        // If the token itself contains an opening paren we must also absorb
        // the matching closing paren (it may span several tokens due to spaces
        // inside parens being a separate token — but in practice SQL types keep
        // the paren-group in one token like `VARCHAR(255)` after trimming).
        parts.push(tok.to_string());
        *idx += 1;

        // If the last part we added contains an open paren but no matching
        // close paren, keep consuming until we close it.
        let joined = parts.join(" ");
        let open = joined.chars().filter(|&c| c == '(').count();
        let close = joined.chars().filter(|&c| c == ')').count();
        if open > close {
            // Absorb tokens until parens balance.
            while *idx < tokens.len() {
                let next = tokens[*idx];
                parts.push(next.to_string());
                *idx += 1;
                let j2 = parts.join(" ");
                let o2 = j2.chars().filter(|&c| c == '(').count();
                let c2 = j2.chars().filter(|&c| c == ')').count();
                if o2 == c2 {
                    break;
                }
            }
        }
    }
    parts.join(" ")
}

/// Parse a REFERENCES clause starting at `idx` (which points to the token
/// *after* `REFERENCES`).  Returns a `ForeignKeyRef`.
fn parse_references(tokens: &[&str], idx: &mut usize) -> ForeignKeyRef {
    if *idx >= tokens.len() {
        return ForeignKeyRef {
            target_table: String::new(),
            target_column: String::new(),
        };
    }
    // The next token is `table_name` or `schema.table_name` or
    // `schema.table_name(col_name)` or `table_name(col_name)`.
    let raw = tokens[*idx];
    *idx += 1;

    // Strip any trailing ON DELETE / ON UPDATE that may have been lumped in.
    // Split at '(' to separate table from column.
    if let Some(paren_pos) = raw.find('(') {
        let table = strip_schema_prefix(&raw[..paren_pos]).to_string();
        // Extract column up to closing ')'
        let rest = &raw[paren_pos + 1..];
        let col = rest.trim_end_matches(')').to_string();
        ForeignKeyRef {
            target_table: table,
            target_column: col,
        }
    } else {
        // No paren – either `table` or `table` followed by `(col)` as a
        // separate token.
        if *idx < tokens.len() && tokens[*idx].starts_with('(') {
            let col_tok = tokens[*idx];
            *idx += 1;
            let col = col_tok
                .trim_start_matches('(')
                .trim_end_matches(')')
                .to_string();
            ForeignKeyRef {
                target_table: strip_schema_prefix(raw).to_string(),
                target_column: col,
            }
        } else {
            ForeignKeyRef {
                target_table: strip_schema_prefix(raw).to_string(),
                target_column: String::new(),
            }
        }
    }
}

/// Capture the DEFAULT expression (could be a bare token or a paren-wrapped
/// expression).
fn capture_default(tokens: &[&str], idx: &mut usize) -> String {
    if *idx >= tokens.len() {
        return String::new();
    }
    let tok = tokens[*idx];
    *idx += 1;

    // If it starts with '(' we need to absorb until balanced.
    if tok.starts_with('(') {
        let mut expr = tok.to_string();
        let mut open = expr.chars().filter(|&c| c == '(').count();
        let mut close = expr.chars().filter(|&c| c == ')').count();
        while open > close && *idx < tokens.len() {
            let next = tokens[*idx];
            *idx += 1;
            expr.push(' ');
            expr.push_str(next);
            open = expr.chars().filter(|&c| c == '(').count();
            close = expr.chars().filter(|&c| c == ')').count();
        }
        expr
    } else {
        tok.to_string()
    }
}

/// Parse a single column definition line into a `ColumnSchema`.
/// Returns `None` if the line is a table-level constraint that should be
/// skipped.
fn parse_column_def(line: &str) -> Option<ColumnSchema> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let upper = trimmed.to_uppercase();

    // Skip table-level constraint lines.
    for kw in &[
        "PRIMARY KEY",
        "FOREIGN KEY",
        "CHECK",
        "CONSTRAINT",
        "UNIQUE",
    ] {
        if upper.starts_with(kw) {
            return None;
        }
    }

    // Tokenise by whitespace (we keep paren-groups together only if they're
    // already in one token, which is typical for `VARCHAR(255)`).
    // We need a smarter tokeniser that keeps paren groups attached to their
    // preceding token when there's no space before the paren.
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let mut idx = 0usize;

    // Column name – possibly quoted.
    let raw_name = tokens[idx];
    idx += 1;
    let col_name = raw_name.trim_matches('"').to_string();

    // Column type.
    let data_type = collect_type(&tokens, &mut idx);

    // Remaining tokens are constraint keywords.
    let mut nullable = true;
    let mut unique = false;
    let mut default: Option<String> = None;
    let mut foreign_key: Option<ForeignKeyRef> = None;
    let mut constraints: Vec<String> = Vec::new();
    let mut is_pk = false;

    while idx < tokens.len() {
        let tok_upper = tokens[idx].to_uppercase();
        match tok_upper.as_str() {
            "NOT" => {
                idx += 1;
                if idx < tokens.len() && tokens[idx].to_uppercase() == "NULL" {
                    nullable = false;
                    idx += 1;
                }
            }
            "NULL" => {
                nullable = true;
                idx += 1;
            }
            "UNIQUE" => {
                unique = true;
                constraints.push("UNIQUE".to_string());
                idx += 1;
            }
            "PRIMARY" => {
                idx += 1;
                if idx < tokens.len() && tokens[idx].to_uppercase() == "KEY" {
                    is_pk = true;
                    constraints.push("PRIMARY KEY".to_string());
                    idx += 1;
                }
            }
            "DEFAULT" => {
                idx += 1;
                let val = capture_default(&tokens, &mut idx);
                default = Some(val);
            }
            "REFERENCES" => {
                idx += 1;
                foreign_key = Some(parse_references(&tokens, &mut idx));
                // Skip trailing ON DELETE / ON UPDATE clauses.
                while idx < tokens.len() {
                    let t = tokens[idx].to_uppercase();
                    if t == "ON" {
                        idx += 1; // consume ON
                        if idx < tokens.len() {
                            idx += 1; // consume DELETE/UPDATE
                        }
                        if idx < tokens.len() {
                            idx += 1; // consume CASCADE/SET/RESTRICT etc.
                        }
                        // Handle SET NULL (two words)
                        if idx < tokens.len()
                            && (tokens[idx].to_uppercase() == "NULL"
                                || tokens[idx].to_uppercase() == "DEFAULT")
                        {
                            idx += 1;
                        }
                    } else {
                        break;
                    }
                }
            }
            "CHECK" | "CONSTRAINT" | "COLLATE" | "GENERATED" | "AUTO_INCREMENT" => {
                // Skip the rest – these are terminal for column parsing.
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }

    if unique && !is_pk {
        // Already pushed "UNIQUE" above.
    }

    Some(ColumnSchema {
        name: col_name,
        data_type,
        nullable,
        default,
        constraints,
        foreign_key,
        // is_pk stored separately; caller sets primary_key on TableSchema
        // by checking constraints or the "PRIMARY KEY" table-level clause.
        // We use a temporary field trick: add pk marker to constraints so
        // caller can extract it.  Actually we already push "PRIMARY KEY" to
        // constraints when is_pk is true.
    })
}

/// Extract a `TableSchema` from the body text of a CREATE TABLE statement.
///
/// `body` is the full SQL from (and including) the opening `(`.
pub fn extract_table_schema(
    body: &str,
    table_name: &str,
    file_path: &str,
    start_line: usize,
) -> TableSchema {
    let inner = match extract_paren_body(body) {
        Some(s) => s,
        None => {
            return TableSchema {
                name: table_name.to_string(),
                columns: Vec::new(),
                primary_key: None,
                indexes: Vec::new(),
                file_path: file_path.to_string(),
                start_line,
            }
        }
    };

    let defs = split_top_level_commas(inner);

    let mut columns: Vec<ColumnSchema> = Vec::new();
    let mut table_pk: Option<Vec<String>> = None;
    // (table_name, col_name, target_table, target_col) for table-level FK
    let mut table_fks: Vec<(String, String, String)> = Vec::new();

    for def in &defs {
        let trimmed = def.trim();
        let upper = trimmed.to_uppercase();

        if upper.starts_with("PRIMARY KEY") {
            // PRIMARY KEY (col1, col2, ...)
            if let Some(pk_body) = extract_paren_body(trimmed) {
                let pk_cols: Vec<String> = pk_body
                    .split(',')
                    .map(|c| c.trim().trim_matches('"').to_string())
                    .filter(|c| !c.is_empty())
                    .collect();
                if !pk_cols.is_empty() {
                    table_pk = Some(pk_cols);
                }
            }
            continue;
        }

        if upper.starts_with("FOREIGN KEY") {
            // FOREIGN KEY (col) REFERENCES target_table(target_col)
            let fk_col = extract_paren_body(trimmed)
                .map(|s| s.trim().trim_matches('"').to_string())
                .unwrap_or_default();
            // Find REFERENCES keyword after the first ')'.
            let after_paren = trimmed.find(')').map(|i| &trimmed[i + 1..]).unwrap_or("");
            let upper_after = after_paren.to_uppercase();
            if let Some(ref_pos) = upper_after.find("REFERENCES") {
                let ref_part = after_paren[ref_pos + "REFERENCES".len()..].trim();
                let tokens: Vec<&str> = ref_part.split_whitespace().collect();
                let mut idx = 0usize;
                let fk_ref = parse_references(&tokens, &mut idx);
                table_fks.push((fk_col, fk_ref.target_table, fk_ref.target_column));
            }
            continue;
        }

        if upper.starts_with("CHECK")
            || upper.starts_with("CONSTRAINT")
            || upper.starts_with("UNIQUE")
        {
            continue;
        }

        if let Some(col) = parse_column_def(trimmed) {
            columns.push(col);
        }
    }

    // Determine primary_key: prefer table-level, else gather from column constraints.
    let pk = if table_pk.is_some() {
        table_pk
    } else {
        let pk_cols: Vec<String> = columns
            .iter()
            .filter(|c| c.constraints.contains(&"PRIMARY KEY".to_string()))
            .map(|c| c.name.clone())
            .collect();
        if pk_cols.is_empty() {
            None
        } else {
            Some(pk_cols)
        }
    };

    // Mark pk columns as not-nullable (implicit for primary keys).
    if let Some(ref pk_cols) = pk {
        for col in &mut columns {
            if pk_cols.contains(&col.name) {
                col.nullable = false;
            }
        }
    }

    // Apply table-level foreign keys to matching columns.
    for (fk_col, target_table, target_column) in table_fks {
        if let Some(col) = columns.iter_mut().find(|c| c.name == fk_col) {
            col.foreign_key = Some(ForeignKeyRef {
                target_table,
                target_column,
            });
        }
    }

    TableSchema {
        name: table_name.to_string(),
        columns,
        primary_key: pk,
        indexes: Vec::new(),
        file_path: file_path.to_string(),
        start_line,
    }
}

/// Extract a `ViewSchema` from a SELECT body, collecting referenced table names.
pub fn extract_view_schema(body: &str, view_name: &str, file_path: &str) -> ViewSchema {
    // Words that appear immediately after FROM or JOIN are table names.
    let mut source_tables: Vec<String> = Vec::new();
    for cap in RE_VIEW_SOURCES.captures_iter(body) {
        let tbl = strip_schema_prefix(&cap[1]).to_string();
        if !source_tables.contains(&tbl) {
            source_tables.push(tbl);
        }
    }
    ViewSchema {
        name: view_name.to_string(),
        source_tables,
        file_path: file_path.to_string(),
    }
}

/// Extract a `DbFunctionSchema` from function body text.
pub fn extract_function_schema(body: &str, func_name: &str, file_path: &str) -> DbFunctionSchema {
    let mut referenced_tables: Vec<String> = Vec::new();
    for cap in RE_FUNCTION_SOURCES.captures_iter(body) {
        let tbl = strip_schema_prefix(&cap[1]).to_string();
        if !referenced_tables.contains(&tbl) {
            referenced_tables.push(tbl);
        }
    }
    DbFunctionSchema {
        name: func_name.to_string(),
        referenced_tables,
        file_path: file_path.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Task 6 – CQL, Cypher, Elasticsearch extraction
// ---------------------------------------------------------------------------

// CQL: CREATE TABLE is SQL-compatible; the WITH clause lives after the closing
// `)` so it is already ignored by `extract_table_schema`.
// No additional CQL-specific code is needed.

/// An entry extracted from a Cypher schema file.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherEntry {
    pub labels: Vec<String>,
    pub entry_type: String, // "constraint", "index", "node", "relationship"
}

impl CypherEntry {
    pub fn contains_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
}

/// Extract Cypher schema entries from file content.
pub fn extract_cypher_schema(content: &str, _file_path: &str) -> Vec<CypherEntry> {
    let mut entries: Vec<CypherEntry> = Vec::new();

    // CREATE CONSTRAINT ... FOR (alias:Label)
    let constraint_re =
        Regex::new(r"(?i)CREATE\s+CONSTRAINT\s+\w+\s+(?:ON|FOR)\s+\(\w+:(\w+)\)").unwrap();
    for cap in constraint_re.captures_iter(content) {
        entries.push(CypherEntry {
            labels: vec![cap[1].to_string()],
            entry_type: "constraint".to_string(),
        });
    }

    // CREATE INDEX ... FOR (alias:Label)
    let index_re = Regex::new(r"(?i)CREATE\s+INDEX\s+\w+\s+(?:ON|FOR)\s+\(\w+:(\w+)\)").unwrap();
    for cap in index_re.captures_iter(content) {
        entries.push(CypherEntry {
            labels: vec![cap[1].to_string()],
            entry_type: "index".to_string(),
        });
    }

    // Bare node label patterns: (alias:Label) — not already captured above.
    let node_re = Regex::new(r"\(\w+:(\w+)\)").unwrap();
    let mut seen_labels: Vec<String> = entries
        .iter()
        .flat_map(|e| e.labels.iter().cloned())
        .collect();
    for cap in node_re.captures_iter(content) {
        let label = cap[1].to_string();
        if !seen_labels.contains(&label) {
            seen_labels.push(label.clone());
            entries.push(CypherEntry {
                labels: vec![label],
                entry_type: "node".to_string(),
            });
        }
    }

    // Relationship types: -[:TYPE]->
    let rel_re = Regex::new(r"-\[:(\w+)\]->").unwrap();
    for cap in rel_re.captures_iter(content) {
        entries.push(CypherEntry {
            labels: vec![cap[1].to_string()],
            entry_type: "relationship".to_string(),
        });
    }

    entries
}

/// A field extracted from an Elasticsearch mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ElasticsearchField {
    pub name: String,
    pub field_type: String,
}

/// An Elasticsearch index mapping schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ElasticsearchSchema {
    pub fields: Vec<ElasticsearchField>,
}

/// Parse an Elasticsearch JSON mapping document.
///
/// Looks for `mappings.properties`, then recursively flattens nested
/// properties using dot notation.  Returns `None` for non-ES JSON.
pub fn extract_elasticsearch_schema(
    content: &str,
    _file_path: &str,
) -> Option<ElasticsearchSchema> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;

    // Navigate: { "mappings": { "properties": { … } } }
    // Also accept top-level { "properties": { … } } for shorthand mappings.
    let properties = v
        .get("mappings")
        .and_then(|m| m.get("properties"))
        .or_else(|| v.get("properties"))?;

    if !properties.is_object() {
        return None;
    }

    let mut fields = Vec::new();
    collect_es_fields(properties, "", &mut fields);

    Some(ElasticsearchSchema { fields })
}

/// Recursively collect Elasticsearch fields, flattening nested properties.
fn collect_es_fields(
    properties: &serde_json::Value,
    prefix: &str,
    fields: &mut Vec<ElasticsearchField>,
) {
    let map = match properties.as_object() {
        Some(m) => m,
        None => return,
    };

    for (key, value) in map {
        let full_name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", prefix, key)
        };

        let field_type = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("object")
            .to_string();

        fields.push(ElasticsearchField {
            name: full_name.clone(),
            field_type,
        });

        // Recurse into nested properties.
        if let Some(nested) = value.get("properties") {
            collect_es_fields(nested, &full_name, fields);
        }
    }
}

// ---------------------------------------------------------------------------
// Task 7 – Prisma schema enrichment
// ---------------------------------------------------------------------------

/// Scalar types built into Prisma – anything else is a relation model.
const PRISMA_SCALAR_TYPES: &[&str] = &[
    "String",
    "Boolean",
    "Int",
    "BigInt",
    "Float",
    "Decimal",
    "DateTime",
    "Json",
    "Bytes",
    "Unsupported",
];

/// Parse a Prisma model body and return an `OrmModelSchema`.
pub fn extract_prisma_schema(
    body: &str,
    model_name: &str,
    file_path: &str,
    start_line: usize,
) -> OrmModelSchema {
    // Determine table name: check for @@map("...") directive.
    let map_re = Regex::new(r#"@@map\("([^"]+)"\)"#).unwrap();
    let table_name = if let Some(cap) = map_re.captures(body) {
        cap[1].to_string()
    } else {
        model_name.to_lowercase()
    };

    let mut fields: Vec<OrmFieldSchema> = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("@@") {
            continue;
        }

        // Each field line: `name  Type  [@modifiers ...]`
        // Use split_whitespace to handle multiple spaces between tokens.
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 2 {
            continue;
        }

        let field_name = tokens[0];
        let raw_type = tokens[1];
        // Everything from token 2 onwards is modifiers / annotations.
        let modifiers = if tokens.len() > 2 {
            tokens[2..].join(" ")
        } else {
            String::new()
        };

        // Strip `?` (optional) and `[]` (array/relation) suffixes for analysis.
        let is_array = raw_type.ends_with("[]");
        let base_type = raw_type.trim_end_matches('?').trim_end_matches("[]");

        // Determine if it's a relation.
        let is_scalar = PRISMA_SCALAR_TYPES.contains(&base_type);
        let is_relation =
            is_array || (!is_scalar && base_type.chars().next().is_some_and(|c| c.is_uppercase()));

        let related_model = if is_relation {
            Some(base_type.to_string())
        } else {
            None
        };

        // Check for @id, @unique, @default, @relation.
        let mut constraints: Vec<String> = Vec::new();
        if modifiers.contains("@id") {
            constraints.push("PRIMARY KEY".to_string());
        }
        if modifiers.contains("@unique") {
            constraints.push("UNIQUE".to_string());
        }
        // @default(...) – presence is all we need for OrmFieldSchema.
        // @relation(...) – already captured via is_relation.

        fields.push(OrmFieldSchema {
            name: field_name.to_string(),
            field_type: raw_type.to_string(),
            is_relation,
            related_model,
        });
    }

    // Suppress the unused start_line warning.
    let _ = start_line;

    OrmModelSchema {
        class_name: model_name.to_string(),
        table_name,
        framework: OrmFramework::Prisma,
        file_path: file_path.to_string(),
        fields,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Task 5 – SQL column-level extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_basic_columns() {
        let sql = "(id INTEGER, name TEXT)";
        let schema = extract_table_schema(sql, "users", "schema.sql", 1);
        assert_eq!(schema.name, "users");
        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(schema.columns[0].data_type, "INTEGER");
        assert_eq!(schema.columns[1].name, "name");
        assert_eq!(schema.columns[1].data_type, "TEXT");
    }

    #[test]
    fn test_inline_primary_key() {
        let sql = "(id SERIAL PRIMARY KEY, name TEXT)";
        let schema = extract_table_schema(sql, "users", "schema.sql", 1);
        assert_eq!(schema.primary_key, Some(vec!["id".to_string()]));
        // pk column is implicitly not nullable
        let id_col = schema.columns.iter().find(|c| c.name == "id").unwrap();
        assert!(!id_col.nullable);
    }

    #[test]
    fn test_table_level_primary_key() {
        let sql = "(id INTEGER, name TEXT, PRIMARY KEY (id))";
        let schema = extract_table_schema(sql, "orders", "schema.sql", 1);
        assert_eq!(schema.primary_key, Some(vec!["id".to_string()]));
    }

    #[test]
    fn test_table_level_composite_pk() {
        let sql = "(user_id INTEGER, role TEXT, PRIMARY KEY (user_id, role))";
        let schema = extract_table_schema(sql, "user_roles", "schema.sql", 1);
        assert_eq!(
            schema.primary_key,
            Some(vec!["user_id".to_string(), "role".to_string()])
        );
    }

    #[test]
    fn test_inline_foreign_key_with_column() {
        let sql = "(id INTEGER, user_id INTEGER REFERENCES users(id))";
        let schema = extract_table_schema(sql, "orders", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_inline_foreign_key_without_column() {
        let sql = "(id INTEGER, user_id INTEGER REFERENCES users)";
        let schema = extract_table_schema(sql, "orders", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "");
    }

    #[test]
    fn test_table_level_foreign_key() {
        let sql = "(id INTEGER, user_id INTEGER, FOREIGN KEY (user_id) REFERENCES users(id))";
        let schema = extract_table_schema(sql, "orders", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_not_null_and_unique() {
        let sql = "(id INTEGER NOT NULL, email TEXT NOT NULL UNIQUE)";
        let schema = extract_table_schema(sql, "users", "schema.sql", 1);
        let id_col = schema.columns.iter().find(|c| c.name == "id").unwrap();
        assert!(!id_col.nullable);
        let email_col = schema.columns.iter().find(|c| c.name == "email").unwrap();
        assert!(!email_col.nullable);
        assert!(email_col.constraints.contains(&"UNIQUE".to_string()));
    }

    #[test]
    fn test_default_value() {
        let sql = "(status TEXT DEFAULT 'active', count INTEGER DEFAULT 0)";
        let schema = extract_table_schema(sql, "items", "schema.sql", 1);
        let status_col = schema.columns.iter().find(|c| c.name == "status").unwrap();
        assert_eq!(status_col.default, Some("'active'".to_string()));
        let count_col = schema.columns.iter().find(|c| c.name == "count").unwrap();
        assert_eq!(count_col.default, Some("0".to_string()));
    }

    #[test]
    fn test_postgresql_types() {
        let sql = "(data JSONB, tags TEXT[], seq SERIAL)";
        let schema = extract_table_schema(sql, "items", "schema.sql", 1);
        let data_col = schema.columns.iter().find(|c| c.name == "data").unwrap();
        assert_eq!(data_col.data_type, "JSONB");
        let tags_col = schema.columns.iter().find(|c| c.name == "tags").unwrap();
        assert_eq!(tags_col.data_type, "TEXT[]");
    }

    #[test]
    fn test_multiword_type_timestamp() {
        let sql = "(created_at TIMESTAMP WITH TIME ZONE NOT NULL)";
        let schema = extract_table_schema(sql, "events", "schema.sql", 1);
        let col = schema
            .columns
            .iter()
            .find(|c| c.name == "created_at")
            .unwrap();
        assert_eq!(col.data_type, "TIMESTAMP WITH TIME ZONE");
        assert!(!col.nullable);
    }

    #[test]
    fn test_multiword_type_double_precision() {
        let sql = "(amount DOUBLE PRECISION)";
        let schema = extract_table_schema(sql, "payments", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "amount").unwrap();
        assert_eq!(col.data_type, "DOUBLE PRECISION");
    }

    #[test]
    fn test_type_with_parens_decimal() {
        let sql = "(price DECIMAL(10,2) NOT NULL)";
        let schema = extract_table_schema(sql, "products", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "price").unwrap();
        assert_eq!(col.data_type, "DECIMAL(10,2)");
        assert!(!col.nullable);
    }

    #[test]
    fn test_type_with_parens_varchar() {
        let sql = "(name VARCHAR(255) NOT NULL)";
        let schema = extract_table_schema(sql, "users", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "name").unwrap();
        assert_eq!(col.data_type, "VARCHAR(255)");
        assert!(!col.nullable);
    }

    #[test]
    fn test_references_with_on_delete() {
        let sql = "(id INTEGER, owner_id INTEGER REFERENCES users(id) ON DELETE CASCADE)";
        let schema = extract_table_schema(sql, "docs", "schema.sql", 1);
        let col = schema
            .columns
            .iter()
            .find(|c| c.name == "owner_id")
            .unwrap();
        let fk = col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_quoted_identifiers() {
        let sql = r#"("id" INTEGER, "user_name" TEXT)"#;
        let schema = extract_table_schema(sql, "users", "schema.sql", 1);
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(schema.columns[1].name, "user_name");
    }

    #[test]
    fn test_table_level_check_skipped() {
        let sql = "(id INTEGER, age INTEGER, CHECK (age > 0))";
        let schema = extract_table_schema(sql, "people", "schema.sql", 1);
        // CHECK constraint definition should not be parsed as a column.
        assert_eq!(schema.columns.len(), 2);
    }

    #[test]
    fn test_schema_qualified_reference() {
        // Schema prefix must be stripped so FKs resolve against the bare table name.
        let sql = "(id INTEGER, user_id INTEGER REFERENCES public.users(id))";
        let schema = extract_table_schema(sql, "orders", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = col.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn test_empty_table() {
        let schema = extract_table_schema("", "empty", "schema.sql", 1);
        assert_eq!(schema.columns.len(), 0);
        assert!(schema.primary_key.is_none());
    }

    #[test]
    fn test_mysql_auto_increment() {
        let sql = "(id INTEGER AUTO_INCREMENT PRIMARY KEY, name TEXT)";
        let schema = extract_table_schema(sql, "items", "schema.sql", 1);
        let col = schema.columns.iter().find(|c| c.name == "id").unwrap();
        // AUTO_INCREMENT stops type collection; PRIMARY KEY handled.
        assert_eq!(col.data_type, "INTEGER");
        assert!(schema.primary_key.is_some());
    }

    #[test]
    fn test_view_extraction() {
        let sql = "SELECT u.id, o.total FROM users u JOIN orders o ON u.id = o.user_id";
        let view = extract_view_schema(sql, "user_orders", "schema.sql");
        assert!(view.source_tables.contains(&"users".to_string()));
        assert!(view.source_tables.contains(&"orders".to_string()));
    }

    #[test]
    fn test_function_extraction() {
        let body = "BEGIN SELECT * FROM orders; UPDATE inventory SET count = count - 1; END";
        let func = extract_function_schema(body, "process_order", "funcs.sql");
        assert!(func.referenced_tables.contains(&"orders".to_string()));
        assert!(func.referenced_tables.contains(&"inventory".to_string()));
    }

    #[test]
    fn test_multiple_foreign_keys() {
        let sql = "(id INTEGER PRIMARY KEY, author_id INTEGER REFERENCES users(id), post_id INTEGER REFERENCES posts(id))";
        let schema = extract_table_schema(sql, "comments", "schema.sql", 1);
        let author = schema
            .columns
            .iter()
            .find(|c| c.name == "author_id")
            .unwrap();
        assert_eq!(author.foreign_key.as_ref().unwrap().target_table, "users");
        let post = schema.columns.iter().find(|c| c.name == "post_id").unwrap();
        assert_eq!(post.foreign_key.as_ref().unwrap().target_table, "posts");
    }

    // -----------------------------------------------------------------------
    // Task 6 – CQL, Cypher, Elasticsearch
    // -----------------------------------------------------------------------

    #[test]
    fn test_cql_basic() {
        // CQL uses same SQL syntax; WITH clause ignored.
        let body = "(id UUID PRIMARY KEY, name TEXT, created_at TIMESTAMP) WITH CLUSTERING ORDER BY (created_at DESC)";
        let schema = extract_table_schema(body, "events", "schema.cql", 1);
        assert_eq!(schema.columns.len(), 3);
        assert_eq!(schema.primary_key, Some(vec!["id".to_string()]));
    }

    #[test]
    fn test_cql_with_clause_ignored() {
        // Ensures WITH clause outside the parens does not produce spurious columns.
        let body = "(pk UUID PRIMARY KEY)";
        let schema = extract_table_schema(body, "t", "schema.cql", 1);
        assert_eq!(schema.columns.len(), 1);
    }

    #[test]
    fn test_cypher_constraint() {
        let content = "CREATE CONSTRAINT person_id FOR (p:Person) ASSERT p.id IS UNIQUE;";
        let entries = extract_cypher_schema(content, "schema.cypher");
        let constraint = entries.iter().find(|e| e.entry_type == "constraint");
        assert!(constraint.is_some());
        assert!(constraint.unwrap().contains_label("Person"));
    }

    #[test]
    fn test_cypher_index() {
        let content = "CREATE INDEX movie_title FOR (m:Movie) ON (m.title);";
        let entries = extract_cypher_schema(content, "schema.cypher");
        let index = entries.iter().find(|e| e.entry_type == "index");
        assert!(index.is_some());
        assert!(index.unwrap().contains_label("Movie"));
    }

    #[test]
    fn test_cypher_node_labels() {
        let content = "MATCH (u:User)-[:FOLLOWS]->(other:User) RETURN u;";
        let entries = extract_cypher_schema(content, "schema.cypher");
        let user_entry = entries.iter().find(|e| e.contains_label("User"));
        assert!(user_entry.is_some());
        let rel_entry = entries.iter().find(|e| e.entry_type == "relationship");
        assert!(rel_entry.is_some());
        assert!(rel_entry.unwrap().contains_label("FOLLOWS"));
    }

    #[test]
    fn test_elasticsearch_mappings() {
        let json = r#"{
            "mappings": {
                "properties": {
                    "title": { "type": "text" },
                    "price": { "type": "float" },
                    "in_stock": { "type": "boolean" }
                }
            }
        }"#;
        let schema = extract_elasticsearch_schema(json, "index.json").unwrap();
        assert!(schema
            .fields
            .iter()
            .any(|f| f.name == "title" && f.field_type == "text"));
        assert!(schema
            .fields
            .iter()
            .any(|f| f.name == "price" && f.field_type == "float"));
        assert!(schema
            .fields
            .iter()
            .any(|f| f.name == "in_stock" && f.field_type == "boolean"));
    }

    #[test]
    fn test_elasticsearch_nested_properties() {
        let json = r#"{
            "mappings": {
                "properties": {
                    "address": {
                        "type": "object",
                        "properties": {
                            "city": { "type": "keyword" },
                            "zip": { "type": "keyword" }
                        }
                    }
                }
            }
        }"#;
        let schema = extract_elasticsearch_schema(json, "index.json").unwrap();
        assert!(schema.fields.iter().any(|f| f.name == "address"));
        assert!(schema
            .fields
            .iter()
            .any(|f| f.name == "address.city" && f.field_type == "keyword"));
        assert!(schema.fields.iter().any(|f| f.name == "address.zip"));
    }

    #[test]
    fn test_non_es_json() {
        let json = r#"{"name": "John", "age": 30}"#;
        let result = extract_elasticsearch_schema(json, "data.json");
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Task 7 – Prisma schema enrichment
    // -----------------------------------------------------------------------

    #[test]
    fn test_prisma_basic_model() {
        let body = r#"
  id    Int     @id @default(autoincrement())
  email String  @unique
  name  String?
"#;
        let model = extract_prisma_schema(body, "User", "schema.prisma", 1);
        assert_eq!(model.class_name, "User");
        assert_eq!(model.table_name, "user");
        let id_field = model.fields.iter().find(|f| f.name == "id").unwrap();
        assert!(id_field.field_type.contains("Int"));
        assert!(!id_field.is_relation);
        let email_field = model.fields.iter().find(|f| f.name == "email").unwrap();
        assert!(email_field.field_type.contains("String"));
    }

    #[test]
    fn test_prisma_map_override() {
        let body = r#"
  id   Int    @id
  name String
  @@map("my_users")
"#;
        let model = extract_prisma_schema(body, "User", "schema.prisma", 1);
        assert_eq!(model.table_name, "my_users");
    }

    #[test]
    fn test_prisma_relation_field() {
        let body = r#"
  id      Int      @id
  posts   Post[]   @relation("UserPosts")
  profile Profile? @relation("UserProfile")
"#;
        let model = extract_prisma_schema(body, "User", "schema.prisma", 1);
        let posts = model.fields.iter().find(|f| f.name == "posts").unwrap();
        assert!(posts.is_relation);
        assert_eq!(posts.related_model, Some("Post".to_string()));
        let profile = model.fields.iter().find(|f| f.name == "profile").unwrap();
        assert!(profile.is_relation);
        assert_eq!(profile.related_model, Some("Profile".to_string()));
    }

    #[test]
    fn fk_with_schema_prefix_is_stripped() {
        // A REFERENCES clause pointing to `public.users` must resolve to `users`.
        let sql = r#"
CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INT REFERENCES public.users(id)
);"#;
        let table = extract_table_schema(sql, "orders", "migrations/001.sql", 1);
        let col = table
            .columns
            .iter()
            .find(|c| c.name == "user_id")
            .expect("user_id column must exist");
        let fk = col.foreign_key.as_ref().expect("user_id must have FK");
        assert_eq!(
            fk.target_table, "users",
            "schema prefix must be stripped: got {:?}",
            fk.target_table
        );
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn view_schema_strips_schema_prefix() {
        let body = "SELECT u.id FROM public.users u JOIN auth.roles r ON u.role_id = r.id";
        let view = extract_view_schema(body, "user_roles", "views.sql");
        assert!(
            view.source_tables.contains(&"users".to_string()),
            "source_tables must contain stripped 'users', got {:?}",
            view.source_tables
        );
        assert!(
            view.source_tables.contains(&"roles".to_string()),
            "source_tables must contain stripped 'roles', got {:?}",
            view.source_tables
        );
    }

    #[test]
    fn function_schema_strips_schema_prefix() {
        let body = "BEGIN SELECT * FROM app.accounts; UPDATE billing.invoices SET paid = true; END";
        let func = extract_function_schema(body, "process_payment", "funcs.sql");
        assert!(
            func.referenced_tables.contains(&"accounts".to_string()),
            "referenced_tables must contain 'accounts', got {:?}",
            func.referenced_tables
        );
        assert!(
            func.referenced_tables.contains(&"invoices".to_string()),
            "referenced_tables must contain 'invoices', got {:?}",
            func.referenced_tables
        );
    }

    #[test]
    fn strip_schema_prefix_helper() {
        assert_eq!(strip_schema_prefix("public.users"), "users");
        assert_eq!(strip_schema_prefix("users"), "users");
        assert_eq!(strip_schema_prefix("a.b.c"), "c");
        assert_eq!(strip_schema_prefix(""), "");
    }
}
