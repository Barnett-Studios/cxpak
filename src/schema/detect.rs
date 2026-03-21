// ORM pattern matchers, Terraform tagging, migration detection

use crate::index::CodebaseIndex;
use crate::parser::language::SymbolKind;
use crate::schema::{
    MigrationChain, MigrationEntry, MigrationFramework, OrmFieldSchema, OrmFramework,
    OrmModelSchema, SchemaIndex, TableSchema,
};
use regex::Regex;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Task 8: ORM pattern matchers
// ---------------------------------------------------------------------------

/// Detect ORM models across all files in the index.
pub fn detect_orm_models(index: &CodebaseIndex) -> Vec<OrmModelSchema> {
    let mut models = Vec::new();

    for file in &index.files {
        let parse_result = match &file.parse_result {
            Some(pr) => pr,
            None => continue,
        };

        for symbol in &parse_result.symbols {
            // Try each ORM detector in priority order
            if let Some(model) = try_detect_django(symbol, &file.relative_path) {
                models.push(model);
            } else if let Some(model) =
                try_detect_sqlalchemy(symbol, &file.relative_path, &parse_result.imports)
            {
                models.push(model);
            } else if let Some(model) =
                try_detect_typeorm(symbol, &file.relative_path, &file.content)
            {
                models.push(model);
            } else if let Some(model) = try_detect_active_record(symbol, &file.relative_path) {
                models.push(model);
            } else if let Some(model) =
                try_detect_prisma(symbol, &file.relative_path, file.language.as_deref())
            {
                models.push(model);
            }
        }
    }

    models
}

// --- Django ---

fn try_detect_django(
    symbol: &crate::parser::language::Symbol,
    file_path: &str,
) -> Option<OrmModelSchema> {
    if symbol.kind != SymbolKind::Class {
        return None;
    }
    if !symbol.signature.contains("models.Model") {
        return None;
    }

    let table_name = extract_django_table_name(&symbol.body, &symbol.name);
    let fields = extract_django_fields(&symbol.body);

    Some(OrmModelSchema {
        class_name: symbol.name.clone(),
        table_name,
        framework: OrmFramework::Django,
        file_path: file_path.to_string(),
        fields,
    })
}

fn extract_django_table_name(body: &str, class_name: &str) -> String {
    // Look for db_table = "X" or db_table = 'X'
    let re = Regex::new(r#"db_table\s*=\s*["']([^"']+)["']"#).unwrap();
    if let Some(cap) = re.captures(body) {
        return cap[1].to_string();
    }
    class_name.to_lowercase()
}

fn extract_django_fields(body: &str) -> Vec<OrmFieldSchema> {
    let mut fields = Vec::new();
    // Match: name = models.FieldType(...)
    let field_re = Regex::new(r"(\w+)\s*=\s*models\.(\w+)\(([^)]*)\)").unwrap();

    for cap in field_re.captures_iter(body) {
        let name = cap[1].to_string();
        let field_type = cap[2].to_string();
        let args = cap[3].to_string();

        // Skip Meta class attributes that happen to match
        if name == "db_table" || name == "ordering" || name == "verbose_name" {
            continue;
        }

        let is_relation = field_type == "ForeignKey"
            || field_type == "ManyToManyField"
            || field_type == "OneToOneField";

        let related_model = if is_relation {
            // First positional argument is the related model
            args.split(',')
                .next()
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty() && !s.starts_with("on_delete") && !s.starts_with("to="))
        } else {
            None
        };

        fields.push(OrmFieldSchema {
            name,
            field_type,
            is_relation,
            related_model,
        });
    }

    fields
}

// --- SQLAlchemy ---

fn try_detect_sqlalchemy(
    symbol: &crate::parser::language::Symbol,
    file_path: &str,
    imports: &[crate::parser::language::Import],
) -> Option<OrmModelSchema> {
    if symbol.kind != SymbolKind::Class {
        return None;
    }

    // Must have (Base) or (DeclarativeBase) in signature
    let sig = &symbol.signature;
    if !sig.contains("(Base)") && !sig.contains("(DeclarativeBase)") {
        return None;
    }

    // CRITICAL import guard: file must import from sqlalchemy
    let has_sqlalchemy_import = imports
        .iter()
        .any(|i| i.source.to_lowercase().contains("sqlalchemy"));
    if !has_sqlalchemy_import {
        return None;
    }

    let table_name = extract_sqlalchemy_table_name(&symbol.body, &symbol.name);
    let fields = extract_sqlalchemy_fields(&symbol.body);

    Some(OrmModelSchema {
        class_name: symbol.name.clone(),
        table_name,
        framework: OrmFramework::SqlAlchemy,
        file_path: file_path.to_string(),
        fields,
    })
}

fn extract_sqlalchemy_table_name(body: &str, class_name: &str) -> String {
    // Look for __tablename__ = "X" or __tablename__ = 'X'
    let re = Regex::new(r#"__tablename__\s*=\s*["']([^"']+)["']"#).unwrap();
    if let Some(cap) = re.captures(body) {
        return cap[1].to_string();
    }
    class_name.to_lowercase()
}

fn extract_sqlalchemy_fields(body: &str) -> Vec<OrmFieldSchema> {
    let mut fields = Vec::new();
    // Match: name = Column(Type, ...)
    let col_re = Regex::new(r"(\w+)\s*=\s*Column\(([^)]*)\)").unwrap();
    let fk_re = Regex::new(r#"ForeignKey\(["']([^"'.]+)\."#).unwrap();

    for cap in col_re.captures_iter(body) {
        let name = cap[1].to_string();
        let args = cap[2].to_string();

        // First arg is the column type
        let field_type = args
            .split(',')
            .next()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        // Check for ForeignKey
        let is_relation = args.contains("ForeignKey(");
        let related_model = if is_relation {
            // Extract table from ForeignKey("table.col")
            fk_re.captures(&args).map(|c| c[1].to_string())
        } else {
            None
        };

        fields.push(OrmFieldSchema {
            name,
            field_type,
            is_relation,
            related_model,
        });
    }

    fields
}

// --- TypeORM ---

/// TypeORM member decorators that signal an ORM entity field
const TYPEORM_MEMBER_DECORATORS: &[&str] = &[
    "@Column",
    "@PrimaryColumn",
    "@PrimaryGeneratedColumn",
    "@ManyToOne",
    "@OneToMany",
    "@ManyToMany",
    "@OneToOne",
];

fn try_detect_typeorm(
    symbol: &crate::parser::language::Symbol,
    file_path: &str,
    file_content: &str,
) -> Option<OrmModelSchema> {
    if symbol.kind != SymbolKind::Class {
        return None;
    }

    // Detect via member decorators in body
    let has_typeorm_decorator = TYPEORM_MEMBER_DECORATORS
        .iter()
        .any(|d| symbol.body.contains(d));
    if !has_typeorm_decorator {
        return None;
    }

    let table_name = extract_typeorm_table_name(file_content, &symbol.name);
    let fields = extract_typeorm_fields(&symbol.body);

    Some(OrmModelSchema {
        class_name: symbol.name.clone(),
        table_name,
        framework: OrmFramework::TypeOrm,
        file_path: file_path.to_string(),
        fields,
    })
}

fn extract_typeorm_table_name(file_content: &str, class_name: &str) -> String {
    // Scan file content for @Entity("X") or @Entity('X')
    let re = Regex::new(r#"@Entity\(["']([^"']+)["']\)"#).unwrap();
    if let Some(cap) = re.captures(file_content) {
        return cap[1].to_string();
    }
    class_name.to_lowercase()
}

fn extract_typeorm_fields(body: &str) -> Vec<OrmFieldSchema> {
    let mut fields = Vec::new();

    // Relation decorators
    let relation_decorators = ["@ManyToOne", "@OneToMany", "@ManyToMany", "@OneToOne"];

    // Match decorator + field declaration pattern
    // e.g.: @Column() name: string
    //        @ManyToOne(() => User, ...) user: User
    let field_re = Regex::new(r"@(\w+)\([^)]*\)\s+(\w+)\s*:\s*(\w+)").unwrap();

    for cap in field_re.captures_iter(body) {
        let decorator = format!("@{}", &cap[1]);
        let name = cap[2].to_string();
        let is_relation = relation_decorators.contains(&decorator.as_str());

        fields.push(OrmFieldSchema {
            name,
            field_type: decorator[1..].to_string(), // strip @
            is_relation,
            related_model: None,
        });
    }

    fields
}

// --- ActiveRecord ---

fn try_detect_active_record(
    symbol: &crate::parser::language::Symbol,
    file_path: &str,
) -> Option<OrmModelSchema> {
    if symbol.kind != SymbolKind::Class {
        return None;
    }

    let sig = &symbol.signature;
    if !sig.contains("< ActiveRecord::Base") && !sig.contains("< ApplicationRecord") {
        return None;
    }

    let table_name = pluralize(&symbol.name);
    let fields = Vec::new(); // ActiveRecord uses convention; fields discovered at runtime

    Some(OrmModelSchema {
        class_name: symbol.name.clone(),
        table_name,
        framework: OrmFramework::ActiveRecord,
        file_path: file_path.to_string(),
        fields,
    })
}

fn pluralize(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with("ss")
        || lower.ends_with("sh")
        || lower.ends_with("ch")
        || lower.ends_with('x')
        || lower.ends_with('z')
        || lower.ends_with('s')
    {
        format!("{lower}es")
    } else if lower.ends_with('y')
        && !lower.ends_with("ay")
        && !lower.ends_with("ey")
        && !lower.ends_with("oy")
        && !lower.ends_with("uy")
    {
        format!("{}ies", &lower[..lower.len() - 1])
    } else {
        format!("{lower}s")
    }
}

// --- Prisma ---

fn try_detect_prisma(
    symbol: &crate::parser::language::Symbol,
    file_path: &str,
    language: Option<&str>,
) -> Option<OrmModelSchema> {
    if symbol.kind != SymbolKind::Struct {
        return None;
    }
    if language != Some("prisma") {
        return None;
    }

    let table_name = extract_prisma_table_name(&symbol.body, &symbol.name);
    let fields = Vec::new(); // Fields extracted by extract.rs (Task 7)

    Some(OrmModelSchema {
        class_name: symbol.name.clone(),
        table_name,
        framework: OrmFramework::Prisma,
        file_path: file_path.to_string(),
        fields,
    })
}

fn extract_prisma_table_name(body: &str, model_name: &str) -> String {
    // Look for @@map("X")
    let re = Regex::new(r#"@@map\(["']([^"']+)["']\)"#).unwrap();
    if let Some(cap) = re.captures(body) {
        return cap[1].to_string();
    }
    model_name.to_lowercase()
}

// ---------------------------------------------------------------------------
// Task 9: Terraform tagging
// ---------------------------------------------------------------------------

const DB_RESOURCE_PREFIXES: &[&str] = &[
    "aws_dynamodb_table",
    "aws_rds_",
    "aws_aurora_",
    "aws_elasticache_",
    "aws_elasticsearch_",
    "aws_opensearch_",
    "google_sql_",
    "google_bigquery_",
    "google_bigtable_",
    "google_datastore_",
    "google_firestore_",
    "azurerm_cosmosdb_",
    "azurerm_mssql_",
    "azurerm_postgresql_",
    "azurerm_mysql_",
    "azurerm_redis_",
    "mongodbatlas_cluster",
];

/// Detect Terraform database resources and add them to the schema index as TableSchema entries.
pub fn detect_terraform_schemas(index: &CodebaseIndex, schema: &mut SchemaIndex) {
    for file in &index.files {
        // Only process HCL files
        if file.language.as_deref() != Some("hcl") {
            continue;
        }

        let parse_result = match &file.parse_result {
            Some(pr) => pr,
            None => continue,
        };

        for symbol in &parse_result.symbols {
            // Check if the symbol name starts with any DB resource prefix
            let is_db_resource = DB_RESOURCE_PREFIXES
                .iter()
                .any(|prefix| symbol.name.starts_with(prefix));

            if is_db_resource {
                let table_schema = TableSchema {
                    name: symbol.name.clone(),
                    columns: Vec::new(),
                    primary_key: None,
                    indexes: Vec::new(),
                    file_path: file.relative_path.clone(),
                    start_line: symbol.start_line,
                };
                schema.tables.insert(symbol.name.clone(), table_schema);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Task 10: Migration detection
// ---------------------------------------------------------------------------

/// Detect migration chains across all files in the index.
pub fn detect_migrations(index: &CodebaseIndex) -> Vec<MigrationChain> {
    // Group files by directory
    let mut dir_groups: HashMap<String, Vec<&crate::index::IndexedFile>> = HashMap::new();
    for file in &index.files {
        let dir = parent_dir(&file.relative_path);
        dir_groups.entry(dir).or_default().push(file);
    }

    let mut chains = Vec::new();

    for (dir, files) in &dir_groups {
        // Try framework-specific patterns in priority order
        if let Some(chain) = try_rails_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_alembic_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_flyway_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_django_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_knex_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_prisma_migrations(dir, files, &dir_groups) {
            chains.push(chain);
        } else if let Some(chain) = try_drizzle_migrations(dir, files) {
            chains.push(chain);
        } else if let Some(chain) = try_generic_migrations(dir, files) {
            chains.push(chain);
        }
    }

    chains.sort_by(|a, b| a.directory.cmp(&b.directory));
    chains
}

fn parent_dir(path: &str) -> String {
    if let Some(pos) = path.rfind('/') {
        path[..pos].to_string()
    } else {
        String::new()
    }
}

fn filename(path: &str) -> &str {
    path.rfind('/').map(|i| &path[i + 1..]).unwrap_or(path)
}

// Rails: db/migrate/ directory, YYYYMMDDHHMMSS_name.rb
fn try_rails_migrations(dir: &str, files: &[&crate::index::IndexedFile]) -> Option<MigrationChain> {
    if !dir.ends_with("db/migrate") && !dir.contains("db/migrate/") {
        return None;
    }

    // Rails migration files end in .rb and have a timestamp prefix
    let ts_re = Regex::new(r"^(\d{14})_(.+)\.rb$").unwrap();
    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = ts_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Rails,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Alembic: alembic/versions/ directory, hash_name.py, reads revision from content
fn try_alembic_migrations(
    dir: &str,
    files: &[&crate::index::IndexedFile],
) -> Option<MigrationChain> {
    if !dir.ends_with("alembic/versions") && !dir.contains("alembic/versions") {
        return None;
    }

    let revision_re = Regex::new(r#"revision\s*=\s*["']([^"']+)["']"#).unwrap();
    let fname_re = Regex::new(r"^([a-f0-9_]+)\.py$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            if !fname.ends_with(".py") {
                return None;
            }
            // Try to read revision from content
            let sequence = if let Some(cap) = revision_re.captures(&f.content) {
                cap[1].to_string()
            } else if let Some(cap) = fname_re.captures(fname) {
                cap[1].to_string()
            } else {
                return None;
            };
            // Name is the part after the first underscore in filename
            let stem = fname.trim_end_matches(".py");
            let name = stem
                .split_once('_')
                .map(|x| x.1)
                .unwrap_or(stem)
                .to_string();
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence,
                name,
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Alembic,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Flyway: any directory, V{N}__name.sql
fn try_flyway_migrations(
    dir: &str,
    files: &[&crate::index::IndexedFile],
) -> Option<MigrationChain> {
    let flyway_re = Regex::new(r"^V(\d+(?:\.\d+)?)__(.+)\.sql$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = flyway_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    // Sort by numeric version
    entries.sort_by(|a, b| {
        let parse_version = |s: &str| -> f64 { s.parse().unwrap_or(0.0) };
        parse_version(&a.sequence)
            .partial_cmp(&parse_version(&b.sequence))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Some(MigrationChain {
        framework: MigrationFramework::Flyway,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Django: */migrations/ directory, NNNN_name.py
fn try_django_migrations(
    dir: &str,
    files: &[&crate::index::IndexedFile],
) -> Option<MigrationChain> {
    if !dir.ends_with("/migrations") && !dir.ends_with("migrations") {
        return None;
    }

    let django_re = Regex::new(r"^(\d{4})_(.+)\.py$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = django_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Django,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Knex: migrations/ directory, YYYYMMDDHHMMSS_name.js/.ts
fn try_knex_migrations(dir: &str, files: &[&crate::index::IndexedFile]) -> Option<MigrationChain> {
    if !dir.ends_with("migrations") {
        return None;
    }

    let knex_re = Regex::new(r"^(\d{14})_(.+)\.(js|ts)$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = knex_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Knex,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Prisma: prisma/migrations/ directory, YYYYMMDDHHMMSS_name/migration.sql
// NOTE: The file would be in prisma/migrations/TIMESTAMP_name/ directory,
//       so the file path would be prisma/migrations/TIMESTAMP_name/migration.sql
//       The "directory" for this file is prisma/migrations/TIMESTAMP_name
//       We need to group by the parent of parent (prisma/migrations)
fn try_prisma_migrations(
    dir: &str,
    _files: &[&crate::index::IndexedFile],
    all_dirs: &HashMap<String, Vec<&crate::index::IndexedFile>>,
) -> Option<MigrationChain> {
    // This function is called with dir = "prisma/migrations/TIMESTAMP_name"
    // We check: does this dir match prisma/migrations/{timestamp}_{name}?
    let prisma_dir_re = Regex::new(r"^(.+/prisma/migrations)/(\d{14})_(.+)$").unwrap();
    let cap = prisma_dir_re.captures(dir)?;

    let base_migrations_dir = cap[1].to_string();
    let timestamp = cap[2].to_string();
    let migration_name = cap[3].to_string();

    // Check if there's a migration.sql in this directory
    let files_in_dir = all_dirs.get(dir)?;
    let has_migration_sql = files_in_dir
        .iter()
        .any(|f| filename(&f.relative_path) == "migration.sql");

    if !has_migration_sql {
        return None;
    }

    // Find the migration.sql file
    let migration_file = files_in_dir
        .iter()
        .find(|f| filename(&f.relative_path) == "migration.sql")?;

    // We want to build a chain for the entire prisma/migrations directory,
    // but we're called once per sub-directory. To avoid duplicates, only
    // process when this is the "first" sub-directory alphabetically for the base.
    // Collect all sub-directories that match this base_migrations_dir.
    let mut all_entries: Vec<MigrationEntry> = Vec::new();

    for (other_dir, other_files) in all_dirs {
        if let Some(other_cap) = prisma_dir_re.captures(other_dir) {
            if other_cap[1] == base_migrations_dir {
                let other_ts = other_cap[2].to_string();
                let other_name = other_cap[3].to_string();
                if let Some(sql_file) = other_files
                    .iter()
                    .find(|f| filename(&f.relative_path) == "migration.sql")
                {
                    all_entries.push(MigrationEntry {
                        file_path: sql_file.relative_path.clone(),
                        sequence: other_ts,
                        name: other_name,
                    });
                }
            }
        }
    }

    // Only emit the chain from the "canonical" (first alphabetically) subdirectory
    // to avoid duplicates. Current dir must be the lexicographic minimum.
    let min_dir = all_dirs
        .keys()
        .filter(|k| {
            prisma_dir_re
                .captures(k)
                .map(|c| c[1] == *base_migrations_dir)
                .unwrap_or(false)
        })
        .min()
        .cloned();

    if min_dir.as_deref() != Some(dir) {
        return None;
    }

    // Use the current entry if all_entries is empty (shouldn't happen at this point)
    if all_entries.is_empty() {
        all_entries.push(MigrationEntry {
            file_path: migration_file.relative_path.clone(),
            sequence: timestamp,
            name: migration_name,
        });
    }

    all_entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Prisma,
        directory: base_migrations_dir,
        migrations: all_entries,
    })
}

// Drizzle: drizzle/ directory, NNNN_name.sql
fn try_drizzle_migrations(
    dir: &str,
    files: &[&crate::index::IndexedFile],
) -> Option<MigrationChain> {
    if !dir.ends_with("drizzle") && !dir.contains("/drizzle/") {
        return None;
    }

    let drizzle_re = Regex::new(r"^(\d{4})_(.+)\.sql$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = drizzle_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    entries.sort_by(|a, b| a.sequence.cmp(&b.sequence));

    Some(MigrationChain {
        framework: MigrationFramework::Drizzle,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// Generic: any dir with 3+ sequenced SQL files, NNN_name.sql or YYYYMMDDHHMMSS_name.sql
fn try_generic_migrations(
    dir: &str,
    files: &[&crate::index::IndexedFile],
) -> Option<MigrationChain> {
    // Match numeric prefix + underscore + name + .sql
    let generic_re = Regex::new(r"^(\d+)_(.+)\.sql$").unwrap();

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = generic_re.captures(fname)?;
            Some(MigrationEntry {
                file_path: f.relative_path.clone(),
                sequence: cap[1].to_string(),
                name: cap[2].to_string(),
            })
        })
        .collect();

    // Require at least 3 sequenced files for a generic chain
    if entries.len() < 3 {
        return None;
    }

    entries.sort_by(|a, b| {
        // Sort numerically
        let a_num: u64 = a.sequence.parse().unwrap_or(0);
        let b_num: u64 = b.sequence.parse().unwrap_or(0);
        a_num.cmp(&b_num)
    });

    Some(MigrationChain {
        framework: MigrationFramework::Generic,
        directory: dir.to_string(),
        migrations: entries,
    })
}

// ---------------------------------------------------------------------------
// Task 11: SchemaIndex orchestrator
// ---------------------------------------------------------------------------

/// Build a `SchemaIndex` by orchestrating all detection passes over the
/// provided `CodebaseIndex`.  Returns `None` when nothing schema-related is
/// found so callers can keep `index.schema = None` for plain code repos.
pub fn build_schema_index(
    index: &crate::index::CodebaseIndex,
) -> Option<crate::schema::SchemaIndex> {
    let mut schema = crate::schema::SchemaIndex::empty();

    // 1. Extract SQL schemas from SQL/CQL files
    for file in &index.files {
        let lang = file.language.as_deref().unwrap_or("");
        if lang == "sql" || file.relative_path.ends_with(".cql") {
            if let Some(pr) = &file.parse_result {
                for symbol in &pr.symbols {
                    match symbol.kind {
                        crate::parser::language::SymbolKind::Table => {
                            let table = crate::schema::extract::extract_table_schema(
                                &symbol.body,
                                &symbol.name,
                                &file.relative_path,
                                symbol.start_line,
                            );
                            schema.tables.insert(symbol.name.clone(), table);
                        }
                        crate::parser::language::SymbolKind::Query => {
                            let view = crate::schema::extract::extract_view_schema(
                                &symbol.body,
                                &symbol.name,
                                &file.relative_path,
                            );
                            schema.views.insert(symbol.name.clone(), view);
                        }
                        crate::parser::language::SymbolKind::Function => {
                            let func = crate::schema::extract::extract_function_schema(
                                &symbol.body,
                                &symbol.name,
                                &file.relative_path,
                            );
                            schema.functions.insert(symbol.name.clone(), func);
                        }
                        _ => {}
                    }
                }
            }
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

    // 5. Enrich Prisma models with field detail from extract
    for file in &index.files {
        if file.language.as_deref() == Some("prisma") {
            if let Some(pr) = &file.parse_result {
                for symbol in &pr.symbols {
                    if symbol.kind == crate::parser::language::SymbolKind::Struct {
                        let enriched = crate::schema::extract::extract_prisma_schema(
                            &symbol.body,
                            &symbol.name,
                            &file.relative_path,
                            symbol.start_line,
                        );
                        // Merge: update existing ORM entry with enriched fields
                        if let Some(existing) = schema.orm_models.get_mut(&symbol.name) {
                            existing.fields = enriched.fields;
                        } else {
                            schema.orm_models.insert(symbol.name.clone(), enriched);
                        }
                    }
                }
            }
        }
    }

    if schema.is_empty() {
        None
    } else {
        Some(schema)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexedFile;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};

    // Helper: build a minimal IndexedFile with a given parse result
    fn make_file(
        path: &str,
        language: Option<&str>,
        content: &str,
        symbols: Vec<Symbol>,
        imports: Vec<Import>,
    ) -> IndexedFile {
        IndexedFile {
            relative_path: path.to_string(),
            language: language.map(|s| s.to_string()),
            size_bytes: content.len() as u64,
            token_count: 0,
            parse_result: Some(ParseResult {
                symbols,
                imports,
                exports: vec![],
            }),
            content: content.to_string(),
        }
    }

    // Helper: build a CodebaseIndex from a list of IndexedFile (no disk access)
    fn make_index(files: Vec<IndexedFile>) -> CodebaseIndex {
        use std::collections::{HashMap, HashSet};
        CodebaseIndex {
            total_files: files.len(),
            total_bytes: files.iter().map(|f| f.size_bytes).sum(),
            total_tokens: 0,
            language_stats: HashMap::new(),
            term_frequencies: HashMap::new(),
            domains: HashSet::new(),
            schema: None,
            files,
        }
    }

    fn make_symbol(name: &str, kind: SymbolKind, signature: &str, body: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            visibility: Visibility::Public,
            signature: signature.to_string(),
            body: body.to_string(),
            start_line: 1,
            end_line: 10,
        }
    }

    // -------------------------------------------------------------------------
    // Task 8: ORM detection tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_django_model_detected() {
        let sym = make_symbol(
            "User",
            SymbolKind::Class,
            "class User(models.Model)",
            "    name = models.CharField(max_length=100)\n    age = models.IntegerField()\n",
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.class_name, "User");
        assert_eq!(m.table_name, "user");
        assert!(matches!(m.framework, OrmFramework::Django));
        assert_eq!(m.fields.len(), 2);
    }

    #[test]
    fn test_django_db_table_override() {
        let sym = make_symbol(
            "UserProfile",
            SymbolKind::Class,
            "class UserProfile(models.Model)",
            r#"
    name = models.CharField(max_length=100)
    class Meta:
        db_table = "custom_users"
"#,
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].table_name, "custom_users");
    }

    #[test]
    fn test_sqlalchemy_detected_with_import_guard() {
        let sym = make_symbol(
            "Product",
            SymbolKind::Class,
            "class Product(Base)",
            r#"
    __tablename__ = "products"
    id = Column(Integer, primary_key=True)
    name = Column(String)
"#,
        );
        let imports = vec![Import {
            source: "sqlalchemy".to_string(),
            names: vec!["Column".to_string(), "Integer".to_string()],
        }];
        let file = make_file("app/models.py", Some("python"), "", vec![sym], imports);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "Product");
        assert_eq!(models[0].table_name, "products");
        assert!(matches!(models[0].framework, OrmFramework::SqlAlchemy));
    }

    #[test]
    fn test_sqlalchemy_default_name() {
        let sym = make_symbol(
            "OrderItem",
            SymbolKind::Class,
            "class OrderItem(Base)",
            "    id = Column(Integer)\n",
        );
        let imports = vec![Import {
            source: "sqlalchemy.orm".to_string(),
            names: vec!["declarative_base".to_string()],
        }];
        let file = make_file("models.py", Some("python"), "", vec![sym], imports);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].table_name, "orderitem");
    }

    #[test]
    fn test_sqlalchemy_false_positive_without_import() {
        // Same class/signature but NO sqlalchemy import — must NOT be detected
        let sym = make_symbol(
            "SomeModel",
            SymbolKind::Class,
            "class SomeModel(Base)",
            "    pass\n",
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert!(
            models.is_empty(),
            "should not detect without sqlalchemy import"
        );
    }

    #[test]
    fn test_typeorm_detected_via_member_decorators() {
        let sym = make_symbol(
            "Order",
            SymbolKind::Class,
            "class Order",
            r#"
    @PrimaryGeneratedColumn()
    id: number
    @Column()
    total: number
"#,
        );
        let content = "import { Entity } from 'typeorm';\n@Entity()\nexport class Order {";
        let file = make_file(
            "src/order.entity.ts",
            Some("typescript"),
            content,
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "Order");
        assert!(matches!(models[0].framework, OrmFramework::TypeOrm));
    }

    #[test]
    fn test_typeorm_entity_name_from_content() {
        let sym = make_symbol(
            "Invoice",
            SymbolKind::Class,
            "class Invoice",
            "    @Column()\n    amount: number\n",
        );
        let content = "@Entity('invoices')\nexport class Invoice {";
        let file = make_file(
            "invoice.entity.ts",
            Some("typescript"),
            content,
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].table_name, "invoices");
    }

    #[test]
    fn test_active_record_detected() {
        let sym = make_symbol(
            "User",
            SymbolKind::Class,
            "class User < ActiveRecord::Base",
            "end\n",
        );
        let file = make_file("app/models/user.rb", Some("ruby"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "User");
        assert_eq!(models[0].table_name, "users");
        assert!(matches!(models[0].framework, OrmFramework::ActiveRecord));
    }

    #[test]
    fn test_active_record_application_record() {
        let sym = make_symbol(
            "Category",
            SymbolKind::Class,
            "class Category < ApplicationRecord",
            "end\n",
        );
        let file = make_file(
            "app/models/category.rb",
            Some("ruby"),
            "",
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "Category");
        assert_eq!(models[0].table_name, "categories");
    }

    #[test]
    fn test_prisma_model_detected() {
        let sym = make_symbol(
            "Post",
            SymbolKind::Struct,
            "model Post",
            "    id   Int    @id\n    title String\n",
        );
        let file = make_file(
            "prisma/schema.prisma",
            Some("prisma"),
            "",
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "Post");
        assert_eq!(models[0].table_name, "post");
        assert!(matches!(models[0].framework, OrmFramework::Prisma));
    }

    #[test]
    fn test_non_orm_class_not_detected() {
        let sym = make_symbol(
            "MyService",
            SymbolKind::Class,
            "class MyService",
            "    def do_stuff(self): pass\n",
        );
        let file = make_file("app/services.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert!(models.is_empty());
    }

    #[test]
    fn test_multiple_models_in_one_file() {
        let sym1 = make_symbol(
            "Author",
            SymbolKind::Class,
            "class Author(models.Model)",
            "    name = models.CharField(max_length=200)\n",
        );
        let sym2 = make_symbol(
            "Book",
            SymbolKind::Class,
            "class Book(models.Model)",
            "    title = models.CharField(max_length=200)\n    author = models.ForeignKey(Author, on_delete=models.CASCADE)\n",
        );
        let file = make_file(
            "app/models.py",
            Some("python"),
            "",
            vec![sym1, sym2],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 2);
        let names: Vec<&str> = models.iter().map(|m| m.class_name.as_str()).collect();
        assert!(names.contains(&"Author"));
        assert!(names.contains(&"Book"));
    }

    #[test]
    fn test_no_orm_patterns_in_plain_file() {
        let sym = make_symbol(
            "Calculator",
            SymbolKind::Class,
            "class Calculator",
            "    def add(self, a, b): return a + b\n",
        );
        let file = make_file("utils/calc.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert!(models.is_empty());
    }

    #[test]
    fn test_pluralize_user() {
        assert_eq!(pluralize("User"), "users");
    }

    #[test]
    fn test_pluralize_category() {
        assert_eq!(pluralize("Category"), "categories");
    }

    #[test]
    fn test_pluralize_address() {
        assert_eq!(pluralize("Address"), "addresses");
    }

    // -------------------------------------------------------------------------
    // Task 9: Terraform tagging tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_terraform_dynamodb_detected() {
        let sym = make_symbol(
            "aws_dynamodb_table.users",
            SymbolKind::Block,
            "resource aws_dynamodb_table users",
            "    hash_key = \"UserId\"\n",
        );
        let file = make_file("infra/main.tf", Some("hcl"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let mut schema = SchemaIndex::empty();
        detect_terraform_schemas(&index, &mut schema);
        assert!(
            schema.tables.contains_key("aws_dynamodb_table.users"),
            "should detect DynamoDB table"
        );
    }

    #[test]
    fn test_terraform_rds_detected() {
        let sym = make_symbol(
            "aws_rds_cluster.main",
            SymbolKind::Block,
            "resource aws_rds_cluster main",
            "    engine = \"aurora-mysql\"\n",
        );
        let file = make_file("infra/rds.tf", Some("hcl"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let mut schema = SchemaIndex::empty();
        detect_terraform_schemas(&index, &mut schema);
        assert!(schema.tables.contains_key("aws_rds_cluster.main"));
    }

    #[test]
    fn test_terraform_non_db_resource_not_detected() {
        let sym = make_symbol(
            "aws_s3_bucket.assets",
            SymbolKind::Block,
            "resource aws_s3_bucket assets",
            "    bucket = \"my-assets\"\n",
        );
        let file = make_file("infra/s3.tf", Some("hcl"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let mut schema = SchemaIndex::empty();
        detect_terraform_schemas(&index, &mut schema);
        assert!(
            !schema.tables.contains_key("aws_s3_bucket.assets"),
            "S3 bucket should not be detected as DB resource"
        );
    }

    // -------------------------------------------------------------------------
    // Task 10: Migration detection tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rails_migrations_detected_and_ordered() {
        let f1 = make_file(
            "db/migrate/20230101120000_create_users.rb",
            Some("ruby"),
            "",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "db/migrate/20230102130000_add_email_to_users.rb",
            Some("ruby"),
            "",
            vec![],
            vec![],
        );
        let f3 = make_file(
            "db/migrate/20230101000000_create_schema.rb",
            Some("ruby"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2, f3]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Rails));
        assert_eq!(chains[0].migrations.len(), 3);
        // Verify ordering
        assert_eq!(chains[0].migrations[0].sequence, "20230101000000");
        assert_eq!(chains[0].migrations[1].sequence, "20230101120000");
        assert_eq!(chains[0].migrations[2].sequence, "20230102130000");
    }

    #[test]
    fn test_django_migrations_detected() {
        let f1 = make_file(
            "myapp/migrations/0001_initial.py",
            Some("python"),
            "",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "myapp/migrations/0002_add_email.py",
            Some("python"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Django));
        assert_eq!(chains[0].migrations.len(), 2);
        assert_eq!(chains[0].migrations[0].name, "initial");
        assert_eq!(chains[0].migrations[1].name, "add_email");
    }

    #[test]
    fn test_flyway_migrations_detected() {
        let f1 = make_file(
            "db/migration/V1__create_tables.sql",
            Some("sql"),
            "",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "db/migration/V2__add_indexes.sql",
            Some("sql"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Flyway));
        assert_eq!(chains[0].migrations[0].sequence, "1");
        assert_eq!(chains[0].migrations[1].sequence, "2");
    }

    #[test]
    fn test_alembic_migrations_reads_revision_from_content() {
        let f1 = make_file(
            "alembic/versions/abc123_create_users.py",
            Some("python"),
            "revision = \"abc123\"\ndown_revision = None\n",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "alembic/versions/def456_add_email.py",
            Some("python"),
            "revision = \"def456\"\ndown_revision = \"abc123\"\n",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Alembic));
        // Sequence is the revision string
        let sequences: Vec<&str> = chains[0]
            .migrations
            .iter()
            .map(|e| e.sequence.as_str())
            .collect();
        assert!(sequences.contains(&"abc123"));
        assert!(sequences.contains(&"def456"));
    }

    #[test]
    fn test_no_migrations_in_plain_repo() {
        let f1 = make_file("src/main.rs", Some("rust"), "fn main() {}", vec![], vec![]);
        let f2 = make_file(
            "src/lib.rs",
            Some("rust"),
            "pub fn foo() {}",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert!(chains.is_empty());
    }

    #[test]
    fn test_mixed_frameworks_detected_separately() {
        let rails1 = make_file(
            "db/migrate/20230101000000_create_users.rb",
            Some("ruby"),
            "",
            vec![],
            vec![],
        );
        let flyway1 = make_file(
            "db/migration/V1__create_tables.sql",
            Some("sql"),
            "",
            vec![],
            vec![],
        );
        let flyway2 = make_file(
            "db/migration/V2__add_indexes.sql",
            Some("sql"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![rails1, flyway1, flyway2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 2);
        let frameworks: Vec<String> = chains
            .iter()
            .map(|c| format!("{:?}", c.framework))
            .collect();
        assert!(frameworks.iter().any(|f| f.contains("Rails")));
        assert!(frameworks.iter().any(|f| f.contains("Flyway")));
    }

    #[test]
    fn test_generic_sql_migrations_detected() {
        let f1 = make_file("db/001_init.sql", Some("sql"), "", vec![], vec![]);
        let f2 = make_file("db/002_add_users.sql", Some("sql"), "", vec![], vec![]);
        let f3 = make_file("db/003_add_orders.sql", Some("sql"), "", vec![], vec![]);
        let index = make_index(vec![f1, f2, f3]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Generic));
        assert_eq!(chains[0].migrations.len(), 3);
        assert_eq!(chains[0].migrations[0].sequence, "001");
        assert_eq!(chains[0].migrations[2].sequence, "003");
    }

    #[test]
    fn test_generic_requires_at_least_3_files() {
        let f1 = make_file("db/001_init.sql", Some("sql"), "", vec![], vec![]);
        let f2 = make_file("db/002_users.sql", Some("sql"), "", vec![], vec![]);
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        // Only 2 files — should NOT emit a generic chain
        assert!(
            chains.is_empty(),
            "generic migration requires at least 3 files, got {:?}",
            chains
        );
    }

    #[test]
    fn test_empty_file_list() {
        let index = make_index(vec![]);
        let chains = detect_migrations(&index);
        assert!(chains.is_empty());
    }
}
