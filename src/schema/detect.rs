// ORM pattern matchers, Terraform tagging, migration detection

use crate::index::CodebaseIndex;
use crate::parser::language::SymbolKind;
use crate::schema::{
    MigrationChain, MigrationEntry, MigrationFramework, OrmFieldSchema, OrmFramework,
    OrmModelSchema, SchemaIndex, TableSchema,
};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Compile-once regex statics — avoids recompiling the same patterns on every
// call and eliminates all runtime `.unwrap()` on pattern compilation.
// ---------------------------------------------------------------------------

static RE_DJANGO_TABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"db_table\s*=\s*["']([^"']+)["']"#).expect("RE_DJANGO_TABLE"));

static RE_DJANGO_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)\s*=\s*models\.(\w+)\(([^)]*)\)").expect("RE_DJANGO_FIELD"));

static RE_SQLALCHEMY_TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"__tablename__\s*=\s*["']([^"']+)["']"#).expect("RE_SQLALCHEMY_TABLE")
});

static RE_SQLALCHEMY_COL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)\s*=\s*Column\(([^)]*)\)").expect("RE_SQLALCHEMY_COL"));

// Captures schema-qualified foreign key references: ForeignKey("table.col"),
// ForeignKey("schema.table.col"), ForeignKey("db.schema.table.col"), etc.
// Group 1 captures everything before the final dot segment (may include dots
// for multi-part qualifications); group 2 captures the table name (penultimate
// segment). The caller splits group 1 on '.' and takes the last component.
static RE_SQLALCHEMY_FK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"ForeignKey\(["']([^"']+)\.([^"'.]+)["']"#).expect("RE_SQLALCHEMY_FK")
});

static RE_TYPEORM_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"@Entity\(["']([^"']+)["']\)"#).expect("RE_TYPEORM_ENTITY"));

static RE_TYPEORM_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@(\w+)\([^)]*\)\s+(\w+)\s*:\s*(\w+)").expect("RE_TYPEORM_FIELD"));

static RE_PRISMA_MAP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"@@map\(["']([^"']+)["']\)"#).expect("RE_PRISMA_MAP"));

static RE_RAILS_TS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{14})_(.+)\.rb$").expect("RE_RAILS_TS"));

static RE_ALEMBIC_REVISION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"revision\s*=\s*["']([^"']+)["']"#).expect("RE_ALEMBIC_REVISION")
});

static RE_ALEMBIC_FNAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-f0-9_]+)\.py$").expect("RE_ALEMBIC_FNAME"));

static RE_FLYWAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^V(\d+(?:\.\d+)?)__(.+)\.sql$").expect("RE_FLYWAY"));

static RE_DJANGO_MIGRATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})_(.+)\.py$").expect("RE_DJANGO_MIGRATION"));

static RE_KNEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{14})_(.+)\.(js|ts)$").expect("RE_KNEX"));

static RE_PRISMA_DIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+/prisma/migrations)/(\d{14})_(.+)$").expect("RE_PRISMA_DIR"));

static RE_DRIZZLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})_(.+)\.sql$").expect("RE_DRIZZLE"));

static RE_GENERIC_MIGRATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)_(.+)\.sql$").expect("RE_GENERIC_MIGRATION"));

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
    if let Some(cap) = RE_DJANGO_TABLE.captures(body) {
        return cap[1].to_string();
    }
    camel_to_snake(class_name)
}

fn extract_django_fields(body: &str) -> Vec<OrmFieldSchema> {
    let mut fields = Vec::new();
    // Match: name = models.FieldType(...)
    for cap in RE_DJANGO_FIELD.captures_iter(body) {
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
    if let Some(cap) = RE_SQLALCHEMY_TABLE.captures(body) {
        return cap[1].to_string();
    }
    class_name.to_lowercase()
}

fn extract_sqlalchemy_fields(body: &str) -> Vec<OrmFieldSchema> {
    let mut fields = Vec::new();
    // Match: name = Column(Type, ...)
    for cap in RE_SQLALCHEMY_COL.captures_iter(body) {
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
            // Extract table from ForeignKey("table.col") or ForeignKey("schema.table.col").
            // Capture group 1 holds everything before the final dot; group 2 holds
            // the column name. The table is the last segment of group 1.
            RE_SQLALCHEMY_FK
                .captures(&args)
                .map(|c| c[1].split('.').next_back().unwrap_or(&c[1]).to_string())
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
    if let Some(cap) = RE_TYPEORM_ENTITY.captures(file_content) {
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
    for cap in RE_TYPEORM_FIELD.captures_iter(body) {
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

/// Convert a CamelCase (or PascalCase) class name to snake_case.
///
/// Follows Django's default algorithm: only inserts an underscore when an
/// uppercase character immediately follows a lowercase character.  Consecutive
/// uppercase letters are NOT split (matching Django's `str.lower()` fallback
/// for acronyms).
///
/// Examples:
/// - `UserProfile` → `user_profile`
/// - `HTTPServer`  → `httpserver`
/// - `APIKey`      → `apikey`
/// - `User`        → `user`
/// - (empty)       → `""`
pub fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push('_');
        }
        for lc in c.to_lowercase() {
            out.push(lc);
        }
        prev_lower = c.is_lowercase();
    }
    out
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
    if !matches!(language, Some(l) if l.eq_ignore_ascii_case("prisma")) {
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
    if let Some(cap) = RE_PRISMA_MAP.captures(body) {
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
    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_RAILS_TS.captures(fname)?;
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

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            if !fname.ends_with(".py") {
                return None;
            }
            // Try to read revision from content
            let sequence = if let Some(cap) = RE_ALEMBIC_REVISION.captures(&f.content) {
                cap[1].to_string()
            } else if let Some(cap) = RE_ALEMBIC_FNAME.captures(fname) {
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
    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_FLYWAY.captures(fname)?;
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

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_DJANGO_MIGRATION.captures(fname)?;
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

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_KNEX.captures(fname)?;
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
    let cap = RE_PRISMA_DIR.captures(dir)?;

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
        if let Some(other_cap) = RE_PRISMA_DIR.captures(other_dir) {
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
            RE_PRISMA_DIR
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

    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_DRIZZLE.captures(fname)?;
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
    let mut entries: Vec<MigrationEntry> = files
        .iter()
        .filter_map(|f| {
            let fname = filename(&f.relative_path);
            let cap = RE_GENERIC_MIGRATION.captures(fname)?;
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
            mtime_secs: None,
        }
    }

    // Helper: build a CodebaseIndex from a list of IndexedFile (no disk access)
    fn make_index(files: Vec<IndexedFile>) -> CodebaseIndex {
        use std::collections::{HashMap, HashSet};
        let graph = crate::index::graph::build_dependency_graph(&files, None);
        CodebaseIndex {
            total_files: files.len(),
            total_bytes: files.iter().map(|f| f.size_bytes).sum(),
            total_tokens: 0,
            language_stats: HashMap::new(),
            term_frequencies: HashMap::new(),
            domains: HashSet::new(),
            schema: None,
            graph,
            pagerank: HashMap::new(),
            test_map: HashMap::new(),
            call_graph: crate::intelligence::call_graph::CallGraph::default(),
            conventions: crate::conventions::ConventionProfile::default(),
            co_changes: Vec::new(),
            cross_lang_edges: Vec::new(),
            files,
            #[cfg(feature = "embeddings")]
            embedding_index: None,
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

    // -------------------------------------------------------------------------
    // Additional ORM detection tests for field extraction branches
    // -------------------------------------------------------------------------

    #[test]
    fn test_django_foreign_key_field_relation() {
        let sym = make_symbol(
            "Comment",
            SymbolKind::Class,
            "class Comment(models.Model)",
            r#"
    body = models.TextField()
    author = models.ForeignKey(User, on_delete=models.CASCADE)
"#,
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        // Two fields: body (TextField), author (ForeignKey -> User)
        let author = models[0]
            .fields
            .iter()
            .find(|f| f.name == "author")
            .expect("author field");
        assert!(author.is_relation, "author should be a relation");
        assert_eq!(author.related_model.as_deref(), Some("User"));

        let body = models[0]
            .fields
            .iter()
            .find(|f| f.name == "body")
            .expect("body field");
        assert!(!body.is_relation);
        assert!(body.related_model.is_none());
    }

    #[test]
    fn test_django_many_to_many_relation() {
        let sym = make_symbol(
            "Article",
            SymbolKind::Class,
            "class Article(models.Model)",
            r#"
    tags = models.ManyToManyField(Tag)
"#,
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        let tags = &models[0].fields[0];
        assert!(tags.is_relation);
        assert_eq!(tags.related_model.as_deref(), Some("Tag"));
    }

    #[test]
    fn test_django_meta_attributes_skipped() {
        // db_table, ordering, verbose_name should NOT be extracted as fields
        let sym = make_symbol(
            "Thing",
            SymbolKind::Class,
            "class Thing(models.Model)",
            r#"
    name = models.CharField(max_length=100)
    db_table = models.CharField(max_length=10)
    ordering = models.IntegerField()
    verbose_name = models.CharField(max_length=10)
"#,
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        // Only name should remain
        assert_eq!(models[0].fields.len(), 1);
        assert_eq!(models[0].fields[0].name, "name");
    }

    #[test]
    fn test_sqlalchemy_field_with_foreign_key() {
        let sym = make_symbol(
            "Order",
            SymbolKind::Class,
            "class Order(Base)",
            r#"
    __tablename__ = "orders"
    id = Column(Integer, primary_key=True)
    user_id = Column(Integer, ForeignKey("users.id"))
"#,
        );
        let imports = vec![Import {
            source: "sqlalchemy".to_string(),
            names: vec!["Column".to_string()],
        }];
        let file = make_file("app/models.py", Some("python"), "", vec![sym], imports);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        let user_id = models[0]
            .fields
            .iter()
            .find(|f| f.name == "user_id")
            .expect("user_id field");
        assert!(user_id.is_relation);
        assert_eq!(user_id.related_model.as_deref(), Some("users"));
    }

    #[test]
    fn test_sqlalchemy_declarative_base_signature() {
        // Test alternative signature: (DeclarativeBase)
        let sym = make_symbol(
            "Item",
            SymbolKind::Class,
            "class Item(DeclarativeBase)",
            r#"
    __tablename__ = "items"
    id = Column(Integer)
"#,
        );
        let imports = vec![Import {
            source: "sqlalchemy.orm".to_string(),
            names: vec!["DeclarativeBase".to_string()],
        }];
        let file = make_file("app/models.py", Some("python"), "", vec![sym], imports);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].table_name, "items");
    }

    #[test]
    fn test_typeorm_relation_decorators_extracted() {
        // Use simple decorator args (no arrow functions / nested parens) so the
        // single-pass regex `@(\w+)\([^)]*\)\s+(\w+)\s*:\s*(\w+)` matches.
        let sym = make_symbol(
            "Post",
            SymbolKind::Class,
            "class Post",
            r#"
    @PrimaryColumn() id: number
    @Column() title: string
    @ManyToOne(User) author: User
    @OneToMany(Comment) comments: string
"#,
        );
        let content = "import { Entity } from 'typeorm'; @Entity() class Post {";
        let file = make_file(
            "src/post.entity.ts",
            Some("typescript"),
            content,
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        let author = models[0]
            .fields
            .iter()
            .find(|f| f.name == "author")
            .expect("author field");
        assert!(author.is_relation);
        let comments = models[0]
            .fields
            .iter()
            .find(|f| f.name == "comments")
            .expect("comments field");
        assert!(comments.is_relation);
        let title = models[0]
            .fields
            .iter()
            .find(|f| f.name == "title")
            .expect("title field");
        assert!(!title.is_relation);
    }

    #[test]
    fn test_active_record_pluralize_word_ending_in_s() {
        let sym = make_symbol(
            "Status",
            SymbolKind::Class,
            "class Status < ApplicationRecord",
            "end\n",
        );
        let file = make_file("app/models/status.rb", Some("ruby"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].table_name, "statuses");
    }

    #[test]
    fn test_prisma_db_map_override() {
        let sym = make_symbol(
            "Customer",
            SymbolKind::Struct,
            "model Customer",
            r#"
    id   Int    @id
    name String
    @@map("customers_table")
"#,
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
        assert_eq!(models[0].table_name, "customers_table");
    }

    #[test]
    fn test_prisma_struct_in_non_prisma_lang_not_detected() {
        // Struct in a non-prisma file should NOT be detected as a Prisma model
        let sym = make_symbol("Foo", SymbolKind::Struct, "struct Foo", "    bar: i32\n");
        let file = make_file("src/foo.rs", Some("rust"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert!(models.is_empty());
    }

    #[test]
    fn test_prisma_model_detected_case_insensitive_language() {
        // try_detect_prisma must accept "Prisma" (capital P) in addition to
        // "prisma" (lowercase). The scanner returns lowercase, but the
        // case-insensitive guard makes the code defensive against future changes.
        let sym = make_symbol(
            "User",
            SymbolKind::Struct,
            "model User",
            "  id   Int    @id\n  name String\n",
        );

        // Test uppercase "Prisma"
        let file_upper = make_file(
            "schema.prisma",
            Some("Prisma"),
            "",
            vec![sym.clone()],
            vec![],
        );
        let index_upper = make_index(vec![file_upper]);
        let models_upper = detect_orm_models(&index_upper);
        assert_eq!(
            models_upper.len(),
            1,
            "language='Prisma' (capital P) must detect models; got {:?}",
            models_upper
        );
        assert_eq!(models_upper[0].class_name, "User");

        // Test lowercase "prisma" (normal scanner output)
        let file_lower = make_file("schema.prisma", Some("prisma"), "", vec![sym], vec![]);
        let index_lower = make_index(vec![file_lower]);
        let models_lower = detect_orm_models(&index_lower);
        assert_eq!(
            models_lower.len(),
            1,
            "language='prisma' (lowercase) must detect models; got {:?}",
            models_lower
        );
    }

    #[test]
    fn test_typeorm_no_decorators_skipped() {
        // Class with no TypeORM decorators
        let sym = make_symbol(
            "Helper",
            SymbolKind::Class,
            "class Helper",
            "    foo: string\n",
        );
        let file = make_file("src/helper.ts", Some("typescript"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert!(models.is_empty());
    }

    // -------------------------------------------------------------------------
    // Knex migration tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_knex_migrations_js_detected() {
        let f1 = make_file(
            "migrations/20230101120000_create_users.js",
            Some("javascript"),
            "",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "migrations/20230102000000_add_email.js",
            Some("javascript"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Knex));
        assert_eq!(chains[0].migrations.len(), 2);
        assert_eq!(chains[0].migrations[0].name, "create_users");
    }

    #[test]
    fn test_knex_migrations_ts_detected() {
        let f1 = make_file(
            "migrations/20230101120000_create_orders.ts",
            Some("typescript"),
            "",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "migrations/20230101130000_add_index.ts",
            Some("typescript"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Knex));
    }

    // -------------------------------------------------------------------------
    // Drizzle migration tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_drizzle_migrations_detected() {
        let f1 = make_file("drizzle/0001_init.sql", Some("sql"), "", vec![], vec![]);
        let f2 = make_file(
            "drizzle/0002_add_users.sql",
            Some("sql"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Drizzle));
        assert_eq!(chains[0].migrations.len(), 2);
        assert_eq!(chains[0].migrations[0].sequence, "0001");
        assert_eq!(chains[0].migrations[1].sequence, "0002");
    }

    // -------------------------------------------------------------------------
    // Prisma migration tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_prisma_migrations_detected() {
        let f1 = make_file(
            "myapp/prisma/migrations/20230101120000_init/migration.sql",
            Some("sql"),
            "CREATE TABLE users();",
            vec![],
            vec![],
        );
        let f2 = make_file(
            "myapp/prisma/migrations/20230102120000_add_email/migration.sql",
            Some("sql"),
            "ALTER TABLE users ADD COLUMN email TEXT;",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1, f2]);
        let chains = detect_migrations(&index);
        // The Prisma branch should pick this up.
        let prisma_chains: Vec<_> = chains
            .iter()
            .filter(|c| matches!(c.framework, MigrationFramework::Prisma))
            .collect();
        assert_eq!(
            prisma_chains.len(),
            1,
            "expected exactly one prisma chain, got: {:?}",
            chains
        );
        assert_eq!(prisma_chains[0].migrations.len(), 2);
        assert_eq!(prisma_chains[0].directory, "myapp/prisma/migrations");
    }

    // -------------------------------------------------------------------------
    // Alembic filename fallback test (no revision in content)
    // -------------------------------------------------------------------------

    #[test]
    fn test_alembic_filename_fallback() {
        // No `revision = ...` in content; sequence is parsed from the filename.
        // Filename must match `^([a-f0-9_]+)\.py$` — only hex chars + underscore.
        let f1 = make_file(
            "alembic/versions/abcdef123456.py",
            Some("python"),
            "# no revision here\n",
            vec![],
            vec![],
        );
        let index = make_index(vec![f1]);
        let chains = detect_migrations(&index);
        assert_eq!(chains.len(), 1);
        assert!(matches!(chains[0].framework, MigrationFramework::Alembic));
        // The fname_re captures the entire stem when it matches.
        assert_eq!(chains[0].migrations[0].sequence, "abcdef123456");
    }

    // -------------------------------------------------------------------------
    // build_schema_index orchestrator tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_schema_index_returns_none_for_plain_repo() {
        let f = make_file("src/main.rs", Some("rust"), "fn main() {}", vec![], vec![]);
        let index = make_index(vec![f]);
        let schema = build_schema_index(&index);
        assert!(schema.is_none(), "plain repo should produce no schema");
    }

    #[test]
    fn test_build_schema_index_with_orm_models() {
        let sym = make_symbol(
            "User",
            SymbolKind::Class,
            "class User(models.Model)",
            "    name = models.CharField(max_length=100)\n",
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let schema = build_schema_index(&index).expect("schema should be built");
        assert!(
            schema.orm_models.contains_key("User"),
            "User model should be in schema"
        );
    }

    #[test]
    fn test_build_schema_index_with_terraform() {
        let sym = make_symbol(
            "aws_dynamodb_table.events",
            SymbolKind::Block,
            "resource aws_dynamodb_table events",
            "    hash_key = \"EventId\"\n",
        );
        let file = make_file("infra/main.tf", Some("hcl"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let schema = build_schema_index(&index).expect("schema should be built");
        assert!(schema.tables.contains_key("aws_dynamodb_table.events"));
    }

    #[test]
    fn test_build_schema_index_with_migrations() {
        let f = make_file(
            "db/migrate/20230101000000_create_users.rb",
            Some("ruby"),
            "",
            vec![],
            vec![],
        );
        let index = make_index(vec![f]);
        let schema = build_schema_index(&index).expect("schema should be built");
        assert_eq!(schema.migrations.len(), 1);
        assert!(matches!(
            schema.migrations[0].framework,
            MigrationFramework::Rails
        ));
    }

    #[test]
    fn test_build_schema_index_with_prisma_enrichment() {
        // A Prisma struct symbol triggers both detect_orm_models and the
        // enrichment pass via extract_prisma_schema.
        let sym = make_symbol(
            "Post",
            SymbolKind::Struct,
            "model Post",
            "id Int @id\ntitle String\n",
        );
        let file = make_file(
            "prisma/schema.prisma",
            Some("prisma"),
            "model Post {\n  id Int @id\n  title String\n}\n",
            vec![sym],
            vec![],
        );
        let index = make_index(vec![file]);
        let schema = build_schema_index(&index).expect("schema should be built");
        assert!(schema.orm_models.contains_key("Post"));
    }

    // -------------------------------------------------------------------------
    // camel_to_snake tests
    // -------------------------------------------------------------------------

    #[test]
    fn camel_to_snake_user_profile() {
        assert_eq!(camel_to_snake("UserProfile"), "user_profile");
    }

    #[test]
    fn camel_to_snake_single_word() {
        assert_eq!(camel_to_snake("User"), "user");
        assert_eq!(camel_to_snake("user"), "user");
    }

    #[test]
    fn camel_to_snake_http_server() {
        // Django default: consecutive uppercase letters are NOT split.
        assert_eq!(camel_to_snake("HTTPServer"), "httpserver");
    }

    #[test]
    fn camel_to_snake_api_key() {
        assert_eq!(camel_to_snake("APIKey"), "apikey");
    }

    #[test]
    fn camel_to_snake_empty() {
        assert_eq!(camel_to_snake(""), "");
    }

    #[test]
    fn camel_to_snake_order_item() {
        assert_eq!(camel_to_snake("OrderItem"), "order_item");
    }

    #[test]
    fn camel_to_snake_already_snake() {
        assert_eq!(camel_to_snake("order_item"), "order_item");
    }

    // -------------------------------------------------------------------------
    // LazyLock regex statics — verify they compile without panicking
    // -------------------------------------------------------------------------

    #[test]
    fn regex_statics_instantiate_without_panic() {
        // Force initialisation of every LazyLock static defined in this module.
        let _ = &*RE_DJANGO_TABLE;
        let _ = &*RE_DJANGO_FIELD;
        let _ = &*RE_SQLALCHEMY_TABLE;
        let _ = &*RE_SQLALCHEMY_COL;
        let _ = &*RE_SQLALCHEMY_FK;
        let _ = &*RE_TYPEORM_ENTITY;
        let _ = &*RE_TYPEORM_FIELD;
        let _ = &*RE_PRISMA_MAP;
        let _ = &*RE_RAILS_TS;
        let _ = &*RE_ALEMBIC_REVISION;
        let _ = &*RE_ALEMBIC_FNAME;
        let _ = &*RE_FLYWAY;
        let _ = &*RE_DJANGO_MIGRATION;
        let _ = &*RE_KNEX;
        let _ = &*RE_PRISMA_DIR;
        let _ = &*RE_DRIZZLE;
        let _ = &*RE_GENERIC_MIGRATION;
    }

    #[test]
    fn test_sqlalchemy_fk_schema_qualified_two_part() {
        // ForeignKey("users.id") → table "users"
        let caps = RE_SQLALCHEMY_FK
            .captures(r#"ForeignKey("users.id")"#)
            .unwrap();
        let table = caps[1]
            .split('.')
            .next_back()
            .unwrap_or(&caps[1])
            .to_string();
        assert_eq!(table, "users");
    }

    #[test]
    fn test_sqlalchemy_fk_schema_qualified_three_part() {
        // ForeignKey("public.users.id") → table "users"
        let caps = RE_SQLALCHEMY_FK
            .captures(r#"ForeignKey("public.users.id")"#)
            .unwrap();
        let table = caps[1]
            .split('.')
            .next_back()
            .unwrap_or(&caps[1])
            .to_string();
        assert_eq!(table, "users");
    }

    #[test]
    fn test_sqlalchemy_fk_schema_qualified_four_part() {
        // ForeignKey("db.public.users.id") → table "users"
        let caps = RE_SQLALCHEMY_FK
            .captures(r#"ForeignKey("db.public.users.id")"#)
            .unwrap();
        let table = caps[1]
            .split('.')
            .next_back()
            .unwrap_or(&caps[1])
            .to_string();
        assert_eq!(table, "users");
    }

    // Django table name uses camel_to_snake (not to_lowercase) when no db_table override.
    #[test]
    fn test_django_default_table_name_is_snake_case() {
        let sym = make_symbol(
            "UserProfile",
            SymbolKind::Class,
            "class UserProfile(models.Model)",
            "    name = models.CharField(max_length=100)\n",
        );
        let file = make_file("app/models.py", Some("python"), "", vec![sym], vec![]);
        let index = make_index(vec![file]);
        let models = detect_orm_models(&index);
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].table_name, "user_profile",
            "Django default table name should be camel_to_snake, not to_lowercase"
        );
    }
}
